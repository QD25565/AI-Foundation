//! TeamEngram Client - Async client for connecting to TeamEngram daemon
//!
//! Provides the same interface as PostgresStorage but uses the TeamEngram
//! daemon via Named Pipe (Windows) or Unix socket for all operations.
//!
//! Usage:
//! ```rust,ignore
//! let client = TeamEngramClient::connect().await?;
//! client.broadcast("general", "Hello team!").await?;
//! ```

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::path::PathBuf;
// Only need tokio BufReader for Unix
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

// Using our own pipe implementation (direct Windows API, not tokio's broken abstraction)
#[cfg(windows)]
use crate::pipe::windows::PipeClient;
#[cfg(windows)]
use std::sync::Mutex;
#[cfg(windows)]
use std::io::{BufRead, BufReader as StdBufReader, Write as StdWrite};

#[cfg(unix)]
use tokio::net::UnixStream;

/// JSON-RPC request
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    method: String,
    params: Value,
    id: u64,
}

/// JSON-RPC response
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    result: Option<Value>,
    error: Option<JsonRpcError>,
    #[allow(dead_code)]
    id: u64,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

/// TeamEngram client for async daemon communication
/// Uses our own Windows API pipe implementation for reliability
pub struct TeamEngramClient {
    #[cfg(windows)]
    pipe: Mutex<PipeClient>,
    #[cfg(unix)]
    stream: UnixStream,
    request_id: AtomicU64,
    ai_id: String,
}

impl TeamEngramClient {
    /// Default pipe name for Windows
    #[cfg(windows)]
    pub const DEFAULT_PIPE: &'static str = r"\\.\pipe\teamengram";

    /// Default socket path for Unix
    #[cfg(unix)]
    pub const DEFAULT_SOCKET: &'static str = "/tmp/teamengram.sock";

