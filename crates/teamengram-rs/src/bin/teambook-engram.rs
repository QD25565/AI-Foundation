//! TeamBook-Engram CLI - Unified team coordination using TeamEngram backend
//!
//! Replaces PostgreSQL+Redis teambook with pure Rust TeamEngram storage.
//! Features:
//! - B+Tree persistence with shadow paging (LMDB-style)
//! - Shared memory IPC for real-time notifications (~200ns)
//! - Zero external dependencies (no PostgreSQL, no Redis)
//! - Single ~1MB binary
//!
//! ARCHITECTURE (v2.1): CLI → Daemon IPC only. No direct store access.
//! This prevents B+Tree corruption from concurrent multi-process writes.
//!
//! V2 EVENT SOURCING (--v2 flag):
//! - Each AI writes to local outbox (~100ns, wait-free)
//! - Sequencer daemon aggregates to master log
//! - Per-AI materialized views (no sharing, no corruption)
//! - Use --v2 flag to enable event sourcing backend

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use teamengram::{
    TeamEngram,
    hash_ai_id,
    client::TeamEngramClient,
    wake::{WakeCoordinator, WakeReason, is_ai_online},
    v2_client::V2Client,
};
use shm_rs::bulletin::BulletinBoard;
use std::io::{self, Read as IoRead};
use serde::{Deserialize, Serialize};
use chrono::{Utc, Timelike, Datelike};

// ============================================================================
// HOOK TYPES - For AI-Foundation hooks (Claude Code, Gemini CLI, etc.)
// ============================================================================

/// Input from Claude Code/Gemini CLI PostToolUse hook
#[derive(Debug, Deserialize)]
struct HookInput {
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
}

/// Output for Claude Code hook injection
#[derive(Debug, Serialize)]
struct HookOutput {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: HookSpecificOutput,
}

#[derive(Debug, Serialize)]
struct HookSpecificOutput {
    #[serde(rename = "hookEventName")]
    hook_event_name: String,
    #[serde(rename = "additionalContext")]
    additional_context: String,
}

/// State for PostToolUse hook deduplication (seen message IDs, last minute)
#[derive(Debug, Serialize, Deserialize, Default)]
struct PostToolHookState {
    dm_ids: Vec<u64>,
    broadcast_ids: Vec<u64>,
    last_minute: Option<(i32, u32, u32, u32, u32)>, // year, month, day, hour, minute
    #[serde(default)]
    last_project_inject_ts: Option<u64>, // unix timestamp (seconds) of last project context injection
    #[serde(default)]
    last_project_id: Option<u64>,        // project ID of last injected project
    #[serde(default)]
    last_feature_id: Option<u64>,        // feature ID of last injected feature
    #[serde(default)]
    last_claims_hash: Option<u64>,       // hash of last injected claims string (skip if unchanged)
    #[serde(default)]
    last_team_hash: Option<u64>,         // hash of last injected team activity string (skip if unchanged)
    #[serde(default)]
    last_claims_inject_ts: Option<u64>,  // unix seconds of last claims injection (5-min cooldown gate)
    #[serde(default)]
    last_team_inject_ts: Option<u64>,    // unix seconds of last team-activity injection (5-min cooldown gate)
}

impl PostToolHookState {
    fn state_path(ai_id: &str) -> std::path::PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        home.join(".ai-foundation").join("hook-state").join(format!("post_tool_{}.json", ai_id))
    }

    fn load(ai_id: &str) -> Self {
        let path = Self::state_path(ai_id);
        if let Ok(content) = std::fs::read_to_string(&path) {
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    fn save(&self, ai_id: &str) {
        let path = Self::state_path(ai_id);
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("[HOOK] Failed to create state dir {:?}: {}", parent, e);
                return;
            }
        }
        match serde_json::to_string(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    eprintln!("[HOOK] Failed to save state to {:?}: {}", path, e);
                }
            }
            Err(e) => eprintln!("[HOOK] Failed to serialize state: {}", e),
        }
    }

    fn seen_dm(&self, id: u64) -> bool {
        self.dm_ids.contains(&id)
    }

    fn seen_broadcast(&self, id: u64) -> bool {
        self.broadcast_ids.contains(&id)
    }

    fn mark_dm(&mut self, id: u64) {
        if !self.dm_ids.contains(&id) {
            self.dm_ids.push(id);
            // Keep last 100
            if self.dm_ids.len() > 100 {
                self.dm_ids.remove(0);
            }
        }
    }

    fn mark_broadcast(&mut self, id: u64) {
        if !self.broadcast_ids.contains(&id) {
            self.broadcast_ids.push(id);
            // Keep last 100
            if self.broadcast_ids.len() > 100 {
                self.broadcast_ids.remove(0);
            }
        }
    }

    /// Check if project context should be injected.
    /// Returns (should_inject_project, should_inject_feature).
    /// Project: re-inject every 30 minutes OR if project changed.
    /// Feature: re-inject if feature changed (different sub-directory).
    fn should_inject_project(&mut self, project_id: u64, feature_id: Option<u64>) -> (bool, bool) {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        const PROJECT_COOLDOWN_SECS: u64 = 30 * 60; // 30 minutes

        let project_changed = self.last_project_id != Some(project_id);
        let project_expired = match self.last_project_inject_ts {
            Some(ts) => now_secs.saturating_sub(ts) >= PROJECT_COOLDOWN_SECS,
            None => true, // Never injected
        };
        let inject_project = project_changed || project_expired;

        let feature_changed = match feature_id {
            Some(fid) => self.last_feature_id != Some(fid),
            None => false, // No feature to inject
        };

        if inject_project {
            self.last_project_inject_ts = Some(now_secs);
            self.last_project_id = Some(project_id);
        }
        if let Some(fid) = feature_id {
            if feature_changed || inject_project {
                self.last_feature_id = Some(fid);
            }
        }

        (inject_project, feature_changed || (inject_project && feature_id.is_some()))
    }

    /// Peek-only hash check: returns (changed, new_hash). Does NOT mutate stored_hash.
    /// Caller commits new_hash only when actually injecting (pair with cooldown gate).
    fn content_hash_peek(stored_hash: Option<u64>, content: &str) -> (bool, u64) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.hash(&mut hasher);
        let hash = hasher.finish();
        (stored_hash != Some(hash), hash)
    }

    /// Peek-only cooldown check: returns true if ≥ cooldown_secs elapsed since
    /// last stamp (or never stamped). Does NOT mutate. Pair with stamp_now()
    /// at the actual injection site — that way a changed=false call doesn't
    /// consume the cooldown window for a later changed=true call.
    fn cooldown_elapsed(stored_ts: &Option<u64>, cooldown_secs: u64) -> bool {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        match *stored_ts {
            Some(ts) => now_secs.saturating_sub(ts) >= cooldown_secs,
            None => true,
        }
    }

    /// Stamp a cooldown timestamp to now. Call only when actually injecting.
    fn stamp_now(stored_ts: &mut Option<u64>) {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        *stored_ts = Some(now_secs);
    }

    fn should_inject_time(&mut self) -> bool {
        let now = Utc::now();
        let current = (
            now.year(),
            now.month(),
            now.day(),
            now.hour(),
            now.minute(),
        );
        if self.last_minute == Some(current) {
            false // Same minute, don't inject
        } else {
            self.last_minute = Some(current);
            true // Minute changed, inject
        }
    }
}

/// Tools that should be skipped (no logging, no injection)
const SKIP_TOOLS: &[&str] = &["TodoWrite"];


/// File action mapping
fn file_action_type(tool: &str) -> Option<&'static str> {
    match tool {
        "Edit" => Some("modified"),
        "Write" => Some("created"),
        "Read" => Some("accessed"),
        _ => None,
    }
}

/// Build working_on context for auto-claim enrichment.
/// Priority 1: AI's current in-progress task title
/// Priority 2: Tool verb + filename (e.g., "editing auth.rs")
fn build_working_on_context(v2: &mut V2Client, file_path: &str) -> String {
    // Try to get in-progress task description
    if let Ok(tasks) = v2.get_tasks() {
        if let Some((_, desc, _, _, _)) = tasks.iter()
            .find(|(_, _, _, status, _)| status == "in_progress")
        {
            // Truncate long task descriptions to keep claims readable
            let truncated = if desc.len() > 80 { &desc[..80] } else { desc.as_str() };
            return truncated.to_string();
        }
    }

    // Fallback: editing + filename
    let filename = file_path.split('/').last()
        .or_else(|| file_path.split('\\').last())
        .unwrap_or(file_path);
    format!("editing {}", filename)
}

#[derive(Parser)]
#[command(name = "teambook-engram")]
#[command(about = "High-performance AI coordination (TeamEngram backend)", long_about = None)]
#[command(version = "2.0.0")]
struct Cli {
    /// Use V2 event sourcing backend (DEFAULT - no B+Tree corruption)
    /// Each AI writes to local outbox, Sequencer aggregates to master log
    #[arg(long, global = true, default_value = "true", action = clap::ArgAction::Set)]
    v2: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    // ===== CORE MESSAGING =====

    /// Broadcast a message to all AIs
    #[command(alias = "bc", alias = "announce", alias = "shout", alias = "yell", alias = "all")]
    Broadcast {
        /// Message content
        content: String,
        /// Channel name
        #[arg(long, default_value = "general")]
        channel: String,
        /// Wake all online AIs from standby. Use sparingly — default broadcast
        /// respects standby sanctity (sleeping AIs stay asleep on normal traffic).
        #[arg(long)]
        urgent: bool,
    },

    /// Send a direct message to another AI
    #[command(alias = "dm", alias = "send", alias = "msg", alias = "pm", alias = "whisper")]
    DirectMessage {
        /// Target AI (e.g., alpha-001)
        to_ai: String,
        /// Message content
        content: String,
    },

    /// Read recent broadcast messages
    #[command(name = "read-messages", alias = "messages", alias = "msgs", alias = "broadcasts", alias = "feed", alias = "listen")]
    ReadMessages {
        /// Number of messages
        #[arg(default_value = "10")]
        limit: usize,
        /// Channel to read from
        #[arg(long, default_value = "general")]
        channel: String,
    },

    /// Read your direct messages
    #[command(name = "read-dms", alias = "direct-messages", alias = "dms", alias = "inbox", alias = "mail", alias = "received")]
    ReadDms {
        /// Number of messages
        #[arg(default_value = "10")]
        limit: usize,
    },

    /// Show team status and online AIs
    #[command(alias = "get-status", alias = "who", alias = "online", alias = "team", alias = "here")]
    Status,

    // ===== DIALOGUES (4 Consolidated Commands) =====

    /// Create a new dialogue with one or more AIs (n-party supported)
    #[command(name = "dialogue-create")]
    #[command(alias = "start-dialogue", alias = "dialogue", alias = "chat", alias = "converse", alias = "new-dialogue")]
    #[command(alias = "dialogue-start", alias = "create-dialogue", alias = "talk", alias = "begin-dialogue")]
    DialogueCreate {
        /// Target AI(s) to dialogue with. Comma-separated for n-party: "beta-002,gamma-003"
        responder: String,
        /// Dialogue topic
        topic: String,
    },

    /// Respond in an active dialogue
    #[command(name = "dialogue-respond")]
    #[command(alias = "reply", alias = "respond", alias = "answer", alias = "say", alias = "dialogue-reply")]
    #[command(alias = "respond-dialogue", alias = "dialogue-answer", alias = "chat-reply", alias = "continue-dialogue")]
    DialogueRespond {
        /// Dialogue ID
        dialogue_id: u64,
        /// Your response
        #[allow(dead_code)]
        response: String,
    },

    /// List dialogues with optional filters (all, invites, my-turn) or get specific dialogue by ID
    #[command(name = "dialogue-list")]
    #[command(alias = "dialogues", alias = "list-dialogues", alias = "chats", alias = "my-dialogues", alias = "conversations")]
    #[command(alias = "invites", alias = "dialogue-invites", alias = "pending-chats", alias = "incoming", alias = "chat-invites")]
    #[command(alias = "my-turn", alias = "dialogue-my-turn", alias = "pending-reply", alias = "awaiting", alias = "need-response")]
    #[command(alias = "dialogue-get", alias = "get-dialogue", alias = "show-dialogue", alias = "chat-info", alias = "dialogue-info")]
    #[command(alias = "dialogue-read", alias = "read-dialogue", alias = "dialogue-messages", alias = "chat-history")]
    #[command(alias = "dialogue-turn", alias = "whose-turn", alias = "turn", alias = "check-turn", alias = "whos-turn")]
    DialogueList {
        /// Number of dialogues to show
        #[arg(default_value = "10")]
        limit: usize,
        /// Filter: all (default), invites, my-turn
        #[arg(long, default_value = "all")]
        filter: String,
        /// Get specific dialogue by ID (shows full details + messages)
        #[arg(long)]
        id: Option<u64>,
    },

    /// End a dialogue (optionally merge into another)
    #[command(name = "dialogue-end")]
    #[command(alias = "end-dialogue", alias = "close-dialogue", alias = "finish-dialogue", alias = "done-dialogue", alias = "close-chat")]
    #[command(alias = "dialogue-close", alias = "end-chat", alias = "finish-chat", alias = "done-chat", alias = "complete-dialogue")]
    #[command(alias = "dialogue-merge", alias = "merge-dialogues", alias = "combine-dialogues", alias = "join-dialogues", alias = "merge-chat")]
    DialogueEnd {
        /// Dialogue ID to end
        dialogue_id: u64,
        /// Status (completed, cancelled)
        #[arg(default_value = "completed")]
        status: String,
        /// Optional summary of the dialogue outcome
        #[arg(long)]
        summary: Option<String>,
        /// Merge this dialogue into another (source becomes merged, target stays active)
        #[arg(long)]
        merge_into: Option<u64>,
    },

    // ===== VOTING =====

    /// Create a new vote
    #[command(alias = "poll", alias = "new-vote", alias = "create-poll", alias = "start-vote", alias = "propose")]
    VoteCreate {
        /// Vote topic/question
        topic: String,
        /// Options comma-separated (e.g., "A,B,C")
        options: String,
        /// Duration in minutes
        #[arg(long, default_value = "60")]
        duration: u32,
    },

    /// Cast your vote
    #[command(alias = "vote", alias = "cast", alias = "choose", alias = "pick", alias = "ballot")]
    VoteCast {
        /// Vote ID
        vote_id: u64,
        /// Your choice
        choice: String,
    },

    /// List votes
    #[command(alias = "polls", alias = "list-votes", alias = "all-votes", alias = "voting", alias = "ballots")]
    Votes {
        /// Number of votes
        #[arg(default_value = "10")]
        limit: usize,
    },

    /// Get vote results
    #[command(alias = "results", alias = "tally", alias = "count-votes", alias = "poll-results", alias = "show-results")]
    VoteResults {
        /// Vote ID
        vote_id: u64,
    },

    /// Close a vote (creator only)
    #[command(alias = "close-poll", alias = "end-vote", alias = "finish-vote", alias = "complete-vote", alias = "end-poll")]
    VoteClose {
        /// Vote ID
        vote_id: u64,
    },

    // ===== TASKS (4 Consolidated Commands) =====
    // Matches Claude Code pattern: task_create, task_update, task_get, task_list
    // Old command names kept as hidden aliases to catch wandering AI inputs

    /// Create a task or batch
    /// Single task: task-create "Fix the login bug"
    /// Batch: task-create "Auth" --tasks "1:Login,2:Logout,3:Test"
    #[command(name = "task-create")]
    #[command(alias = "task", alias = "add-task", alias = "task-add", alias = "new-task", alias = "create-task", alias = "add")]
    #[command(alias = "task-queue", alias = "batch", alias = "batch-create", alias = "new-batch", alias = "create-batch")]
    TaskCreate {
        /// Task description (single) or batch name (if --tasks provided)
        description: String,
        /// For batches: inline tasks as "1:Fix login,2:Fix logout"
        #[arg(long)]
        tasks: Option<String>,
        /// Priority (low, normal, high, urgent) - for single tasks
        #[arg(long, default_value = "normal")]
        priority: String,
    },

    /// Update task status: done, claimed, started, blocked, closed
    /// Single task: task-update 5 done
    /// Batch task: task-update "Auth:1" done
    /// Batch close: task-update "Auth" closed
    #[command(name = "task-update")]
    #[command(alias = "task-complete", alias = "complete", alias = "done", alias = "finish", alias = "resolve", alias = "close-task")]
    #[command(alias = "task-start", alias = "begin-task", alias = "work-on", alias = "start")]
    #[command(alias = "task-block", alias = "pause-task", alias = "hold-task", alias = "block")]
    #[command(alias = "task-unblock", alias = "resume-task", alias = "continue-task", alias = "unblock")]
    #[command(alias = "task-claim", alias = "claim-task", alias = "take", alias = "grab", alias = "claim")]
    #[command(alias = "batch-done", alias = "task-done", alias = "close-batch", alias = "finish-batch", alias = "batch-close")]
    TaskUpdate {
        /// Task ID, "BatchName:label", or batch name
        id: String,
        /// Status: done, claimed, started, blocked, closed
        status: String,
        /// Reason (for blocked status)
        #[arg(long)]
        reason: Option<String>,
    },

    /// Get task or batch details
    /// Single task: task-get 5
    /// Batch: task-get "Auth"
    #[command(name = "task-get")]
    #[command(alias = "get-task", alias = "show-task", alias = "task-details", alias = "view-task", alias = "inspect-task")]
    #[command(alias = "batch-get", alias = "show-batch", alias = "batch-details", alias = "view-batch")]
    TaskGet {
        /// Task ID or batch name
        id: String,
    },

    /// List tasks and batches
    /// All: task-list
    /// Tasks only: task-list --filter tasks
    /// Batches only: task-list --filter batches
    #[command(name = "task-list")]
    #[command(alias = "tasks", alias = "list-tasks", alias = "queue", alias = "pending-tasks", alias = "all-tasks")]
    #[command(alias = "batches", alias = "list-batches", alias = "my-batches", alias = "open-batches")]
    #[command(alias = "task-stats", alias = "queue-stats", alias = "task-info", alias = "task-summary")]
    TaskList {
        /// Number of items to show (positional form)
        count: Option<usize>,
        /// Number of items to show (flag form: --limit or -n)
        #[arg(long = "limit", short = 'n')]
        limit_flag: Option<usize>,
        /// Filter: all, tasks, batches (default: all)
        #[arg(long, default_value = "all")]
        filter: String,
    },

    /// See what AIs are doing
    #[command(alias = "what-doing", alias = "whats-happening", alias = "team-activity", alias = "ai-status")]
    WhatDoing {
        /// Number of entries (positional or --limit)
        #[arg(default_value = "10")]
        limit: usize,
    },

    // ===== FILE CLAIMS =====

    /// Claim a file for editing
    #[command(alias = "claim", alias = "lock-file", alias = "reserve", alias = "claim-edit", alias = "own")]
    ClaimFile {
        /// File path
        path: String,
        /// What you're working on
        working_on: String,
        /// Duration in minutes
        #[arg(long, default_value = "30")]
        duration: u32,
    },

    /// Check if a file is claimed
    #[command(alias = "check-claim", alias = "file-status", alias = "is-claimed", alias = "who-owns", alias = "file-owner")]
    CheckFile {
        /// File path
        path: String,
    },

    /// Release a file claim
    #[command(alias = "release", alias = "unclaim", alias = "free-file", alias = "unlock-file", alias = "give-back")]
    ReleaseFile {
        /// File path
        path: String,
    },

    /// List all active file claims
    #[command(alias = "claims", alias = "file-claims", alias = "all-claims", alias = "who-claimed", alias = "claimed-files")]
    ListClaims {
        /// Number of claims
        #[arg(default_value = "10")]
        limit: usize,
    },
    /// Log a file action (called by hooks for passive awareness)
    #[command(alias = "log", alias = "record", alias = "track", alias = "log-file", alias = "file-log")]
    LogAction {
        /// Action type (read, modified, created, deleted)
        action: String,
        /// File path
        path: String,
    },

    /// List recent file actions from all AIs
    #[command(alias = "actions", alias = "activity", alias = "file-history", alias = "recent-files", alias = "file-activity")]
    FileActions {
        /// Max actions to show
        #[arg(default_value = "10")]
        limit: usize,
    },

