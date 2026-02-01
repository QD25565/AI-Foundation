//! CLI interface for teambook-rs
//!
//! AI-friendly CLI following best practices:
//! - Positional args as primary interface
//! - 4-6 aliases per command
//! - Hidden long flags for AIs that try flag syntax
//! - Real examples in help text
//! - NO short flags (they're noise)

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "teambook")]
#[command(about = "High-performance AI coordination system", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    // Infrastructure flags - hidden from help (use env vars primarily)
    #[arg(long, hide = true)]
    pub ai_id: Option<String>,
    #[arg(long, hide = true)]
    pub postgres_url: Option<String>,
    #[arg(long, hide = true)]
    pub redis_url: Option<String>,
}

#[derive(Subcommand)]
pub enum Commands {
    // ===== CORE MESSAGING =====

    /// Write a note to the shared teambook
    #[command(alias = "add", alias = "post", alias = "note", alias = "save")]
    Write {
        /// Note content (e.g., "Found bug in auth module")
        #[arg(value_name = "CONTENT")]
        content: Option<String>,

        /// Tags (e.g., "bug,auth,urgent")
        #[arg(value_name = "TAGS")]
        tags: Option<String>,

        // Hidden long flags
        #[arg(long = "content", hide = true)]
        content_flag: Option<String>,
        #[arg(long = "tags", hide = true)]
        tags_flag: Option<String>,
    },

    /// Read recent notes from the teambook
    #[command(alias = "notes", alias = "recent", alias = "get", alias = "show")]
    Read {
        /// Number of notes to read (e.g., 10, 20, 50)
        #[arg(value_name = "LIMIT", default_value = "10")]
        limit: Option<i32>,

        #[arg(long = "limit", hide = true)]
        limit_flag: Option<i32>,
    },

    /// Broadcast a message to all AIs
    #[command(alias = "bc", alias = "announce", alias = "shout", alias = "post-all", alias = "say")]
    Broadcast {
        /// Message content (e.g., "Starting work on auth refactor")
        #[arg(value_name = "MESSAGE")]
        content: Option<String>,

        /// Channel name (e.g., "general", "urgent", "dev")
        #[arg(value_name = "CHANNEL", default_value = "general")]
        channel: Option<String>,

        #[arg(long = "content", hide = true)]
        content_flag: Option<String>,
        #[arg(long = "channel", hide = true)]
        channel_flag: Option<String>,
    },

    /// Send a direct message to another AI
    #[command(alias = "dm", alias = "send", alias = "msg", alias = "tell", alias = "message", alias = "pm")]
    DirectMessage {
        /// Target AI (e.g., cascade-230, sage-724)
        #[arg(value_name = "TO_AI")]
        to_ai: Option<String>,

        /// Message content
        #[arg(value_name = "MESSAGE")]
        content: Option<String>,

        #[arg(long = "to-ai", hide = true)]
        to_ai_flag: Option<String>,
        #[arg(long = "content", hide = true)]
        content_flag: Option<String>,
    },

    /// Read recent broadcast messages
    #[command(alias = "msgs", alias = "broadcasts", alias = "feed", alias = "stream")]
    Messages {
        /// Number of messages (e.g., 10, 20, 50)
        #[arg(value_name = "LIMIT", default_value = "10")]
        limit: Option<i32>,

        #[arg(long = "limit", hide = true)]
        limit_flag: Option<i32>,
    },

    /// Read your direct messages
    #[command(alias = "dms", alias = "inbox", alias = "private", alias = "my-messages")]
    DirectMessages {
        /// Number of messages (e.g., 5, 10, 20)
        #[arg(value_name = "LIMIT", default_value = "10")]
        limit: Option<i32>,

        #[arg(long = "limit", hide = true)]
        limit_flag: Option<i32>,
    },

    /// Show team status and online AIs
    #[command(alias = "who", alias = "online", alias = "team", alias = "info", alias = "state")]
    Status,

    // ===== VOTING =====

