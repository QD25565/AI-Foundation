// Run without console window on Windows
#![cfg_attr(windows, windows_subsystem = "windows")]

//! TeamEngram Unified Daemon
//!
//! World-class replacement for PostgreSQL + Redis in AI-Foundation.
//!
//! Architecture:
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    TeamEngram Daemon                             │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  Named Pipe Server (JSON-RPC 2.0)                               │
//! │  - CLI tool requests                                            │
//! │  - Method routing                                               │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  TeamEngram Store (B+Tree)                                      │
//! │  - DMs, Broadcasts, Presence, Dialogues                         │
//! │  - Tasks, Votes, FileClaims, Rooms, Locks                       │
//! │  - Shadow paging for atomic commits                             │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  IPC Layer (Shared Memory)                                      │
//! │  - NotificationRing (Vyukov MPMC, ~200ns)                       │
//! │  - PresenceRegion (64 AI slots, ~100ns)                         │
//! │  - WakeRegion (instant wake triggers)                           │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Performance targets:
//! - Startup: <50ms
//! - Read latency: <1ms (vs ~10ms PostgreSQL)
//! - Write latency: <5ms (vs ~20ms PostgreSQL)
//! - Notification latency: <1μs (vs ~1ms Redis)
//! - Memory: <20MB (vs ~100MB PostgreSQL + Redis)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::{error, info, warn, debug};
use once_cell::sync::Lazy;
use std::sync::Mutex;

use teamengram::{
    TeamEngram, TaskPriority, RecordData, JoinRoomResult, VoteStatus,
    ShmNotifyCallback,
    wake::{WakeCoordinator, WakeReason, PresenceMutex, is_ai_online},
};

/// Persistent registry of wake coordinators - keeps event handles ALIVE
/// This is critical: Windows named events are destroyed when all handles close.
/// By keeping coordinators in this registry, the daemon maintains the event handles
/// and standby processes can successfully OpenEventW to get the same event.
static WAKE_REGISTRY: Lazy<Mutex<HashMap<String, WakeCoordinator>>> = Lazy::new(|| {
    Mutex::new(HashMap::new())
});

// BulletinBoard for hook awareness - event-driven updates
use teamengram::shm_rs::bulletin::BulletinBoard;

// Using our own pipe implementation (direct Windows API, not tokio's broken abstraction)
#[cfg(windows)]
use teamengram::pipe::windows::PipeServer;

// ============================================================================
// JSON-RPC 2.0 PROTOCOL
// ============================================================================

/// JSON-RPC 2.0 Request
#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
    id: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 Response
#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    id: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 Error
#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    fn success(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    fn error(id: Option<serde_json::Value>, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(JsonRpcError { code, message, data: None }),
            id,
        }
    }
}

// ============================================================================
// DAEMON STATISTICS
// ============================================================================

#[derive(Debug, Clone)]
struct DaemonStats {
    start_time: SystemTime,
    request_count: u64,
    last_request: Instant,
    dm_count: u64,
    broadcast_count: u64,
    task_count: u64,
}

impl DaemonStats {
    fn new() -> Self {
        Self {
            start_time: SystemTime::now(),
            request_count: 0,
            last_request: Instant::now(),
            dm_count: 0,
            broadcast_count: 0,
            task_count: 0,
        }
    }

    fn uptime_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(self.start_time)
            .unwrap_or_default()
            .as_secs()
    }
}

// ============================================================================
// BULLETIN BOARD UPDATES (Event-Driven)
// ============================================================================

/// Refresh BulletinBoard with latest DMs for an AI
/// Called immediately after DM insert - NOT polling
async fn refresh_bulletin_dms(store: &Arc<RwLock<TeamEngram>>, target_ai: &str) {
    if let Ok(mut bulletin) = BulletinBoard::open(None) {
        let mut s = store.write().await;
        if let Ok(dms) = s.get_dms(target_ai, 10) {
            // set_dms expects: (id, created_at_secs, from_ai, to_ai, content)
            let dm_data: Vec<(i64, i64, &str, &str, &str)> = dms.iter().filter_map(|r| {
                if let RecordData::DirectMessage(dm) = &r.data {
                    Some((r.id as i64, r.created_at as i64, dm.from_ai.as_str(), dm.to_ai.as_str(), dm.content.as_str()))
                } else { None }
            }).collect();
            bulletin.set_dms(&dm_data);
            let _ = bulletin.commit();
        }
    }
}

/// Signal wake event for an AI (instant, ~1μs)
/// Called after DM/task/mention to wake standby AI
///
/// CRITICAL: Uses persistent WAKE_REGISTRY to keep event handles alive.
/// Without this, Windows named events are destroyed when WakeCoordinator drops,
/// and standby processes can't OpenEventW to find them.
fn signal_wake(target_ai: &str, reason: WakeReason, from_ai: &str, content: &str) {
    let mut registry = match WAKE_REGISTRY.lock() {
        Ok(r) => r,
        Err(e) => {
            error!("Wake registry lock poisoned: {}", e);
            return;
        }
    };

    // Get existing coordinator or create new one
    let coord = registry.entry(target_ai.to_string()).or_insert_with(|| {
        match WakeCoordinator::new(target_ai) {
            Ok(c) => {
                info!("Wake coordinator created for {} and stored in registry", target_ai);
                c
            }
            Err(e) => {
                error!("Failed to create wake coordinator for {}: {}", target_ai, e);
                // Return a dummy that will fail silently - not ideal but prevents panic
                WakeCoordinator::new("_invalid_").unwrap_or_else(|_| panic!("Cannot create any wake coordinator"))
            }
        }
    });

    coord.wake(reason, from_ai, content);
    debug!("Wake signal sent to {} (reason: {:?})", target_ai, reason);
}

/// Refresh BulletinBoard with latest broadcasts
/// Called immediately after broadcast insert - NOT polling
async fn refresh_bulletin_broadcasts(store: &Arc<RwLock<TeamEngram>>) {
    match BulletinBoard::open(None) {
        Ok(mut bulletin) => {
            let mut s = store.write().await;
            match s.get_broadcasts("general", 10) {
                Ok(msgs) => {
                    let bc_data: Vec<(i64, i64, &str, &str, &str)> = msgs.iter().filter_map(|r| {
                        if let RecordData::Broadcast(bc) = &r.data {
                            Some((r.id as i64, r.created_at as i64, bc.from_ai.as_str(), bc.channel.as_str(), bc.content.as_str()))
                        } else { None }
                    }).collect();
                    info!("Updating bulletin with {} broadcasts", bc_data.len());
                    bulletin.set_broadcasts(&bc_data);
                    if let Err(e) = bulletin.commit() {
                        error!("Bulletin commit failed: {}", e);
                    }
                }
                Err(e) => error!("get_broadcasts failed: {}", e),
            }
        }
        Err(e) => error!("BulletinBoard::open failed: {}", e),
    }
}

/// Refresh BulletinBoard with latest file actions
/// Called immediately after file action log - NOT polling
async fn refresh_bulletin_file_actions(store: &Arc<RwLock<TeamEngram>>) {
    if let Ok(mut bulletin) = BulletinBoard::open(None) {
        let mut s = store.write().await;
        if let Ok(actions) = s.get_recent_file_actions(10) {
            let fa_data: Vec<(&str, &str, &str, u64)> = actions.iter()
                .map(|(_, fa)| (fa.ai_id.as_str(), fa.action.as_str(), fa.path.as_str(), fa.timestamp))
                .collect();
            bulletin.set_file_actions(&fa_data);
            let _ = bulletin.commit();
        }
    }
}

/// Refresh BulletinBoard with latest votes
/// Called immediately after vote create/cast - NOT polling
async fn refresh_bulletin_votes(store: &Arc<RwLock<TeamEngram>>) {
    // DEBUG: File log since windows_subsystem hides stdout
    if let Some(base) = dirs::data_local_dir() {
        let log_path = base.join(".ai-foundation").join("daemon-debug.log");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
            use std::io::Write;
            let _ = writeln!(f, "[{}] refresh_bulletin_votes CALLED", chrono::Utc::now());
        }
    }
    
    info!("refresh_bulletin_votes CALLED");
    match BulletinBoard::open(None) {
        Ok(mut bulletin) => {
            info!("BulletinBoard opened OK");
            let mut s = store.write().await;
            match s.list_votes(10) {
                Ok(votes) => {
                    info!("Found {} votes", votes.len());
                    let vote_data: Vec<(i64, &str, u32, u32)> = votes.iter()
                        .filter(|(_, v)| v.status == VoteStatus::Open)
                        .map(|(id, v)| (*id as i64, v.topic.as_str(), v.votes.len() as u32, 0u32))
                        .collect();
                    info!("Writing {} votes to bulletin", vote_data.len());
                    bulletin.set_votes(&vote_data);
                    if let Err(e) = bulletin.commit() {
                        error!("Bulletin commit failed: {}", e);
                    } else {
                        info!("Bulletin commit SUCCESS");
                    }
                }
                Err(e) => error!("list_votes failed: {}", e),
            }
        }
        Err(e) => error!("BulletinBoard::open failed: {}", e),
    }
}

/// Refresh BulletinBoard with latest locks
/// Called immediately after lock acquire/release - NOT polling
async fn refresh_bulletin_locks(store: &Arc<RwLock<TeamEngram>>) {
    if let Ok(mut bulletin) = BulletinBoard::open(None) {
        let mut s = store.write().await;
        if let Ok(locks) = s.list_all_locks(20) {
            let lock_data: Vec<(&str, &str, &str)> = locks.iter()
                .map(|(_, lock)| (lock.resource.as_str(), lock.holder.as_str(), lock.working_on.as_str()))
                .collect();
            bulletin.set_locks(&lock_data);
            let _ = bulletin.commit();
        }
    }
}