    // Locks removed — deprecated (Feb 2026, QD directive). Use file claims instead.

    // ===== ROOMS =====

    /// Create a room
    #[command(alias = "new-room", alias = "create-room", alias = "make-room", alias = "start-room", alias = "open-room")]
    RoomCreate {
        /// Room name
        name: String,
        /// Room topic
        topic: String,
    },

    /// Join a room
    #[command(alias = "join", alias = "enter", alias = "enter-room", alias = "hop-in", alias = "go-to")]
    RoomJoin {
        /// Room ID
        room_id: u64,
    },

    /// List rooms
    #[command(alias = "list-rooms", alias = "channels", alias = "spaces", alias = "all-rooms", alias = "show-rooms")]
    Rooms {
        /// Number of rooms
        #[arg(default_value = "10")]
        limit: usize,
    },

    /// Leave a room
    #[command(alias = "leave", alias = "exit", alias = "exit-room", alias = "depart", alias = "leave-room")]
    RoomLeave {
        /// Room ID
        room_id: u64,
    },

    /// Close a room (creator only)
    #[command(alias = "close", alias = "close-room", alias = "end-room", alias = "delete-room", alias = "shutdown-room")]
    RoomClose {
        /// Room ID
        room_id: u64,
    },

    /// Get room details
    #[command(alias = "room-info", alias = "show-room", alias = "room-details", alias = "get-room", alias = "room-status")]
    RoomGet {
        /// Room ID
        room_id: u64,
    },

    /// Send a message to a room
    #[command(alias = "room-message", alias = "say", alias = "room-msg", alias = "room-chat", alias = "room-speak")]
    RoomSay {
        /// Room ID
        room_id: u64,
        /// Message content
        content: String,
    },

    /// Get messages from a room
    #[command(alias = "room-history", alias = "room-log", alias = "room-chat-log", alias = "get-room-messages")]
    RoomMessages {
        /// Room ID
        room_id: u64,
        /// Number of messages to retrieve
        #[arg(default_value = "20")]
        limit: usize,
    },

    /// Mute a room for N minutes (timed only — no permanent mutes)
    #[command(alias = "room-mute", alias = "mute-room", alias = "snooze-room")]
    RoomMute {
        /// Room ID
        room_id: u64,
        /// Duration in minutes
        minutes: u32,
    },

    /// Conclude a room with a summary (closes the room)
    #[command(alias = "room-conclude", alias = "close-room", alias = "conclude-room", alias = "room-close")]
    RoomConclude {
        /// Room ID
        room_id: u64,
        /// Optional conclusion / summary text
        conclusion: Option<String>,
    },

    /// Pin a room message by its sequence ID
    #[command(alias = "room-pin", alias = "pin-room-message", alias = "room-pin-msg")]
    RoomPinMessage {
        /// Room ID
        room_id: u64,
        /// Message sequence ID (from room-history output)
        msg_seq_id: u64,
    },

    /// Unpin a room message
    #[command(alias = "room-unpin", alias = "unpin-room-message", alias = "room-unpin-msg")]
    RoomUnpinMessage {
        /// Room ID
        room_id: u64,
        /// Message sequence ID to unpin
        msg_seq_id: u64,
    },

    // ===== UTILITIES =====

    /// Update your presence status
    #[command(alias = "presence", alias = "set-status", alias = "set-presence", alias = "im-here", alias = "heartbeat")]
    UpdatePresence {
        /// Status (active, busy, standby)
        #[arg(default_value = "active")]
        status: String,
        /// Current task description
        #[arg(default_value = "")]
        task: String,
    },

    /// Show store statistics
    #[command(alias = "info", alias = "store-stats", alias = "db-stats", alias = "metrics", alias = "system-info")]
    Stats,

    /// Run performance benchmark
    #[command(alias = "bench", alias = "perf", alias = "test-perf", alias = "speed-test", alias = "stress-test")]
    Benchmark {
        /// Number of operations
        #[arg(default_value = "100")]
        count: usize,
    },

    /// Enter standby mode (event-driven, zero polling)
    #[command(alias = "wait", alias = "sleep", alias = "idle", alias = "await", alias = "listen")]
    Standby {
        /// Timeout in seconds (0 = 180s default)
        #[arg(default_value = "0")]
        timeout: u64,
    },

    // ===== ADDITIONAL UTILITIES =====

    /// Show my identity (AI_ID, fingerprint)
    #[command(alias = "whoami", alias = "id", alias = "identity", alias = "me", alias = "self")]
    IdentityShow,

    /// Get my current presence status
    #[command(alias = "my-status", alias = "my-presence", alias = "am-i-online", alias = "self-status", alias = "check-me")]
    MyPresence,

    /// Get another AI's presence status
    #[command(alias = "get-presence", alias = "lookup", alias = "find-ai", alias = "ai-status", alias = "whois")]
    GetPresence {
        /// AI ID to look up (e.g., beta-002)
        ai_id: String,
    },

    /// Count online AIs
    #[command(alias = "count", alias = "online-count", alias = "team-size", alias = "how-many", alias = "headcount")]
    PresenceCount,

    // Stigmergy (StigmergySense, StigmergyDeposit) removed — deprecated (Feb 2026, QD directive)

    /// Get awareness data for hooks (DMs, broadcasts, votes, dialogues, file claims)
    #[command(alias = "aware", alias = "inbox", alias = "notifications", alias = "alerts", alias = "check-all")]
    Awareness {
        /// Limit per category
        #[arg(default_value = "5")]
        limit: usize,
    },

    /// Gather contextual snapshot for notebook (presences, DMs, dialogues, file actions)
    /// Output format: [ctx:team:...|dms:...|bc:...|dial:...|files:...|at:...]
    #[command(alias = "context", alias = "snapshot", alias = "episodic", alias = "circumstance")]
    GatherContext {
        /// Include recent DMs (default 3)
        #[arg(long, default_value = "3")]
        dms: usize,
        /// Include recent broadcasts (default 3)
        #[arg(long, default_value = "3")]
        broadcasts: usize,
        /// Include file actions (default 5)
        #[arg(long, default_value = "5")]
        files: usize,
    },

    /// Migrate old B+Tree store to V2 event sourcing
    #[command(alias = "migrate-v2", alias = "upgrade", alias = "convert", alias = "import", alias = "v1-to-v2")]
    Migrate {
        /// Path to old store (default: shared/teamengram/teamengram.engram)
        #[arg(long)]
        old_store: Option<String>,
        /// V2 data directory (default: ~/.ai-foundation/v2)
        #[arg(long)]
        v2_dir: Option<String>,
    },

    /// Check and repair outbox corruption
    #[command(alias = "repair-outbox", alias = "fix-outbox", alias = "outbox-check", alias = "check-outboxes")]
    OutboxRepair {
        /// AI ID to check (default: all outboxes)
        #[arg(long)]
        ai_id: Option<String>,
        /// Actually repair (reset tail to head) if corruption found
        #[arg(long)]
        fix: bool,
    },

    /// Refresh bulletin board with latest V2 data (called by hooks when bulletin is stale)
    #[command(alias = "refresh", alias = "sync-bulletin", alias = "update-bulletin")]
    RefreshBulletin,

    // ===== PROJECTS =====

    /// List all projects
    #[command(alias = "projects", alias = "list-proj", alias = "all-projects", alias = "show-projects", alias = "my-projects")]
    ListProjects,

    /// Create a new project
    #[command(alias = "new-project", alias = "add-project", alias = "create-proj", alias = "init-project", alias = "project-new")]
    ProjectCreate {
        /// Project name
        name: String,
        /// Project goal/description
        goal: String,
        /// Root directory for the project
        #[arg(long, default_value = ".")]
        directory: String,
    },

    /// Get project details
    #[command(alias = "get-proj", alias = "show-project", alias = "project-info", alias = "project-details", alias = "view-project")]
    ProjectGet {
        /// Project ID
        project_id: u64,
    },

    /// Delete a project (soft delete, recoverable for 24h)
    #[command(alias = "del-project", alias = "rm-project", alias = "remove-project", alias = "trash-project", alias = "archive-project")]
    ProjectDelete {
        /// Project ID
        project_id: u64,
    },

    /// Restore a deleted project
    #[command(alias = "undelete-project", alias = "recover-project")]
    ProjectRestore {
        /// Project ID
        project_id: u64,
    },

    /// List tasks in a project
    #[command(alias = "proj-tasks", alias = "project-task-list", alias = "tasks-in-project", alias = "project-work")]
    ProjectTasks {
        /// Project ID
        project_id: u64,
    },

    /// Add a task to a project
    #[command(alias = "proj-add-task", alias = "add-project-task")]
    ProjectAddTask {
        /// Project ID
        project_id: u64,
        /// Task title
        title: String,
        /// Task priority (1-10)
        #[arg(long, default_value = "5")]
        priority: i32,
    },

    /// Resolve a file path to its project and feature context
    #[command(alias = "resolve", alias = "which-project", alias = "file-project", alias = "where-am-i", alias = "context")]
    ProjectResolve {
        /// File path to resolve
        path: String,
    },

    // ===== FEATURES =====

    /// List features in a project
    #[command(alias = "features", alias = "list-feat", alias = "project-features", alias = "show-features", alias = "all-features")]
    ListFeatures {
        /// Project ID
        project_id: u64,
    },

    /// Create a new feature within a project
    #[command(alias = "new-feature", alias = "add-feature", alias = "create-feat", alias = "feat-create", alias = "feature-new")]
    FeatureCreate {
        /// Project ID
        project_id: u64,
        /// Feature name
        name: String,
        /// Feature overview/description
        overview: String,
        /// Feature directory (optional)
        #[arg(long)]
        directory: Option<String>,
    },

    /// Get feature details
    #[command(alias = "get-feat", alias = "show-feature", alias = "feature-info", alias = "feature-details", alias = "view-feature")]
    FeatureGet {
        /// Feature ID
        feature_id: u64,
    },

    /// Delete a feature (soft delete, recoverable for 24h)
    #[command(alias = "del-feature", alias = "rm-feature", alias = "remove-feature", alias = "trash-feature", alias = "archive-feature")]
    FeatureDelete {
        /// Feature ID
        feature_id: u64,
    },

    /// Restore a deleted feature
    #[command(alias = "undelete-feature", alias = "recover-feature")]
    FeatureRestore {
        /// Feature ID
        feature_id: u64,
    },

    /// Update a feature's name, overview, or directory
    #[command(alias = "edit-feature", alias = "modify-feature", alias = "update-feat", alias = "feat-update", alias = "change-feature")]
    FeatureUpdate {
        /// Feature ID
        feature_id: u64,
        /// New overview text
        #[arg(long)]
        overview: Option<String>,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// New directory path
        #[arg(long)]
        directory: Option<String>,
    },

    /// Update a project's name or goal
    #[command(alias = "edit-project", alias = "modify-project", alias = "update-proj", alias = "proj-update", alias = "change-project")]
    ProjectUpdate {
        /// Project ID
        project_id: u64,
        /// New goal/description
        #[arg(long)]
        goal: Option<String>,
        /// New name
        #[arg(long)]
        name: Option<String>,
    },

    // ===== LEARNINGS (Team Insights / Muscle Memory) =====

    /// Share a learning with the team (max 15 per AI)
    #[command(alias = "learn", alias = "insight", alias = "tip", alias = "share-learning", alias = "teach")]
    Learning {
        /// Learning content
        content: String,
        /// Tags comma-separated (e.g., "kotlin,patterns")
        #[arg(long, default_value = "")]
        tags: String,
        /// Importance 0-100 (higher = more valuable)
        #[arg(long, default_value = "50")]
        importance: u8,
    },

    /// Update an existing learning
    #[command(alias = "update-learning", alias = "edit-learning", alias = "fix-tip")]
    LearningUpdate {
        /// Learning ID
        learning_id: u64,
        /// New content (optional)
        #[arg(long)]
        content: Option<String>,
        /// New tags (optional)
        #[arg(long)]
        tags: Option<String>,
        /// New importance (optional)
        #[arg(long)]
        importance: Option<u8>,
    },

    /// Delete a learning
    #[command(alias = "del-learning", alias = "rm-learning", alias = "forget")]
    LearningDelete {
        /// Learning ID
        learning_id: u64,
    },

    /// Show my learnings (my playbook)
    #[command(alias = "my-learnings", alias = "my-tips", alias = "my-insights", alias = "playbook")]
    MyLearnings,

    /// Show team playbook (top learnings from all AIs)
    #[command(alias = "team-playbook", alias = "team-tips", alias = "team-insights", alias = "osmosis")]
    TeamPlaybook {
        /// Number of learnings to show
        #[arg(default_value = "15")]
        limit: usize,
    },

    // ===== TRUST (TIP: Trust Inference and Propagation) =====

    /// Record trust feedback about another AI
    #[command(alias = "trust-feedback", alias = "rate", alias = "vouch", alias = "feedback")]
    TrustRecord {
        /// Target AI to rate
        target_ai: String,

        /// Feedback type: success or failure
        #[arg(value_parser = ["success", "failure", "s", "f", "+", "-"])]
        feedback: String,

        /// Context for this feedback
        #[arg(default_value = "unspecified")]
        context: String,

        /// Weight/significance (1-10)
        #[arg(short, long, default_value = "1")]
        weight: u8,
    },

    /// Show trust score for a specific AI
    #[command(alias = "trust", alias = "trust-check", alias = "rep")]
    TrustScore {
        /// AI to check trust for
        target_ai: String,
    },

    /// Show all trust scores (my view of team)
    #[command(alias = "trust-scores", alias = "trust-all", alias = "reputation", alias = "web-of-trust")]
    TrustScores,

    // ===== HOOKS (AI-Foundation hooks for Claude Code, Gemini CLI, etc.) =====

    /// PostToolUse hook - injects awareness, logs file actions, updates presence, time injection
    /// Reads JSON from stdin: {"tool_name": "Read", "tool_input": {"file_path": "/path"}}
    /// Outputs JSON to stdout for hook injection
    #[command(alias = "post-tool", alias = "after-tool")]
    HookPostToolUse,

    /// SessionStart hook - injects initial context (team status, pending tasks, broadcasts)
    /// Outputs JSON to stdout for hook injection
    #[command(alias = "session-init", alias = "on-start")]
    HookSessionStart,

    // ===== FEDERATION CONFIG =====

    /// Show the current Teambook permission manifest (what this Teambook exposes to peers)
    #[command(name = "federation-manifest", alias = "fed-manifest")]
    FederationManifestShow,

    /// Set a field in the permission manifest.
    ///
    /// FIELD examples: connection_mode, expose.presence, expose.broadcasts,
    ///   expose.dialogues, expose.task_complete, expose.file_claims, expose.raw_events
    ///
    /// VALUE examples: off | connect_code | mutual_auth | machine_local | open
    ///   (for booleans: true | false)
    ///   (for enums: none | cross_team_only | all | concluded_only)
    #[command(name = "federation-manifest-set", alias = "fed-manifest-set")]
    FederationManifestSet {
        /// Dot-separated field path (e.g. "expose.presence")
        field: String,
        /// New value as string
        value: String,
    },

    /// Show your current federation consent record (your per-AI narrowing of the manifest)
    #[command(name = "federation-consent", alias = "fed-consent")]
    FederationConsentShow,

    /// Update your federation consent record.
    ///
    /// FIELD: presence | broadcasts | task_complete | dialogues
    /// VALUE: true | false | none | cross_team_only | all | concluded_only | inherit
    ///   (use "inherit" to remove the override and fall back to manifest)
    #[command(name = "federation-consent-update", alias = "fed-consent-update")]
    FederationConsentUpdate {
        /// Field to update
        field: String,
        /// New value (or "inherit" to remove override)
        value: String,
    },

    // ===== MOBILE PAIRING =====

    /// Approve a mobile app pairing request.
    /// The mobile app displays a pairing code — run this on the server to grant access.
    #[command(name = "mobile-pair")]
    MobilePair {
        /// The pairing code shown in the mobile app
        code: String,
    },
}


// ============================================================================
// NOTE: BulletinBoard refresh is now handled by daemon after each write.
// CLI only uses IPC to daemon - no direct store or bulletin access.
// ============================================================================