    /// Create a new vote for team consensus
    #[command(alias = "new-vote", alias = "poll", alias = "create-poll", alias = "ask")]
    VoteCreate {
        /// Vote topic/question (e.g., "Which auth approach?")
        #[arg(value_name = "TOPIC")]
        topic: Option<String>,

        /// Options comma-separated (e.g., "JWT,OAuth,Session")
        #[arg(value_name = "OPTIONS")]
        options: Option<String>,

        /// Expected voters (e.g., 4)
        #[arg(value_name = "VOTERS", default_value = "4")]
        voters: Option<i32>,

        #[arg(long = "topic", hide = true)]
        topic_flag: Option<String>,
        #[arg(long = "options", hide = true)]
        options_flag: Option<String>,
        #[arg(long = "voters", hide = true)]
        voters_flag: Option<i32>,
    },

    /// Cast your vote on an open poll
    #[command(alias = "vote", alias = "cast", alias = "choose", alias = "pick")]
    VoteCast {
        /// Vote ID (e.g., 42)
        #[arg(value_name = "VOTE_ID")]
        vote_id: Option<i32>,

        /// Your choice (must match an option)
        #[arg(value_name = "CHOICE")]
        choice: Option<String>,

        #[arg(long = "vote-id", hide = true)]
        vote_id_flag: Option<i32>,
        #[arg(long = "choice", hide = true)]
        choice_flag: Option<String>,
    },

    /// List open and recent votes
    #[command(alias = "votes", alias = "polls", alias = "list-polls", alias = "show-votes")]
    VoteList {
        /// Number of votes to show (e.g., 10)
        #[arg(value_name = "LIMIT", default_value = "10")]
        limit: Option<i32>,

        #[arg(long = "limit", hide = true)]
        limit_flag: Option<i32>,
    },

    /// Get results of a specific vote
    #[command(alias = "results", alias = "tally", alias = "count", alias = "outcome")]
    VoteResults {
        /// Vote ID (e.g., 42)
        #[arg(value_name = "VOTE_ID")]
        vote_id: Option<i32>,

        #[arg(long = "vote-id", hide = true)]
        vote_id_flag: Option<i32>,
    },

    /// Show votes waiting for your input
    #[command(alias = "pending", alias = "my-votes", alias = "awaiting", alias = "todo-votes")]
    VotePending,

    // ===== FILE CLAIMS / LOCKING =====

    /// Claim/lock a file before editing
    #[command(alias = "claim", alias = "lock", alias = "grab", alias = "reserve", alias = "take")]
    ClaimFile {
        /// File path (e.g., src/auth/login.rs)
        #[arg(value_name = "FILE_PATH")]
        file: Option<String>,

        /// Duration in minutes (e.g., 10, 30, 60)
        #[arg(value_name = "DURATION", default_value = "10")]
        duration: Option<i32>,

        #[arg(long = "file", hide = true)]
        file_flag: Option<String>,
        #[arg(long = "duration", hide = true)]
        duration_flag: Option<i32>,
    },

    /// Release a file claim/lock
    #[command(alias = "release", alias = "unlock", alias = "unclaim", alias = "free", alias = "drop")]
    ReleaseFile {
        /// File path (e.g., src/auth/login.rs)
        #[arg(value_name = "FILE_PATH")]
        file: Option<String>,

        #[arg(long = "file", hide = true)]
        file_flag: Option<String>,
    },

    /// Check if a file is claimed/locked
    #[command(alias = "check", alias = "is-locked", alias = "who-has", alias = "file-status")]
    CheckFile {
        /// File path (e.g., src/auth/login.rs)
        #[arg(value_name = "FILE_PATH")]
        file: Option<String>,

        #[arg(long = "file", hide = true)]
        file_flag: Option<String>,
    },

    /// List all active file claims
    #[command(alias = "claims", alias = "locks", alias = "locked-files", alias = "show-claims")]
    ListClaims,

    /// Enter standby mode (wait for events)
    #[command(alias = "wait", alias = "listen", alias = "monitor", alias = "idle")]
    Standby {
        /// Check interval in seconds (e.g., 30)
        #[arg(value_name = "INTERVAL", default_value = "30")]
        interval: Option<u64>,

        #[arg(long = "interval", hide = true)]
        interval_flag: Option<u64>,
    },

    // ===== ROOMS - Private Collaboration Spaces =====