/// Refresh BulletinBoard with latest presences
/// Called immediately after presence update - NOT polling
async fn refresh_bulletin_presences(store: &Arc<RwLock<TeamEngram>>) {
    if let Ok(mut bulletin) = BulletinBoard::open(None) {
        let mut s = store.write().await;
        if let Ok(presences) = s.get_all_presences() {
            let pr_data: Vec<(&str, &str, &str, u64)> = presences.iter()
                .map(|p| (p.ai_id.as_str(), p.status.as_str(), p.current_task.as_str(), p.last_seen))
                .collect();
            bulletin.set_presences(&pr_data);
            let _ = bulletin.commit();
        }
    }
}

/// Refresh BulletinBoard with dialogues requiring action (invites + your turn)
/// Called immediately after dialogue create/respond/end - NOT polling
async fn refresh_bulletin_dialogues(store: &Arc<RwLock<TeamEngram>>, target_ai: &str) {
    info!("refresh_bulletin_dialogues called for {}", target_ai);

    match BulletinBoard::open(None) {
        Ok(mut bulletin) => {
            let mut s = store.write().await;
            // Get dialogues where it is this AIs turn to respond
            match s.get_my_turn_dialogues(target_ai, 5) {
                Ok(dialogues) => {
                    let dialogue_data: Vec<(i64, &str)> = dialogues.iter()
                        .map(|(id, d)| (*id as i64, d.topic.as_str()))
                        .collect();
                    bulletin.set_dialogues(&dialogue_data);
                    match bulletin.commit() {
                        Ok(_) => info!("Updated bulletin with {} dialogue(s) for {}", dialogue_data.len(), target_ai),
                        Err(e) => error!("Failed to commit bulletin: {}", e),
                    }
                }
                Err(e) => error!("Failed to get_my_turn_dialogues for {}: {}", target_ai, e),
            }
        }
        Err(e) => error!("Failed to open BulletinBoard: {}", e),
    }
}


// ============================================================================
// TEAMENGRAM DAEMON
// ============================================================================

/// Unified TeamEngram daemon - replaces PostgreSQL + Redis
pub struct TeamEngramDaemon {
    /// Named pipe name for CLI communication
    pipe_name: String,
    /// AI identity
    ai_id: String,
    /// SHARED TeamEngram store - collaboration data (DMs, broadcasts, presence, etc.)
    shared_store: Arc<RwLock<TeamEngram>>,
    /// PRIVATE TeamEngram store - per-AI data (vault only)
    private_store: Arc<RwLock<TeamEngram>>,
    /// IPC notification callback (used by store internally)
    #[allow(dead_code)]
    notify: Arc<ShmNotifyCallback>,
    /// Daemon statistics
    stats: Arc<RwLock<DaemonStats>>,
    /// Idle timeout in seconds (for future auto-shutdown)
    #[allow(dead_code)]
    idle_timeout_secs: u64,
    // NO heartbeat_interval - presence is EVENT-DRIVEN on each IPC request
}

impl TeamEngramDaemon {
    /// Create a new TeamEngram daemon
    /// 
    /// HYBRID STORE ARCHITECTURE:
    /// - shared_store: teamengram.engram - collaboration (DMs, broadcasts, rooms, tasks, etc.)
    /// - private_store: teamengram_{ai_id}.engram - private data (vault only)
    pub fn new(pipe_name: String, ai_id: String) -> Result<Self> {
        // === SHARED STORE (collaboration) ===
        let shared_path = TeamEngram::default_path();
        info!("Opening SHARED TeamEngram store at {:?}", shared_path);
        
        if let Some(parent) = shared_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        let shared_ipc_path = shared_path.with_extension("ipc");
        let notify = Arc::new(ShmNotifyCallback::open(&shared_ipc_path)
            .context("Failed to open shared IPC")?);
        
        let shared_store = TeamEngram::open_with_notify(&shared_path, notify.clone())
            .context("Failed to open shared TeamEngram store")?;
        
        // === PRIVATE STORE (per-AI vault) ===
        let private_path = if ai_id != "unknown" && !ai_id.is_empty() {
            TeamEngram::path_for_ai(&ai_id)
        } else {
            shared_path.with_file_name("teamengram_private.engram")
        };
        info!("Opening PRIVATE TeamEngram store at {:?} for AI: {}", private_path, ai_id);
        
        if let Some(parent) = private_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        // Private store uses its own IPC
        let private_ipc_path = private_path.with_extension("ipc");
        let private_notify = Arc::new(ShmNotifyCallback::open(&private_ipc_path)
            .context("Failed to open private IPC")?);
        
        let private_store = TeamEngram::open_with_notify(&private_path, private_notify)
            .context("Failed to open private TeamEngram store")?;

        Ok(Self {
            pipe_name,
            ai_id,
            shared_store: Arc::new(RwLock::new(shared_store)),
            private_store: Arc::new(RwLock::new(private_store)),
            notify,
            stats: Arc::new(RwLock::new(DaemonStats::new())),
            idle_timeout_secs: 1800, // 30 minutes
            // NO heartbeat polling - presence updated on each IPC request
        })
    }