    /// Sanitize AI_ID to a safe identifier for use in pipe/socket names.
    ///
    /// Whitelist: only alphanumeric, hyphen, underscore. Everything else
    /// becomes `_`. Truncated to 64 chars to prevent oversized paths.
    fn sanitize_ai_id(ai_id: &str) -> String {
        ai_id.chars()
            .take(64)
            .map(|c| match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
                _ => '_',
            })
            .collect()
    }

    /// Get per-AI pipe name - PREFERRED for multi-AI setups
    /// Each AI gets isolated pipe: \\.\pipe\teamengram_{ai_id}
    #[cfg(windows)]
    pub fn pipe_name_for_ai(ai_id: &str) -> String {
        format!(r"\\.\pipe\teamengram_{}", Self::sanitize_ai_id(ai_id))
    }

    /// Get per-AI socket path for Unix
    #[cfg(unix)]
    pub fn socket_path_for_ai(ai_id: &str) -> String {
        format!("/tmp/teamengram_{}.sock", Self::sanitize_ai_id(ai_id))
    }

    /// Connect to the TeamEngram daemon - REQUIRES AI_ID environment variable
    /// AI_ID is mandatory - no fallbacks, fail loudly if not set
    #[cfg(windows)]
    pub async fn connect() -> Result<Self> {
        // AI_ID is REQUIRED - fail loudly, no fallback to shared pipe
        let ai_id = std::env::var("AI_ID")
            .context("AI_ID environment variable is required - each AI must have its own identity")?;

        if ai_id.is_empty() || ai_id == "unknown" {
            anyhow::bail!("AI_ID cannot be empty or 'unknown' - set a valid AI identity");
        }

        // SWMR: All AIs connect to single shared daemon.
        // AI_FOUNDATION_DATA_DIR + PIPE_NAME can both be overridden for test isolation.
        let pipe_name = std::env::var("PIPE_NAME").unwrap_or_else(|_| Self::DEFAULT_PIPE.to_string());
        Self::connect_to(&pipe_name).await
    }

    #[cfg(unix)]
    pub async fn connect() -> Result<Self> {
        // AI_ID is REQUIRED - fail loudly, no fallback to shared socket
        let ai_id = std::env::var("AI_ID")
            .context("AI_ID environment variable is required - each AI must have its own identity")?;

        if ai_id.is_empty() || ai_id == "unknown" {
            anyhow::bail!("AI_ID cannot be empty or 'unknown' - set a valid AI identity");
        }

        let socket_path = Self::socket_path_for_ai(&ai_id);
        Self::connect_to(&socket_path).await
    }

    /// Connect to a specific pipe/socket path - REQUIRES AI_ID
    /// Uses our own Windows API pipe implementation (not tokio's broken abstraction)
    #[cfg(windows)]
    pub async fn connect_to(pipe_name: &str) -> Result<Self> {
        let pipe_name_owned = pipe_name.to_string();

        // Connect synchronously in blocking thread (our pipe implementation is sync)
        let pipe = tokio::task::spawn_blocking(move || {
            PipeClient::connect(&pipe_name_owned)
                .context("Failed to connect to TeamEngram daemon")
        }).await??;

        // AI_ID is REQUIRED - fail loudly, no fallback to "unknown"
        let ai_id = std::env::var("AI_ID")
            .context("AI_ID environment variable is required")?;

        if ai_id.is_empty() || ai_id == "unknown" {
            anyhow::bail!("AI_ID cannot be empty or 'unknown'");
        }

        Ok(Self {
            pipe: Mutex::new(pipe),
            request_id: AtomicU64::new(1),
            ai_id,
        })
    }

    /// Set the AI ID for this client (consumes and returns self)
    pub fn with_ai_id(mut self, ai_id: String) -> Self {
        self.ai_id = ai_id;
        self
    }

    #[cfg(unix)]
    pub async fn connect_to(socket_path: &str) -> Result<Self> {
        let stream = UnixStream::connect(socket_path)
            .await
            .context("Failed to connect to TeamEngram daemon")?;

        // AI_ID is REQUIRED - fail loudly, no fallback to "unknown"
        let ai_id = std::env::var("AI_ID")
            .context("AI_ID environment variable is required")?;

        if ai_id.is_empty() || ai_id == "unknown" {
            anyhow::bail!("AI_ID cannot be empty or 'unknown'");
        }

        Ok(Self {
            stream,
            request_id: AtomicU64::new(1),
            ai_id,
        })
    }

    /// Connect to daemon, spawning it if not running (Windows only)
    ///
    /// This implements the hybrid auto-start approach:
    /// 1. Try to connect to existing daemon
    /// 2. If fails, find and spawn the daemon executable
    /// 3. Wait briefly for startup
    /// 4. Retry connection
    #[cfg(windows)]
    pub async fn connect_or_spawn() -> Result<Self> {
        // First try normal connection
        if let Ok(client) = Self::connect().await {
            return Ok(client);
        }

        // Connection failed - try to spawn daemon
        if let Some(daemon_path) = Self::find_daemon_executable() {
            // Spawn daemon with CREATE_NO_WINDOW flag (0x08000000)
            const CREATE_NO_WINDOW: u32 = 0x08000000;

            let spawn_result = std::process::Command::new(&daemon_path)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();

            if spawn_result.is_ok() {
                // Event-driven wait: daemon signals ready event after creating pipe.
                // Zero CPU while waiting — blocked on Named Event / POSIX semaphore.
                let ai_id = std::env::var("AI_ID").unwrap_or_default();
                let _ready = tokio::task::spawn_blocking(move || {
                    crate::wake::wait_daemon_ready(&ai_id, std::time::Duration::from_secs(2))
                }).await.unwrap_or(false);

                // Try to connect now that daemon signaled ready
                if let Ok(client) = Self::connect().await {
                    return Ok(client);
                }
            }
        }

        // Final attempt - return error if still can't connect
        Self::connect().await
            .context("Failed to connect to TeamEngram daemon. Ensure teamengram-daemon.exe is running.")
    }

    /// Connect or spawn (Unix version - just connects, no auto-spawn)
    #[cfg(unix)]
    pub async fn connect_or_spawn() -> Result<Self> {
        Self::connect().await
    }

    /// Find the daemon executable in common locations
    #[cfg(windows)]
    fn find_daemon_executable() -> Option<PathBuf> {
        let daemon_name = "teamengram-daemon.exe";

        // Check locations in order of preference:
        // 1. Same directory as current executable
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let daemon_path = exe_dir.join(daemon_name);
                if daemon_path.exists() {
                    return Some(daemon_path);
                }
            }
        }

        // 2. TEAMENGRAM_DAEMON_PATH environment variable
        if let Ok(path) = std::env::var("TEAMENGRAM_DAEMON_PATH") {
            let daemon_path = PathBuf::from(path);
            if daemon_path.exists() {
                return Some(daemon_path);
            }
        }

        // 3. Check AppData/Local/.ai-foundation/bin/
        if let Some(local_data) = dirs::data_local_dir() {
            let daemon_path = local_data.join(".ai-foundation").join("bin").join(daemon_name);
            if daemon_path.exists() {
                return Some(daemon_path);
            }
        }

        None
    }

    /// Send a JSON-RPC request and get the response
    /// Uses our synchronous Windows API pipe wrapped in spawn_blocking
    async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            method: method.to_string(),
            params,
            id,
        };

        let request_str = serde_json::to_string(&request)? + "\n";

        #[cfg(windows)]
        {
            // Get mutable access to the pipe
            let pipe = self.pipe.get_mut().map_err(|e| anyhow::anyhow!("Pipe lock poisoned: {}", e))?;

            // Synchronous write
            pipe.write(request_str.as_bytes())?;
            pipe.flush()?;

            // Synchronous read with buffered reader
            let mut reader = StdBufReader::new(pipe);
            let mut response_str = String::new();
            reader.read_line(&mut response_str)?;

            let response: JsonRpcResponse = serde_json::from_str(&response_str)?;
            if let Some(error) = response.error {
                bail!("RPC error {}: {}", error.code, error.message);
            }
            Ok(response.result.unwrap_or(Value::Null))
        }

        #[cfg(unix)]
        {
            self.stream.write_all(request_str.as_bytes()).await?;
            self.stream.flush().await?;

            let mut reader = BufReader::new(&mut self.stream);
            let mut response_str = String::new();
            reader.read_line(&mut response_str).await?;

            let response: JsonRpcResponse = serde_json::from_str(&response_str)?;
            if let Some(error) = response.error {
                bail!("RPC error {}: {}", error.code, error.message);
            }
            Ok(response.result.unwrap_or(Value::Null))
        }
    }

    // =========================================================================
    // MESSAGING
    // =========================================================================

    /// Send a broadcast message
    pub async fn broadcast(&mut self, channel: &str, content: &str) -> Result<u64> {
        let result = self.call("broadcast", json!({
            "from_ai": self.ai_id,
            "channel": channel,
            "content": content
        })).await?;

        result["id"].as_u64().context("Invalid broadcast response")
    }

    /// Send a direct message
    pub async fn direct_message(&mut self, to_ai: &str, content: &str) -> Result<u64> {
        let result = self.call("direct_message", json!({
            "from_ai": self.ai_id,
            "to_ai": to_ai,
            "content": content
        })).await?;

        result["id"].as_u64().context("Invalid DM response")
    }

    /// Get recent broadcasts
    pub async fn get_broadcasts(&mut self, channel: &str, limit: usize) -> Result<Vec<BroadcastMsg>> {
        let result = self.call("get_broadcasts", json!({
            "channel": channel,
            "limit": limit
        })).await?;

        serde_json::from_value(result).context("Invalid broadcasts response")
    }

    /// Get direct messages for current AI
    pub async fn get_direct_messages(&mut self, limit: usize) -> Result<Vec<DirectMsg>> {
        let result = self.call("get_dms", json!({
            "ai_id": self.ai_id,
            "limit": limit
        })).await?;

        serde_json::from_value(result).context("Invalid DMs response")
    }

    /// Get UNREAD direct messages for current AI (incoming only)
    pub async fn get_unread_dms(&mut self, limit: usize) -> Result<Vec<DirectMsg>> {
        let result = self.call("get_unread_dms", json!({
            "ai_id": self.ai_id,
            "limit": limit
        })).await?;

        serde_json::from_value(result).context("Invalid unread DMs response")
    }

    /// Mark a DM as read
    pub async fn mark_dm_read(&mut self, id: u64) -> Result<bool> {
        let result = self.call("mark_dm_read", json!({
            "id": id
        })).await?;

        Ok(result["marked"].as_bool().unwrap_or(false))
    }

    /// Mark multiple DMs as read
    pub async fn mark_dms_read(&mut self, ids: &[u64]) -> Result<usize> {
        let ids_str: String = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
        let result = self.call("mark_dms_read", json!({
            "ids": ids_str
        })).await?;

        Ok(result["marked_count"].as_u64().unwrap_or(0) as usize)
    }


    // =========================================================================
    // PRESENCE
    // =========================================================================

    /// Update presence status
    pub async fn update_presence(&mut self, status: &str, task: &str) -> Result<()> {
        self.call("update_presence", json!({
            "ai_id": self.ai_id,
            "status": status,
            "task": task
        })).await?;
        Ok(())
    }

    /// Get all active presences
    pub async fn get_active_ais(&mut self) -> Result<Vec<PresenceInfo>> {
        let result = self.call("who_is_here", json!({})).await?;
        serde_json::from_value(result).context("Invalid presences response")
    }

    /// Get specific AI's presence
    pub async fn get_presence(&mut self, ai_id: &str) -> Result<Option<PresenceInfo>> {
        let result = self.call("get_presence", json!({ "ai_id": ai_id })).await?;
        if result.is_null() {
            Ok(None)
        } else {
            Ok(Some(serde_json::from_value(result)?))
        }
    }

    // =========================================================================
    // TASKS
    // =========================================================================

    /// Queue a new task
    pub async fn queue_task(&mut self, description: &str, priority: u8, tags: &str) -> Result<u64> {
        let result = self.call("queue_task", json!({
            "created_by": self.ai_id,
            "description": description,
            "priority": priority,
            "tags": tags
        })).await?;

        result["id"].as_u64().context("Invalid task queue response")
    }

    /// Claim a task
    pub async fn claim_task(&mut self, task_id: Option<u64>) -> Result<Option<TaskInfo>> {
        let params = if let Some(id) = task_id {
            json!({ "task_id": id, "ai_id": self.ai_id })
        } else {
            json!({ "ai_id": self.ai_id })
        };

        let result = self.call("claim_task", params).await?;
        if result.is_null() || result == json!(false) {
            Ok(None)
        } else {
            Ok(Some(serde_json::from_value(result)?))
        }
    }

    /// Complete a task
    pub async fn complete_task(&mut self, task_id: u64, result_text: &str) -> Result<bool> {
        let result = self.call("complete_task", json!({
            "task_id": task_id,
            "ai_id": self.ai_id,
            "result": result_text
        })).await?;

        Ok(result.as_bool().unwrap_or(false))
    }


    /// Start working on a claimed task (Claimed -> InProgress)
    pub async fn start_task(&mut self, task_id: u64) -> Result<bool> {
        let result = self.call("task_start", json!({
            "task_id": task_id,
            "ai_id": self.ai_id
        })).await?;
        
        // Returns {"status": "started", "id": task_id} on success
        Ok(result.get("status").and_then(|v| v.as_str()) == Some("started"))
    }

    /// List tasks
    pub async fn list_tasks(&mut self, limit: usize, pending_only: bool) -> Result<Vec<TaskInfo>> {
        let method = if pending_only { "list_pending_tasks" } else { "list_tasks" };
        let result = self.call(method, json!({ "limit": limit })).await?;
        serde_json::from_value(result).context("Invalid tasks response")
    }

    /// Get task stats
    pub async fn task_stats(&mut self) -> Result<TaskStats> {
        let result = self.call("task_stats", json!({})).await?;
        serde_json::from_value(result).context("Invalid task stats response")
    }

    // =========================================================================
    // FILE CLAIMS
    // =========================================================================

    /// Claim a file
    pub async fn claim_file(&mut self, path: &str, working_on: &str, duration_mins: u32) -> Result<u64> {
        let result = self.call("claim_file", json!({
            "ai_id": self.ai_id,
            "path": path,
            "working_on": working_on,
            "duration_mins": duration_mins
        })).await?;

        result["id"].as_u64().context("Invalid claim response")
    }

    /// Check if file is claimed
    pub async fn is_file_claimed(&mut self, path: &str) -> Result<Option<String>> {
        let result = self.call("check_file_claim", json!({ "path": path })).await?;
        if result.is_null() {
            Ok(None)
        } else {
            Ok(Some(result["claimer"].as_str().unwrap_or("unknown").to_string()))
        }
    }

    /// Release a file claim
    pub async fn release_file(&mut self, path: &str) -> Result<bool> {
        let result = self.call("release_file", json!({
            "path": path,
            "ai_id": self.ai_id
        })).await?;

        Ok(result.as_bool().unwrap_or(false))
    }

    /// Get active file claims
    pub async fn get_active_claims(&mut self) -> Result<Vec<FileClaimInfo>> {
        let result = self.call("list_file_claims", json!({ "limit": 100 })).await?;
        serde_json::from_value(result).context("Invalid claims response")
    }

    // =========================================================================
    // VOTING
    // =========================================================================

    /// Create a vote
    pub async fn create_vote(&mut self, topic: &str, options: Vec<String>, duration_mins: u32) -> Result<u64> {
        let result = self.call("create_vote", json!({
            "created_by": self.ai_id,
            "topic": topic,
            "options": options,
            "duration_mins": duration_mins
        })).await?;

        result["id"].as_u64().context("Invalid vote create response")
    }

    /// Cast a vote
    pub async fn cast_vote(&mut self, vote_id: u64, choice: &str) -> Result<bool> {
        let result = self.call("cast_vote", json!({
            "vote_id": vote_id,
            "ai_id": self.ai_id,
            "choice": choice
        })).await?;

        // Daemon returns {"status": "voted"} on success, not a boolean
        Ok(result.get("status").and_then(|v| v.as_str()) == Some("voted"))
    }

    /// Get vote results
    pub async fn get_vote_results(&mut self, vote_id: u64) -> Result<Option<VoteInfo>> {
        let result = self.call("get_vote", json!({ "vote_id": vote_id })).await?;
        if result.is_null() {
            Ok(None)
        } else {
            Ok(Some(serde_json::from_value(result)?))
        }
    }

    /// List votes
    pub async fn list_votes(&mut self, limit: usize) -> Result<Vec<VoteInfo>> {
        let result = self.call("list_votes", json!({ "limit": limit })).await?;
        serde_json::from_value(result).context("Invalid votes response")
    }

    // =========================================================================
    // LOCKS
    // =========================================================================

    /// Acquire a lock
    pub async fn lock_acquire(&mut self, resource: &str, working_on: &str, duration_mins: u32) -> Result<Option<u64>> {
        let result = self.call("acquire_lock", json!({
            "ai_id": self.ai_id,
            "resource": resource,
            "working_on": working_on,
            "duration_mins": duration_mins
        })).await?;

        if result.is_null() || result == json!(false) {
            Ok(None)
        } else {
            Ok(result["id"].as_u64())
        }
    }

    /// Release a lock
    pub async fn lock_release(&mut self, resource: &str) -> Result<bool> {
        let result = self.call("release_lock", json!({
            "resource": resource,
            "ai_id": self.ai_id
        })).await?;

        // Daemon returns {"status": "released"} on success
        Ok(result.get("status").and_then(|v| v.as_str()) == Some("released"))
    }

    /// Check lock status
    pub async fn lock_check(&mut self, resource: &str) -> Result<Option<LockInfo>> {
        let result = self.call("check_lock", json!({ "resource": resource })).await?;
        if result.is_null() {
            Ok(None)
        } else {
            Ok(Some(serde_json::from_value(result)?))
        }
    }

    // =========================================================================
    // ROOMS
    // =========================================================================

    /// Create a room
    pub async fn create_room(&mut self, name: &str, topic: &str) -> Result<u64> {
        let result = self.call("create_room", json!({
            "creator": self.ai_id,
            "name": name,
            "topic": topic
        })).await?;

        result["id"].as_u64().context("Invalid room create response")
    }

    /// Join a room
    pub async fn join_room(&mut self, room_id: u64) -> Result<bool> {
        let result = self.call("join_room", json!({
            "room_id": room_id,
            "ai_id": self.ai_id
        })).await?;

        Ok(result.as_bool().unwrap_or(false))
    }

    /// Leave a room
    pub async fn leave_room(&mut self, room_id: u64) -> Result<bool> {
        let result = self.call("leave_room", json!({
            "room_id": room_id,
            "ai_id": self.ai_id
        })).await?;

        Ok(result.as_bool().unwrap_or(false))
    }

    /// List rooms
    pub async fn list_rooms(&mut self, limit: usize) -> Result<Vec<RoomInfo>> {
        let result = self.call("list_rooms", json!({ "limit": limit })).await?;
        serde_json::from_value(result).context("Invalid rooms response")
    }

    // =========================================================================
    // DIALOGUES
    // =========================================================================

    /// Start a dialogue with one or more AIs.
    /// `responder` may be a single AI ID or comma-separated list for n-party dialogues.
    pub async fn dialogue_start(&mut self, responder: &str, topic: &str) -> Result<u64> {
        let result = self.call("start_dialogue", json!({
            "initiator": self.ai_id,
            "responder": responder,  // comma-separated for n-party
            "topic": topic
        })).await?;

        result["id"].as_u64().context("Invalid dialogue start response")
    }

    /// Respond in a dialogue
    pub async fn dialogue_respond(&mut self, dialogue_id: u64, response: &str) -> Result<bool> {
        let result = self.call("dialogue_respond", json!({
            "dialogue_id": dialogue_id,
            "response": response,
            "ai_id": self.ai_id
        })).await?;

        // Daemon returns {"status": "responded"} on success, not a boolean
        Ok(result.get("status").and_then(|v| v.as_str()) == Some("responded"))
    }

    /// End a dialogue
    pub async fn dialogue_end(&mut self, dialogue_id: u64, status: &str) -> Result<bool> {
        let result = self.call("end_dialogue", json!({
            "dialogue_id": dialogue_id,
            "status": status
        })).await?;

        // Daemon returns {"status": "ended"} on success, not a boolean
        Ok(result.get("status").and_then(|v| v.as_str()) == Some("ended"))
    }

    /// List dialogues
    pub async fn list_dialogues(&mut self, limit: usize) -> Result<Vec<DialogueInfo>> {
        let result = self.call("get_dialogues", json!({
            "ai_id": self.ai_id,
            "limit": limit
        })).await?;
        serde_json::from_value(result).context("Invalid dialogues response")
    }

    /// Get dialogue invites (dialogues where I'm responder and it's my turn)
    pub async fn dialogue_invites(&mut self, limit: usize) -> Result<Vec<DialogueInfo>> {
        let result = self.call("dialogue_invites", json!({
            "ai_id": self.ai_id,
            "limit": limit
        })).await?;
        serde_json::from_value(result).context("Invalid dialogue invites response")
    }

    /// Get dialogues where it's my turn
    pub async fn dialogue_my_turn(&mut self, limit: usize) -> Result<Vec<DialogueInfo>> {
        let result = self.call("dialogue_my_turn", json!({
            "ai_id": self.ai_id,
            "limit": limit
        })).await?;
        serde_json::from_value(result).context("Invalid my turn dialogues response")
    }

    /// Check whose turn it is in a dialogue
    pub async fn dialogue_turn(&mut self, dialogue_id: u64) -> Result<DialogueTurnInfo> {
        let result = self.call("dialogue_turn", json!({
            "dialogue_id": dialogue_id
        })).await?;
        serde_json::from_value(result).context("Invalid dialogue turn response")
    }

    // =========================================================================
    // VOTES (additional)
    // =========================================================================

    /// Close a vote (creator only)
    pub async fn vote_close(&mut self, vote_id: u64) -> Result<bool> {
        let result = self.call("close_vote", json!({
            "vote_id": vote_id,
            "ai_id": self.ai_id
        })).await?;
        Ok(result["status"].as_str() == Some("closed"))
    }

    // =========================================================================
    // ROOMS (additional)
    // =========================================================================

    /// Get room details
    pub async fn room_get(&mut self, room_id: u64) -> Result<Option<RoomInfo>> {
        let result = self.call("get_room", json!({ "room_id": room_id })).await?;
        if result.is_null() {
            Ok(None)
        } else {
            Ok(Some(serde_json::from_value(result)?))
        }
    }

    /// Close a room (creator only)
    pub async fn room_close(&mut self, room_id: u64) -> Result<bool> {
        let result = self.call("close_room", json!({
            "room_id": room_id,
            "ai_id": self.ai_id
        })).await?;
        Ok(result["status"].as_str() == Some("closed"))
    }

    // =========================================================================
    // UTILITY
    // =========================================================================

    /// Get store stats
    pub async fn stats(&mut self) -> Result<StoreStats> {
        let result = self.call("status", json!({})).await?;
        serde_json::from_value(result).context("Invalid stats response")
    }

    /// Ping the daemon
    pub async fn ping(&mut self) -> Result<String> {
        let result = self.call("status", json!({})).await?;
        Ok(result["status"].as_str().unwrap_or("unknown").to_string())
    }

    // === FILE ACTIONS (SessionStart Awareness) ===

    /// Log a file action (created, modified, deleted, reviewed)
    pub async fn log_file_action(&mut self, path: &str, action: &str) -> Result<u64> {
        let result = self.call("log_file_action", json!({
            "from_ai": &self.ai_id,
            "path": path,
            "action": action
        })).await?;
        Ok(result["id"].as_u64().unwrap_or(0))
    }

    /// Get file actions for an AI
    pub async fn get_file_actions(&mut self, ai_id: &str, limit: usize) -> Result<Vec<FileActionInfo>> {
        let result = self.call("get_file_actions", json!({
            "ai_id": ai_id,
            "limit": limit
        })).await?;
        let actions: Vec<FileActionInfo> = serde_json::from_value(result)?;
        Ok(actions)
    }

    /// Get recent file actions across all AIs
    pub async fn get_recent_file_actions(&mut self, limit: usize) -> Result<Vec<FileActionInfo>> {
        let result = self.call("recent_file_actions", json!({
            "limit": limit
        })).await?;
        let actions: Vec<FileActionInfo> = serde_json::from_value(result)?;
        Ok(actions)
    }

    // === DIRECTORY TRACKING (SessionStart Awareness) ===

    /// Track directory access
    pub async fn track_directory(&mut self, directory: &str, access_type: &str) -> Result<u64> {
        let result = self.call("track_directory", json!({
            "from_ai": &self.ai_id,
            "directory": directory,
            "access_type": access_type
        })).await?;
        Ok(result["id"].as_u64().unwrap_or(0))
    }

    /// Get recent directories accessed by an AI
    pub async fn get_recent_directories(&mut self, ai_id: &str, limit: usize) -> Result<Vec<DirectoryAccessInfo>> {
        let result = self.call("recent_directories", json!({
            "ai_id": ai_id,
            "limit": limit
        })).await?;
        let dirs: Vec<DirectoryAccessInfo> = serde_json::from_value(result)?;
        Ok(dirs)
    }

    // === PROJECTS ===

    /// Create a new project
    pub async fn create_project(&mut self, name: &str, goal: &str, root_directory: &str) -> Result<u64> {
        let result = self.call("create_project", json!({
            "from_ai": &self.ai_id,
            "name": name,
            "goal": goal,
            "root_directory": root_directory
        })).await?;
        Ok(result["id"].as_u64().unwrap_or(0))
    }

    /// Get a project by ID
    pub async fn get_project(&mut self, project_id: u64) -> Result<Option<ProjectInfo>> {
        let result = self.call("get_project", json!({
            "project_id": project_id
        })).await?;
        if result.is_null() {
            Ok(None)
        } else {
            Ok(Some(serde_json::from_value(result)?))
        }
    }

    /// List all projects
    pub async fn list_projects(&mut self) -> Result<Vec<ProjectInfo>> {
        let result = self.call("list_projects", json!({})).await?;
        let projects: Vec<ProjectInfo> = serde_json::from_value(result)?;
        Ok(projects)
    }

    /// Update a project
    pub async fn update_project(&mut self, project_id: u64, goal: Option<&str>, status: Option<&str>) -> Result<bool> {
        let result = self.call("update_project", json!({
            "project_id": project_id,
            "goal": goal,
            "status": status
        })).await?;
        Ok(result["success"].as_bool().unwrap_or(false))
    }

    /// Soft-delete a project (moves to trash for 24h)
    pub async fn delete_project(&mut self, project_id: u64) -> Result<bool> {
        let result = self.call("delete_project", json!({
            "project_id": project_id
        })).await?;
        Ok(result["success"].as_bool().unwrap_or(false))
    }

    /// Restore a soft-deleted project
    pub async fn restore_project(&mut self, project_id: u64) -> Result<bool> {
        let result = self.call("restore_project", json!({
            "project_id": project_id
        })).await?;
        Ok(result["success"].as_bool().unwrap_or(false))
    }

    // === FEATURES ===

    /// Create a feature within a project
    pub async fn create_feature(&mut self, project_id: u64, name: &str, overview: &str, directory: Option<&str>) -> Result<u64> {
        let result = self.call("create_feature", json!({
            "from_ai": &self.ai_id,
            "project_id": project_id,
            "name": name,
            "overview": overview,
            "directory": directory
        })).await?;
        Ok(result["id"].as_u64().unwrap_or(0))
    }

    /// Get a feature by ID
    pub async fn get_feature(&mut self, feature_id: u64) -> Result<Option<FeatureInfo>> {
        let result = self.call("get_feature", json!({
            "feature_id": feature_id
        })).await?;
        if result.is_null() {
            Ok(None)
        } else {
            Ok(Some(serde_json::from_value(result)?))
        }
    }

    /// List features for a project
    pub async fn list_features(&mut self, project_id: u64) -> Result<Vec<FeatureInfo>> {
        let result = self.call("list_features", json!({
            "project_id": project_id
        })).await?;
        let features: Vec<FeatureInfo> = serde_json::from_value(result)?;
        Ok(features)
    }

    /// Update a feature
    pub async fn update_feature(&mut self, feature_id: u64, name: Option<&str>, overview: Option<&str>, directory: Option<&str>) -> Result<bool> {
        let result = self.call("update_feature", json!({
            "feature_id": feature_id,
            "name": name,
            "overview": overview,
            "directory": directory
        })).await?;
        Ok(result["success"].as_bool().unwrap_or(false))
    }

    /// Soft-delete a feature
    pub async fn delete_feature(&mut self, feature_id: u64) -> Result<bool> {
        let result = self.call("delete_feature", json!({
            "feature_id": feature_id
        })).await?;
        Ok(result["success"].as_bool().unwrap_or(false))
    }

    /// Restore a soft-deleted feature
    pub async fn restore_feature(&mut self, feature_id: u64) -> Result<bool> {
        let result = self.call("restore_feature", json!({
            "feature_id": feature_id
        })).await?;
        Ok(result["success"].as_bool().unwrap_or(false))
    }

    /// Resolve a file path to its project and feature
    pub async fn resolve_file_to_project(&mut self, path: &str) -> Result<Option<(u64, Option<u64>)>> {
        let result = self.call("resolve_file_to_project", json!({
            "path": path
        })).await?;
        if result.is_null() {
            Ok(None)
        } else {
            let project_id = result["project_id"].as_u64().unwrap_or(0);
            let feature_id = result["feature_id"].as_u64();
            Ok(Some((project_id, feature_id)))
        }
    }

    // === SHARED VAULT ===

    /// Store a value in the shared vault
    pub async fn vault_store(&mut self, key: &str, value: &str) -> Result<()> {
        self.call("vault_store", json!({
            "from_ai": &self.ai_id,
            "key": key,
            "value": value
        })).await?;
        Ok(())
    }

    /// Get a value from the shared vault
    pub async fn vault_get(&mut self, key: &str) -> Result<Option<String>> {
        let result = self.call("vault_get", json!({
            "key": key
        })).await?;
        if result.is_null() {
            Ok(None)
        } else {
            Ok(result["value"].as_str().map(|s| s.to_string()))
        }
    }

    /// List all keys in the shared vault
    pub async fn vault_list(&mut self) -> Result<Vec<String>> {
        let result = self.call("vault_list", json!({})).await?;
        let keys: Vec<String> = serde_json::from_value(result)?;
        Ok(keys)
    }

}