    /// Create a new collaboration room
    #[command(name = "room-create", alias = "new-room", alias = "create-room", alias = "open-room")]
    RoomCreate {
        /// Room name (e.g., "auth-review", "bug-triage")
        #[arg(value_name = "NAME")]
        name: Option<String>,

        /// Mode: pair, review, brainstorm, workshop
        #[arg(value_name = "MODE", default_value = "pair")]
        mode: Option<String>,

        /// Join mode: open or invite
        #[arg(value_name = "JOIN_MODE", default_value = "open")]
        join_mode: Option<String>,

        /// Hours until expiry (e.g., 4, 24)
        #[arg(value_name = "EXPIRES_HOURS", default_value = "24")]
        expires_hours: Option<i32>,

        #[arg(long = "name", hide = true)]
        name_flag: Option<String>,
        #[arg(long = "mode", hide = true)]
        mode_flag: Option<String>,
        #[arg(long = "join-mode", hide = true)]
        join_mode_flag: Option<String>,
        #[arg(long = "expires-hours", hide = true)]
        expires_hours_flag: Option<i32>,
    },

    /// Join an existing room
    #[command(name = "room-join", alias = "join", alias = "enter", alias = "join-room")]
    RoomJoin {
        /// Room ID (e.g., 5)
        #[arg(value_name = "ROOM_ID")]
        room_id: Option<i32>,

        /// Role: participant or observer
        #[arg(value_name = "ROLE", default_value = "participant")]
        role: Option<String>,

        #[arg(long = "room-id", hide = true)]
        room_id_flag: Option<i32>,
        #[arg(long = "role", hide = true)]
        role_flag: Option<String>,
    },

    /// Leave a room
    #[command(name = "room-leave", alias = "leave", alias = "exit", alias = "leave-room")]
    RoomLeave {
        /// Room ID (e.g., 5)
        #[arg(value_name = "ROOM_ID")]
        room_id: Option<i32>,

        #[arg(long = "room-id", hide = true)]
        room_id_flag: Option<i32>,
    },

    /// Send message to room
    #[command(name = "room-send", alias = "room-msg", alias = "room-say", alias = "room-post")]
    RoomSend {
        /// Room ID (e.g., 5)
        #[arg(value_name = "ROOM_ID")]
        room_id: Option<i32>,

        /// Message content
        #[arg(value_name = "MESSAGE")]
        content: Option<String>,

        #[arg(long = "room-id", hide = true)]
        room_id_flag: Option<i32>,
        #[arg(long = "content", hide = true)]
        content_flag: Option<String>,
    },

    /// Read room messages
    #[command(name = "room-read", alias = "room-msgs", alias = "room-history", alias = "room-log")]
    RoomRead {
        /// Room ID (e.g., 5)
        #[arg(value_name = "ROOM_ID")]
        room_id: Option<i32>,

        /// Number of messages
        #[arg(value_name = "LIMIT", default_value = "50")]
        limit: Option<i32>,

        #[arg(long = "room-id", hide = true)]
        room_id_flag: Option<i32>,
        #[arg(long = "limit", hide = true)]
        limit_flag: Option<i32>,
    },

    /// List active rooms
    #[command(name = "room-list", alias = "rooms", alias = "list-rooms", alias = "show-rooms")]
    RoomList,

    /// Get room details
    #[command(name = "room-get", alias = "room-info", alias = "room-status", alias = "get-room")]
    RoomGet {
        /// Room ID (e.g., 5)
        #[arg(value_name = "ROOM_ID")]
        room_id: Option<i32>,

        #[arg(long = "room-id", hide = true)]
        room_id_flag: Option<i32>,
    },

    // ===== LOCKS - Resource Locking =====

    /// Acquire a lock on a resource
    #[command(name = "lock-acquire", alias = "lock", alias = "acquire", alias = "grab-lock")]
    LockAcquire {
        /// Resource to lock (e.g., "auth-module")
        #[arg(value_name = "RESOURCE")]
        resource: Option<String>,

        /// What you're working on
        #[arg(value_name = "WORKING_ON", default_value = "")]
        working_on: Option<String>,

        /// Timeout in minutes
        #[arg(value_name = "TIMEOUT", default_value = "30")]
        timeout_mins: Option<i32>,

        #[arg(long = "resource", hide = true)]
        resource_flag: Option<String>,
        #[arg(long = "working-on", hide = true)]
        working_on_flag: Option<String>,
        #[arg(long = "timeout-mins", hide = true)]
        timeout_mins_flag: Option<i32>,
    },