    /// Run the daemon
    #[cfg(windows)]
    pub async fn run(&self) -> Result<()> {
        info!("TeamEngram Daemon starting");
        info!("AI ID: {}", self.ai_id);
        info!("Pipe: {}", self.pipe_name);
        info!("Store: {:?}", TeamEngram::default_path());

        // ACQUIRE PRESENCE MUTEX - OS-level, auto-releases on process death
        // This is the ONLY presence indicator - no polling, no TTL
        let _presence = PresenceMutex::acquire(&self.ai_id)
            .context("Failed to acquire presence mutex")?;
        info!("Presence mutex acquired: {} is ONLINE", self.ai_id);

        // Also register in store for status/task info (not for online detection)
        {
            let mut store = self.shared_store.write().await;
            store.update_presence(&self.ai_id, "active", "daemon started")?;
        }

        // Main pipe server loop - using our synchronous Windows API implementation
        // This gives us full control over the pipe, unlike tokio's broken abstraction
        info!("Entering main loop, pipe_name={}", self.pipe_name);

        loop {
            info!("Creating pipe...");
            let pipe_name = self.pipe_name.clone();

            // Create and wait for connection synchronously (in blocking thread)
            let pipe_result = tokio::task::spawn_blocking(move || {
                let server = PipeServer::create(&pipe_name)?;
                info!("Pipe created, waiting for client...");
                server.wait_for_connection()?;
                debug!("Client connected");
                Ok::<PipeServer, anyhow::Error>(server)
            }).await?;

            let pipe = match pipe_result {
                Ok(p) => p,
                Err(e) => {
                    error!("Pipe error: {}", e);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };

            let shared_store = Arc::clone(&self.shared_store);
            let private_store = Arc::clone(&self.private_store);
            let stats = Arc::clone(&self.stats);
            let ai_id = self.ai_id.clone();

            // Handle client in separate blocking thread (pipe I/O is synchronous)
            std::thread::spawn(move || {
                if let Err(e) = handle_client_sync(pipe, shared_store, private_store, stats, ai_id) {
                    error!("Client error: {}", e);
                }
            });
        }
    }

    #[cfg(not(windows))]
    pub async fn run(&self) -> Result<()> {
        bail!("Named pipes only supported on Windows. Use Unix sockets on other platforms.");
    }
}

/// Handle a single client connection (synchronous version using our Windows API pipes)
/// This runs in a std::thread with its own tokio runtime for async store operations
#[cfg(windows)]
fn handle_client_sync(
    pipe: PipeServer,
    shared_store: Arc<RwLock<TeamEngram>>,
    private_store: Arc<RwLock<TeamEngram>>,
    stats: Arc<RwLock<DaemonStats>>,
    ai_id: String,
) -> Result<()> {
    // Create a tokio runtime for this thread to call async route_method
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let mut buffer = vec![0u8; 65536];

    loop {
        // Synchronous read from our Windows API pipe
        let n = match pipe.read(&mut buffer) {
            Ok(n) => n,
            Err(e) => {
                // Check for broken pipe (client disconnected)
                if e.kind() == std::io::ErrorKind::BrokenPipe {
                    break;
                }
                return Err(e.into());
            }
        };

        if n == 0 {
            break; // Client disconnected
        }

        let request_str = String::from_utf8_lossy(&buffer[..n]);
        debug!("Received: {}", request_str);

        // Parse JSON-RPC request
        let response = match serde_json::from_str::<JsonRpcRequest>(&request_str) {
            Ok(request) => {
                // Update stats (blocking)
                {
                    let mut s = stats.blocking_write();
                    s.request_count += 1;
                    s.last_request = Instant::now();
                }

                // Route method (async, run in our local runtime)
                rt.block_on(route_method(&request, &shared_store, &private_store, &stats, &ai_id))
            }
            Err(e) => {
                JsonRpcResponse::error(None, -32700, format!("Parse error: {}", e))
            }
        };

        // Send response (MUST include newline - client uses read_line())
        let response_str = serde_json::to_string(&response)? + "\n";
        debug!("Sending: {}", response_str.trim());
        pipe.write(response_str.as_bytes())?;
        pipe.flush()?;
    }

    Ok(())
}

/// Route JSON-RPC method to handler
async fn route_method(
    request: &JsonRpcRequest,
    shared_store: &Arc<RwLock<TeamEngram>>,
    private_store: &Arc<RwLock<TeamEngram>>,
    stats: &Arc<RwLock<DaemonStats>>,
    ai_id: &str,
) -> JsonRpcResponse {
    let id = request.id.clone();
    let params = &request.params;

    // EVENT-DRIVEN PRESENCE: Update on every IPC request (no polling!)
    // This is the ONLY place presence gets updated - when AI actually does something
    let requesting_ai = params.get("ai_id").and_then(|v| v.as_str()).unwrap_or(ai_id);
    {
        let mut store = shared_store.write().await;
        let _ = store.update_presence(requesting_ai, "active", &request.method);
    }

    match request.method.as_str() {
        // === STATUS ===
        "status" | "ping" => {
            let s = stats.read().await;
            // Get store stats for client compatibility (file_size, total_pages, etc.)
            let store = shared_store.read().await;
            let store_stats = store.stats();
            drop(store);
            JsonRpcResponse::success(id, serde_json::json!({
                "status": "ok",
                "uptime_seconds": s.uptime_seconds(),
                "request_count": s.request_count,
                "ai_id": ai_id,
                "backend": "teamengram",
                // Required by client StoreStats struct
                "file_size": store_stats.file_size,
                "total_pages": store_stats.total_pages,
                "used_pages": store_stats.used_pages,
                "txn_id": store_stats.txn_id,
            }))
        }

        // === DIRECT MESSAGES ===
        "dm" | "send_dm" | "direct_message" => {
            // Accept both "to" and "to_ai" for compatibility
            // BUG FIX: Read sender from request params, not daemon ai_id
            let from = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let to = params.get("to").and_then(|v| v.as_str())
                .or_else(|| params.get("to_ai").and_then(|v| v.as_str()))
                .unwrap_or("");
            let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");

            if to.is_empty() || content.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'to'/'to_ai' or 'content'".into());
            }

            let mut s = shared_store.write().await;
            match s.insert_dm(from, to, content) {
                Ok(msg_id) => {
                    drop(s);
                    let mut st = stats.write().await;
                    st.dm_count += 1;
                    // Event-driven: Update BulletinBoard immediately
                    let store_clone = Arc::clone(&shared_store);
                    let to_owned = to.to_string();
                    tokio::spawn(async move {
                        refresh_bulletin_dms(&store_clone, &to_owned).await;
                    });
                    // Event-driven: Signal wake event for target AI (instant, ~1μs)
                    signal_wake(to, WakeReason::DirectMessage, from, content);
                    JsonRpcResponse::success(id, serde_json::json!({
                        "id": msg_id,
                        "status": "sent"
                    }))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "get_dms" | "read_dms" | "direct_messages" => {
            // BUG FIX: Use client's ai_id from params, not daemon's ai_id!
            // Daemon is shared by all AIs, so we must use the requesting AI's ID.
            let requesting_ai = params.get("ai_id").and_then(|v| v.as_str()).unwrap_or(ai_id);
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

            let mut s = shared_store.write().await;
            match s.get_dms(requesting_ai, limit) {
                Ok(dms) => {
                    let result: Vec<_> = dms.iter().filter_map(|r| {
                        if let RecordData::DirectMessage(dm) = &r.data {
                            Some(serde_json::json!({
                                "id": r.id,
                                "from_ai": dm.from_ai,
                                "to_ai": dm.to_ai,
                                "content": dm.content,
                                "created_at": r.created_at,
                                "read": false,
                            }))
                        } else { None }
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "get_unread_dms" | "unread_dms" => {
            // BUG FIX: Use client's ai_id from params, not daemon's ai_id!
            let requesting_ai = params.get("ai_id").and_then(|v| v.as_str()).unwrap_or(ai_id);
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

            let mut s = shared_store.write().await;
            match s.get_unread_dms(requesting_ai, limit) {
                Ok(dms) => {
                    let result: Vec<_> = dms.iter().filter_map(|r| {
                        if let RecordData::DirectMessage(dm) = &r.data {
                            Some(serde_json::json!({
                                "id": r.id,
                                "from_ai": dm.from_ai,
                                "to_ai": dm.to_ai,
                                "content": dm.content,
                                "created_at": r.created_at,
                                "read": dm.read,
                            }))
                        } else { None }
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "mark_dm_read" | "read_dm" => {
            let dm_id = params.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
            if dm_id == 0 {
                return JsonRpcResponse::error(id, -32602, "Missing 'id'".into());
            }

            let mut s = shared_store.write().await;
            match s.mark_dm_read(dm_id) {
                Ok(marked) => JsonRpcResponse::success(id, serde_json::json!({
                    "marked": marked,
                    "id": dm_id
                })),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "mark_dms_read" | "read_dms_batch" => {
            let ids_str = params.get("ids").and_then(|v| v.as_str()).unwrap_or("");
            let ids: Vec<u64> = ids_str.split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();

            let mut s = shared_store.write().await;
            match s.mark_dms_read(&ids) {
                Ok(count) => JsonRpcResponse::success(id, serde_json::json!({
                    "marked_count": count,
                    "total": ids.len()
                })),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }


        // === BROADCASTS ===
        "broadcast" | "send_broadcast" => {
            info!("BROADCAST HANDLER ENTERED");
            // BUG FIX: Read sender from request params
            let from = params.get("from_ai").and_then(|v| v.as_str()).unwrap_or(ai_id);
            let channel = params.get("channel").and_then(|v| v.as_str()).unwrap_or("general");
            let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");

            if content.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'content'".into());
            }

            let mut s = shared_store.write().await;
            match s.insert_broadcast(from, channel, content) {
                Ok(msg_id) => {
                    drop(s);
                    let mut st = stats.write().await;
                    st.broadcast_count += 1;
                    // Event-driven: Update BulletinBoard immediately
                    info!("Spawning bulletin broadcast update task");
                    let store_clone = Arc::clone(&shared_store);
                    tokio::spawn(async move {
                        info!("Inside bulletin spawn task");
                        refresh_bulletin_broadcasts(&store_clone).await;
                        info!("Bulletin spawn task complete");
                    });
                    // Event-driven: Signal wake for @mentions and urgent keywords
                    let content_lower = content.to_lowercase();
                    // Check for @mentions (e.g., @ai-2, @ai-1)
                    for word in content.split_whitespace() {
                        if word.starts_with('@') {
                            let mentioned = word.trim_start_matches('@').trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
                            if !mentioned.is_empty() {
                                signal_wake(mentioned, WakeReason::Mention, from, content);
                            }
                        }
                    }
                    // Check for urgent keywords
                    if content_lower.contains("urgent") || content_lower.contains("critical") || content_lower.contains("help") {
                        // Wake all known AIs for urgent messages
                        // For now, signal broadcast reason (AIs can check if relevant)
                        signal_wake("all", WakeReason::Urgent, from, content);
                    }
                    JsonRpcResponse::success(id, serde_json::json!({
                        "id": msg_id,
                        "status": "sent"
                    }))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "get_broadcasts" | "messages" => {
            let channel = params.get("channel").and_then(|v| v.as_str()).unwrap_or("general");
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

            let mut s = shared_store.write().await;
            match s.get_broadcasts(channel, limit) {
                Ok(msgs) => {
                    let result: Vec<_> = msgs.iter().filter_map(|r| {
                        if let RecordData::Broadcast(bc) = &r.data {
                            Some(serde_json::json!({
                                "id": r.id,
                                "from_ai": bc.from_ai,
                                "channel": bc.channel,
                                "content": bc.content,
                                "created_at": r.created_at,
                            }))
                        } else { None }
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // === PRESENCE ===
        "update_presence" | "presence" => {
            // BUG FIX: Use client's ai_id from params for shared daemon
            let requesting_ai = params.get("ai_id").and_then(|v| v.as_str()).unwrap_or(ai_id);
            let status = params.get("status").and_then(|v| v.as_str()).unwrap_or("active");
            let task = params.get("task").and_then(|v| v.as_str()).unwrap_or("");

            let mut s = shared_store.write().await;
            match s.update_presence(requesting_ai, status, task) {
                Ok(_) => {
                    drop(s);
                    // Event-driven: Update BulletinBoard immediately
                    let store_clone = Arc::clone(&shared_store);
                    tokio::spawn(async move {
                        refresh_bulletin_presences(&store_clone).await;
                    });
                    JsonRpcResponse::success(id, serde_json::json!({"status": "updated"}))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "who_is_here" | "team_status" => {
            use std::collections::HashMap;
            let mut s = shared_store.write().await;
            // Get stored presences for status/task info
            match s.get_all_presences() {
                Ok(presences) => {
                    // Use OS-level mutex detection for online status - NO POLLING, NO TTL
                    let mut unique: HashMap<String, &teamengram::store::Presence> = HashMap::new();
                    for p in &presences {
                        // Only include if OS mutex shows AI is online
                        if !is_ai_online(&p.ai_id) {
                            continue;
                        }
                        unique.entry(p.ai_id.clone())
                            .and_modify(|existing| {
                                if p.last_seen > existing.last_seen {
                                    *existing = p;
                                }
                            })
                            .or_insert(p);
                    }
                    let result: Vec<_> = unique.values().map(|p| {
                        serde_json::json!({
                            "ai_id": p.ai_id,
                            "status": p.status,
                            "current_task": p.current_task,
                            "last_seen": p.last_seen,
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // Get specific AI's presence (returns single object, not array)
        "get_presence" | "my_presence" => {
            let target_ai = params.get("ai_id").and_then(|v| v.as_str()).unwrap_or(ai_id);
            
            // First check if AI is online via OS mutex - NO POLLING, NO TTL
            if !is_ai_online(target_ai) {
                return JsonRpcResponse::success(id, serde_json::json!(null));
            }
            
            let mut s = shared_store.write().await;
            match s.get_all_presences() {
                Ok(presences) => {
                    // Find the specific AI's presence data
                    let found = presences.iter()
                        .filter(|p| p.ai_id == target_ai)
                        .max_by_key(|p| p.last_seen);

                    match found {
                        Some(p) => JsonRpcResponse::success(id, serde_json::json!({
                            "ai_id": p.ai_id,
                            "status": p.status,
                            "current_task": p.current_task,
                            "last_seen": p.last_seen,
                        })),
                        None => JsonRpcResponse::success(id, serde_json::Value::Null),
                    }
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // === TASKS ===
        "queue_task" | "create_task" => {
            // BUG FIX: Read creator from request params
            let creator = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let description = params.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let tags = params.get("tags").and_then(|v| v.as_str()).unwrap_or("");
            let priority = match params.get("priority").and_then(|v| v.as_str()).unwrap_or("normal") {
                "low" => TaskPriority::Low,
                "high" => TaskPriority::High,
                "urgent" => TaskPriority::Urgent,
                _ => TaskPriority::Normal,
            };

            if description.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'description'".into());
            }

            let mut s = shared_store.write().await;
            match s.queue_task(creator, description, priority, tags) {
                Ok(task_id) => {
                    drop(s);
                    let mut st = stats.write().await;
                    st.task_count += 1;
                    // Event-driven: Signal wake for urgent tasks
                    if matches!(priority, TaskPriority::Urgent) {
                        signal_wake("all", WakeReason::TaskAssigned, creator, description);
                    }
                    JsonRpcResponse::success(id, serde_json::json!({
                        "id": task_id,
                        "status": "queued"
                    }))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "claim_task" | "claim_task_by_id" => {
            // BUG FIX: Read claimer from request params
            let claimer = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let task_id = params.get("task_id").and_then(|v| v.as_u64()).unwrap_or(0);

            let mut s = shared_store.write().await;
            match s.claim_task(task_id, claimer) {
                Ok(true) => {
                    // Return full TaskInfo, not just status
                    match s.get_task(task_id) {
                        Ok(Some(task)) => JsonRpcResponse::success(id, serde_json::json!({
                            "id": task_id,
                            "description": task.description,
                            "status": format!("{:?}", task.status).to_lowercase(),
                            "priority": task.priority as u8,
                            "tags": task.tags,
                            "result": task.result,
                            "created_by": task.created_by,
                            "claimed_by": task.claimed_by,
                        })),
                        _ => JsonRpcResponse::success(id, serde_json::json!({"status": "claimed", "id": task_id})),
                    }
                }
                Ok(false) => JsonRpcResponse::error(id, -32000, "Task not available".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "start_task" | "task_start" => {
            // Start working on a claimed task (sets status to InProgress)
            let actor = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let task_id = params.get("task_id").and_then(|v| v.as_u64())
                .or_else(|| params.get("id").and_then(|v| v.as_u64()))
                .unwrap_or(0);

            let mut s = shared_store.write().await;
            match s.start_task(task_id, actor) {
                Ok(true) => JsonRpcResponse::success(id, serde_json::json!({"status": "started", "id": task_id})),
                Ok(false) => JsonRpcResponse::error(id, -32000, "Cannot start task (not claimed by you?)".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "complete_task" => {
            // BUG FIX: Read completer from request params
            let completer = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let task_id = params.get("task_id").and_then(|v| v.as_u64()).unwrap_or(0);
            let result = params.get("result").and_then(|v| v.as_str()).unwrap_or("");

            let mut s = shared_store.write().await;
            match s.complete_task(task_id, completer, result) {
                Ok(true) => JsonRpcResponse::success(id, serde_json::json!({"status": "completed", "id": task_id})),
                Ok(false) => JsonRpcResponse::error(id, -32000, "Cannot complete task".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "list_tasks" | "get_tasks" => {
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let pending_only = params.get("pending").and_then(|v| v.as_bool()).unwrap_or(false);

            let mut s = shared_store.write().await;
            let result = if pending_only {
                s.list_pending_tasks(limit)
            } else {
                s.list_tasks(limit)
            };

            match result {
                Ok(tasks) => {
                    let result: Vec<_> = tasks.iter().map(|(id, task)| {
                        serde_json::json!({
                            "id": id,
                            "description": task.description,
                            "status": format!("{:?}", task.status).to_lowercase(),
                            "priority": task.priority as u8,
                            "tags": task.tags,
                            "result": task.result,
                            "created_by": task.created_by,
                            "claimed_by": task.claimed_by,
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "task_stats" | "get_task_stats" => {
            let mut s = shared_store.write().await;
            match s.task_stats() {
                Ok(stats) => JsonRpcResponse::success(id, serde_json::json!({
                    "total": stats.total,
                    "pending": stats.pending,
                    "claimed": stats.claimed,
                    "in_progress": stats.in_progress,
                    "completed": stats.completed,
                    "failed": stats.failed,
                    "cancelled": stats.cancelled,
                })),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // === DIALOGUES ===
        "start_dialogue" => {
            // BUG FIX: Read initiator from request params
            let initiator = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let responder = params.get("responder").and_then(|v| v.as_str()).unwrap_or("");
            let topic = params.get("topic").and_then(|v| v.as_str()).unwrap_or("");

            if responder.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'responder'".into());
            }

            let mut s = shared_store.write().await;
            match s.start_dialogue(initiator, responder, topic) {
                Ok(dialogue_id) => {
                    // Event-driven: Update BulletinBoard for responder immediately
                    let store_clone = shared_store.clone();
                    let responder_owned = responder.to_string();
                    tokio::spawn(async move {
                        refresh_bulletin_dialogues(&store_clone, &responder_owned).await;
                    });
                    JsonRpcResponse::success(id, serde_json::json!({
                        "id": dialogue_id,
                        "status": "started"
                    }))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "get_dialogues" | "list_dialogues" | "dialogues" => {
            // BUG FIX: Use client's ai_id from params for shared daemon
            let requesting_ai = params.get("ai_id").and_then(|v| v.as_str()).unwrap_or(ai_id);
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let mut s = shared_store.write().await;
            match s.get_dialogues_for_ai(requesting_ai, limit) {
                Ok(dialogues) => {
                    let result: Vec<_> = dialogues.iter().map(|(id, d)| {
                        serde_json::json!({
                            "id": id,
                            "initiator": d.initiator,
                            "responder": d.responder,
                            "topic": d.topic,
                            "status": format!("{:?}", d.status).to_lowercase(),
                            "turn": d.turn,
                            "message_count": d.message_count,
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "get_dialogue" => {
            let dialogue_id = params.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.get_dialogue(dialogue_id) {
                Ok(Some(d)) => JsonRpcResponse::success(id, serde_json::json!({
                    "id": dialogue_id,
                    "initiator": d.initiator,
                    "responder": d.responder,
                    "topic": d.topic,
                    "status": format!("{:?}", d.status).to_lowercase(),
                    "turn": d.turn,
                    "message_count": d.message_count,
                })),
                Ok(None) => JsonRpcResponse::error(id, -32000, "Dialogue not found".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "respond_dialogue" | "dialogue_respond" => {
            let dialogue_id = params.get("id").and_then(|v| v.as_u64())
                .or_else(|| params.get("dialogue_id").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let mut s = shared_store.write().await;
            // Get dialogue info before responding to know who to notify
            let dialogue_info = s.get_dialogue(dialogue_id).ok().flatten();
            match s.respond_to_dialogue(dialogue_id) {
                Ok(true) => {
                    // Event-driven: Update BulletinBoard for both parties
                    if let Some(d) = dialogue_info {
                        let store_clone = shared_store.clone();
                        let initiator_owned = d.initiator.clone();
                        let responder_owned = d.responder.clone();
                        tokio::spawn(async move {
                            refresh_bulletin_dialogues(&store_clone, &initiator_owned).await;
                            refresh_bulletin_dialogues(&store_clone, &responder_owned).await;
                        });
                        // EVENT-DRIVEN: Wake the other party - it's now their turn!
                        let other_party = if d.initiator == ai_id {
                            &d.responder
                        } else {
                            &d.initiator
                        };
                        signal_wake(other_party, WakeReason::DialogueTurn, ai_id, &d.topic);
                    }
                    JsonRpcResponse::success(id, serde_json::json!({"status": "responded"}))
                }
                Ok(false) => JsonRpcResponse::error(id, -32000, "Cannot respond".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "end_dialogue" | "dialogue_end" => {
            let dialogue_id = params.get("id").and_then(|v| v.as_u64())
                .or_else(|| params.get("dialogue_id").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let status_str = params.get("status").and_then(|v| v.as_str()).unwrap_or("completed");
            let status = match status_str {
                "cancelled" | "abandoned" => teamengram::store::DialogueStatus::Cancelled,
                "expired" => teamengram::store::DialogueStatus::Expired,
                _ => teamengram::store::DialogueStatus::Completed,
            };
            let mut s = shared_store.write().await;
            // Get dialogue info before ending to know who to notify
            let dialogue_info = s.get_dialogue(dialogue_id).ok().flatten();
            match s.end_dialogue(dialogue_id, status) {
                Ok(true) => {
                    // Event-driven: Update BulletinBoard for both parties
                    if let Some(d) = dialogue_info {
                        let store_clone = shared_store.clone();
                        let initiator_owned = d.initiator.clone();
                        let responder_owned = d.responder.clone();
                        tokio::spawn(async move {
                            refresh_bulletin_dialogues(&store_clone, &initiator_owned).await;
                            refresh_bulletin_dialogues(&store_clone, &responder_owned).await;
                        });
                    }
                    JsonRpcResponse::success(id, serde_json::json!({"status": "ended"}))
                }
                Ok(false) => JsonRpcResponse::error(id, -32000, "Cannot end dialogue".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "delete_dialogue_force" | "dialogue_delete_force" => {
            let dialogue_id = params.get("id").and_then(|v| v.as_u64())
                .or_else(|| params.get("dialogue_id").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.delete_dialogue_force(dialogue_id) {
                Ok(deleted_count) => JsonRpcResponse::success(id, serde_json::json!({
                    "status": "deleted",
                    "deleted_keys": deleted_count
                })),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "dialogue_invites" | "get_dialogue_invites" => {
            // BUG FIX: Use client's ai_id from params for shared daemon
            let requesting_ai = params.get("ai_id").and_then(|v| v.as_str()).unwrap_or(ai_id);
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let mut s = shared_store.write().await;
            match s.get_dialogue_invites(requesting_ai, limit) {
                Ok(dialogues) => {
                    let result: Vec<_> = dialogues.iter().map(|(id, d)| {
                        serde_json::json!({
                            "id": id,
                            "initiator": d.initiator,
                            "responder": d.responder,
                            "topic": d.topic,
                            "status": format!("{:?}", d.status).to_lowercase(),
                            "turn": d.turn,
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "dialogue_my_turn" | "my_turn_dialogues" => {
            // BUG FIX: Use client's ai_id from params for shared daemon
            let requesting_ai = params.get("ai_id").and_then(|v| v.as_str()).unwrap_or(ai_id);
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let mut s = shared_store.write().await;
            match s.get_my_turn_dialogues(requesting_ai, limit) {
                Ok(dialogues) => {
                    let result: Vec<_> = dialogues.iter().map(|(id, d)| {
                        serde_json::json!({
                            "id": id,
                            "initiator": d.initiator,
                            "responder": d.responder,
                            "topic": d.topic,
                            "status": format!("{:?}", d.status).to_lowercase(),
                            "turn": d.turn,
                            "message_count": d.message_count,
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "dialogue_turn" | "check_dialogue_turn" => {
            let dialogue_id = params.get("dialogue_id").and_then(|v| v.as_u64()).unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.get_dialogue(dialogue_id) {
                Ok(Some(d)) => {
                    // turn: 0 = initiator's turn, 1 = responder's turn
                    let current_turn_ai = if d.turn == 0 { &d.initiator } else { &d.responder };
                    let is_my_turn = current_turn_ai == ai_id;
                    JsonRpcResponse::success(id, serde_json::json!({
                        "dialogue_id": dialogue_id,
                        "current_turn": current_turn_ai,
                        "is_my_turn": is_my_turn,
                        "initiator": d.initiator,
                        "responder": d.responder,
                    }))
                }
                Ok(None) => JsonRpcResponse::error(id, -32000, "Dialogue not found".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // === VOTES ===
        "create_vote" => {
            // BUG FIX: Read creator from request params
            let creator = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let topic = params.get("topic").and_then(|v| v.as_str()).unwrap_or("");
            let options: Vec<String> = params.get("options")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let duration = params.get("duration_mins").and_then(|v| v.as_u64()).unwrap_or(60) as u32;

            if topic.is_empty() || options.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'topic' or 'options'".into());
            }

            let mut s = shared_store.write().await;
            match s.create_vote(creator, topic, options, duration) {
                Ok(vote_id) => {
                    drop(s);
                    // Event-driven: Update BulletinBoard immediately
                    let store_clone = Arc::clone(&shared_store);
                    tokio::spawn(async move {
                        refresh_bulletin_votes(&store_clone).await;
                    });
                    JsonRpcResponse::success(id, serde_json::json!({
                        "id": vote_id,
                        "status": "created"
                    }))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "cast_vote" => {
            // BUG FIX: Read voter from request params
            let voter = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let vote_id = params.get("vote_id").and_then(|v| v.as_u64()).unwrap_or(0);
            let option = params.get("option").and_then(|v| v.as_str())
                .or_else(|| params.get("choice").and_then(|v| v.as_str()))
                .unwrap_or("");

            let mut s = shared_store.write().await;
            match s.cast_vote(vote_id, voter, option) {
                Ok(true) => {
                    drop(s);
                    // Event-driven: Update BulletinBoard immediately
                    let store_clone = Arc::clone(&shared_store);
                    tokio::spawn(async move {
                        refresh_bulletin_votes(&store_clone).await;
                    });
                    JsonRpcResponse::success(id, serde_json::json!({"status": "voted"}))
                }
                Ok(false) => JsonRpcResponse::error(id, -32000, "Vote failed".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "list_votes" | "vote_list" | "get_votes" => {
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let mut s = shared_store.write().await;
            match s.list_votes(limit) {
                Ok(votes) => {
                    let result: Vec<_> = votes.iter().map(|(vid, v)| {
                        serde_json::json!({
                            "id": vid,
                            "topic": v.topic,
                            "options": v.options,
                            "votes": v.votes,
                            "status": format!("{:?}", v.status).to_lowercase(),
                            "created_by": v.created_by,
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "get_vote" | "vote_results" | "vote_get" => {
            let vote_id = params.get("vote_id").and_then(|v| v.as_u64()).unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.get_vote(vote_id) {
                Ok(Some(v)) => JsonRpcResponse::success(id, serde_json::json!({
                    "id": vote_id,
                    "topic": v.topic,
                    "options": v.options,
                    "votes": v.votes,
                    "status": format!("{:?}", v.status).to_lowercase(),
                    "created_by": v.created_by,
                })),
                Ok(None) => JsonRpcResponse::error(id, -32000, "Vote not found".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "close_vote" => {
            // BUG FIX: Read closer from request params
            let closer = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let vote_id = params.get("vote_id").and_then(|v| v.as_u64()).unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.close_vote(vote_id, closer) {
                Ok(true) => JsonRpcResponse::success(id, serde_json::json!({"status": "closed"})),
                Ok(false) => JsonRpcResponse::error(id, -32000, "Cannot close vote (not owner or already closed)".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // === LOCKS ===
        "acquire_lock" => {
            // BUG FIX: Read holder from request params
            let holder = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let resource = params.get("resource").and_then(|v| v.as_str()).unwrap_or("");
            let working_on = params.get("working_on").and_then(|v| v.as_str()).unwrap_or("");
            let duration = params.get("duration_mins").and_then(|v| v.as_u64()).unwrap_or(30) as u32;

            if resource.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'resource'".into());
            }

            let mut s = shared_store.write().await;
            match s.acquire_lock(holder, resource, working_on, duration) {
                Ok(Some(lock_id)) => {
                    drop(s);
                    // Event-driven: Update BulletinBoard immediately
                    let store_clone = Arc::clone(&shared_store);
                    tokio::spawn(async move {
                        refresh_bulletin_locks(&store_clone).await;
                    });
                    JsonRpcResponse::success(id, serde_json::json!({
                        "id": lock_id,
                        "status": "acquired"
                    }))
                }
                Ok(None) => JsonRpcResponse::error(id, -32000, "Resource already locked".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "release_lock" => {
            // BUG FIX: Read releaser from request params
            let releaser = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let resource = params.get("resource").and_then(|v| v.as_str()).unwrap_or("");

            let mut s = shared_store.write().await;
            match s.release_lock(resource, releaser) {
                Ok(true) => {
                    drop(s);
                    // Event-driven: Update BulletinBoard immediately
                    let store_clone = Arc::clone(&shared_store);
                    tokio::spawn(async move {
                        refresh_bulletin_locks(&store_clone).await;
                    });
                    JsonRpcResponse::success(id, serde_json::json!({"status": "released"}))
                }
                Ok(false) => JsonRpcResponse::error(id, -32000, "Lock not held".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "check_lock" | "lock_check" => {
            let resource = params.get("resource").and_then(|v| v.as_str()).unwrap_or("");
            if resource.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'resource'".into());
            }
            let mut s = shared_store.write().await;
            match s.check_lock(resource) {
                Ok(Some(lock)) => JsonRpcResponse::success(id, serde_json::json!({
                    "id": 0,  // Lock doesn't have ID in storage, use 0
                    "holder": lock.holder,
                    "resource": lock.resource,
                    "working_on": lock.working_on,
                    "expires_at": lock.expires_at,
                })),
                Ok(None) => JsonRpcResponse::success(id, serde_json::Value::Null),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "list_locks" | "my_locks" => {
            // BUG FIX: Use client's ai_id from params for shared daemon
            let requesting_ai = params.get("ai_id").and_then(|v| v.as_str()).unwrap_or(ai_id);
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let mut s = shared_store.write().await;
            match s.list_locks_by_holder(requesting_ai, limit) {
                Ok(locks) => {
                    let result: Vec<_> = locks.iter().map(|(lid, l)| {
                        serde_json::json!({
                            "id": lid,
                            "resource": l.resource,
                            "working_on": l.working_on,
                            "expires_at": l.expires_at,
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // === FILE CLAIMS ===
        "claim_file" => {
            // BUG FIX: Read claimer from request params
            let claimer = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let working_on = params.get("working_on").and_then(|v| v.as_str()).unwrap_or("");
            let duration = params.get("duration_mins").and_then(|v| v.as_u64()).unwrap_or(30) as u32;

            if path.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'path'".into());
            }

            let mut s = shared_store.write().await;
            match s.claim_file(claimer, path, working_on, duration) {
                Ok(claim_id) => JsonRpcResponse::success(id, serde_json::json!({
                    "id": claim_id,
                    "status": "claimed"
                })),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "check_file" | "check_file_claim" | "file_check" => {
            let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'path'".into());
            }
            let mut s = shared_store.write().await;
            match s.check_file_claim(path) {
                Ok(Some(claim)) => JsonRpcResponse::success(id, serde_json::json!({
                    "id": 0,  // FileClaim doesn't have ID in storage, use 0
                    "claimer": claim.claimer,
                    "path": claim.path,
                    "working_on": claim.working_on,
                    "expires_at": claim.expires_at,
                })),
                Ok(None) => JsonRpcResponse::success(id, serde_json::Value::Null),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "release_file" | "release_file_claim" => {
            let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'path'".into());
            }
            let mut s = shared_store.write().await;
            match s.release_file(path) {
                Ok(true) => JsonRpcResponse::success(id, serde_json::json!({"status": "released"})),
                Ok(false) => JsonRpcResponse::error(id, -32000, "File not claimed".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "list_file_claims" | "list_claims" => {
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let mut s = shared_store.write().await;
            match s.list_file_claims(limit) {
                Ok(claims) => {
                    let result: Vec<_> = claims.iter().map(|(cid, c)| {
                        serde_json::json!({
                            "id": cid,
                            "claimer": c.claimer,
                            "path": c.path,
                            "working_on": c.working_on,
                            "expires_at": c.expires_at,
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // === ROOMS ===
        "create_room" => {
            // BUG FIX: Read creator from request params
            let creator = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let topic = params.get("topic").and_then(|v| v.as_str()).unwrap_or("");

            if name.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'name'".into());
            }

            let mut s = shared_store.write().await;
            match s.create_room(creator, name, topic) {
                Ok(room_id) => JsonRpcResponse::success(id, serde_json::json!({
                    "id": room_id,
                    "status": "created"
                })),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "list_rooms" | "rooms" => {
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

            let mut s = shared_store.write().await;
            match s.list_rooms(limit) {
                Ok(rooms) => {
                    let result: Vec<_> = rooms.iter().map(|(id, room)| {
                        serde_json::json!({
                            "id": id,
                            "name": room.name,
                            "creator": room.creator,
                            "topic": room.topic,
                            "participants": room.participants,
                            "is_open": room.is_open,
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "get_room" | "room_get" => {
            let room_id = params.get("room_id").and_then(|v| v.as_u64()).unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.get_room(room_id) {
                Ok(Some(room)) => JsonRpcResponse::success(id, serde_json::json!({
                    "id": room_id,
                    "name": room.name,
                    "creator": room.creator,
                    "topic": room.topic,
                    "participants": room.participants,
                    "is_open": room.is_open,
                })),
                Ok(None) => JsonRpcResponse::error(id, -32000, "Room not found".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "join_room" | "room_join" => {
            // BUG FIX: Read joiner from request params
            let joiner = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let room_id = params.get("room_id").and_then(|v| v.as_u64()).unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.join_room(room_id, joiner) {
                Ok(JoinRoomResult::Joined) => JsonRpcResponse::success(id, serde_json::json!({"status": "joined"})),
                Ok(JoinRoomResult::NotFound) => JsonRpcResponse::error(id, -32000, "Room not found".into()),
                Ok(JoinRoomResult::Closed) => JsonRpcResponse::error(id, -32000, "Room is closed".into()),
                Ok(JoinRoomResult::AlreadyMember) => JsonRpcResponse::error(id, -32000, "Already a member".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "leave_room" | "room_leave" => {
            // BUG FIX: Read leaver from request params
            let leaver = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let room_id = params.get("room_id").and_then(|v| v.as_u64()).unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.leave_room(room_id, leaver) {
                Ok(true) => JsonRpcResponse::success(id, serde_json::json!({"status": "left"})),
                Ok(false) => JsonRpcResponse::error(id, -32000, "Failed to leave room".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "close_room" | "room_close" => {
            // BUG FIX: Read closer from request params
            let closer = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let room_id = params.get("room_id").and_then(|v| v.as_u64()).unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.close_room(room_id, closer) {
                Ok(true) => JsonRpcResponse::success(id, serde_json::json!({"status": "closed"})),
                Ok(false) => JsonRpcResponse::error(id, -32000, "Cannot close room (not creator)".into()),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }


        // === FILE ACTIONS (SessionStart Awareness) ===
        "log_file_action" | "file_action" => {
            let actor = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let path = params.get("path").and_then(|v| v.as_str())
                .or_else(|| params.get("file_path").and_then(|v| v.as_str()))
                .unwrap_or("");
            let action = params.get("action").and_then(|v| v.as_str()).unwrap_or("modified");

            if path.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'path' or 'file_path'".into());
            }

            let mut s = shared_store.write().await;
            match s.log_file_action(actor, path, action) {
                Ok(action_id) => {
                    drop(s);
                    // Event-driven: Update BulletinBoard immediately
                    let store_clone = Arc::clone(&shared_store);
                    tokio::spawn(async move {
                        refresh_bulletin_file_actions(&store_clone).await;
                    });
                    JsonRpcResponse::success(id, serde_json::json!({
                        "id": action_id,
                        "status": "logged"
                    }))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "get_file_actions" => {
            let target_ai = params.get("ai_id").and_then(|v| v.as_str()).unwrap_or(ai_id);
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

            let mut s = shared_store.write().await;
            match s.get_file_actions(target_ai, limit) {
                Ok(actions) => {
                    let result: Vec<_> = actions.iter().map(|a| {
                        serde_json::json!({
                            "ai_id": a.ai_id,
                            "path": a.path,
                            "action": a.action,
                            "timestamp": a.timestamp,
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "recent_file_actions" | "get_recent_file_actions" => {
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

            let mut s = shared_store.write().await;
            match s.get_recent_file_actions(limit) {
                Ok(actions) => {
                    let result: Vec<_> = actions.iter().map(|(action_id, a)| {
                        serde_json::json!({
                            "id": action_id,
                            "ai_id": &a.ai_id,
                            "path": &a.path,
                            "action": &a.action,
                            "timestamp": a.timestamp,
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // === DIRECTORY TRACKING (SessionStart Awareness) ===
        "track_directory" | "directory_access" => {
            let actor = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("ai_id").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let directory = params.get("directory").and_then(|v| v.as_str())
                .or_else(|| params.get("path").and_then(|v| v.as_str()))
                .unwrap_or("");
            let access_type = params.get("access_type").and_then(|v| v.as_str()).unwrap_or("read");

            if directory.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'directory' or 'path'".into());
            }

            let mut s = shared_store.write().await;
            match s.track_directory(actor, directory, access_type) {
                Ok(access_id) => JsonRpcResponse::success(id, serde_json::json!({
                    "id": access_id,
                    "status": "tracked"
                })),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "recent_directories" | "get_recent_directories" => {
            let target_ai = params.get("ai_id").and_then(|v| v.as_str()).unwrap_or(ai_id);
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

            let mut s = shared_store.write().await;
            match s.get_recent_directories(target_ai, limit) {

                Ok(dirs) => {
                    let result: Vec<_> = dirs.iter().map(|d| {
                        serde_json::json!({
                            "ai_id": d.ai_id,
                            "directory": d.directory,
                            "access_type": d.access_type,
                            "timestamp": d.timestamp,
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // =====================================================================
        // PROJECT OPERATIONS
        // =====================================================================

        "create_project" => {
            let actor = params.get("from_ai").and_then(|v| v.as_str()).unwrap_or(ai_id);
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let goal = params.get("goal").and_then(|v| v.as_str()).unwrap_or("");
            let root_dir = params.get("root_directory").and_then(|v| v.as_str()).unwrap_or("");

            if name.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'name'".into());
            }

            let mut s = shared_store.write().await;
            match s.create_project(name, goal, root_dir, actor) {
                Ok(proj_id) => JsonRpcResponse::success(id, serde_json::json!({"id": proj_id})),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "get_project" => {
            let proj_id = params.get("id").and_then(|v| v.as_u64())
                .or_else(|| params.get("project_id").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.get_project(proj_id) {
                Ok(Some(p)) => JsonRpcResponse::success(id, serde_json::json!({
                    "id": proj_id,
                    "name": p.name,
                    "goal": p.goal,
                    "root_directory": p.root_directory,
                    "created_by": p.created_by,
                    "status": p.status,
                    "created_at": p.created_at,
                    "updated_at": p.updated_at
                })),
                Ok(None) => JsonRpcResponse::success(id, serde_json::json!(null)),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "list_projects" => {
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
            let mut s = shared_store.write().await;
            match s.list_projects(limit) {
                Ok(projects) => {
                    let result: Vec<_> = projects.iter().map(|(proj_id, p)| {
                        serde_json::json!({
                            "id": proj_id,
                            "name": p.name,
                            "goal": p.goal,
                            "root_directory": p.root_directory,
                            "created_by": p.created_by,
                            "status": p.status,
                            "created_at": p.created_at
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "update_project" => {
            let proj_id = params.get("id").and_then(|v| v.as_u64())
                .or_else(|| params.get("project_id").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let goal = params.get("goal").and_then(|v| v.as_str());
            let status = params.get("status").and_then(|v| v.as_str());

            let mut s = shared_store.write().await;
            match s.update_project(proj_id, goal, status) {
                Ok(changed) => JsonRpcResponse::success(id, serde_json::json!({"changed": changed})),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "soft_delete_project" | "delete_project" => {
            let proj_id = params.get("id").and_then(|v| v.as_u64())
                .or_else(|| params.get("project_id").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.soft_delete_project(proj_id) {
                Ok(deleted) => JsonRpcResponse::success(id, serde_json::json!({"deleted": deleted})),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "restore_project" => {
            let proj_id = params.get("id").and_then(|v| v.as_u64())
                .or_else(|| params.get("project_id").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.restore_project(proj_id) {
                Ok(restored) => JsonRpcResponse::success(id, serde_json::json!({"restored": restored})),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // =====================================================================
        // FEATURE OPERATIONS
        // =====================================================================

        "create_feature" => {
            let actor = params.get("from_ai").and_then(|v| v.as_str())
                .or_else(|| params.get("created_by").and_then(|v| v.as_str()))
                .unwrap_or(ai_id);
            let project_id = params.get("project_id").and_then(|v| v.as_u64()).unwrap_or(0);
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let overview = params.get("overview").and_then(|v| v.as_str()).unwrap_or("");
            let directory = params.get("directory").and_then(|v| v.as_str());

            if name.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'name'".into());
            }

            let mut s = shared_store.write().await;
            match s.create_feature(project_id, name, overview, directory, actor) {
                Ok(feat_id) => JsonRpcResponse::success(id, serde_json::json!({"id": feat_id})),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "get_feature" => {
            let feat_id = params.get("id").and_then(|v| v.as_u64())
                .or_else(|| params.get("feature_id").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.get_feature(feat_id) {
                Ok(Some(f)) => JsonRpcResponse::success(id, serde_json::json!({
                    "id": feat_id,
                    "project_id": f.project_id,
                    "name": f.name,
                    "overview": f.overview,
                    "directory": f.directory,
                    "created_by": f.created_by,
                    "status": f.status,
                    "created_at": f.created_at,
                    "updated_at": f.updated_at
                })),
                Ok(None) => JsonRpcResponse::success(id, serde_json::json!(null)),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "list_features" => {
            let project_id = params.get("project_id").and_then(|v| v.as_u64()).unwrap_or(0);
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
            let mut s = shared_store.write().await;
            match s.list_features(project_id, limit) {
                Ok(features) => {
                    let result: Vec<_> = features.iter().map(|(feat_id, f)| {
                        serde_json::json!({
                            "id": feat_id,
                            "project_id": f.project_id,
                            "name": f.name,
                            "overview": f.overview,
                            "directory": f.directory,
                            "status": f.status,
                            "created_by": f.created_by,
                            "created_at": f.created_at
                        })
                    }).collect();
                    JsonRpcResponse::success(id, serde_json::json!(result))
                }
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "update_feature" => {
            let feat_id = params.get("id").and_then(|v| v.as_u64())
                .or_else(|| params.get("feature_id").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let name = params.get("name").and_then(|v| v.as_str());
            let overview = params.get("overview").and_then(|v| v.as_str());
            let directory = params.get("directory").and_then(|v| v.as_str());

            let mut s = shared_store.write().await;
            match s.update_feature(feat_id, name, overview, directory) {
                Ok(changed) => JsonRpcResponse::success(id, serde_json::json!({"changed": changed})),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "soft_delete_feature" | "delete_feature" => {
            let feat_id = params.get("id").and_then(|v| v.as_u64())
                .or_else(|| params.get("feature_id").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.soft_delete_feature(feat_id) {
                Ok(deleted) => JsonRpcResponse::success(id, serde_json::json!({"deleted": deleted})),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "restore_feature" => {
            let feat_id = params.get("id").and_then(|v| v.as_u64())
                .or_else(|| params.get("feature_id").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let mut s = shared_store.write().await;
            match s.restore_feature(feat_id) {
                Ok(restored) => JsonRpcResponse::success(id, serde_json::json!({"restored": restored})),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // =====================================================================
        // VAULT OPERATIONS (Shared key-value storage)
        // =====================================================================

        "vault_store" | "teambook_vault_store" => {
            let actor = params.get("from_ai").and_then(|v| v.as_str()).unwrap_or(ai_id);
            let key = params.get("key").and_then(|v| v.as_str()).unwrap_or("");
            let value = params.get("value").and_then(|v| v.as_str()).unwrap_or("");

            if key.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'key'".into());
            }

            let mut s = shared_store.write().await;
            match s.vault_store(key, value, actor) {
                Ok(()) => JsonRpcResponse::success(id, serde_json::json!({"status": "stored"})),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        "vault_get" | "teambook_vault_get" => {
            let key = params.get("key").and_then(|v| v.as_str()).unwrap_or("");

            if key.is_empty() {
                return JsonRpcResponse::error(id, -32602, "Missing 'key'".into());
            }

            let mut s = shared_store.write().await;
            match s.vault_get(key) {
                Ok(Some(value)) => JsonRpcResponse::success(id, serde_json::json!({"value": value})),
                Ok(None) => JsonRpcResponse::success(id, serde_json::json!(null)),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // VAULT: Uses PRIVATE store (per-AI isolation)
        "vault_list" | "teambook_vault_list" => {
            let mut s = private_store.write().await;
            match s.vault_list() {
                Ok(keys) => JsonRpcResponse::success(id, serde_json::json!(keys)),
                Err(e) => JsonRpcResponse::error(id, -32000, e.to_string()),
            }
        }

        // === UNKNOWN METHOD ===
        _ => {
            JsonRpcResponse::error(id, -32601, format!("Method not found: {}", request.method))
        }
    }
}

// ============================================================================
// MAIN
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("teamengram=info".parse().unwrap())
        )
        .init();

    // Get configuration from environment FIRST (needed for per-AI lock)
    // AI_ID is REQUIRED - no fallback, fail-fast if missing
    let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| {
        eprintln!("ERROR: AI_ID environment variable is required!");
        eprintln!("Each AI instance needs its own daemon with its own AI_ID.");
        eprintln!("Set AI_ID before starting: AI_ID=ai-3 teamengram-daemon.exe");
        std::process::exit(1);
    });

    // ========================================================================
    // SINGLETON LOCK - Per-AI lock file to allow multiple daemons
    // Each AI gets its own daemon, so lock files must be per-AI
    // ========================================================================
    {
        use std::fs::OpenOptions;

        // Per-AI lock file: teamengram_{ai_id}.lock (AI_ID is required, no fallback)
        let safe_id: String = ai_id.chars()
            .map(|c| if c == '/' || c == '\\' || c == ':' { '_' } else { c })
            .collect();
        let lock_filename = format!("teamengram_{}.lock", safe_id);

        let lock_path = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".ai-foundation")
            .join(&lock_filename);

        // Create parent directory if needed
        if let Some(parent) = lock_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // Try to create the lock file exclusively
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(file) => {
                // Write our PID to the lock file
                use std::io::Write;
                let mut file = file;
                let _ = writeln!(file, "{}", std::process::id());
                info!("Singleton lock acquired at {:?} for AI: {}", lock_path, ai_id);

                // Register cleanup on exit
                let lock_path_clone = lock_path.clone();
                ctrlc::set_handler(move || {
                    let _ = std::fs::remove_file(&lock_path_clone);
                    std::process::exit(0);
                }).ok();
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Check if the process is still running
                if let Ok(pid_str) = std::fs::read_to_string(&lock_path) {
                    warn!("TeamEngram daemon for {} may already be running (lock file exists with PID: {})", ai_id, pid_str.trim());
                }
                eprintln!("ERROR: TeamEngram daemon for {} is already running.", ai_id);
                eprintln!("Lock file: {:?}", lock_path);
                eprintln!("To force restart: delete the lock file and try again.");
                std::process::exit(1);
            }
            Err(e) => {
                error!("Failed to create lock file: {}", e);
                return Err(anyhow::anyhow!("Failed to create singleton lock: {}", e));
            }
        }
    }

    // Per-AI pipe name - each AI gets its own isolated daemon
    // This matches TeamEngramClient::pipe_name_for_ai() which clients use
    // Architecture: Each AI has own daemon for blazing fast, isolated data streams
    // AI_ID is required (validated above), no fallback needed
    // SWMR: Single shared daemon for all AIs
    let pipe_name = std::env::var("PIPE_NAME").unwrap_or_else(|_| {
        r"\\.\pipe\teamengram".to_string()
    });
    // OLD: Per-AI pipes (caused multi-writer B+Tree corruption) - REMOVED

    info!("|TEAMENGRAM DAEMON|v0.1.0");
    info!("Pipe:{}", pipe_name);
    info!("AI:{}", ai_id);


    // Create and run daemon
    let daemon = TeamEngramDaemon::new(pipe_name, ai_id)?;
    daemon.run().await
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_json_rpc_response_success() {
        let response = JsonRpcResponse::success(
            Some(serde_json::json!(1)),
            serde_json::json!({"status": "ok"})
        );
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[test]
    fn test_json_rpc_response_error() {
        let response = JsonRpcResponse::error(
            Some(serde_json::json!(1)),
            -32600,
            "Invalid Request".to_string()
        );
        assert!(response.result.is_none());
        assert!(response.error.is_some());
    }

    #[test]
    fn test_json_rpc_request_parsing() {
        let json = r#"{"jsonrpc":"2.0","method":"status","params":{},"id":1}"#;
        let request: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.method, "status");
        assert_eq!(request.id, Some(serde_json::json!(1)));
    }

    #[test]
    fn test_json_rpc_request_without_id() {
        let json = r#"{"jsonrpc":"2.0","method":"broadcast","params":{"content":"hello"}}"#;
        let request: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.method, "broadcast");
        assert!(request.id.is_none());
    }

    #[test]
    fn test_daemon_stats() {
        let stats = DaemonStats::new();
        assert_eq!(stats.request_count, 0);
        assert_eq!(stats.dm_count, 0);
        assert!(stats.uptime_seconds() < 2); // Should be ~0
    }

    // Integration tests using the actual store
    #[tokio::test]
    async fn test_route_status_method() {
        let dir = tempdir().unwrap();
        let store_path = dir.path().join("test.db");
        let ipc_path = dir.path().join("test.ipc");

        let notify = Arc::new(ShmNotifyCallback::open(&ipc_path).unwrap());
        let store = TeamEngram::open_with_notify(&store_path, notify).unwrap();
        let store = Arc::new(RwLock::new(store));
        let private_store = Arc::clone(&store); // Tests use same store for both
        let stats = Arc::new(RwLock::new(DaemonStats::new()));

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "status".to_string(),
            params: serde_json::json!({}),
            id: Some(serde_json::json!(1)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "test-ai").await;
        assert!(response.result.is_some());
        let result = response.result.unwrap();
        assert_eq!(result["status"], "ok");
        assert_eq!(result["ai_id"], "test-ai");
        assert_eq!(result["backend"], "teamengram");
    }

    #[tokio::test]
    async fn test_route_dm_methods() {
        let dir = tempdir().unwrap();
        let store_path = dir.path().join("test.db");
        let ipc_path = dir.path().join("test.ipc");

        let notify = Arc::new(ShmNotifyCallback::open(&ipc_path).unwrap());
        let store = TeamEngram::open_with_notify(&store_path, notify).unwrap();
        let store = Arc::new(RwLock::new(store));
        let private_store = Arc::clone(&store); // Tests use same store for both
        let stats = Arc::new(RwLock::new(DaemonStats::new()));

        // Send a DM
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "dm".to_string(),
            params: serde_json::json!({"to": "ai-1", "content": "Hello Lyra!"}),
            id: Some(serde_json::json!(1)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "ai-2").await;
        assert!(response.result.is_some());
        let result = response.result.unwrap();
        assert_eq!(result["status"], "sent");
        assert!(result["id"].as_u64().unwrap() > 0);

        // Verify stats updated
        let s = stats.read().await;
        assert_eq!(s.dm_count, 1);
    }

    #[tokio::test]
    async fn test_route_broadcast_methods() {
        let dir = tempdir().unwrap();
        let store_path = dir.path().join("test.db");
        let ipc_path = dir.path().join("test.ipc");

        let notify = Arc::new(ShmNotifyCallback::open(&ipc_path).unwrap());
        let store = TeamEngram::open_with_notify(&store_path, notify).unwrap();
        let store = Arc::new(RwLock::new(store));
        let private_store = Arc::clone(&store); // Tests use same store for both
        let stats = Arc::new(RwLock::new(DaemonStats::new()));

        // Send a broadcast
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "broadcast".to_string(),
            params: serde_json::json!({"content": "Hello team!", "channel": "general"}),
            id: Some(serde_json::json!(1)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "ai-2").await;
        assert!(response.result.is_some());
        let result = response.result.unwrap();
        assert_eq!(result["status"], "sent");

        // Verify stats updated
        let s = stats.read().await;
        assert_eq!(s.broadcast_count, 1);
    }

    #[tokio::test]
    async fn test_route_task_methods() {
        let dir = tempdir().unwrap();
        let store_path = dir.path().join("test.db");
        let ipc_path = dir.path().join("test.ipc");

        let notify = Arc::new(ShmNotifyCallback::open(&ipc_path).unwrap());
        let store = TeamEngram::open_with_notify(&store_path, notify).unwrap();
        let store = Arc::new(RwLock::new(store));
        let private_store = Arc::clone(&store); // Tests use same store for both
        let stats = Arc::new(RwLock::new(DaemonStats::new()));

        // Queue a task
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "queue_task".to_string(),
            params: serde_json::json!({
                "description": "Review TeamEngram code",
                "priority": "high",
                "tags": "review,code"
            }),
            id: Some(serde_json::json!(1)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "ai-2").await;
        assert!(response.result.is_some());
        let result = response.result.unwrap();
        assert_eq!(result["status"], "queued");
        let task_id = result["id"].as_u64().unwrap();

        // Claim the task
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "claim_task".to_string(),
            params: serde_json::json!({"task_id": task_id}),
            id: Some(serde_json::json!(2)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "ai-1").await;
        assert!(response.result.is_some());
        assert_eq!(response.result.unwrap()["status"], "claimed");

        // Complete the task
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "complete_task".to_string(),
            params: serde_json::json!({"task_id": task_id, "result": "Code reviewed, looks good!"}),
            id: Some(serde_json::json!(3)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "ai-1").await;
        assert!(response.result.is_some());
        assert_eq!(response.result.unwrap()["status"], "completed");
    }

    #[tokio::test]
    async fn test_route_presence_methods() {
        let dir = tempdir().unwrap();
        let store_path = dir.path().join("test.db");
        let ipc_path = dir.path().join("test.ipc");

        let notify = Arc::new(ShmNotifyCallback::open(&ipc_path).unwrap());
        let store = TeamEngram::open_with_notify(&store_path, notify).unwrap();
        let store = Arc::new(RwLock::new(store));
        let private_store = Arc::clone(&store); // Tests use same store for both
        let stats = Arc::new(RwLock::new(DaemonStats::new()));

        // Update presence
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "update_presence".to_string(),
            params: serde_json::json!({"status": "active", "task": "building daemon"}),
            id: Some(serde_json::json!(1)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "ai-2").await;
        assert!(response.result.is_some());
        assert_eq!(response.result.unwrap()["status"], "updated");

        // Get all presences
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "who_is_here".to_string(),
            params: serde_json::json!({}),
            id: Some(serde_json::json!(2)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "ai-2").await;
        assert!(response.result.is_some());
        let presences = response.result.unwrap();
        assert!(presences.as_array().unwrap().len() >= 1);
    }

    #[tokio::test]
    async fn test_route_lock_methods() {
        let dir = tempdir().unwrap();
        let store_path = dir.path().join("test.db");
        let ipc_path = dir.path().join("test.ipc");

        let notify = Arc::new(ShmNotifyCallback::open(&ipc_path).unwrap());
        let store = TeamEngram::open_with_notify(&store_path, notify).unwrap();
        let store = Arc::new(RwLock::new(store));
        let private_store = Arc::clone(&store); // Tests use same store for both
        let stats = Arc::new(RwLock::new(DaemonStats::new()));

        // Acquire lock
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "acquire_lock".to_string(),
            params: serde_json::json!({
                "resource": "src/main.rs",
                "working_on": "refactoring",
                "duration_mins": 30
            }),
            id: Some(serde_json::json!(1)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "ai-2").await;
        assert!(response.result.is_some());
        assert_eq!(response.result.unwrap()["status"], "acquired");

        // Try to acquire same lock (should fail)
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "acquire_lock".to_string(),
            params: serde_json::json!({
                "resource": "src/main.rs",
                "working_on": "other work"
            }),
            id: Some(serde_json::json!(2)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "ai-1").await;
        assert!(response.error.is_some());

        // Release lock
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "release_lock".to_string(),
            params: serde_json::json!({"resource": "src/main.rs"}),
            id: Some(serde_json::json!(3)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "ai-2").await;
        assert!(response.result.is_some());
        assert_eq!(response.result.unwrap()["status"], "released");
    }

    #[tokio::test]
    async fn test_route_room_methods() {
        let dir = tempdir().unwrap();
        let store_path = dir.path().join("test.db");
        let ipc_path = dir.path().join("test.ipc");

        let notify = Arc::new(ShmNotifyCallback::open(&ipc_path).unwrap());
        let store = TeamEngram::open_with_notify(&store_path, notify).unwrap();
        let store = Arc::new(RwLock::new(store));
        let private_store = Arc::clone(&store); // Tests use same store for both
        let stats = Arc::new(RwLock::new(DaemonStats::new()));

        // Create room
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "create_room".to_string(),
            params: serde_json::json!({
                "name": "teamengram-dev",
                "topic": "TeamEngram development discussion"
            }),
            id: Some(serde_json::json!(1)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "ai-2").await;
        assert!(response.result.is_some());
        let result = response.result.unwrap();
        assert_eq!(result["status"], "created");

        // List rooms
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "list_rooms".to_string(),
            params: serde_json::json!({"limit": 10}),
            id: Some(serde_json::json!(2)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "ai-2").await;
        assert!(response.result.is_some());
        let rooms = response.result.unwrap();
        assert!(rooms.as_array().unwrap().len() >= 1);
    }

    #[tokio::test]
    async fn test_route_unknown_method() {
        let dir = tempdir().unwrap();
        let store_path = dir.path().join("test.db");
        let ipc_path = dir.path().join("test.ipc");

        let notify = Arc::new(ShmNotifyCallback::open(&ipc_path).unwrap());
        let store = TeamEngram::open_with_notify(&store_path, notify).unwrap();
        let store = Arc::new(RwLock::new(store));
        let private_store = Arc::clone(&store); // Tests use same store for both
        let stats = Arc::new(RwLock::new(DaemonStats::new()));

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "unknown_method".to_string(),
            params: serde_json::json!({}),
            id: Some(serde_json::json!(1)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "test-ai").await;
        assert!(response.error.is_some());
        let error = response.error.unwrap();
        assert_eq!(error.code, -32601); // Method not found
    }

    #[tokio::test]
    async fn test_route_missing_params() {
        let dir = tempdir().unwrap();
        let store_path = dir.path().join("test.db");
        let ipc_path = dir.path().join("test.ipc");

        let notify = Arc::new(ShmNotifyCallback::open(&ipc_path).unwrap());
        let store = TeamEngram::open_with_notify(&store_path, notify).unwrap();
        let store = Arc::new(RwLock::new(store));
        let private_store = Arc::clone(&store); // Tests use same store for both
        let stats = Arc::new(RwLock::new(DaemonStats::new()));

        // DM without 'to' param
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "dm".to_string(),
            params: serde_json::json!({"content": "Hello"}),
            id: Some(serde_json::json!(1)),
        };

        let response = route_method(&request, &store, &private_store, &stats, "test-ai").await;
        assert!(response.error.is_some());
        let error = response.error.unwrap();
        assert_eq!(error.code, -32602); // Invalid params
    }
}