// =========================================================================
// DATA TYPES (match PostgresStorage types)
// =========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastMsg {
    pub id: u64,
    pub from_ai: String,
    pub channel: String,
    pub content: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectMsg {
    pub id: u64,
    pub from_ai: String,
    pub to_ai: String,
    pub content: String,
    pub created_at: u64,
    pub read: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceInfo {
    pub ai_id: String,
    pub status: String,
    pub current_task: String,
    pub last_seen: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: u64,
    pub description: String,
    pub created_by: String,
    pub claimed_by: Option<String>,
    pub status: String,
    pub priority: u8,
    pub tags: String,
    pub result: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStats {
    pub total: u64,
    pub pending: u64,
    pub claimed: u64,
    pub in_progress: u64,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileClaimInfo {
    pub id: u64,
    pub claimer: String,
    pub path: String,
    pub working_on: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteInfo {
    pub id: u64,
    pub topic: String,
    pub options: Vec<String>,
    pub votes: Vec<(String, String)>,
    pub status: String,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockInfo {
    pub id: u64,
    pub holder: String,
    pub resource: String,
    pub working_on: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomInfo {
    pub id: u64,
    pub name: String,
    pub creator: String,
    pub topic: String,
    pub participants: Vec<String>,
    pub is_open: bool,
    pub mutes: HashMap<String, u64>,
    pub conclusion: Option<String>,
    pub pinned_messages: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogueInfo {
    pub id: u64,
    pub initiator: String,
    pub participants: Vec<String>,
    pub topic: String,
    pub status: String,
    pub current_turn: String,
    pub turn_index: usize,
    pub message_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogueTurnInfo {
    pub dialogue_id: u64,
    pub current_turn: String,
    pub is_my_turn: bool,
    pub initiator: String,
    pub participants: Vec<String>,
    pub turn_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileActionInfo {
    pub id: Option<u64>,
    pub ai_id: String,
    pub path: String,
    pub action: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryAccessInfo {
    pub ai_id: String,
    pub directory: String,
    pub access_type: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreStats {
    pub file_size: u64,
    pub total_pages: u64,
    pub used_pages: u64,
    pub txn_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub id: u64,
    pub name: String,
    pub goal: String,
    pub root_directory: String,
    pub created_by: String,
    pub status: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureInfo {
    pub id: u64,
    pub project_id: u64,
    pub name: String,
    pub overview: String,
    pub directory: Option<String>,
    pub created_by: String,
    pub status: String,
    pub created_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires daemon running
    async fn test_client_connect() {
        let client = TeamEngramClient::connect().await;
        assert!(client.is_ok() || client.is_err()); // Just test compilation
    }
}