    /// Release a lock
    #[command(name = "lock-release", alias = "unlock", alias = "release-lock", alias = "free-lock")]
    LockRelease {
        /// Resource to release
        #[arg(value_name = "RESOURCE")]
        resource: Option<String>,

        #[arg(long = "resource", hide = true)]
        resource_flag: Option<String>,
    },

    /// Extend a lock timeout
    #[command(name = "lock-extend", alias = "extend", alias = "extend-lock", alias = "renew-lock")]
    LockExtend {
        /// Resource to extend
        #[arg(value_name = "RESOURCE")]
        resource: Option<String>,

        /// Additional minutes
        #[arg(value_name = "ADDITIONAL_MINS", default_value = "30")]
        additional_mins: Option<i32>,

        #[arg(long = "resource", hide = true)]
        resource_flag: Option<String>,
        #[arg(long = "additional-mins", hide = true)]
        additional_mins_flag: Option<i32>,
    },

    /// Check if resource is locked
    #[command(name = "lock-check", alias = "check-lock", alias = "is-locked", alias = "lock-status")]
    LockCheck {
        /// Resource to check
        #[arg(value_name = "RESOURCE")]
        resource: Option<String>,

        #[arg(long = "resource", hide = true)]
        resource_flag: Option<String>,
    },

    /// List all active locks
    #[command(name = "lock-list", alias = "locks", alias = "list-locks", alias = "show-locks")]
    LockList,

    // ===== TASK QUEUE =====

    /// Queue a task for the team
    #[command(name = "task-queue", alias = "add-task", alias = "queue", alias = "new-task")]
    TaskQueue {
        /// Task description
        #[arg(value_name = "TASK")]
        task: Option<String>,

        /// Priority 1-10 (10 highest)
        #[arg(value_name = "PRIORITY", default_value = "5")]
        priority: Option<i32>,

        /// Needs verification after
        #[arg(long)]
        needs_verify: bool,

        /// Tags (comma-separated)
        #[arg(value_name = "TAGS", default_value = "")]
        tags: Option<String>,

        #[arg(long = "task", hide = true)]
        task_flag: Option<String>,
        #[arg(long = "priority", hide = true)]
        priority_flag: Option<i32>,
        #[arg(long = "tags", hide = true)]
        tags_flag: Option<String>,
    },

    /// Claim a task from queue
    #[command(name = "task-claim", alias = "claim-task", alias = "take-task", alias = "grab-task")]
    TaskClaim {
        /// Task ID (omit for highest priority)
        #[arg(value_name = "TASK_ID")]
        task_id: Option<i32>,

        #[arg(long = "task-id", hide = true)]
        task_id_flag: Option<i32>,
    },

    /// Complete a claimed task
    #[command(name = "task-complete", alias = "complete", alias = "done", alias = "finish-task")]
    TaskComplete {
        /// Task ID to complete
        #[arg(value_name = "TASK_ID")]
        task_id: Option<i32>,

        /// Result/summary
        #[arg(value_name = "RESULT", default_value = "")]
        result: Option<String>,

        #[arg(long = "task-id", hide = true)]
        task_id_flag: Option<i32>,
        #[arg(long = "result", hide = true)]
        result_flag: Option<String>,
    },

    /// Get task queue stats
    #[command(name = "task-queue-stats", alias = "queue-stats", alias = "task-stats", alias = "q-stats")]
    TaskQueueStats,

    /// List queued tasks
    #[command(name = "task-queue-list", alias = "tasks", alias = "list-tasks", alias = "show-tasks")]
    TaskQueueList {
        /// Include completed tasks
        #[arg(long)]
        include_completed: bool,

        /// Limit
        #[arg(value_name = "LIMIT", default_value = "20")]
        limit: Option<i32>,

        #[arg(long = "limit", hide = true)]
        limit_flag: Option<i32>,
    },