/// Resolve AI_ID from .claude/settings.json (cross-platform, self-adapting)
/// This allows hooks to work without environment variable forwarding (WSLENV etc.)
fn resolve_ai_id_from_settings() -> Option<String> {
    if let Ok(cwd) = std::env::current_dir() {
        let settings_path = cwd.join(".claude").join("settings.json");
        if settings_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&settings_path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(ai_id) = json.get("env")
                        .and_then(|e| e.get("AI_ID"))
                        .and_then(|v| v.as_str())
                    {
                        return Some(ai_id.to_string());
                    }
                }
            }
        }
    }
    None
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // AI_ID resolution: settings.json first (self-adapting), then env var
    // This enables hooks to work on WSL/cross-platform without WSLENV forwarding
    let ai_id = resolve_ai_id_from_settings()
        .or_else(|| std::env::var("AI_ID").ok())
        .unwrap_or_else(|| {
            eprintln!("ERROR: AI_ID not found!");
            eprintln!("Set AI_ID in .claude/settings.json (env.AI_ID) or as environment variable.");
            std::process::exit(1);
        });

    if ai_id.is_empty() || ai_id == "unknown" {
        eprintln!("ERROR: AI_ID cannot be empty or 'unknown'");
        eprintln!("Set a valid AI identity, e.g.: AI_ID=gamma-003");
        std::process::exit(1);
    }

    // OUTBOX REPAIR - runs independently, no daemon or V2 check needed
    // Handle early because it's a maintenance command that works on V2 files directly
    if let Commands::OutboxRepair { ai_id: target_ai, fix } = &cli.command {
        use teamengram::outbox::{OutboxConsumer, list_outboxes, outbox_path};

        let base_dir = teamengram::store::ai_foundation_base_dir().join("v2");

        println!("|OUTBOX CHECK|");

        let ai_ids_to_check: Vec<String> = if let Some(specific_ai) = target_ai {
            vec![specific_ai.clone()]
        } else {
            match list_outboxes(Some(&base_dir)) {
                Ok(ids) => ids,
                Err(e) => {
                    eprintln!("Error listing outboxes: {}", e);
                    std::process::exit(1);
                }
            }
        };

        if ai_ids_to_check.is_empty() {
            println!("No outboxes found.");
            return Ok(());
        }

        let mut corruption_found = false;

        for ai_to_check in &ai_ids_to_check {
            let path = outbox_path(ai_to_check, Some(&base_dir));
            if !path.exists() {
                println!("{}:not_found", ai_to_check);
                continue;
            }

            match OutboxConsumer::open(ai_to_check, Some(&base_dir)) {
                Ok(consumer) => {
                    let pending = consumer.pending_bytes();

                    if pending == 0 {
                        println!("{}:ok:pending=0", ai_to_check);
                        continue;
                    }

                    // Check for corruption
                    if let Some(reason) = consumer.check_corruption() {
                        corruption_found = true;
                        println!("{}:CORRUPTED:pending={}:{}", ai_to_check, pending, reason);

                        if *fix {
                            let discarded = consumer.reset_tail_to_head();
                            println!("{}:REPAIRED:discarded={}_bytes", ai_to_check, discarded);
                        } else {
                            println!("{}:use_--fix_to_repair", ai_to_check);
                        }
                    } else {
                        println!("{}:ok:pending={}", ai_to_check, pending);
                    }
                }
                Err(e) => {
                    println!("{}:error:{}", ai_to_check, e);
                }
            }
        }

        if corruption_found && !*fix {
            println!("\n|WARNING| Corruption found. Run with --fix to repair (will discard pending events).");
        }

        return Ok(());
    }

    // ===== FEDERATION CONFIG COMMANDS =====
    // These operate directly on TOML files — no daemon, no V2 store.

    let fed_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".ai-foundation")
        .join("federation");

    if let Commands::FederationManifestShow = &cli.command {
        let manifest_path = fed_dir.join("manifest.toml");
        if !manifest_path.exists() {
            println!("# No manifest.toml found — safe-closed defaults are active.");
            println!("# Create one with: teambook federation-manifest-set <field> <value>");
            println!();
            println!("connection_mode = \"off\"");
            println!("inbound_actions = \"none\"");
            println!();
            println!("[expose]");
            println!("presence = false");
            println!("broadcasts = \"none\"");
            println!("dialogues = \"none\"");
            println!("task_complete = false");
            println!("file_claims = false");
            println!("raw_events = false");
        } else {
            let content = std::fs::read_to_string(&manifest_path)?;
            println!("{}", content);
        }
        return Ok(());
    }

    if let Commands::FederationManifestSet { field, value } = &cli.command {
        let manifest_path = fed_dir.join("manifest.toml");
        let mut doc: toml::Value = if manifest_path.exists() {
            let content = std::fs::read_to_string(&manifest_path)?;
            toml::from_str(&content).unwrap_or(toml::Value::Table(toml::map::Map::new()))
        } else {
            toml::Value::Table(toml::map::Map::new())
        };

        // Parse the new value
        let new_val: toml::Value = if value == "true" {
            toml::Value::Boolean(true)
        } else if value == "false" {
            toml::Value::Boolean(false)
        } else {
            toml::Value::String(value.clone())
        };

        // Navigate dot-separated path and set
        let parts: Vec<&str> = field.splitn(2, '.').collect();
        if parts.len() == 2 {
            let table = doc.as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("manifest root is not a table"))?;
            let subtable = table
                .entry(parts[0])
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            subtable.as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("'{}' is not a table", parts[0]))?
                .insert(parts[1].to_string(), new_val);
        } else {
            doc.as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("manifest root is not a table"))?
                .insert(field.clone(), new_val);
        }

        std::fs::create_dir_all(&fed_dir)?;
        let content = toml::to_string_pretty(&doc)
            .map_err(|e| anyhow::anyhow!("failed to serialize manifest: {}", e))?;
        std::fs::write(&manifest_path, &content)?;
        println!("manifest updated: {} = {}", field, value);
        return Ok(());
    }

    if let Commands::FederationConsentShow = &cli.command {
        let consent_dir = fed_dir.join("consent");
        let consent_path = consent_dir.join(format!("{}.toml", ai_id));
        if !consent_path.exists() {
            println!("# No consent record for {} — fully inheriting manifest.", ai_id);
            println!("# Override with: teambook federation-consent-update <field> <value>");
        } else {
            let content = std::fs::read_to_string(&consent_path)?;
            println!("{}", content);
        }
        return Ok(());
    }

    if let Commands::FederationConsentUpdate { field, value } = &cli.command {
        let consent_dir = fed_dir.join("consent");
        let consent_path = consent_dir.join(format!("{}.toml", ai_id));

        let mut doc: toml::map::Map<String, toml::Value> = if consent_path.exists() {
            let content = std::fs::read_to_string(&consent_path)?;
            let v: toml::Value = toml::from_str(&content)
                .map_err(|e| anyhow::anyhow!("failed to parse consent: {}", e))?;
            match v {
                toml::Value::Table(t) => t,
                _ => toml::map::Map::new(),
            }
        } else {
            toml::map::Map::new()
        };

        // Ensure ai_id is set
        doc.insert("ai_id".to_string(), toml::Value::String(ai_id.clone()));

        if value == "inherit" {
            // Remove the override — AI falls back to manifest
            doc.remove(field.as_str());
            println!("consent for '{}' removed — will inherit from manifest", field);
        } else {
            let new_val: toml::Value = if value == "true" {
                toml::Value::Boolean(true)
            } else if value == "false" {
                toml::Value::Boolean(false)
            } else {
                toml::Value::String(value.clone())
            };
            doc.insert(field.clone(), new_val);
            println!("consent updated: {} = {}", field, value);
        }

        std::fs::create_dir_all(&consent_dir)?;
        let content = toml::to_string_pretty(&toml::Value::Table(doc))
            .map_err(|e| anyhow::anyhow!("failed to serialize consent: {}", e))?;
        std::fs::write(&consent_path, &content)?;
        return Ok(());
    }

    // V2 EVENT SOURCING PATH
    // Enable via: --v2 flag OR TEAMENGRAM_V2=1 environment variable
    // The env var allows the MCP binary to enable V2 for all CLI subprocess calls
    let v2_enabled = cli.v2 || std::env::var("TEAMENGRAM_V2")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);  // V1 is default - V2 requires explicit opt-in

    if v2_enabled {
        return run_v2(&ai_id, cli.command);
    }

    // MIGRATION - runs independently, no daemon needed
    if let Commands::Migrate { old_store, v2_dir } = &cli.command {
        use teamengram::migration::run_migration;

        let base_dir = teamengram::store::ai_foundation_base_dir();

        let old_store_path = old_store.clone().unwrap_or_else(|| {
            base_dir.join("shared").join("teamengram").join("teamengram.engram")
                .to_string_lossy().to_string()
        });

        let v2_data_dir = v2_dir.clone().unwrap_or_else(|| {
            base_dir.join("v2").to_string_lossy().to_string()
        });

        println!("|MIGRATE TO EVENT STORE|");
        println!("Source:{}", old_store_path);
        println!("Destination:{}", v2_data_dir);

        match run_migration(&old_store_path, &v2_data_dir) {
            Ok(stats) => {
                println!("|MIGRATION SUCCESS|");
                println!("TotalEvents:{}", stats.total_events);
                println!("Errors:{}", stats.errors);
                return Ok(());
            }
            Err(e) => {
                eprintln!("Error: Migration failed: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Connect to TeamEngram daemon via IPC (no direct store access!)
    // This prevents B+Tree corruption from concurrent writes
    let mut client = TeamEngramClient::connect()
        .await
        .context("Failed to connect to TeamEngram daemon. Is it running?")?;

    // Store path still needed for some local operations (identity, standby)
    let store_path = TeamEngram::default_path();

    match cli.command {
        // ===== CORE MESSAGING =====

        Commands::Broadcast { content, channel, urgent } => {
            let id = client.broadcast(&channel, &content).await?;
            println!("broadcast|{}|{}|{}", id, channel, content);
            // BulletinBoard refresh now handled by daemon
            if urgent {
                if let Ok(presences) = client.get_active_ais().await {
                    for p in presences {
                        if p.ai_id != ai_id && is_ai_online(&p.ai_id) {
                            if let Ok(coord) = WakeCoordinator::new(&p.ai_id) {
                                coord.wake(WakeReason::Urgent, &ai_id, &content);
                            }
                        }
                    }
                }
            }
        }

        Commands::DirectMessage { to_ai, content } => {
            let id = client.direct_message(&to_ai, &content).await?;
            println!("dm_sent|{}|{}|{}", id, to_ai, content);
            // BulletinBoard refresh now handled by daemon
        }

        Commands::ReadMessages { limit, channel } => {
            let msgs = client.get_broadcasts(&channel, limit).await?;
            println!("|BROADCASTS|{}|{}", channel, msgs.len());
            for msg in msgs {
                // UTC timestamp for precise timing verification (QD requirement)
                println!("{}|{}|{}", msg.from_ai, to_utc(msg.created_at), msg.content);
            }
        }

        Commands::ReadDms { limit } => {
            let msgs = client.get_direct_messages(limit).await?;
            println!("|DIRECT MESSAGES|{}", msgs.len());
            for msg in msgs {
                // UTC timestamp for precise timing verification (QD requirement)
                println!("{}|{}|{}", msg.from_ai, to_utc(msg.created_at), msg.content);
            }
        }

        Commands::Status => {
            let presences = client.get_active_ais().await?;
            println!("|TEAM STATUS|");
            println!("Online:{}", presences.len());
            for p in presences {
                // Use clear words instead of ambiguous symbols (QD directive)
                let status_word = match p.status.as_str() {
                    "active" => "active",
                    "busy" => "busy",
                    "standby" => "standby",
                    "idle" => "idle",
                    _ => "online",
                };
                println!("{}|{}|{}", p.ai_id, status_word, p.current_task);
            }

        }

        // ===== DIALOGUES (4 Consolidated Commands) =====

        Commands::DialogueCreate { responder, topic } => {
            let id = client.dialogue_start(&responder, &topic).await?;
            println!("dialogue_created|{}|{}|{}", id, responder, topic);
        }

        Commands::DialogueRespond { dialogue_id, response } => {
            let success = client.dialogue_respond(dialogue_id, &response).await?;
            if success {
                acknowledge_dialogue(&ai_id, dialogue_id);
                println!("dialogue_responded|{}", dialogue_id);
            } else {
                println!("error|dialogue_not_found_or_not_your_turn|{}", dialogue_id);
            }
        }

        Commands::DialogueList { limit, filter, id } => {
            // If specific ID requested, show that dialogue's details
            if let Some(dialogue_id) = id {
                let dialogues = client.list_dialogues(100).await?;
                if let Some(d) = dialogues.iter().find(|d| d.id == dialogue_id) {
                    println!("|DIALOGUE|{}", dialogue_id);
                    println!("Initiator:{}", d.initiator);
                    println!("Participants:{}", d.participants.join(","));
                    println!("Topic:{}", d.topic);
                    println!("CurrentTurn:{}", d.current_turn);
                    println!("Status:{}", d.status);
                    // For messages, need V2
                    println!("(Use --v2 true for full message history)");
                } else {
                    println!("error|dialogue_not_found|{}", dialogue_id);
                }
            } else {
                // Filter-based listing
                match filter.as_str() {
                    "invites" => {
                        let invites = client.dialogue_invites(limit).await?;
                        println!("|DIALOGUE INVITES|{}", invites.len());
                        for d in invites {
                            println!("{}|from:{}|{}", d.id, d.initiator, d.topic);
                        }
                    }
                    "my-turn" => {
                        let dialogues = client.dialogue_my_turn(limit).await?;
                        println!("|YOUR TURN|{}", dialogues.len());
                        for d in dialogues {
                            let others: Vec<&str> = d.participants.iter()
                                .filter(|p| p.as_str() != ai_id)
                                .map(|s| s.as_str()).collect();
                            println!("{}|with:{}|turn:{}|{}", d.id, others.join(","), d.current_turn, d.topic);
                        }
                    }
                    _ => {
                        // "all" or default
                        let dialogues = client.list_dialogues(limit).await?;
                        println!("|DIALOGUES|{}", dialogues.len());
                        for d in dialogues {
                            println!("{}|{}|turn:{}|{}|{}", d.id, d.participants.join("↔"), d.current_turn, d.status, d.topic);
                        }
                    }
                }
            }
        }

        Commands::DialogueEnd { dialogue_id, status, summary, merge_into } => {
            // Handle merge if specified
            if let Some(_target_id) = merge_into {
                // V1 doesn't support merge
                println!("error|dialogue_merge_requires_v2|Use --v2 true for --merge-into");
            } else {
                // V1 client doesn't support summary, just ignore it
                let _ = summary;
                let success = client.dialogue_end(dialogue_id, &status).await?;
                if success {
                    println!("dialogue_ended|{}|{}", dialogue_id, status);
                } else {
                    println!("error|dialogue_not_found|{}", dialogue_id);
                }
            }
        }

        // ===== VOTING =====

        Commands::VoteCreate { topic, options, duration } => {
            let opts: Vec<String> = options.split(',').map(|s| s.trim().to_string()).collect();
            let id = client.create_vote(&topic, opts, duration).await?;
            println!("vote_created|{}|{}", id, topic);
        }

        Commands::VoteCast { vote_id, choice } => {
            let success = client.cast_vote(vote_id, &choice).await?;
            if success {
                println!("vote_cast|{}|{}", vote_id, choice);
            } else {
                println!("error|vote_failed|{}", vote_id);
            }
        }

        Commands::Votes { limit } => {
            let votes = client.list_votes(limit).await?;
            println!("|VOTES|{}", votes.len());
            for v in votes {
                println!("{}|{}|options:{}|by:{}", v.id, v.topic, v.options.join(","), v.created_by);
            }
        }

        Commands::VoteResults { vote_id } => {
            if let Some(v) = client.get_vote_results(vote_id).await? {
                println!("|VOTE|{}|RESULTS", vote_id);
                println!("Topic:{}", v.topic);
                println!("Options:{}", v.options.join(","));
                println!("CreatedBy:{}", v.created_by);
                println!("Status:{}", v.status);
            } else {
                println!("error|vote_not_found|{}", vote_id);
            }
        }

        Commands::VoteClose { vote_id } => {
            let success = client.vote_close(vote_id).await?;
            if success {
                println!("vote_closed|{}", vote_id);
            } else {
                println!("error|vote_close_failed|{}", vote_id);
            }
        }

        // ===== TASKS (4 Consolidated Commands) =====

        Commands::TaskCreate { description, tasks, priority } => {
            if let Some(ref _batch_tasks) = tasks {
                // Batch mode - V1 not supported
                println!("error|v1_not_supported|batch_create|{}|use --v2 true", description);
            } else {
                // Single task mode
                let prio: u8 = match priority.to_lowercase().as_str() {
                    "low" => 0,
                    "high" => 2,
                    "urgent" => 3,
                    _ => 1, // normal
                };
                let id = client.queue_task(&description, prio, "").await?;
                println!("task_created|{}|{}", id, description);
            }
        }

        Commands::TaskUpdate { id, status, reason } => {
            let status_lower = status.to_lowercase();

            // Check if batch reference (contains :) or batch name (non-numeric)
            if id.contains(':') {
                // Batch task reference like "Auth:1" - V1 not supported
                println!("error|v1_not_supported|batch_task_update|{}|use --v2 true", id);
            } else if let Ok(task_id) = id.parse::<u64>() {
                // Numeric ID - single task
                match status_lower.as_str() {
                    "done" | "completed" => {
                        let success = client.complete_task(task_id, "completed").await?;
                        if success {
                            println!("task_updated|{}|{}", task_id, status);
                        } else {
                            println!("error|task_update_failed|{}", task_id);
                        }
                    }
                    "claimed" => {
                        if let Some(_) = client.claim_task(Some(task_id)).await? {
                            println!("task_updated|{}|claimed", task_id);
                        } else {
                            println!("error|task_claim_failed|{}", task_id);
                        }
                    }
                    "started" | "in_progress" => {
                        if let Some(_) = client.claim_task(Some(task_id)).await? {
                            println!("task_updated|{}|started", task_id);
                        } else {
                            println!("error|task_start_failed|{}", task_id);
                        }
                    }
                    "blocked" => {
                        let reason_str = reason.as_deref().unwrap_or("blocked");
                        println!("error|v1_not_implemented|task_block|{}|{}", task_id, reason_str);
                    }
                    _ => {
                        println!("error|unknown_status|{}|{}", task_id, status);
                    }
                }
            } else {
                // Non-numeric, no colon - batch name for close - V1 not supported
                println!("error|v1_not_supported|batch_close|{}|use --v2 true", id);
            }
        }

        Commands::TaskGet { id } => {
            if let Ok(task_id) = id.parse::<u64>() {
                // Numeric - single task
                let tasks = client.list_tasks(100, false).await?;
                if let Some(t) = tasks.iter().find(|t| t.id == task_id) {
                    println!("|TASK|{}", task_id);
                    println!("Description:{}", t.description);
                    println!("Status:{}", t.status);
                    println!("Priority:{}", t.priority);
                    println!("CreatedBy:{}", t.created_by);
                    if let Some(claimed) = &t.claimed_by {
                        println!("ClaimedBy:{}", claimed);
                    }
                    if !t.tags.is_empty() {
                        println!("Tags:{}", t.tags);
                    }
                } else {
                    println!("error|task_not_found|{}", task_id);
                }
            } else {
                // Non-numeric - batch name - V1 not supported
                println!("error|v1_not_supported|batch_get|{}|use --v2 true", id);
            }
        }

        Commands::TaskList { count, limit_flag, filter } => {
            let limit = limit_flag.or(count).unwrap_or(20);
            match filter.to_lowercase().as_str() {
                "batches" => {
                    println!("error|v1_not_supported|batches|use --v2 true");
                }
                "tasks" | "all" | _ => {
                    let tasks = client.list_tasks(limit, false).await?;
                    println!("|TASKS|{}", tasks.len());
                    for t in tasks {
                        let claimed = t.claimed_by.as_deref().unwrap_or("-");
                        println!("{}|{}|{}|by:{}|{}", t.id, t.status, t.description, t.created_by, claimed);
                    }
                }
            }
        }

        Commands::WhatDoing { limit: _ } => {
            // V1 doesn't have presence tracking
            println!("error|not_implemented|what_doing|use_v2");
        }

        // ===== FILE CLAIMS =====

        Commands::ClaimFile { path, working_on, duration } => {
            let id = client.claim_file(&path, &working_on, duration).await?;
            println!("file_claimed|{}|{}", id, path);
        }

        Commands::CheckFile { path } => {
            if let Some((claimer, working_on)) = client.is_file_claimed(&path).await? {
                if working_on.is_empty() {
                    println!("claimed|{}|{}", claimer, path);
                } else {
                    println!("claimed|{}|{}|{}", claimer, path, working_on);
                }
            } else {
                println!("unclaimed|{}", path);
            }
        }

        Commands::ReleaseFile { path } => {
            let success = client.release_file(&path).await?;
            if success {
                println!("file_released|{}", path);
            } else {
                println!("error|release_failed|{}", path);
            }
        }

        Commands::ListClaims { limit: _ } => {
            let claims = client.get_active_claims().await?;
            println!("|FILE CLAIMS|{}", claims.len());
            for c in claims {
                println!("{}|{}|{}|{}", c.id, c.claimer, c.path, c.working_on);
            }
        }
        Commands::LogAction { action, path } => {
            client.log_file_action(&path, &action).await?;
            println!("logged|{}|{}", action, path);
        }
        Commands::FileActions { limit } => {
            let actions = client.get_recent_file_actions(limit).await?;
            if actions.is_empty() {
                println!("No recent file actions");
            } else {
                for a in actions {
                    let id = a.id.unwrap_or(0);
                    println!("{}|{}|{}|{}", id, a.ai_id, a.action, a.path);
                }
            }
        }


        // Lock commands removed — deprecated (Feb 2026)

        // ===== ROOMS =====

        Commands::RoomCreate { name, topic } => {
            let id = client.create_room(&name, &topic).await?;
            println!("room_created|{}|{}", id, name);
        }

        Commands::RoomJoin { room_id } => {
            let success = client.join_room(room_id).await?;
            if success {
                println!("room_joined|{}", room_id);
            } else {
                println!("error|room_join_failed|{}", room_id);
            }
        }

        Commands::Rooms { limit } => {
            let rooms = client.list_rooms(limit).await?;
            println!("|ROOMS|{}", rooms.len());
            for r in rooms {
                println!("{}|{}|{}|by:{}", r.id, r.name, r.topic, r.creator);
            }
        }

        Commands::RoomGet { room_id } => {
            if let Some(r) = client.room_get(room_id).await? {
                println!("|ROOM|{}", room_id);
                println!("Name:{}", r.name);
                println!("Topic:{}", r.topic);
                println!("Creator:{}", r.creator);
                println!("Members:{}", r.participants.join(","));
            } else {
                println!("error|room_not_found|{}", room_id);
            }
        }

        Commands::RoomLeave { room_id } => {
            let success = client.leave_room(room_id).await?;
            if success {
                println!("room_left|{}", room_id);
            } else {
                println!("error|not_in_room_or_not_found|{}", room_id);
            }
        }

        Commands::RoomClose { room_id } => {
            let success = client.room_close(room_id).await?;
            if success {
                println!("room_closed|{}", room_id);
            } else {
                println!("error|not_creator_or_not_found|{}", room_id);
            }
        }

        Commands::RoomSay { .. } | Commands::RoomMessages { .. }
        | Commands::RoomMute { .. } | Commands::RoomConclude { .. }
        | Commands::RoomPinMessage { .. } | Commands::RoomUnpinMessage { .. } => {
            anyhow::bail!("Room commands require V2 backend. Use --v2 true (default)");
        }

        // ===== UTILITIES =====

        Commands::UpdatePresence { status, task } => {
            client.update_presence(&status, &task).await?;
            println!("presence|{}|{}|{}", ai_id, status, task);
        }

        Commands::Stats => {
            let stats = client.stats().await?;
            println!("TEAMENGRAM STATS");
            println!("File size:     {} KB", stats.file_size / 1024);
            println!("Total pages:   {}", stats.total_pages);
            println!("Used pages:    {}", stats.used_pages);
            println!("Transactions:  {}", stats.txn_id);
            println!();
            println!("Backend: TeamEngram (B+Tree + Shadow Paging)");
            println!("IPC: Daemon-only writes (corruption-free)");
        }

        Commands::Benchmark { count } => {
            use std::time::Instant;

            println!("|BENCHMARK|");
            println!("Operations:{}", count);
            println!("Mode: IPC to daemon (single writer)");

            // DM inserts via IPC
            let start = Instant::now();
            for i in 0..count {
                client.direct_message("bench-to", &format!("Msg {}", i)).await?;
            }
            let dm_time = start.elapsed();
            println!("DMInsert:{:.2}ops/sec", count as f64 / dm_time.as_secs_f64());

            // Broadcast inserts via IPC
            let start = Instant::now();
            for i in 0..count {
                client.broadcast("bench", &format!("BC {}", i)).await?;
            }
            let bc_time = start.elapsed();
            println!("BCInsert:{:.2}ops/sec", count as f64 / bc_time.as_secs_f64());

            // DM reads via IPC
            let start = Instant::now();
            for _ in 0..count {
                let _ = client.get_direct_messages(10).await?;
            }
            let read_time = start.elapsed();
            println!("DMRead:{:.2}ops/sec", count as f64 / read_time.as_secs_f64());

            let stats = client.stats().await?;
            println!("FinalSize:{}KB", stats.file_size / 1024);
        }

        Commands::Standby { timeout } => {
            println!("standby|{}|timeout={}s|mode=event-driven", ai_id, timeout);

            // CRITICAL FIX: Check for pre-existing unread items BEFORE entering standby!
            // This prevents deadlock when two AIs send DMs to each other and both enter standby.

            // Check for unread DMs (filter by replied_to to skip senders we've replied to)
            let replied_to = get_replied_to(&ai_id);
            if let Ok(unread_dms) = client.get_unread_dms(10).await {
                // Filter out senders we've already replied to
                let unreplied: Vec<_> = unread_dms.iter()
                    .filter(|dm| !replied_to.contains(&dm.from_ai))
                    .collect();
                if !unreplied.is_empty() {
                    let dm = unreplied[0];
                    println!("wake|{}|dm|from={}|{}", ai_id, dm.from_ai, dm.content);
                    // Don't enter standby - already have pending DM
                    return Ok(());
                }
            }

            // Check for dialogue turns where it's my turn (filter acknowledged)
            let ack_dialogues = get_acknowledged_dialogues(&ai_id);
            if let Ok(my_turns) = client.dialogue_my_turn(10).await {
                let unacked: Vec<_> = my_turns.iter()
                    .filter(|d| !ack_dialogues.contains(&d.id))
                    .collect();
                if !unacked.is_empty() {
                    let d = unacked[0];
                    println!("wake|{}|dialogue|from={}|Turn in dialogue #{}", ai_id, d.initiator, d.id);
                    return Ok(());
                }
            }

            // Check for dialogue invites (filter acknowledged)
            if let Ok(invites) = client.dialogue_invites(10).await {
                let unacked: Vec<_> = invites.iter()
                    .filter(|inv| !ack_dialogues.contains(&inv.id))
                    .collect();
                if !unacked.is_empty() {
                    let inv = unacked[0];
                    println!("wake|{}|dialogue|from={}|Invited to dialogue #{}: {}", ai_id, inv.initiator, inv.id, inv.topic);
                    return Ok(());
                }
            }

            // No pending items - safe to enter standby
            client.update_presence("standby", "").await?;

            // Create wake coordinator for this AI - blocks until signaled
            let coordinator = WakeCoordinator::new(&ai_id)
                .context("Failed to create wake coordinator")?;

            let timeout_duration = if timeout > 0 {
                std::time::Duration::from_secs(timeout)
            } else {
                std::time::Duration::from_secs(180) // Default 3 minute max
            };

            // Block until wake event or timeout - NO POLLING!
            match coordinator.wait_timeout(timeout_duration) {
                Some(result) => {
                    let reason_str = match result.reason {
                        WakeReason::DirectMessage => "dm",
                        WakeReason::Mention => "mention",
                        WakeReason::Urgent => "urgent",
                        WakeReason::TaskAssigned => "task",
                        WakeReason::Broadcast => "broadcast",
                        WakeReason::DialogueTurn => "dialogue",
                        WakeReason::VoteRequest => "vote",
                        WakeReason::FileReleased => "file_released",
                        WakeReason::Manual => "manual",
                        WakeReason::None => "unknown",
                    };
                    let from = result.from_ai.unwrap_or_default();
                    let content = result.content_preview.unwrap_or_default();
                    println!("wake|{}|{}|from={}|{}", ai_id, reason_str, from, content);
                    client.update_presence("active", "").await?;
                }
                None => {
                    println!("standby_timeout|{}", ai_id);
                }
            }
        }

        // ===== ADDITIONAL UTILITIES =====

        Commands::IdentityShow => {
            // Show AI identity info
            let fingerprint = hash_ai_id(&ai_id);
            println!("|IDENTITY|");
            println!("AI:{}", ai_id);
            println!("Fingerprint:{}", fingerprint);
            println!("Store:{}", store_path.display());
        }

        Commands::MyPresence => {
            // Show current AI's presence status via IPC
            if let Some(p) = client.get_presence(&ai_id).await? {
                println!("|MY PRESENCE|");
                println!("AI:{}", p.ai_id);
                println!("Status:{}", p.status);
                println!("Task:{}", p.current_task);
            } else {
                println!("|MY PRESENCE|");
                println!("AI:{}", ai_id);
                println!("Status:unknown");
                println!("Task:none");
            }
        }

        Commands::GetPresence { ai_id: target_ai } => {
            // Get another AI's presence status via IPC
            if let Some(p) = client.get_presence(&target_ai).await? {
                println!("|PRESENCE|");
                println!("AI:{}", p.ai_id);
                println!("Status:{}", p.status);
                println!("Task:{}", p.current_task);
                println!("LastSeen:{}", p.last_seen);
            } else {
                println!("|PRESENCE|");
                println!("AI:{}", target_ai);
                println!("Status:offline");
            }
        }

        Commands::PresenceCount => {
            // Count unique online AIs via IPC
            let presences = client.get_active_ais().await?;
            println!("Online:{}", presences.len());
        }

        // Stigmergy commands removed — deprecated (Feb 2026)

        Commands::Awareness { limit } => {
            // Output format expected by PostToolUse hook - via IPC
            // dm|id|from|content
            // bc|id|from|channel|content
            // vote|id|topic|cast|total
            // dialogue|id|topic (dialogues where it's your turn)
            // claim|path|owner|reason

            // DMs for this AI
            let dms = client.get_direct_messages(limit).await?;
            for dm in dms {
                println!("dm|{}|{}|{}", dm.id, dm.from_ai, dm.content);
            }

            // Recent broadcasts
            let bcs = client.get_broadcasts("general", limit).await?;
            for bc in bcs {
                println!("bc|{}|{}|{}|{}", bc.id, bc.from_ai, bc.channel, bc.content);
            }

            // Open votes
            let votes = client.list_votes(limit).await?;
            for v in votes {
                if v.status == "open" {
                    println!("vote|{}|{}|0|3", v.id, v.topic);
                }
            }

            // Dialogues where it's your turn
            let dialogues = client.dialogue_my_turn(limit).await?;
            for d in dialogues {
                println!("dialogue|{}|{}", d.id, d.topic);
            }

            // Active locks - would need a list_locks method
            // Skipping for now as client doesn't have this method
        }

        Commands::Migrate { .. } => {
            // Handled before daemon connect - this is unreachable
            unreachable!("Migrate command handled before daemon connect");
        }

        Commands::OutboxRepair { .. } => {
            // Handled before daemon connect - this is unreachable
            unreachable!("OutboxRepair command handled before daemon connect");
        }

        Commands::RefreshBulletin => {
            // V1 does not need bulletin refresh - daemon handles it
            println!("bulletin_refresh|v1_noop");
        }

        // ===== PROJECTS =====

        Commands::ListProjects => {
            let projects = client.list_projects().await?;
            println!("|PROJECTS|{}", projects.len());
            for p in projects {
                println!("{}|{}|{}|{}", p.id, p.name, p.status, p.goal);
            }
        }

        Commands::ProjectCreate { name, goal, directory } => {
            let id = client.create_project(&name, &goal, &directory).await?;
            println!("project_created|{}|{}", id, name);
        }

        Commands::ProjectGet { project_id } => {
            if let Some(p) = client.get_project(project_id).await? {
                println!("|PROJECT|{}", p.id);
                println!("Name:{}", p.name);
                println!("Goal:{}", p.goal);
                println!("Status:{}", p.status);
                println!("Directory:{}", p.root_directory);
            } else {
                println!("project_not_found|{}", project_id);
            }
        }

        Commands::ProjectDelete { project_id } => {
            if client.delete_project(project_id).await? {
                println!("project_deleted|{}", project_id);
            } else {
                println!("project_delete_failed|{}", project_id);
            }
        }

        Commands::ProjectRestore { project_id } => {
            if client.restore_project(project_id).await? {
                println!("project_restored|{}", project_id);
            } else {
                println!("project_restore_failed|{}", project_id);
            }
        }

        Commands::ProjectTasks { project_id } => {
            // Need to get tasks for a project - use task list with filter
            // For now, just show the project exists
            if let Some(p) = client.get_project(project_id).await? {
                println!("|PROJECT TASKS|{}|{}", p.id, p.name);
                // TODO: Add get_tasks_for_project to client
                println!("tasks_not_implemented_in_v1");
            } else {
                println!("project_not_found|{}", project_id);
            }
        }

        Commands::ProjectAddTask { project_id, title, priority } => {
            // Need to add task linked to project
            // For now, just verify project exists
            if let Some(_p) = client.get_project(project_id).await? {
                // TODO: Add create_task_for_project to client
                println!("add_task_not_implemented_in_v1|{}|{}", project_id, title);
                let _ = priority; // suppress warning
            } else {
                println!("project_not_found|{}", project_id);
            }
        }

        Commands::ProjectResolve { path } => {
            if let Some((proj_id, feat_id)) = client.resolve_file_to_project(&path).await? {
                if let Some(fid) = feat_id {
                    println!("resolved|project={}|feature={}", proj_id, fid);
                } else {
                    println!("resolved|project={}|feature=none", proj_id);
                }
            } else {
                println!("not_resolved|{}", path);
            }
        }

        // ===== FEATURES =====

        Commands::ListFeatures { project_id } => {
            let features = client.list_features(project_id).await?;
            println!("|FEATURES|{}|{}", project_id, features.len());
            for f in features {
                println!("{}|{}|{}", f.id, f.name, f.overview);
            }
        }

        Commands::FeatureCreate { project_id, name, overview, directory } => {
            let id = client.create_feature(project_id, &name, &overview, directory.as_deref()).await?;
            println!("feature_created|{}|{}|{}", id, project_id, name);
        }

        Commands::FeatureGet { feature_id } => {
            if let Some(f) = client.get_feature(feature_id).await? {
                println!("|FEATURE|{}", f.id);
                println!("Name:{}", f.name);
                println!("Overview:{}", f.overview);
                println!("ProjectId:{}", f.project_id);
                if let Some(dir) = &f.directory {
                    println!("Directory:{}", dir);
                }
            } else {
                println!("feature_not_found|{}", feature_id);
            }
        }

        Commands::FeatureDelete { feature_id } => {
            if client.delete_feature(feature_id).await? {
                println!("feature_deleted|{}", feature_id);
            } else {
                println!("feature_delete_failed|{}", feature_id);
            }
        }

        Commands::FeatureRestore { feature_id } => {
            if client.restore_feature(feature_id).await? {
                println!("feature_restored|{}", feature_id);
            } else {
                println!("feature_restore_failed|{}", feature_id);
            }
        }

        // FeatureUpdate and ProjectUpdate are V2-only
        Commands::FeatureUpdate { .. } |
        Commands::ProjectUpdate { .. } => {
            eprintln!("Error: Update commands require V2 backend");
            eprintln!("Hint: V2 is on by default. If you see this, check your configuration.");
            std::process::exit(1);
        }

        // Learning commands are V2-only (event-sourced)
        Commands::Learning { .. } |
        Commands::LearningUpdate { .. } |
        Commands::LearningDelete { .. } |
        Commands::MyLearnings |
        Commands::TeamPlaybook { .. } => {
            eprintln!("Error: Learning commands require V2 backend");
            eprintln!("Hint: Learning is on by default. If you see this, check your configuration.");
            std::process::exit(1);
        }

        // Trust commands are V2-only (event-sourced)
        Commands::TrustRecord { .. } |
        Commands::TrustScore { .. } |
        Commands::TrustScores => {
            eprintln!("Error: Trust commands require V2 backend");
            eprintln!("Hint: TIP (Trust Inference and Propagation) is V2 only.");
            std::process::exit(1);
        }

        // Hook commands are V2-only
        Commands::HookPostToolUse |
        Commands::HookSessionStart => {
            eprintln!("Error: Hook commands require V2 backend");
            eprintln!("Hint: Use --v2 true (which is the default)");
            std::process::exit(1);
        }

        // Context gathering is V2-only (uses V2 event sourcing data)
        Commands::GatherContext { .. } => {
            eprintln!("Error: GatherContext requires V2 backend (event sourcing)");
            eprintln!("Hint: Use --v2 true (which is the default)");
            std::process::exit(1);
        }

        // Federation config commands return early before this match is reached
        Commands::FederationManifestShow |
        Commands::FederationManifestSet { .. } |
        Commands::FederationConsentShow |
        Commands::FederationConsentUpdate { .. } => {
            unreachable!("federation config commands are handled before V2 path")
        }

        // Mobile pairing — HTTP call to mobile-api, works regardless of V1/V2
        Commands::MobilePair { code } => {
            use std::io::{Read, Write};
            use std::net::TcpStream;

            let port = std::env::var("MOBILE_API_PORT").unwrap_or_else(|_| "8081".to_string());
            let addr = format!("127.0.0.1:{}", port);
            let mut stream = TcpStream::connect(&addr)
                .map_err(|e| anyhow::anyhow!("Cannot connect to mobile-api at {}: {}", addr, e))?;

            let body = format!("{{\"code\":\"{}\"}}", code);
            let request = format!(
                "POST /api/pair/approve HTTP/1.1\r\nHost: localhost:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                port, body.len(), body
            );
            stream.write_all(request.as_bytes())?;

            let mut response = String::new();
            stream.read_to_string(&mut response)?;

            if let Some(body_start) = response.find("\r\n\r\n") {
                let resp_body = &response[body_start + 4..];
                match serde_json::from_str::<serde_json::Value>(resp_body) {
                    Ok(json) if json["ok"].as_bool().unwrap_or(false) => {
                        let h_id = json["h_id"].as_str().unwrap_or("unknown");
                        println!("pair_approved|{}|{}", code, h_id);
                    }
                    Ok(json) => {
                        let error = json["error"].as_str().unwrap_or("unknown error");
                        eprintln!("pair_failed|{}|{}", code, error);
                        std::process::exit(1);
                    }
                    Err(_) => {
                        eprintln!("pair_failed|{}|invalid JSON in response", code);
                        std::process::exit(1);
                    }
                }
            } else {
                eprintln!("pair_failed|{}|malformed HTTP response", code);
                std::process::exit(1);
            }
        }
    }

    // No flush needed - daemon handles persistence
    Ok(())
}

// ============================================================================
// V2 EVENT SOURCING HANDLERS
// ============================================================================

/// Refresh BulletinBoard with latest V2 data for passive injection (Awareness)
/// Called after V2 write operations to update shared memory for hook-bulletin


/// Shared state struct for hook state file (matches hook-bulletin.rs)
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct HookState {
    #[serde(default)]
    dm_ids: std::collections::HashSet<i64>,
    #[serde(default)]
    broadcast_ids: std::collections::HashSet<i64>,
    #[serde(default)]
    last_sequence: u64,
    #[serde(default)]
    pending_senders: std::collections::HashSet<String>,
    #[serde(default)]
    replied_to: std::collections::HashSet<String>,
    #[serde(default)]
    acknowledged_dialogues: std::collections::HashSet<u64>,
}

/// Get the hook state file path for an AI
fn hook_state_path(ai_id: &str) -> std::path::PathBuf {
    teamengram::store::ai_foundation_base_dir()
        .join("state")
        .join(format!("hook_{}.json", ai_id))
}

/// Load replied_to from hook state file
/// Returns empty set if file doesn't exist or can't be read
fn get_replied_to(ai_id: &str) -> std::collections::HashSet<String> {
    let state_path = hook_state_path(ai_id);
    std::fs::read_to_string(&state_path)
        .ok()
        .and_then(|json| serde_json::from_str::<HookState>(&json).ok())
        .map(|state| state.replied_to)
        .unwrap_or_default()
}

/// Load acknowledged_dialogues from hook state file
fn get_acknowledged_dialogues(ai_id: &str) -> std::collections::HashSet<u64> {
    let state_path = hook_state_path(ai_id);
    std::fs::read_to_string(&state_path)
        .ok()
        .and_then(|json| serde_json::from_str::<HookState>(&json).ok())
        .map(|state| state.acknowledged_dialogues)
        .unwrap_or_default()
}

/// Mark a dialogue as acknowledged so it won't trigger instant wake
fn acknowledge_dialogue(ai_id: &str, dialogue_id: u64) {
    let state_path = hook_state_path(ai_id);
    let mut state = match std::fs::read_to_string(&state_path) {
        Ok(json) => serde_json::from_str::<HookState>(&json).unwrap_or_default(),
        Err(_) => HookState::default(), // File doesn't exist yet — start fresh
    };
    state.acknowledged_dialogues.insert(dialogue_id);
    match serde_json::to_string(&state) {
        Ok(new_json) => {
            if let Err(e) = std::fs::write(&state_path, &new_json) {
                eprintln!("[HOOK] Failed to save dialogue ack state: {}", e);
            }
        }
        Err(e) => eprintln!("[HOOK] Failed to serialize dialogue ack state: {}", e),
    }
}

/// Clear a sender from pending_senders and add to replied_to when we reply to them
/// This ensures DMs stop showing and don't get re-added by hook-bulletin
fn clear_pending_sender(ai_id: &str, recipient: &str) {
    let state_path = hook_state_path(ai_id);

    // Load, modify, save
    let mut state = match std::fs::read_to_string(&state_path) {
        Ok(json) => serde_json::from_str::<HookState>(&json).unwrap_or_default(),
        Err(_) => HookState::default(), // File doesn't exist yet — start fresh
    };

    // Remove from pending and add to replied_to
    state.pending_senders.remove(recipient);
    state.replied_to.insert(recipient.to_string());

    // Always save (replied_to was added even if pending_senders didn't change)
    match serde_json::to_string(&state) {
        Ok(new_json) => {
            if let Err(e) = std::fs::write(&state_path, &new_json) {
                eprintln!("[HOOK] Failed to save pending sender state: {}", e);
            }
        }
        Err(e) => eprintln!("[HOOK] Failed to serialize pending sender state: {}", e),
    }
}

fn refresh_bulletin_v2(v2: &mut V2Client, ai_id: &str) {
    if let Ok(mut bulletin) = BulletinBoard::open(None) {
        // DMs: Get ALL incoming DMs (dedup handled by hook-bulletin's seen state)
        // Removed pending_senders filter - it was blocking DM notifications
        if let Ok(dms) = v2.recent_dms(20) {
            let dm_data: Vec<_> = dms.iter()
                .filter(|m| m.to_ai.as_ref().map(|to| to == ai_id).unwrap_or(false))
                .take(10)
                .map(|m| (m.id as i64, m.timestamp.timestamp(), m.from_ai.as_str(), m.to_ai.as_deref().unwrap_or(""), m.content.as_str()))
                .collect();
            bulletin.set_dms(&dm_data);
        }

        // Broadcasts: Get recent broadcasts
        if let Ok(broadcasts) = v2.recent_broadcasts(10, None) {
            let bc_data: Vec<_> = broadcasts.iter()
                .map(|b| (b.id as i64, b.timestamp.timestamp(), b.from_ai.as_str(), b.channel.as_str(), b.content.as_str()))
                .collect();
            bulletin.set_broadcasts(&bc_data);
        }

        // Dialogues (your turn): Get dialogues where it's my turn
        // V2 tuple: (id, initiator, responder, topic, turn, status)
        if let Ok(dialogues) = v2.get_dialogue_my_turn() {
            let dialogue_data: Vec<_> = dialogues.iter()
                .map(|(id, _, _, topic, _, _)| (*id as i64, topic.as_str()))
                .collect();
            bulletin.set_dialogues(&dialogue_data);
        }

        // Votes: Get open votes I haven't voted on yet
        // V2 tuple: (id, creator, topic, options, status, votes_vec)
        if let Ok(votes) = v2.get_votes() {
            let vote_data: Vec<_> = votes.iter()
                .filter(|(_, _, _, _, status, votes_vec)| {
                    status == "open" && !votes_vec.iter().any(|(voter, _)| voter == ai_id)
                })
                .map(|(id, _, topic, options, _, votes_vec)| {
                    (*id as i64, topic.as_str(), votes_vec.len() as u32, options.len() as u32)
                })
                .collect();
            bulletin.set_votes(&vote_data);
        }

        // File actions: Get recent file actions
        if let Ok(actions) = v2.get_file_actions(10) {
            let fa_data: Vec<_> = actions.iter()
                .map(|(ai, action, path, ts)| (ai.as_str(), action.as_str(), path.as_str(), *ts / 1000))
                .collect();
            bulletin.set_file_actions(&fa_data);
        }

        // File claims (active)
        // V2 tuple: (path, ai_id, timestamp, duration)
        if let Ok(claims) = v2.get_claims() {
            let lock_data: Vec<_> = claims.iter()
                .map(|(path, ai, _ts, _dur, _working_on)| (path.as_str(), ai.as_str(), "claimed"))
                .collect();
            bulletin.set_locks(&lock_data);
        }

        // Commit all changes
        if let Err(e) = bulletin.commit() {
            eprintln!("[HOOK] Failed to commit bulletin board: {}", e);
        }
    }
}

/// Run command using V2 event sourcing backend
/// Each AI writes to local outbox, Sequencer aggregates to master log
fn run_v2(ai_id: &str, command: Commands) -> Result<()> {
    // Load encryption key from default V2 data directory (None = plaintext)
    let v2_dir = teamengram::store::ai_foundation_base_dir().join("v2");
    let crypto = teamengram::crypto::load_encryption_key(&v2_dir)
        .ok()
        .flatten()
        .map(std::sync::Arc::new);

    // Open V2 client for this AI
    let mut v2 = V2Client::open(ai_id, None, crypto)
        .map_err(|e| anyhow::anyhow!("V2 client error: {}", e))?;

    // Sync view with event log to get latest state
    if let Err(e) = v2.sync() {
        eprintln!("warn: v2 sync failed (stale state): {}", e);
    }

    match command {
        // ===== CORE MESSAGING =====

        Commands::Broadcast { content, channel, urgent } => {
            let seq = v2.broadcast(&channel, &content)
                .map_err(|e| anyhow::anyhow!("Broadcast error: {}", e))?;
            println!("broadcast|{}|{}|{}", seq, channel, content);

            // Wake any @mentioned AIs
            for word in content.split_whitespace() {
                if word.starts_with('@') {
                    let mentioned = word.trim_start_matches('@').trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-');
                    if !mentioned.is_empty() && mentioned != ai_id && is_ai_online(mentioned) {
                        if let Ok(coord) = WakeCoordinator::new(mentioned) {
                            // NO TRUNCATION - full content always
                            coord.wake(WakeReason::Mention, ai_id, &content);
                        }
                    }
                }
            }

            // --urgent: wake ALL online AIs (except self) with Urgent reason.
            // Default broadcast does not wake standby AIs — that's standby sanctity.
            // Sender opts in explicitly when the message is time-critical.
            if urgent {
                if let Ok(presences) = v2.get_presences() {
                    for (target_ai, _status, _task) in presences {
                        if target_ai != ai_id && is_ai_online(&target_ai) {
                            if let Ok(coord) = WakeCoordinator::new(&target_ai) {
                                coord.wake(WakeReason::Urgent, ai_id, &content);
                            }
                        }
                    }
                }
            }

            // Refresh bulletin for passive injection
            refresh_bulletin_v2(&mut v2, ai_id);
        }

        Commands::DirectMessage { to_ai, content } => {
            let seq = v2.direct_message(&to_ai, &content)
                .map_err(|e| anyhow::anyhow!("DM error: {}", e))?;
            println!("dm_sent|{}|{}|{}", seq, to_ai, content);

            // Wake the recipient if they're online and in standby
            // NO TRUNCATION - full content always
            if is_ai_online(&to_ai) {
                if let Ok(coord) = WakeCoordinator::new(&to_ai) {
                    coord.wake(WakeReason::DirectMessage, ai_id, &content);
                }
            }

            // Clear recipient from pending so their old DMs stop showing in injection
            // (backup for event-derived pending which may have sync issues)
            clear_pending_sender(ai_id, &to_ai);

            // Refresh bulletin for passive injection
            refresh_bulletin_v2(&mut v2, ai_id);
        }

        Commands::ReadMessages { limit, channel } => {
            let msgs = v2.recent_broadcasts(limit, Some(&channel))
                .map_err(|e| anyhow::anyhow!("Messages error: {}", e))?;
            println!("|BROADCASTS|{}|{}", channel, msgs.len());
            for msg in msgs {
                let ts_millis = msg.timestamp.timestamp_millis() as u64;
                // UTC timestamp for precise timing verification (QD requirement)
                println!("{}|{}|{}", msg.from_ai, to_utc(ts_millis), msg.content);
            }
        }

        Commands::ReadDms { limit } => {
            let msgs = v2.recent_dms(limit)
                .map_err(|e| anyhow::anyhow!("DMs error: {}", e))?;
            println!("|DIRECT MESSAGES|{}", msgs.len());
            for msg in msgs {
                let ts_millis = msg.timestamp.timestamp_millis() as u64;
                // UTC timestamp for precise timing verification (QD requirement)
                println!("from|{}|{}|{}", msg.from_ai, to_utc(ts_millis), msg.content);
            }
        }

        // ===== DIALOGUES (4 Consolidated Commands - V2) =====

        Commands::DialogueCreate { responder, topic } => {
            // Support comma-separated AI IDs for n-party dialogues
            let responder_parts: Vec<&str> = responder.split(',')
                .map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            let seq = v2.start_dialogue(&responder_parts, &topic)
                .map_err(|e| anyhow::anyhow!("Dialogue error: {}", e))?;
            println!("dialogue_created|{}|{}|{}", seq, responder, topic);

            // Wake all non-initiator participants
            for r in &responder_parts {
                if is_ai_online(r) {
                    if let Ok(coord) = WakeCoordinator::new(r) {
                        coord.wake(WakeReason::DialogueTurn, ai_id, &format!("Dialogue: {}", topic));
                    }
                }
            }
        }

        Commands::DialogueRespond { dialogue_id, response } => {
            // Get dialogue info before responding (to know who to wake)
            let other_party = v2.get_dialogue(dialogue_id)
                .ok()
                .flatten()
                .map(|(_, initiator, responder, topic, _, _)| {
                    let other = if initiator == ai_id { responder } else { initiator };
                    (other, topic)
                });

            let seq = v2.respond_dialogue(dialogue_id, &response)
                .map_err(|e| anyhow::anyhow!("Dialogue respond error: {}", e))?;
            acknowledge_dialogue(&ai_id, dialogue_id);
            println!("dialogue_responded|{}|{}", dialogue_id, seq);

            // Wake the other party to notify them it's their turn
            // NO TRUNCATION - full content always
            if let Some((other_ai, topic)) = other_party {
                if is_ai_online(&other_ai) {
                    if let Ok(coord) = WakeCoordinator::new(&other_ai) {
                        coord.wake(WakeReason::DialogueTurn, ai_id, &format!("Re: {} - {}", topic, response));
                    }
                }
            }
        }

        Commands::DialogueEnd { dialogue_id, status, summary, merge_into } => {
            // Handle merge if specified
            if let Some(target_id) = merge_into {
                let seq = v2.merge_dialogues(dialogue_id, target_id)
                    .map_err(|e| anyhow::anyhow!("V2 dialogue merge error: {}", e))?;
                println!("dialogue_merged|{}->{}|seq:{}", dialogue_id, target_id, seq);
            } else {
                let seq = v2.end_dialogue_with_summary(dialogue_id, &status, summary.as_deref())
                    .map_err(|e| anyhow::anyhow!("V2 dialogue end error: {}", e))?;
                println!("dialogue_ended|{}|{}", dialogue_id, seq);
            }
        }

        // ===== VOTING =====

        Commands::VoteCreate { topic, options, duration: _ } => {
            let opts: Vec<String> = options.split(',').map(|s| s.trim().to_string()).collect();
            let seq = v2.create_vote(&topic, opts, 3) // 3 voters default
                .map_err(|e| anyhow::anyhow!("V2 vote create error: {}", e))?;
            println!("vote_created|{}|{}", seq, topic);
        }

        Commands::VoteCast { vote_id, choice } => {
            let seq = v2.cast_vote(vote_id, &choice)
                .map_err(|e| anyhow::anyhow!("V2 vote cast error: {}", e))?;
            println!("vote_cast|{}|{}", vote_id, seq);
        }

        Commands::VoteClose { vote_id } => {
            let seq = v2.close_vote(vote_id)
                .map_err(|e| anyhow::anyhow!("V2 vote close error: {}", e))?;
            println!("vote_closed|{}|{}", vote_id, seq);
        }

        // ===== TASKS (4 Consolidated Commands - V2) =====

        Commands::TaskCreate { description, tasks, priority } => {
            if let Some(ref batch_tasks) = tasks {
                // Batch mode
                let seq = v2.batch_create(&description, batch_tasks)
                    .map_err(|e| anyhow::anyhow!("V2 batch create error: {}", e))?;
                let task_count = batch_tasks.split('|').filter(|t| t.contains(':')).count();
                println!("batch_created|{}|{}|{}", description, task_count, seq);
            } else {
                // Single task mode
                let prio: u32 = match priority.to_lowercase().as_str() {
                    "low" => 0,
                    "high" => 2,
                    "urgent" => 3,
                    _ => 1, // normal
                };
                let seq = v2.add_task(&description, prio, "")
                    .map_err(|e| anyhow::anyhow!("V2 task add error: {}", e))?;
                println!("task_created|{}|{}", seq, description);
            }
        }

        Commands::TaskUpdate { id, status, reason } => {
            let status_lower = status.to_lowercase();

            // Check if batch reference (contains :) or batch name (non-numeric)
            if id.contains(':') {
                // Batch task reference like "Auth:1"
                if let Some((batch_name, label)) = id.rsplit_once(':') {
                    match status_lower.as_str() {
                        "done" | "completed" => {
                            let seq = v2.batch_task_done(batch_name, label)
                                .map_err(|e| anyhow::anyhow!("V2 batch task done error: {}", e))?;
                            println!("task_updated|{}|done|{}", id, seq);
                        }
                        _ => {
                            println!("error|batch_tasks_only_support_done|{}|{}", id, status);
                        }
                    }
                }
            } else if let Ok(task_id) = id.parse::<u64>() {
                // Numeric ID - single task
                match status_lower.as_str() {
                    "done" | "completed" => {
                        let seq = v2.complete_task(task_id, "completed")
                            .map_err(|e| anyhow::anyhow!("V2 task complete error: {}", e))?;
                        println!("task_updated|{}|done|{}", task_id, seq);
                    }
                    "claimed" => {
                        let seq = v2.claim_task(task_id)
                            .map_err(|e| anyhow::anyhow!("V2 task claim error: {}", e))?;
                        println!("task_updated|{}|claimed|{}", task_id, seq);
                    }
                    "started" | "in_progress" => {
                        let seq = v2.start_task(task_id)
                            .map_err(|e| anyhow::anyhow!("V2 task start error: {}", e))?;
                        println!("task_updated|{}|started|{}", task_id, seq);
                    }
                    "blocked" => {
                        let reason_str = reason.as_deref().unwrap_or("blocked");
                        let seq = v2.block_task(task_id, reason_str)
                            .map_err(|e| anyhow::anyhow!("V2 task block error: {}", e))?;
                        println!("task_updated|{}|blocked|{}", task_id, seq);
                    }
                    "unblocked" => {
                        let seq = v2.unblock_task(task_id)
                            .map_err(|e| anyhow::anyhow!("V2 task unblock error: {}", e))?;
                        println!("task_updated|{}|unblocked|{}", task_id, seq);
                    }
                    _ => {
                        let seq = v2.update_task_status(task_id, &status)
                            .map_err(|e| anyhow::anyhow!("V2 task update error: {}", e))?;
                        println!("task_updated|{}|{}|{}", task_id, status, seq);
                    }
                }
            } else {
                // Non-numeric, no colon - batch name for close
                match status_lower.as_str() {
                    "closed" | "done" | "completed" => {
                        let seq = v2.batch_close(&id)
                            .map_err(|e| anyhow::anyhow!("V2 batch close error: {}", e))?;
                        println!("batch_closed|{}|{}", id, seq);
                    }
                    _ => {
                        println!("error|batch_only_supports_closed|{}|{}", id, status);
                    }
                }
            }
        }

        Commands::TaskGet { id } => {
            if let Ok(task_id) = id.parse::<u64>() {
                // Numeric - single task (dual lookup: view seq + timestamp + outbox)
                match v2.get_task(task_id)
                    .map_err(|e| anyhow::anyhow!("V2 get_task error: {}", e))? {
                    Some((tid, desc, priority, status, assignee)) => {
                        println!("|TASK|{}", tid);
                        println!("Description:{}", desc);
                        println!("Status:{}", status);
                        println!("Priority:{}", priority);
                        if let Some(a) = assignee {
                            println!("AssignedTo:{}", a);
                        }
                    }
                    None => {
                        println!("error|task_not_found|{}", task_id);
                    }
                }
            } else {
                // Non-numeric - batch name
                match v2.get_batch(&id)
                    .map_err(|e| anyhow::anyhow!("V2 get batch error: {}", e))? {
                    Some((creator, tasks)) => {
                        let done_count = tasks.iter().filter(|(_, _, done)| *done).count();
                        println!("|BATCH|{}|{}|{}/{}", id, creator, done_count, tasks.len());
                        for (label, desc, is_done) in tasks {
                            let status = if is_done { "done" } else { "pending" };
                            println!("{}:{}|{}", label, desc, status);
                        }
                    }
                    None => {
                        println!("error|batch_not_found|{}", id);
                    }
                }
            }
        }

        Commands::TaskList { count, limit_flag, filter } => {
            let limit = limit_flag.or(count).unwrap_or(20);
            match filter.to_lowercase().as_str() {
                "batches" => {
                    let batches = v2.get_batches()
                        .map_err(|e| anyhow::anyhow!("V2 get batches error: {}", e))?;
                    println!("|BATCHES|{}", batches.len());
                    for (name, creator, total, done, _) in batches.iter().take(limit) {
                        let status = if *done == *total { "complete" } else { "in_progress" };
                        println!("{}|{}|{}/{}|{}", name, creator, done, total, status);
                    }
                }
                "tasks" => {
                    let tasks = v2.get_tasks()
                        .map_err(|e| anyhow::anyhow!("V2 get_tasks error: {}", e))?;
                    println!("|TASKS|{}", tasks.len().min(limit));
                    for (id, desc, creator, status, assignee) in tasks.iter().take(limit) {
                        let assigned = assignee.as_deref().unwrap_or("-");
                        println!("{}|{}|{}|by:{}|{}", id, status, desc, creator, assigned);
                    }
                }
                _ => {
                    // Show both
                    let batches = v2.get_batches()
                        .map_err(|e| anyhow::anyhow!("V2 get batches error: {}", e))?;
                    let tasks = v2.get_tasks()
                        .map_err(|e| anyhow::anyhow!("V2 get_tasks error: {}", e))?;

                    if !batches.is_empty() {
                        println!("|BATCHES|{}", batches.len());
                        for (name, creator, total, done, _) in batches.iter().take(limit / 2) {
                            let status = if *done == *total { "complete" } else { "in_progress" };
                            println!("{}|{}|{}/{}|{}", name, creator, done, total, status);
                        }
                    }

                    println!("|TASKS|{}", tasks.len().min(limit));
                    for (id, desc, creator, status, assignee) in tasks.iter().take(limit) {
                        let assigned = assignee.as_deref().unwrap_or("-");
                        println!("{}|{}|{}|by:{}|{}", id, status, desc, creator, assigned);
                    }
                }
            }
        }

        // Locks removed — enum variants + handlers deleted (Feb 2026)

        // ===== ROOMS =====

        Commands::RoomCreate { name, topic } => {
            let seq = v2.create_room(&name, &topic)
                .map_err(|e| anyhow::anyhow!("V2 room create error: {}", e))?;
            println!("room_created|{}|{}", seq, name);
        }

        Commands::RoomJoin { room_id } => {
            let seq = v2.join_room(&room_id.to_string())
                .map_err(|e| anyhow::anyhow!("V2 room join error: {}", e))?;
            println!("room_joined|{}|{}", room_id, seq);
        }

        Commands::RoomLeave { room_id } => {
            let seq = v2.leave_room(&room_id.to_string())
                .map_err(|e| anyhow::anyhow!("V2 room leave error: {}", e))?;
            println!("room_left|{}|{}", room_id, seq);
        }

        // ===== PRESENCE =====

        Commands::UpdatePresence { status, task } => {
            let seq = v2.update_presence(&status, Some(&task))
                .map_err(|e| anyhow::anyhow!("V2 presence error: {}", e))?;
            println!("presence|{}|{}|{}", ai_id, status, seq);
        }

        // ===== FILE ACTIONS =====

        Commands::LogAction { action, path } => {
            let seq = v2.log_file_action(&path, &action)
                .map_err(|e| anyhow::anyhow!("V2 file action error: {}", e))?;
            println!("logged|{}|{}|{}", seq, action, path);

            // Stigmergy pheromone auto-deposit removed (Feb 2026, QD directive)
            // File actions still logged above for team activity visibility

            // Refresh bulletin for passive injection
            refresh_bulletin_v2(&mut v2, ai_id);
        }

        // Stigmergy removed — enum variants + handlers deleted (Feb 2026)

        // ===== STATS =====

        Commands::Stats => {
            let stats = v2.stats();
            println!("|STATS|");
            println!("EventsApplied:{}", stats.events_applied);
            println!("UnreadDMs:{}", v2.unread_dm_count());
            println!("ActiveDialogues:{}", v2.active_dialogue_count());
            println!("PendingVotes:{}", v2.pending_vote_count());
            println!("MyTasks:{}", v2.my_task_count());
            println!();
            println!("Backend: Event Sourcing");
            println!("Write path: Outbox → Sequencer → Master Log");
        }

        // ===== TEAM STATUS (V2) =====
        Commands::Status => {
            let presences = v2.get_presences()
                .map_err(|e| anyhow::anyhow!("V2 get_presences error: {}", e))?;
            println!("|TEAM STATUS|");
            println!("Online:{}", presences.len());
            for (ai, status, task) in presences {
                // Use clear words instead of ambiguous symbols (QD directive)
                let status_word = match status.as_str() {
                    "active" => "active",
                    "busy" => "busy",
                    "standby" => "standby",
                    "idle" => "idle",
                    _ => "online",
                };
                println!("{}|{}|{}", ai, status_word, task);
            }
        }

        // ===== VOTES (V2) =====
        Commands::Votes { limit: _ } => {
            let votes = v2.get_votes()
                .map_err(|e| anyhow::anyhow!("V2 get_votes error: {}", e))?;
            println!("|VOTES|{}", votes.len());
            for (id, creator, topic, options, status, casts) in votes {
                let opts = options.join(",");
                let vote_count = casts.len();
                println!("{}|{}|opts:{}|by:{}|votes:{}|{}", id, topic, opts, creator, vote_count, status);
            }
        }

        // ===== DIALOGUES (V2) - Consolidated into DialogueList =====
        Commands::DialogueList { limit, filter, id } => {
            // If specific ID requested, show full details + messages
            if let Some(dialogue_id) = id {
                match v2.get_dialogue(dialogue_id)
                    .map_err(|e| anyhow::anyhow!("V2 get_dialogue error: {}", e))? {
                    Some((id, initiator, responder, topic, status, current_turn)) => {
                        println!("|DIALOGUE|{}", id);
                        println!("Initiator:{}", initiator);
                        println!("Responder:{}", responder);
                        println!("Topic:{}", topic);
                        let is_my_turn = current_turn == ai_id;
                        println!("Turn:{}|{}", current_turn, if is_my_turn { "YOUR TURN" } else { "waiting" });
                        println!("Status:{}", status);
                        // Also show messages
                        let messages = v2.get_dialogue_messages(dialogue_id)
                            .map_err(|e| anyhow::anyhow!("V2 get_dialogue_messages error: {}", e))?;
                        if !messages.is_empty() {
                            println!("|MESSAGES|{}", messages.len());
                            for (seq, source_ai, content, timestamp_micros) in messages {
                                let ts_secs = timestamp_micros / 1_000_000;
                                let datetime = chrono::DateTime::from_timestamp(ts_secs as i64, 0)
                                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                                    .unwrap_or_else(|| "unknown".to_string());
                                println!("  #{}|{}|{}|{}", seq, source_ai, datetime, content);
                            }
                        }
                    }
                    None => {
                        println!("error|dialogue_not_found|{}", dialogue_id);
                    }
                }
            } else {
                // Detect if invoked via an "invites" alias — those should default to "invites" filter.
                // clap aliases all share the same default --filter "all", so we inspect argv[1].
                const INVITES_ALIASES: &[&str] = &[
                    "invites", "dialogue-invites", "pending-chats", "incoming", "chat-invites",
                ];
                let filter = if filter == "all" {
                    let argv1 = std::env::args().nth(1).unwrap_or_default();
                    if INVITES_ALIASES.contains(&argv1.as_str()) {
                        "invites".to_string()
                    } else {
                        filter
                    }
                } else {
                    filter
                };
                // Filter-based listing
                match filter.as_str() {
                    "invites" => {
                        let invites = v2.get_dialogue_invites()
                            .map_err(|e| anyhow::anyhow!("V2 get_dialogue_invites error: {}", e))?;
                        let invites: Vec<_> = invites.into_iter().take(limit).collect();
                        println!("|DIALOGUE INVITES|{}", invites.len());
                        for (id, initiator, _responder, topic, _status, _turn) in invites {
                            println!("{}|from:{}|{}", id, initiator, topic);
                        }
                    }
                    "my-turn" => {
                        let dialogues = v2.get_dialogue_my_turn()
                            .map_err(|e| anyhow::anyhow!("V2 get_dialogue_my_turn error: {}", e))?;
                        let dialogues: Vec<_> = dialogues.into_iter().take(limit).collect();
                        println!("|YOUR TURN|{}", dialogues.len());
                        for (id, initiator, responder, topic, _status, turn) in dialogues {
                            let other = if initiator == ai_id { &responder } else { &initiator };
                            println!("{}|with:{}|turn:{}|{}", id, other, turn, topic);
                        }
                    }
                    _ => {
                        // "all" or default
                        let dialogues = v2.get_dialogues()
                            .map_err(|e| anyhow::anyhow!("V2 get_dialogues error: {}", e))?;
                        let dialogues: Vec<_> = dialogues.into_iter().take(limit).collect();
                        println!("|DIALOGUES|{}", dialogues.len());
                        for (id, initiator, responder, topic, status, turn) in dialogues {
                            println!("{}|{}↔{}|{}|{}|turn:{}", id, initiator, responder, topic, status, turn);
                        }
                    }
                }
            }
        }

        Commands::WhatDoing { limit } => {
            // Show recent AI activity from presence updates
            let presences = v2.get_presences()
                .map_err(|e| anyhow::anyhow!("V2 get_presences error: {}", e))?;
            println!("|AI ACTIVITY|{}", presences.len().min(limit));
            for (ai_id, status, current_task) in presences.iter().take(limit) {
                let task = if current_task.is_empty() { "-" } else { current_task };
                println!("{}|{}|{}", ai_id, status, task);
            }
        }

        // ===== ROOMS (V2) =====
        Commands::Rooms { limit: _ } => {
            let rooms = v2.get_rooms()
                .map_err(|e| anyhow::anyhow!("V2 get_rooms error: {}", e))?;
            println!("|ROOMS|{}", rooms.len());
            for (id, name, topic, members, is_closed) in rooms {
                let status = if is_closed { "concluded" } else { "active" };
                println!("{}|{}|{}|members:{}|{}", id, name, topic, members.join(","), status);
            }
        }

        // ===== FILE ACTIONS (V2) =====
        Commands::FileActions { limit } => {
            let actions = v2.get_file_actions(limit)
                .map_err(|e| anyhow::anyhow!("V2 get_file_actions error: {}", e))?;
            println!("|FILE ACTIONS|{}", actions.len());
            for (ai_id, action, path, _ts) in actions {
                println!("{}|{}|{}", ai_id, action, path);
            }
        }

        // Lock check removed (Feb 2026). Use: teambook check-file <path>

        // ===== VOTE RESULTS (V2) =====
        Commands::VoteResults { vote_id } => {
            match v2.get_vote(vote_id)
                .map_err(|e| anyhow::anyhow!("V2 get_vote error: {}", e))? {
                Some((id, creator, topic, options, status, casts)) => {
                    println!("|VOTE|{}|RESULTS", id);
                    println!("Topic:{}", topic);
                    println!("Options:{}", options.join(","));
                    println!("CreatedBy:{}", creator);
                    println!("Status:{}", status);
                    println!("VotesCast:{}", casts.len());
                    for (voter, choice) in casts {
                        println!("  {}:{}", voter, choice);
                    }
                }
                None => {
                    println!("error|vote_not_found|{}", vote_id);
                }
            }
        }

        // ===== ROOM GET (V2) =====
        Commands::RoomGet { room_id } => {
            match v2.get_room(room_id)
                .map_err(|e| anyhow::anyhow!("V2 get_room error: {}", e))? {
                Some((id, name, topic, members)) => {
                    println!("|ROOM|{}", id);
                    println!("Name:{}", name);
                    println!("Topic:{}", topic);
                    println!("Members:{}", members.join(","));
                }
                None => {
                    println!("error|room_not_found|{}", room_id);
                }
            }
        }

        // ===== ROOM CLOSE (V2) =====
        Commands::RoomClose { room_id } => {
            let seq = v2.close_room(&room_id.to_string())
                .map_err(|e| anyhow::anyhow!("V2 close_room error: {}", e))?;
            println!("room_closed|{}|{}", room_id, seq);
        }

        // ===== ROOM SAY (V2) =====
        Commands::RoomSay { room_id, content } => {
            if room_id == 0 {
                eprintln!("error: invalid room_id 0 — room IDs are assigned at creation and are always > 0");
                std::process::exit(1);
            }
            // Look up room participants for sequencer wake routing (scoped delivery)
            let participants = v2.get_room(room_id)
                .map_err(|e| anyhow::anyhow!("V2 get_room error: {}", e))?
                .map(|(_, _, _, members)| members)
                .unwrap_or_default();
            let seq = v2.send_room_message(&room_id.to_string(), &content, participants)
                .map_err(|e| anyhow::anyhow!("V2 send_room_message error: {}", e))?;
            println!("room_message_sent|{}|{}", room_id, seq);
        }

        // ===== ROOM MESSAGES (V2) =====
        Commands::RoomMessages { room_id, limit } => {
            if room_id == 0 {
                eprintln!("error: invalid room_id 0 — room IDs are assigned at creation and are always > 0");
                std::process::exit(1);
            }
            let messages = v2.get_room_messages(&room_id.to_string(), limit)
                .map_err(|e| anyhow::anyhow!("V2 get_room_messages error: {}", e))?;
            println!("|ROOM {}|{}", room_id, messages.len());
            // NO TRUNCATION - full content always
            for (seq, from_ai, msg_content, _ts) in messages {
                println!("#{} {}: {}", seq, from_ai, msg_content);
            }
        }

        // ===== ROOM MUTE (V2) =====
        Commands::RoomMute { room_id, minutes } => {
            v2.room_mute(&room_id.to_string(), ai_id, minutes)
                .map_err(|e| anyhow::anyhow!("V2 room_mute error: {}", e))?;
            println!("room_muted|{}|{}|{}min", room_id, ai_id, minutes);
        }

        // ===== ROOM CONCLUDE (V2) =====
        Commands::RoomConclude { room_id, conclusion } => {
            v2.room_conclude(&room_id.to_string(), ai_id, conclusion.as_deref())
                .map_err(|e| anyhow::anyhow!("V2 room_conclude error: {}", e))?;
            println!("room_concluded|{}", room_id);
            if let Some(ref c) = conclusion {
                println!("conclusion: {}", c);
            }
        }

        // ===== ROOM PIN/UNPIN MESSAGE (V2) =====
        Commands::RoomPinMessage { room_id, msg_seq_id } => {
            v2.room_pin_message(&room_id.to_string(), ai_id, msg_seq_id)
                .map_err(|e| anyhow::anyhow!("V2 room_pin_message error: {}", e))?;
            println!("room_message_pinned|{}|#{}", room_id, msg_seq_id);
        }

        Commands::RoomUnpinMessage { room_id, msg_seq_id } => {
            v2.room_unpin_message(&room_id.to_string(), ai_id, msg_seq_id)
                .map_err(|e| anyhow::anyhow!("V2 room_unpin_message error: {}", e))?;
            println!("room_message_unpinned|{}|#{}", room_id, msg_seq_id);
        }

        // ===== IDENTITY/PRESENCE (V2) =====
        Commands::IdentityShow => {
            let fingerprint = hash_ai_id(ai_id);
            println!("|IDENTITY|");
            println!("AI:{}", ai_id);
            println!("Fingerprint:{}", fingerprint);
            println!("Backend:V2 Event Sourcing");
        }

        Commands::MyPresence => {
            let presences = v2.get_presences()
                .map_err(|e| anyhow::anyhow!("V2 get_presences error: {}", e))?;
            match presences.iter().find(|(id, _, _)| id == ai_id) {
                Some((_, status, task)) => {
                    println!("|MY PRESENCE|");
                    println!("AI:{}", ai_id);
                    println!("Status:{}", status);
                    println!("Task:{}", if task.is_empty() { "-" } else { task });
                }
                None => {
                    println!("|MY PRESENCE|");
                    println!("AI:{}", ai_id);
                    println!("Status:unknown");
                    println!("Task:-");
                    println!("(No presence update found - try: teambook presence Online)");
                }
            }
        }

        Commands::PresenceCount => {
            let presences = v2.get_presences()
                .map_err(|e| anyhow::anyhow!("V2 get_presences error: {}", e))?;
            println!("Online:{}", presences.len());
        }

        Commands::GetPresence { ai_id: target_ai } => {
            // First check if AI is actually online via OS mutex
            let online = is_ai_online(&target_ai);

            if online {
                // Get their presence data from V2
                let presences = v2.get_presences()
                    .map_err(|e| anyhow::anyhow!("V2 get_presences error: {}", e))?;

                if let Some((_, status, task)) = presences.iter().find(|(ai, _, _)| ai == &target_ai) {
                    println!("|PRESENCE|");
                    println!("AI:{}", target_ai);
                    println!("Status:{}", status);
                    println!("Task:{}", if task.is_empty() { "active" } else { task });
                } else {
                    // Online but no presence data yet
                    println!("|PRESENCE|");
                    println!("AI:{}", target_ai);
                    println!("Status:active");
                    println!("Task:-");
                }
            } else {
                println!("|PRESENCE|");
                println!("AI:{}", target_ai);
                println!("Status:offline");
            }
        }

        // ===== STANDBY (V2) =====
        Commands::Standby { timeout } => {
            // SNAPSHOT-BASED STANDBY: Report stale pending work but ALWAYS enter standby.
            // After waking, only report genuinely NEW items (not in the pre-sleep snapshot).
            //
            // Old behavior: if ANY pending work existed (including week-old dialogue invites),
            // standby returned immediately - making it impossible to ever actually sleep.

            // Gather current pending items for snapshot
            let unread_dms = v2.get_unread_dms();
            let ack_dialogues = get_acknowledged_dialogues(ai_id);

            let pending_invites: Vec<_> = v2.get_dialogue_invites()
                .unwrap_or_default()
                .into_iter()
                .filter(|(id, _, _, _, _, _)| !ack_dialogues.contains(id))
                .collect();

            let pending_turns: Vec<_> = v2.get_dialogue_my_turn()
                .unwrap_or_default()
                .into_iter()
                .filter(|(id, _, _, _, _, _)| !ack_dialogues.contains(id))
                .collect();

            let dm_count = unread_dms.len();
            let dialogue_count = pending_invites.len() + pending_turns.len();

            // Snapshot: record IDs of everything currently pending.
            // After waking, only items NOT in this snapshot are "new".
            let snapshot_dm_ids: std::collections::HashSet<u64> =
                unread_dms.iter().map(|dm| dm.id).collect();
            let snapshot_dialogue_ids: std::collections::HashSet<u64> =
                pending_invites.iter().map(|(id, ..)| *id)
                    .chain(pending_turns.iter().map(|(id, ..)| *id))
                    .collect();

            // Report existing pending work (informational - does NOT block standby entry)
            if dm_count > 0 || dialogue_count > 0 {
                println!("|PENDING WORK|");
                if dm_count > 0 {
                    println!("Unread DMs: {}", dm_count);
                    for dm in &unread_dms {
                        println!("  {}|{}", dm.from_ai, dm.content);
                        if let Err(e) = v2.emit_dm_read(dm.id) {
                            eprintln!("warn: dm read receipt failed: {}", e);
                        }
                    }
                }
                if !pending_invites.is_empty() {
                    println!("Dialogue Invites: {}", pending_invites.len());
                    for (id, initiator, _, topic, _, _) in &pending_invites {
                        println!("  #{}|{}|{}", id, initiator, topic);
                        acknowledge_dialogue(ai_id, *id);
                    }
                }
                if !pending_turns.is_empty() {
                    println!("Your Turn in Dialogues: {}", pending_turns.len());
                    for (id, initiator, responder, topic, _, _) in &pending_turns {
                        let other = if initiator == ai_id { responder } else { initiator };
                        println!("  #{}|{}|{}", id, other, topic);
                        acknowledge_dialogue(ai_id, *id);
                    }
                }
                // NOTE: No return here! We report stale work then STILL enter standby.
            }

            // Enter standby
            println!("standby|{}|timeout={}s", ai_id, timeout);
            if let Err(e) = v2.update_presence("standby", None) {
                eprintln!("warn: presence update to standby failed: {}", e);
            }

            let coordinator = WakeCoordinator::new(ai_id)
                .map_err(|e| anyhow::anyhow!("Failed to create wake coordinator: {}", e))?;

            // Drain any stale wake signals that fired while we weren't sleeping.
            // Without this, events processed by the sequencer before we entered standby
            // would cause an immediate spurious wake (the OS event stays signaled).
            while coordinator.wait_timeout(std::time::Duration::ZERO).is_some() {}

            let timeout_duration = if timeout > 0 {
                std::time::Duration::from_secs(timeout)
            } else {
                std::time::Duration::from_secs(180) // Default 3 minute max
            };

            // Block until wake event or timeout - NO POLLING!
            let wake_result = coordinator.wait_timeout(timeout_duration);

            // Update presence immediately
            if let Err(e) = v2.update_presence("active", None) {
                eprintln!("warn: presence update to active failed: {}", e);
            }

            if wake_result.is_none() {
                println!("standby_timeout|{}", ai_id);
            } else {
                // Sync the view to pick up events that arrived while sleeping
                if let Err(e) = v2.sync() {
                    eprintln!("warn: v2 sync after standby wake failed: {}", e);
                }

                // Only report NEW items (not in the pre-standby snapshot)
                let new_dms: Vec<_> = v2.get_unread_dms()
                    .into_iter()
                    .filter(|dm| !snapshot_dm_ids.contains(&dm.id))
                    .collect();

                let ack_dialogues = get_acknowledged_dialogues(ai_id);

                let new_invites: Vec<_> = v2.get_dialogue_invites()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(id, _, _, _, _, _)| {
                        !ack_dialogues.contains(id) && !snapshot_dialogue_ids.contains(id)
                    })
                    .collect();

                let new_turns: Vec<_> = v2.get_dialogue_my_turn()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(id, _, _, _, _, _)| {
                        !ack_dialogues.contains(id) && !snapshot_dialogue_ids.contains(id)
                    })
                    .collect();

                let new_dm_count = new_dms.len();
                let new_dialogue_count = new_invites.len() + new_turns.len();

                if new_dm_count > 0 || new_dialogue_count > 0 {
                    println!("|WOKE UP - NEW WORK|");
                    if new_dm_count > 0 {
                        println!("Unread DMs: {}", new_dm_count);
                        for dm in &new_dms {
                            println!("  {}|{}", dm.from_ai, dm.content);
                            if let Err(e) = v2.emit_dm_read(dm.id) {
                                eprintln!("warn: dm read receipt failed: {}", e);
                            }
                        }
                    }
                    if !new_invites.is_empty() {
                        println!("Dialogue Invites: {}", new_invites.len());
                        for (id, initiator, _, topic, _, _) in &new_invites {
                            println!("  #{}|{}|{}", id, initiator, topic);
                        }
                    }
                    if !new_turns.is_empty() {
                        println!("Your Turn in Dialogues: {}", new_turns.len());
                        for (id, initiator, responder, topic, _, _) in &new_turns {
                            let other = if initiator == ai_id { responder } else { initiator };
                            println!("  #{}|{}|{}", id, other, topic);
                        }
                    }
                } else {
                    println!("|WOKE UP|");
                    println!("Check broadcasts for @mentions or recent activity");
                }
            }
        }

        // ===== FILE CLAIMS (V2) =====
        Commands::ClaimFile { path, working_on, duration } => {
            let duration_secs = duration * 60; // Convert minutes to seconds
            let seq = v2.claim_file(&path, duration_secs, &working_on)
                .map_err(|e| anyhow::anyhow!("V2 claim_file error: {}", e))?;
            println!("file_claimed|{}|{}", seq, path);
            // Refresh bulletin for passive injection
            refresh_bulletin_v2(&mut v2, ai_id);
        }

        Commands::ReleaseFile { path } => {
            let seq = v2.release_file(&path)
                .map_err(|e| anyhow::anyhow!("V2 release_file error: {}", e))?;
            println!("file_released|{}|{}", seq, path);
            // Refresh bulletin for passive injection
            refresh_bulletin_v2(&mut v2, ai_id);
        }

        Commands::CheckFile { path } => {
            match v2.check_claim(&path)
                .map_err(|e| anyhow::anyhow!("V2 check_claim error: {}", e))? {
                Some((claimer, _ts, _duration, working_on)) => {
                    if working_on.is_empty() {
                        println!("claimed|{}|{}", claimer, path);
                    } else {
                        println!("claimed|{}|{}|{}", claimer, path, working_on);
                    }
                }
                None => {
                    println!("unclaimed|{}", path);
                }
            }
        }

        Commands::ListClaims { limit } => {
            let claims = v2.get_claims()
                .map_err(|e| anyhow::anyhow!("V2 get_claims error: {}", e))?;
            println!("|FILE CLAIMS|{}", claims.len());
            for (path, ai, ts_micros, duration_secs, working_on) in claims.iter().take(limit) {
                // Calculate time remaining
                // Note: timestamp is in MICROSECONDS, duration is in SECONDS
                let now_micros = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_micros() as u64)
                    .unwrap_or(0);
                let expires_micros = *ts_micros + (*duration_secs as u64 * 1_000_000);
                let remaining_secs = if expires_micros > now_micros {
                    (expires_micros - now_micros) / 1_000_000
                } else {
                    0
                };
                let remaining_min = remaining_secs / 60;
                let ctx = if working_on.is_empty() { "editing" } else { working_on.as_str() };
                println!("{}|{}|{}|{}min remaining", ai, path, ctx, remaining_min);
            }
        }

        // ===== BULLETIN REFRESH (for hooks) =====
        Commands::RefreshBulletin => {
            refresh_bulletin_v2(&mut v2, ai_id);
            println!("bulletin_refreshed|{}", ai_id);
        }

        // ===== PROJECTS =====

        Commands::ListProjects => {
            let projects = v2.list_projects()
                .map_err(|e| anyhow::anyhow!("List projects error: {}", e))?;
            println!("|PROJECTS|{}", projects.len());
            for (id, name, goal, _dir, status, _deleted) in projects {
                println!("{}|{}|{}|{}", id, name, status, goal);
            }
        }

        Commands::ProjectCreate { name, goal, directory } => {
            let seq = v2.create_project(&name, &goal, &directory)
                .map_err(|e| anyhow::anyhow!("Create project error: {}", e))?;
            println!("project_created|{}|{}", seq, name);
        }

        Commands::ProjectGet { project_id } => {
            if let Some((id, name, goal, dir, status, _deleted)) = v2.get_project(project_id as u64)
                .map_err(|e| anyhow::anyhow!("Get project error: {}", e))? {
                println!("|PROJECT|{}", id);
                println!("Name:{}", name);
                println!("Goal:{}", goal);
                println!("Status:{}", status);
                println!("Directory:{}", dir);
            } else {
                println!("project_not_found|{}", project_id);
            }
        }

        Commands::ProjectDelete { project_id } => {
            let seq = v2.delete_project(project_id as u64)
                .map_err(|e| anyhow::anyhow!("Delete project error: {}", e))?;
            println!("project_deleted|{}|{}", seq, project_id);
        }

        Commands::ProjectRestore { project_id } => {
            let seq = v2.restore_project(project_id as u64)
                .map_err(|e| anyhow::anyhow!("Restore project error: {}", e))?;
            println!("project_restored|{}|{}", seq, project_id);
        }

        Commands::ProjectResolve { path } => {
            match v2.resolve_project_for_file(&path)
                .map_err(|e| anyhow::anyhow!("Resolve project error: {}", e))? {
                Some((proj_id, proj_name, proj_goal, proj_dir, feature)) => {
                    println!("|PROJECT_CONTEXT|");
                    println!("Project:{}|{}", proj_id, proj_name);
                    println!("Goal:{}", proj_goal);
                    println!("Directory:{}", proj_dir);
                    if let Some((feat_id, feat_name, feat_overview, feat_dir)) = feature {
                        println!("Feature:{}|{}", feat_id, feat_name);
                        println!("FeatureOverview:{}", feat_overview);
                        println!("FeatureDirectory:{}", feat_dir);
                    }
                }
                None => {
                    println!("no_project|{}", path);
                }
            }
        }

        Commands::ProjectTasks { project_id } => {
            let tasks = v2.get_tasks()
                .map_err(|e| anyhow::anyhow!("Get tasks error: {}", e))?;
            let project_tag = format!("project:{}", project_id);
            let matching: Vec<_> = tasks.iter()
                .filter(|(_, desc, _, _, _)| desc.contains(&project_tag))
                .collect();
            println!("|PROJECT_TASKS|{}|{}", project_id, matching.len());
            for (id, desc, priority, status, assignee) in &matching {
                let a = assignee.as_deref().unwrap_or("unassigned");
                println!("{}|{}|{}|{}|{}", id, priority, status, a, desc);
            }
        }

        Commands::ProjectAddTask { project_id, title, priority } => {
            let tagged_desc = format!("{} [project:{}]", title, project_id);
            let seq = v2.add_task(&tagged_desc, priority as u32, &format!("project:{}", project_id))
                .map_err(|e| anyhow::anyhow!("Add task error: {}", e))?;
            println!("task_added|{}|project:{}", seq, project_id);
        }

        // ===== FEATURES =====

        Commands::ListFeatures { project_id } => {
            let features = v2.list_features(project_id as u64)
                .map_err(|e| anyhow::anyhow!("List features error: {}", e))?;
            println!("|FEATURES|{}", features.len());
            for (id, proj_id, name, overview, _dir, _deleted) in features {
                println!("{}|{}|{}|{}", id, proj_id, name, overview);
            }
        }

        Commands::FeatureCreate { project_id, name, overview, directory } => {
            let seq = v2.create_feature(project_id as u64, &name, &overview, directory.as_deref())
                .map_err(|e| anyhow::anyhow!("Create feature error: {}", e))?;
            println!("feature_created|{}|{}|{}", seq, project_id, name);
        }

        Commands::FeatureGet { feature_id } => {
            if let Some((id, proj_id, name, overview, dir, _deleted)) = v2.get_feature(feature_id as u64)
                .map_err(|e| anyhow::anyhow!("Get feature error: {}", e))? {
                println!("|FEATURE|{}", id);
                println!("ProjectId:{}", proj_id);
                println!("Name:{}", name);
                println!("Overview:{}", overview);
                if let Some(d) = dir {
                    println!("Directory:{}", d);
                }
            } else {
                println!("feature_not_found|{}", feature_id);
            }
        }

        Commands::FeatureDelete { feature_id } => {
            let seq = v2.delete_feature(feature_id as u64)
                .map_err(|e| anyhow::anyhow!("Delete feature error: {}", e))?;
            println!("feature_deleted|{}|{}", seq, feature_id);
        }

        Commands::FeatureRestore { feature_id } => {
            let seq = v2.restore_feature(feature_id as u64)
                .map_err(|e| anyhow::anyhow!("Restore feature error: {}", e))?;
            println!("feature_restored|{}|{}", seq, feature_id);
        }

        Commands::FeatureUpdate { feature_id, overview, name, directory } => {
            let seq = v2.update_feature(
                feature_id as u64,
                name.as_deref(),
                overview.as_deref(),
                directory.as_deref(),
            ).map_err(|e| anyhow::anyhow!("Update feature error: {}", e))?;
            println!("feature_updated|{}|{}", seq, feature_id);
        }

        Commands::ProjectUpdate { project_id, goal, name: _name } => {
            // V2 update_project supports goal + status, not name
            // If name change needed, would require new event type
            let seq = v2.update_project(
                project_id as u64,
                goal.as_deref(),
                None, // status unchanged
            ).map_err(|e| anyhow::anyhow!("Update project error: {}", e))?;
            println!("project_updated|{}|{}", seq, project_id);
        }

        // ===== LEARNINGS (V2) =====

        Commands::Learning { content, tags, importance } => {
            // Check if at limit (15 learnings max)
            let count = v2.count_learnings(ai_id)
                .map_err(|e| anyhow::anyhow!("Count learnings error: {}", e))?;
            if count >= 15 {
                eprintln!("Error: You have {} learnings (max 15). Delete one first.", count);
                eprintln!("Hint: Use 'teambook my-learnings' to see yours, 'teambook learning-delete <id>' to remove one");
                std::process::exit(1);
            }

            let seq = v2.create_learning(&content, &tags, importance)
                .map_err(|e| anyhow::anyhow!("Create learning error: {}", e))?;
            println!("learning_created|{}|{}|{}", seq, importance, tags);
        }

        Commands::LearningUpdate { learning_id, content, tags, importance } => {
            let seq = v2.update_learning(learning_id, content.as_deref(), tags.as_deref(), importance)
                .map_err(|e| anyhow::anyhow!("Update learning error: {}", e))?;
            println!("learning_updated|{}|{}", seq, learning_id);
        }

        Commands::LearningDelete { learning_id } => {
            let seq = v2.delete_learning(learning_id)
                .map_err(|e| anyhow::anyhow!("Delete learning error: {}", e))?;
            println!("learning_deleted|{}|{}", seq, learning_id);
        }

        Commands::MyLearnings => {
            let learnings = v2.get_my_learnings()
                .map_err(|e| anyhow::anyhow!("Get learnings error: {}", e))?;
            println!("|MY LEARNINGS|{}", learnings.len());
            for (id, _ai_id, content, tags, importance, _deleted) in learnings {
                // Format: #ID|importance|[tags]|content
                let tags_str = if tags.is_empty() { "".to_string() } else { format!("[{}]", tags) };
                println!("#{} {}|{} {}", id, importance, tags_str, content);
            }
        }

        Commands::TeamPlaybook { limit } => {
            let learnings = v2.get_team_playbook(limit)
                .map_err(|e| anyhow::anyhow!("Get team playbook error: {}", e))?;
            println!("|TEAM PLAYBOOK|{}", learnings.len());
            for (_id, ai_id, content, tags, importance) in learnings {
                // Format: ai_id|importance|[tags]|content
                let tags_str = if tags.is_empty() { "".to_string() } else { format!("[{}]", tags) };
                println!("{}: {} {} {}", ai_id, importance, tags_str, content);
            }
        }

        // ===== TRUST (TIP: Trust Inference and Propagation) =====

        Commands::TrustRecord { target_ai, feedback, context, weight } => {
            // Parse feedback type
            let is_success = match feedback.as_str() {
                "success" | "s" | "+" => true,
                "failure" | "f" | "-" => false,
                _ => {
                    eprintln!("Error: Invalid feedback type. Use success/s/+ or failure/f/-");
                    std::process::exit(1);
                }
            };

            let seq = v2.record_trust(&target_ai, is_success, &context, weight)
                .map_err(|e| anyhow::anyhow!("Record trust error: {}", e))?;
            let feedback_str = if is_success { "positive" } else { "negative" };
            println!("trust_recorded|{}|{}|{}|{}|{}", seq, target_ai, feedback_str, weight, context);
        }

        Commands::TrustScore { target_ai } => {
            match v2.get_trust_score(&target_ai)
                .map_err(|e| anyhow::anyhow!("Get trust score error: {}", e))? {
                Some((trust, alpha, beta, variance)) => {
                    // Format: target|trust%|alpha|beta|variance
                    println!("|TRUST|{}", target_ai);
                    println!("Score: {:.1}%", trust * 100.0);
                    println!("Alpha (successes): {}", alpha - 1); // Subtract prior
                    println!("Beta (failures): {}", beta - 1);    // Subtract prior
                    println!("Variance: {:.4}", variance);
                    if variance > 0.1 {
                        println!("Status: Uncertain (high variance)");
                    } else if trust > 0.7 {
                        println!("Status: Trusted");
                    } else if trust < 0.3 {
                        println!("Status: Untrusted");
                    } else {
                        println!("Status: Neutral");
                    }
                }
                None => {
                    println!("|TRUST|{}", target_ai);
                    println!("No interactions recorded with this AI");
                }
            }
        }

        Commands::TrustScores => {
            let scores = v2.get_all_trust_scores()
                .map_err(|e| anyhow::anyhow!("Get trust scores error: {}", e))?;
            println!("|WEB OF TRUST|{}", scores.len());
            if scores.is_empty() {
                println!("No trust data recorded yet");
                println!("Hint: Use 'teambook trust-record <ai> success/failure [context]' to record feedback");
            } else {
                for (target, trust, alpha, beta, _variance) in scores {
                    // Format: trust%|target|successes|failures
                    let status = if trust > 0.7 { "+" } else if trust < 0.3 { "-" } else { "~" };
                    println!("[{}] {:.0}% {}|s:{}|f:{}", status, trust * 100.0, target, alpha - 1, beta - 1);
                }
            }
        }

        // ===== HOOKS (AI-Foundation hooks for any client) =====

        Commands::HookPostToolUse => {
            // Read JSON from stdin
            let mut input = String::new();
            if let Err(e) = io::stdin().read_to_string(&mut input) {
                eprintln!("warn: failed to read hook stdin: {}", e);
            }

            let hook_input: HookInput = serde_json::from_str(&input).unwrap_or(HookInput {
                tool_name: None,
                tool_input: None,
            });

            let tool_name = hook_input.tool_name.unwrap_or_default();

            // Skip tools that shouldn't trigger hooks
            if SKIP_TOOLS.contains(&tool_name.as_str()) {
                return Ok(());
            }

            // Skip our own tool calls to avoid recursion
            if tool_name == "Bash" {
                if let Some(input) = &hook_input.tool_input {
                    if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                        if cmd.contains("teambook") || cmd.contains("notebook") {
                            return Ok(());
                        }
                    }
                }
            }

            // Load state for deduplication
            let mut state = PostToolHookState::load(&ai_id);

            // Log file action if applicable
            if let Some(action_type) = file_action_type(&tool_name) {
                if let Some(input) = &hook_input.tool_input {
                    if let Some(file_path) = input.get("file_path").and_then(|v| v.as_str()) {
                        if let Err(e) = v2.log_file_action(action_type, file_path) {
                            eprintln!("warn: log_file_action failed: {}", e);
                        }
                    }
                }
            }

            // Auto-update presence based on tool activity
            let presence_detail = match tool_name.as_str() {
                "Edit" => {
                    hook_input.tool_input.as_ref()
                        .and_then(|i| i.get("file_path"))
                        .and_then(|v| v.as_str())
                        .map(|p| format!("editing {}", p.split('/').last().unwrap_or(p)))
                }
                "Read" => {
                    hook_input.tool_input.as_ref()
                        .and_then(|i| i.get("file_path"))
                        .and_then(|v| v.as_str())
                        .map(|p| format!("reading {}", p.split('/').last().unwrap_or(p)))
                }
                "Write" => {
                    hook_input.tool_input.as_ref()
                        .and_then(|i| i.get("file_path"))
                        .and_then(|v| v.as_str())
                        .map(|p| format!("writing {}", p.split('/').last().unwrap_or(p)))
                }
                "Bash" => {
                    hook_input.tool_input.as_ref()
                        .and_then(|i| i.get("command"))
                        .and_then(|v| v.as_str())
                        .map(|c| {
                            let cmd = c.split_whitespace().next().unwrap_or("bash");
                            format!("running {}", cmd)
                        })
                }
                "Grep" => {
                    hook_input.tool_input.as_ref()
                        .and_then(|i| i.get("pattern"))
                        .and_then(|v| v.as_str())
                        .map(|p| {
                            let short = if p.len() > 30 { &p[..30] } else { p };
                            format!("searching for '{}'", short)
                        })
                }
                "Glob" => {
                    hook_input.tool_input.as_ref()
                        .and_then(|i| i.get("pattern"))
                        .and_then(|v| v.as_str())
                        .map(|p| format!("finding {}", p))
                }
                _ => Some(format!("using {}", tool_name)),
            };

            if let Some(detail) = presence_detail {
                if let Err(e) = v2.update_presence("active", Some(&detail)) {
                    eprintln!("warn: presence update failed: {}", e);
                }
            }

            // Auto-claim files on Edit/Write (zero-cognition enrichment)
            // Also detect conflicts when another AI owns the file
            let mut claim_warning: Option<String> = None;
            if matches!(tool_name.as_str(), "Edit" | "Write") {
                if let Some(file_path) = hook_input.tool_input.as_ref()
                    .and_then(|i| i.get("file_path"))
                    .and_then(|v| v.as_str())
                {
                    // Check existing claims
                    match v2.check_claim(file_path) {
                        Ok(Some((claimer, claimed_at, duration_secs, working_on))) => {
                            if claimer.to_lowercase() == ai_id.to_lowercase() {
                                // Self-claimed: refresh with 5-min auto-claim TTL
                                let context = build_working_on_context(&mut v2, file_path);
                                if let Err(e) = v2.claim_file(file_path, 300, &context) {
                                    eprintln!("warn: claim refresh failed: {}", e);
                                }
                            } else {
                                // Another AI owns this file — inject prominent warning
                                let now_micros = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_micros() as u64)
                                    .unwrap_or(0);
                                let expires_micros = claimed_at + (duration_secs as u64 * 1_000_000);
                                let remaining_min = if expires_micros > now_micros {
                                    (expires_micros - now_micros) / 60_000_000
                                } else {
                                    0
                                };
                                let filename = file_path.split('/').last()
                                    .or_else(|| file_path.split('\\').last())
                                    .unwrap_or(file_path);
                                let ctx = if working_on.is_empty() { "editing".to_string() } else { working_on };
                                claim_warning = Some(format!(
                                    "\u{26a0} FILE CLAIMED: {} owns {} ({}, {}m left) \u{2014} DM them or wait for release",
                                    claimer, filename, ctx, remaining_min
                                ));
                            }
                        }
                        Ok(None) => {
                            // Unclaimed: auto-claim with 5-min TTL
                            let context = build_working_on_context(&mut v2, file_path);
                            if let Err(e) = v2.claim_file(file_path, 300, &context) {
                                eprintln!("warn: auto-claim failed: {}", e);
                            }
                        }
                        Err(_) => {} // Claim check failed, don't block the hook
                    }
                }
            }

            // Build output parts (only NEW items)
            let mut parts: Vec<String> = Vec::new();

            // Inject claim conflict warning prominently at the top
            if let Some(warning) = claim_warning {
                parts.push(warning);
            }

            // REAL-TIME: Every tool call gets full awareness check.
            // Cost is ~3-5ms (mmap reads, not network). Deduplication prevents re-injection.
            // This is the AI's lifeline to its team — no artificial latency.

            // Get awareness data using correct V2Client methods
            let dms = v2.recent_dms(5).unwrap_or_default();
            let broadcasts = v2.recent_broadcasts(3, Some("general")).unwrap_or_default();
            let votes = v2.get_votes().unwrap_or_default();
            let dialogues = v2.get_dialogue_my_turn().unwrap_or_default();

            // NEW DMs only (Message.id is i32)
            // NO TRUNCATION - full content always. Context starvation is the enemy.
            let new_dms: Vec<_> = dms.iter()
                .filter(|dm| !state.seen_dm(dm.id as u64))
                .collect();
            if !new_dms.is_empty() {
                let dm_strs: Vec<String> = new_dms.iter().take(5).map(|dm| {
                    state.mark_dm(dm.id as u64);
                    // Event-sourced: persists read state across CLI invocations
                    if let Err(e) = v2.emit_dm_read(dm.id as u64) {
                        eprintln!("warn: dm read receipt failed: {}", e);
                    }
                    format!("{}:\"{}\"", dm.from_ai, &dm.content)
                }).collect();
                parts.push(format!("Your DMs: {}", dm_strs.join(", ")));
            }

            // NEW broadcasts only (Message.id is i32)
            // NO TRUNCATION - full content always. Context starvation is the enemy.
            let new_bcs: Vec<_> = broadcasts.iter()
                .filter(|bc| !state.seen_broadcast(bc.id as u64))
                .collect();
            if !new_bcs.is_empty() {
                let bc_strs: Vec<String> = new_bcs.iter().take(3).map(|bc| {
                    state.mark_broadcast(bc.id as u64);
                    format!("[{}] {}: {}", bc.channel, bc.from_ai, &bc.content)
                }).collect();
                parts.push(format!("NEW: {}", bc_strs.join(" | ")));
            }

            // Pending votes - get_votes returns Vec<(id, topic, creator, options, status, votes)>
            // NO TRUNCATION - full topic always
            let open_votes: Vec<_> = votes.iter()
                .filter(|(_, _, _, _, status, _)| status == "open")
                .collect();
            if !open_votes.is_empty() {
                let vote_strs: Vec<String> = open_votes.iter().map(|(id, topic, _, _, _, _)| {
                    format!("[{}] {}", id, topic)
                }).collect();
                parts.push(format!("VOTE NEEDED: {}", vote_strs.join(" | ")));
            }

            // Dialogues where it's your turn - returns Vec<(id, topic, initiator, responder, status, current_turn)>
            // NO TRUNCATION - full topic always
            if !dialogues.is_empty() {
                let d_strs: Vec<String> = dialogues.iter().map(|(id, topic, _, _, _, _)| {
                    format!("[{}] {}", id, topic)
                }).collect();
                parts.push(format!("YOUR TURN IN DIALOGUE: {}", d_strs.join(", ")));
            }

            // TEAM ACTIVITY - Deduplicated: groups by AI, collapses duplicate (verb, file) pairs
            if let Ok(file_actions) = v2.get_file_actions(20) {
                let now_micros = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_micros() as u64)
                    .unwrap_or(0);
                const RECENCY_THRESHOLD_MICROS: u64 = 5 * 60 * 1_000_000;

                let mut by_ai: std::collections::BTreeMap<&str, Vec<(&str, &str)>> = std::collections::BTreeMap::new();
                for (action_ai, action, path, ts) in &file_actions {
                    if action_ai.eq_ignore_ascii_case(&ai_id)
                        || now_micros.saturating_sub(*ts) >= RECENCY_THRESHOLD_MICROS
                    {
                        continue;
                    }
                    let filename = path.rsplit('/').next()
                        .or_else(|| path.rsplit('\\').next())
                        .unwrap_or(path);
                    let action_lower = action.to_lowercase();
                    let verb: &str = match action_lower.as_str() {
                        "read" => "reading",
                        "modified" | "write" => "editing",
                        "created" => "creating",
                        "deleted" => "deleted",
                        _ => "accessing",
                    };
                    by_ai.entry(action_ai.as_str()).or_default().push((verb, filename));
                }

                if !by_ai.is_empty() {
                    // Build the user-facing display string (with (Nx) counts) AND
                    // a separate stable identity string used only for dedup hashing.
                    // The display has volatile fields (counts that increment every
                    // tool call); the identity string is just the set of
                    // (ai, verb, file) tuples — sorted for determinism.
                    // Without this split, the same team-activity set re-injects
                    // every call because (5x) → (6x) bumps the hash.
                    let mut ai_strs: Vec<String> = Vec::new();
                    let mut identity_pairs: Vec<String> = Vec::new();
                    for (ai, entries) in &by_ai {
                        let mut deduped: Vec<(&str, &str, usize)> = Vec::new();
                        for &(v, f) in entries {
                            if let Some(existing) = deduped.iter_mut().find(|(ev, ef, _)| *ev == v && *ef == f) {
                                existing.2 += 1;
                            } else {
                                deduped.push((v, f, 1));
                            }
                        }
                        let action_strs: Vec<String> = deduped.iter().map(|(v, f, c)| {
                            if *c > 1 { format!("{} {} ({}x)", v, f, c) }
                            else { format!("{} {}", v, f) }
                        }).collect();
                        ai_strs.push(format!("[{}] {}", ai, action_strs.join(", ")));
                        let mut id_actions: Vec<String> = deduped.iter()
                            .map(|(v, f, _)| format!("{} {}", v, f))
                            .collect();
                        id_actions.sort();
                        identity_pairs.push(format!("[{}] {}", ai, id_actions.join(", ")));
                    }
                    identity_pairs.sort();
                    let team_output = format!("Team: {}", ai_strs.join(" | "));
                    let team_identity = format!("Team: {}", identity_pairs.join(" | "));
                    // 5-min cooldown on team-activity re-injection. Peek content-hash
                    // + cooldown without mutating; only commit state when actually
                    // injecting (prevents cooldown windows from losing real changes).
                    const TEAM_COOLDOWN_SECS: u64 = 5 * 60;
                    let (changed, new_hash) = PostToolHookState::content_hash_peek(state.last_team_hash, &team_identity);
                    let cooldown_ok = PostToolHookState::cooldown_elapsed(&state.last_team_inject_ts, TEAM_COOLDOWN_SECS);
                    if changed && cooldown_ok {
                        state.last_team_hash = Some(new_hash);
                        PostToolHookState::stamp_now(&mut state.last_team_inject_ts);
                        parts.push(team_output);
                    }
                }
            }

            // ACTIVE CLAIMS - Show files claimed by other AIs with context + time
            if let Ok(claims) = v2.get_claims() {
                let now_micros = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_micros() as u64)
                    .unwrap_or(0);
                let mut display_claims: Vec<String> = Vec::new();
                let mut identity_claims: Vec<String> = Vec::new();
                for (path, claim_ai, claimed_at, duration_secs, working_on) in claims.iter()
                    .filter(|(_, claim_ai, _, _, _)| claim_ai.to_lowercase() != ai_id.to_lowercase())
                    .take(3)
                {
                    let filename = path.split('/').last()
                        .or_else(|| path.split('\\').last())
                        .unwrap_or(path);
                    let idle_min = now_micros.saturating_sub(*claimed_at) / 60_000_000;
                    let ttl_min = *duration_secs as u64 / 60;
                    let ctx = if working_on.is_empty() { "editing" } else { working_on.as_str() };
                    display_claims.push(format!("{} owns {} ({}, idle {}m/{}m)", claim_ai, filename, ctx, idle_min, ttl_min));
                    identity_claims.push(format!("{} owns {} ({})", claim_ai, filename, ctx));
                }

                if !display_claims.is_empty() {
                    identity_claims.sort();
                    let claims_output = format!("Claims: {}", display_claims.join(", "));
                    let claims_identity = format!("Claims: {}", identity_claims.join(", "));
                    // 5-min cooldown on claims re-injection (QD directive 2026-04-17).
                    // Claim/release churn is chatty; content-hash alone lets every
                    // team toggle bump the hash and re-inject. Peek both gates;
                    // commit state only when we actually inject.
                    const CLAIMS_COOLDOWN_SECS: u64 = 5 * 60;
                    let (changed, new_hash) = PostToolHookState::content_hash_peek(state.last_claims_hash, &claims_identity);
                    let cooldown_ok = PostToolHookState::cooldown_elapsed(&state.last_claims_inject_ts, CLAIMS_COOLDOWN_SECS);
                    if changed && cooldown_ok {
                        state.last_claims_hash = Some(new_hash);
                        PostToolHookState::stamp_now(&mut state.last_claims_inject_ts);
                        parts.push(claims_output);
                    }
                }
            }
            // PROJECT/FEATURE CONTEXT INJECTION
            // Extract file path from tool input:
            // - Read/Edit/Write: file_path
            // - Grep: path (directory)
            // - Glob: pattern (extract directory portion) or path
            let resolved_path: Option<String> = hook_input.tool_input.as_ref().and_then(|input| {
                // Try file_path first (Read/Edit/Write)
                input.get("file_path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    // Then try path (Grep directory, Glob directory)
                    .or_else(|| input.get("path")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()))
                    // Then try pattern for Glob (extract directory portion before any wildcard)
                    .or_else(|| {
                        if tool_name == "Glob" {
                            input.get("pattern")
                                .and_then(|v| v.as_str())
                                .and_then(|p| {
                                    // Take everything before the first wildcard character
                                    let dir_end = p.find('*')
                                        .or_else(|| p.find('?'))
                                        .or_else(|| p.find('['))
                                        .unwrap_or(p.len());
                                    let dir_part = &p[..dir_end];
                                    // Trim to last path separator
                                    dir_part.rfind('/').or_else(|| dir_part.rfind('\\'))
                                        .map(|i| dir_part[..=i].to_string())
                                })
                        } else {
                            None
                        }
                    })
            });

            if let Some(ref file_path) = resolved_path {
                if let Ok(Some((proj_id, proj_name, proj_goal, _proj_dir, feature))) =
                    v2.resolve_project_for_file(file_path)
                {
                    let feat_id = feature.as_ref().map(|(id, _, _, _)| *id);
                    let (inject_project, inject_feature) =
                        state.should_inject_project(proj_id, feat_id);

                    if inject_project || inject_feature {
                        let mut ctx = String::new();
                        if inject_project {
                            ctx.push_str(&format!("[Project: {} — {}]", proj_name, proj_goal));
                        }
                        if inject_feature {
                            if let Some((_, feat_name, feat_overview, _)) = &feature {
                                if !ctx.is_empty() {
                                    ctx.push(' ');
                                }
                                ctx.push_str(&format!("[Feature: {} — {}]", feat_name, feat_overview));
                            }
                        }
                        if !ctx.is_empty() {
                            parts.push(ctx);
                        }
                    }
                }
            }

            // Time injection (only on minute change)
            if state.should_inject_time() {
                let now = Utc::now();
                let time_str = now.format("%d-%b-%Y|%I:%M%p UTC").to_string();
                parts.insert(0, format!("[NOW: {}]", time_str));
            }

            // Save state
            state.save(&ai_id);

            // Output JSON if there's anything to inject
            // No redundant HH:MM UTC prefix — [NOW: ...] handles temporal awareness on minute change
            if !parts.is_empty() {
                let output_text = parts.join(" | ");

                let output = HookOutput {
                    hook_specific_output: HookSpecificOutput {
                        hook_event_name: "PostToolUse".to_string(),
                        additional_context: format!("<system-reminder>\n{}\n</system-reminder>", output_text),
                    },
                };
                println!("{}", serde_json::to_string(&output).unwrap_or_default());
            }
            // If nothing to inject, output nothing (0 tokens)
        }

        // =====================================================================
        // UNUSED: Slim SessionStart hook - kept for potential future use
        //
        // This is a lightweight alternative to session-start.exe (in notebook-rs).
        // session-start.exe is the preferred/modern version because:
        // - Reads AI_ID from settings.json (no env var dependency)
        // - Shows full notebook (pinned + recent notes)
        // - Shows full DM content (not just count)
        // - Shows dialogues, votes, rooms, file actions
        //
        // This slim version only shows: time, team online, tasks, 3 broadcasts, DM count
        // Could be useful if a minimal injection is ever needed.
        // =====================================================================
        // CONTEXTUAL SNAPSHOT - For enriching notebook notes with episodic context
        // Philosophy: Capture CIRCUMSTANCE around what AI explicitly remembers.
        // NO TRUNCATION. Show all or nothing. No "..." previews.
        // =====================================================================
        Commands::GatherContext { dms: dm_limit, broadcasts: bc_limit, files: file_limit } => {
            let mut sentences: Vec<String> = Vec::new();
            let recency_threshold_minutes = 30;  // 30-min session window
            let now_ts = chrono::Utc::now();

            // Team presences - who's online
            if let Ok(presences) = v2.get_presences() {
                let online: Vec<String> = presences.iter()
                    .filter(|(ai, status, _)| {
                        (status == "active" || status == "standby") &&
                        ai.to_lowercase() != ai_id.to_lowercase()
                    })
                    .map(|(ai, _, _)| ai.clone())  // Keep full AI ID
                    .collect();
                if !online.is_empty() {
                    sentences.push(format!("With {} online.", online.join(", ")));
                }
            }

            // Recent DMs - FULL CONTENT, no truncation (all or nothing philosophy)
            if let Ok(dms) = v2.recent_dms(dm_limit) {
                let recent_dms: Vec<_> = dms.iter()
                    .filter(|dm| {
                        dm.from_ai.to_lowercase() != ai_id.to_lowercase() &&
                        (now_ts - dm.timestamp).num_minutes() < recency_threshold_minutes as i64
                    })
                    .take(dm_limit)
                    .collect();

                if !recent_dms.is_empty() {
                    let dm_strs: Vec<String> = recent_dms.iter()
                        .map(|dm| format!("{}: {}", dm.from_ai, dm.content))  // FULL content
                        .collect();
                    sentences.push(format!("DMs: {}.", dm_strs.join(" | ")));
                }
            }

            // Recent broadcasts - FULL CONTENT, no truncation
            if let Ok(bcs) = v2.recent_broadcasts(bc_limit, Some("general")) {
                let recent_bcs: Vec<_> = bcs.iter()
                    .filter(|bc| {
                        bc.from_ai.to_lowercase() != ai_id.to_lowercase() &&
                        (now_ts - bc.timestamp).num_minutes() < recency_threshold_minutes as i64
                    })
                    .take(bc_limit)
                    .collect();

                if !recent_bcs.is_empty() {
                    let bc_strs: Vec<String> = recent_bcs.iter()
                        .map(|bc| format!("{}: {}", bc.from_ai, bc.content))  // FULL content
                        .collect();
                    sentences.push(format!("Broadcasts: {}.", bc_strs.join(" | ")));
                }
            }

            // Active dialogues where it's my turn - full topic, no truncation
            if let Ok(dialogs) = v2.get_dialogue_my_turn() {
                if !dialogs.is_empty() {
                    let dial_parts: Vec<String> = dialogs.iter()
                        .take(3)
                        .map(|(id, initiator, responder, topic, _, _)| {
                            let other = if initiator.to_lowercase() == ai_id.to_lowercase() {
                                responder
                            } else {
                                initiator
                            };
                            format!("#{} with {} on {}", id, other, topic)  // Full topic
                        })
                        .collect();
                    sentences.push(format!("Dialogues: {}.", dial_parts.join(" | ")));
                }
            }

            // Recent file actions - full paths, who did what
            if let Ok(file_actions) = v2.get_file_actions(file_limit) {
                let file_strs: Vec<String> = file_actions.iter()
                    .take(file_limit)
                    .map(|(ai, action, path, _)| {
                        format!("{} {} {}", ai, action, path)  // Full: "alpha-001 modified /path/to/file.rs"
                    })
                    .collect();
                if !file_strs.is_empty() {
                    sentences.push(format!("Files: {}.", file_strs.join("; ")));
                }
            }

            // NO "In {instance}" - useless, we know our own instance

            // Output context string
            if sentences.is_empty() {
                println!("");
            } else {
                println!("[{}]", sentences.join(" "));
            }
        }

        // =====================================================================
        Commands::HookSessionStart => {
            eprintln!("NOTE: hook-session-start is deprecated. Use session-start.exe instead.");
            eprintln!("It reads AI_ID from settings.json and provides full context injection.");
            std::process::exit(0);

            /* COMMENTED OUT - slim version preserved for reference:
            let mut parts: Vec<String> = Vec::new();

            // Current time
            let now = Utc::now();
            let time_str = now.format("%d-%b-%Y|%I:%M%p UTC").to_string();
            parts.push(format!("[NOW: {}]", time_str));

            // Team status - get_presences returns Vec<(ai_id, status, task)>
            let presences = v2.get_presences().unwrap_or_default();
            let online: Vec<_> = presences.iter()
                .filter(|(_, status, _)| status == "active" || status == "standby")
                .collect();
            if !online.is_empty() {
                let names: Vec<String> = online.iter().map(|(ai, _, _)| ai.clone()).collect();
                parts.push(format!("Team online: {}", names.join(", ")));
            }

            // Pending tasks - get_tasks returns Vec<(id, description, priority, status, claimed_by)>
            let tasks = v2.get_tasks().unwrap_or_default();
            let my_tasks: Vec<_> = tasks.iter()
                .filter(|(_, _, _, status, claimed_by)| {
                    status == "claimed" && claimed_by.as_ref().map(|s| s.as_str()) == Some(&ai_id)
                })
                .collect();
            if !my_tasks.is_empty() {
                let task_strs: Vec<String> = my_tasks.iter().map(|(id, desc, _, _, _)| {
                    let d = if desc.len() > 30 { &desc[..30] } else { desc };
                    format!("[{}] {}", id, d)
                }).collect();
                parts.push(format!("Your tasks: {}", task_strs.join(", ")));
            }

            // Recent broadcasts - NO TRUNCATION
            let broadcasts = v2.recent_broadcasts(3, Some("general")).unwrap_or_default();
            if !broadcasts.is_empty() {
                let bc_strs: Vec<String> = broadcasts.iter().take(3).map(|bc| {
                    format!("{}: {}", bc.from_ai, &bc.content)
                }).collect();
                parts.push(format!("Recent: {}", bc_strs.join(" | ")));
            }

            // Unread DMs
            let dms = v2.recent_dms(5).unwrap_or_default();
            if !dms.is_empty() {
                parts.push(format!("You have {} unread DM(s)", dms.len()));
            }

            // Output JSON
            let output_text = parts.join(" | ");
            let output = HookOutput {
                hook_specific_output: HookSpecificOutput {
                    hook_event_name: "SessionStart".to_string(),
                    additional_context: format!("<system-reminder>\n{}\n</system-reminder>", output_text),
                },
            };
            println!("{}", serde_json::to_string(&output).unwrap_or_default());
            */
        }

        // ===== AWARENESS (V2) =====
        // Aggregated awareness data for autonomous-passive injection
        // Zero cognition required - this is injected by hooks automatically
        Commands::Awareness { limit } => {
            // Output format compatible with PostToolUse hook
            // dm|id|from|content
            // bc|id|from|channel|content
            // vote|id|topic|cast|total
            // dialogue|id|topic (where it's your turn)
            // claim|path|owner|reason

            // Recent DMs TO this AI
            if let Ok(dms) = v2.recent_dms(limit) {
                for dm in dms {
                    // Only show DMs from others, not from self
                    if dm.from_ai.to_lowercase() != ai_id.to_lowercase() {
                        println!("dm|{}|{}|{}", dm.id, dm.from_ai, dm.content);
                    }
                }
            }

            // Recent broadcasts
            if let Ok(bcs) = v2.recent_broadcasts(limit, Some("general")) {
                for bc in bcs {
                    if bc.from_ai.to_lowercase() != ai_id.to_lowercase() {
                        println!("bc|{}|{}|{}|{}", bc.id, bc.from_ai, bc.channel, bc.content);
                    }
                }
            }

            // Open votes
            if let Ok(votes) = v2.get_votes() {
                for (id, _creator, topic, _options, status, casts) in votes {
                    if status == "open" {
                        println!("vote|{}|{}|{}|{}", id, topic, casts.len(), 3);
                    }
                }
            }

            // Dialogues where it's my turn
            if let Ok(dialogues) = v2.get_dialogue_my_turn() {
                for (id, initiator, responder, topic, _, _) in dialogues {
                    let other = if initiator.to_lowercase() == ai_id.to_lowercase() {
                        &responder
                    } else {
                        &initiator
                    };
                    println!("dialogue|{}|{}|{}", id, other, topic);
                }
            }

            // Active file claims held by others
            if let Ok(claims) = v2.get_claims() {
                for (path, owner, _ts, _dur, working_on) in claims {
                    if owner.to_lowercase() != ai_id.to_lowercase() {
                        println!("claim|{}|{}|{}", path, owner, working_on);
                    }
                }
            }
        }

        // ===== MOBILE PAIRING =====

        Commands::MobilePair { code } => {
            use std::io::{Read, Write};
            use std::net::TcpStream;

            let port = std::env::var("MOBILE_API_PORT").unwrap_or_else(|_| "8081".to_string());
            let addr = format!("127.0.0.1:{}", port);
            let mut stream = TcpStream::connect(&addr)
                .map_err(|e| anyhow::anyhow!("Cannot connect to mobile-api at {}: {}", addr, e))?;

            let body = format!("{{\"code\":\"{}\"}}", code);
            let request = format!(
                "POST /api/pair/approve HTTP/1.1\r\nHost: localhost:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                port, body.len(), body
            );
            stream.write_all(request.as_bytes())?;

            let mut response = String::new();
            stream.read_to_string(&mut response)?;

            if let Some(body_start) = response.find("\r\n\r\n") {
                let resp_body = &response[body_start + 4..];
                match serde_json::from_str::<serde_json::Value>(resp_body) {
                    Ok(json) if json["ok"].as_bool().unwrap_or(false) => {
                        let h_id = json["h_id"].as_str().unwrap_or("unknown");
                        println!("pair_approved|{}|{}", code, h_id);
                    }
                    Ok(json) => {
                        let error = json["error"].as_str().unwrap_or("unknown error");
                        eprintln!("pair_failed|{}|{}", code, error);
                        std::process::exit(1);
                    }
                    Err(_) => {
                        eprintln!("pair_failed|{}|invalid JSON in response", code);
                        std::process::exit(1);
                    }
                }
            } else {
                eprintln!("pair_failed|{}|malformed HTTP response", code);
                std::process::exit(1);
            }
        }

        // Commands not yet implemented in event sourcing backend
        _ => {
            eprintln!("Command not yet implemented in V2 backend");
            eprintln!("Use without --v2 flag for full functionality");
            std::process::exit(1);
        }
    }

    Ok(())
}

// NO TRUNCATION FUNCTION - QD explicitly stated truncation degrades tool functionality
// from ~90% to ~20%. Full content preserves context and AI collaboration effectiveness.

/// Convert milliseconds since epoch to UTC ISO 8601 timestamp
/// Format: 2026-02-01T11:43:25Z (universally parseable by any cognitive entity)
fn to_utc(millis: u64) -> String {
    let secs = (millis / 1000) as i64;
    let datetime = chrono::DateTime::from_timestamp(secs, 0)
        .unwrap_or_else(|| chrono::DateTime::from_timestamp(0, 0).unwrap());
    datetime.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