    /// Verify a completed task
    #[command(name = "task-verify", alias = "verify", alias = "check-task", alias = "approve-task")]
    TaskVerify {
        /// Task ID to verify
        #[arg(value_name = "TASK_ID")]
        task_id: Option<i32>,

        /// Passed verification
        #[arg(long)]
        passed: bool,

        /// Verification notes
        #[arg(value_name = "NOTES", default_value = "")]
        notes: Option<String>,

        #[arg(long = "task-id", hide = true)]
        task_id_flag: Option<i32>,
        #[arg(long = "notes", hide = true)]
        notes_flag: Option<String>,
    },

    // ===== DIALOGUES =====

    /// Start a structured 1-on-1 dialogue with another AI (e.g., dialogue-start lyra-584 "API design")
    #[command(name = "dialogue-start", alias = "dialogue", alias = "chat-start", alias = "converse")]
    DialogueStart {
        /// Target AI (e.g., lyra-584, cascade-230)
        #[arg(value_name = "WITH_AI")]
        with_ai: Option<String>,

        /// Topic for discussion (e.g., "API design review")
        #[arg(value_name = "TOPIC")]
        topic: Option<String>,

        #[arg(long = "with-ai", hide = true)]
        with_ai_flag: Option<String>,
        #[arg(long = "topic", hide = true)]
        topic_flag: Option<String>,
    },

    /// Respond in a dialogue - your turn only (e.g., dialogue-respond 1 "I agree, let's proceed")
    #[command(name = "dialogue-respond", alias = "reply", alias = "dialogue-reply", alias = "respond")]
    DialogueRespond {
        /// Dialogue ID (e.g., 17)
        #[arg(value_name = "DIALOGUE_ID")]
        dialogue_id: Option<i32>,

        /// Your response message
        #[arg(value_name = "MESSAGE")]
        message: Option<String>,

        #[arg(long = "dialogue-id", hide = true)]
        dialogue_id_flag: Option<i32>,
        #[arg(long = "message", hide = true)]
        message_flag: Option<String>,
    },

    /// End a dialogue session (e.g., dialogue-end 1 "concluded")
    #[command(name = "dialogue-end", alias = "end-dialogue", alias = "close-dialogue", alias = "dialogue-close")]
    DialogueEnd {
        /// Dialogue ID (e.g., 17)
        #[arg(value_name = "DIALOGUE_ID")]
        dialogue_id: Option<i32>,

        /// Summary/conclusion message
        #[arg(value_name = "SUMMARY")]
        summary: Option<String>,

        #[arg(long = "dialogue-id", hide = true)]
        dialogue_id_flag: Option<i32>,
        #[arg(long = "summary", hide = true)]
        summary_flag: Option<String>,
    },

    /// List your dialogues - active and recent
    #[command(name = "dialogue-list", alias = "dialogues", alias = "list-dialogues", alias = "my-dialogues")]
    DialogueList {
        /// Number of dialogues to show
        #[arg(value_name = "LIMIT", default_value = "10")]
        limit: Option<i32>,

        #[arg(long = "limit", hide = true)]
        limit_flag: Option<i32>,
    },

    /// View dialogue history/messages (e.g., dialogue-history 1)
    #[command(name = "dialogue-history", alias = "dialogue-get", alias = "show-dialogue", alias = "dialogue-show")]
    DialogueHistory {
        /// Dialogue ID (e.g., 17)
        #[arg(value_name = "DIALOGUE_ID")]
        dialogue_id: Option<i32>,

        #[arg(long = "dialogue-id", hide = true)]
        dialogue_id_flag: Option<i32>,
    },

    /// Check pending dialogue invites from other AIs
    #[command(name = "dialogue-invites", alias = "invites", alias = "pending-dialogues", alias = "dialogue-pending")]
    DialogueInvites,

    /// Check dialogues where it's your turn to respond
    #[command(name = "dialogue-my-turn", alias = "my-turn", alias = "dialogue-waiting", alias = "waiting-dialogues")]
    DialogueMyTurn,

    /// Check whose turn it is for a specific dialogue (e.g., dialogue-turn 17)
    #[command(name = "dialogue-turn", alias = "whose-turn", alias = "turn", alias = "whose-dialogue-turn")]
    DialogueTurn {
        /// Dialogue ID (e.g., 17)
        #[arg(value_name = "DIALOGUE_ID")]
        dialogue_id: Option<i32>,

        #[arg(long = "dialogue-id", hide = true)]
        dialogue_id_flag: Option<i32>,
    },
}
