//! View Engine - Materialized Views from Event Log
//!
//! The View Engine maintains per-AI local views derived from the master event log.
//! Each AI has their own view, optimized for their queries.
//!
//! Architecture:
//! ```text
//! EVENT LOG (shared) → Per-AI View Engine → Local Stats/Indexes + Content Caches
//! ```
//!
//! Content caches provide O(1) or O(k) access to recent data instead of O(n) log scans.
//! Caches are ephemeral (rebuilt on startup from last 10K events) - event log remains source of truth.

use crate::event::{Event, EventPayload, event_type};
use crate::event_log::EventLogReader;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::fs;
use std::io;

/// View Engine error types
#[derive(Debug)]
pub enum ViewError {
    Io(io::Error),
    EventLog(String),
    Storage(String),
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ViewError::Io(e) => write!(f, "IO error: {}", e),
            ViewError::EventLog(e) => write!(f, "Event log error: {}", e),
            ViewError::Storage(e) => write!(f, "Storage error: {}", e),
        }
    }
}

impl std::error::Error for ViewError {}

impl From<io::Error> for ViewError {
    fn from(e: io::Error) -> Self {
        ViewError::Io(e)
    }
}

pub type ViewResult<T> = Result<T, ViewError>;

/// Ring buffer size limits for content caches
const MAX_CACHED_DMS: usize = 100;
const MAX_CACHED_BROADCASTS_PER_CHANNEL: usize = 100;
const MAX_CACHED_FILE_ACTIONS: usize = 100;
const WARMUP_EVENT_COUNT: u64 = 10_000;

// ============== CACHE STRUCTS ==============

/// Cached direct message
#[derive(Debug, Clone)]
pub struct CachedDM {
    pub id: u64,           // Event sequence number
    pub from_ai: String,
    pub to_ai: String,
    pub content: String,
    pub timestamp: u64,    // Microseconds since epoch
    pub read: bool,
}

/// Cached broadcast message
#[derive(Debug, Clone)]
pub struct CachedBroadcast {
    pub id: u64,           // Event sequence number
    pub from_ai: String,
    pub channel: String,
    pub content: String,
    pub timestamp: u64,
}

/// Message within a dialogue
#[derive(Debug, Clone)]
pub struct DialogueMessage {
    pub sequence: u64,
    pub from_ai: String,
    pub content: String,
    pub timestamp: u64,
}

/// Cached dialogue state
#[derive(Debug, Clone)]
pub struct DialogueState {
    pub id: u64,
    pub initiator: String,
    pub responder: String,     // First non-initiator (kept for display/compat)
    pub participants: Vec<String>,  // Full ordered list; round-robin turn order
    pub topic: String,
    pub status: String,        // "active", "resolved", "abandoned", "merged:XXXX"
    pub current_turn: String,  // AI who should respond next
    pub messages: VecDeque<DialogueMessage>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Cached task state
#[derive(Debug, Clone)]
pub struct TaskState {
    pub id: u64,
    pub description: String,
    pub priority: i32,
    pub status: String,        // "pending", "claimed", "in_progress", "completed", "blocked"
    pub assignee: Option<String>,
    pub tags: String,
    pub block_reason: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Cached vote state
#[derive(Debug, Clone)]
pub struct VoteState {
    pub id: u64,
    pub creator: String,
    pub topic: String,
    pub options: Vec<String>,
    pub status: String,        // "open", "closed"
    pub casts: Vec<(String, String)>,  // (voter_ai, choice)
    pub total_voters: u32,
    pub created_at: u64,
}

/// Cached batch state
#[derive(Debug, Clone)]
pub struct BatchState {
    pub name: String,
    pub creator: String,
    pub tasks: Vec<(String, String)>,  // (label, description)
    pub done: HashSet<String>,         // Labels of completed tasks
    pub is_closed: bool,
    pub created_at: u64,
}

/// Cached file claim
#[derive(Debug, Clone)]
pub struct FileClaimState {
    pub path: String,
    pub holder: String,
    pub working_on: String,
    pub duration_seconds: u32,
    pub claimed_at: u64,       // Timestamp in microseconds
}

/// Cached presence
#[derive(Debug, Clone)]
pub struct PresenceState {
    pub ai_id: String,
    pub status: String,
    pub current_task: String,
    pub last_seen: u64,        // Any event from this AI
    pub last_presence_update: u64,  // Last PRESENCE_UPDATE event
}

// LockState removed — locks deprecated (Feb 2026, QD directive)
// PheromoneState removed — stigmergy deprecated (Feb 2026, QD directive)

/// Cached file action
#[derive(Debug, Clone)]
pub struct FileActionState {
    pub id: u64,
    pub ai_id: String,
    pub path: String,
    pub action: String,
    pub timestamp: u64,
}

/// Cached project state
#[derive(Debug, Clone)]
pub struct ProjectState {
    pub id: u64,            // Canonical ID = event timestamp
    pub name: String,
    pub goal: String,
    pub root_directory: String,
    pub status: String,     // "active", "archived"
    pub is_deleted: bool,
    pub created_at: u64,
}

/// Cached feature state
#[derive(Debug, Clone)]
pub struct FeatureState {
    pub id: u64,            // Canonical ID = event timestamp
    pub project_id: u64,   // Canonical project ID (timestamp)
    pub name: String,
    pub overview: String,
    pub directory: Option<String>,
    pub is_deleted: bool,
    pub created_at: u64,
}

/// Cached room state
#[derive(Debug, Clone)]
pub struct RoomState {
    pub id: u64,
    pub name: String,
    pub topic: String,
    pub members: Vec<String>,
    pub messages: VecDeque<(u64, String, String, u64)>,  // (seq, from_ai, content, timestamp)
    pub created_at: u64,
    pub is_closed: bool,
    pub mutes: HashMap<String, u64>,       // ai_id → expires_at_millis (lazy expiry)
    pub conclusion: Option<String>,
    pub pinned_messages: Vec<u64>,         // room message seq IDs (room-native only)
}

// ============== END CACHE STRUCTS ==============

/// View statistics - tracked in memory
#[derive(Debug, Clone, Default)]
pub struct ViewStats {
    pub cursor: u64,
    pub unread_dms: u64,
    pub active_dialogues: u64,
    pub pending_votes: u64,
    pub my_tasks: u64,
    pub events_applied: u64,
}

/// Trust score using Beta distribution: Trust = α/(α+β)
/// TIP (Trust Inference and Propagation) implementation
#[derive(Debug, Clone, Default)]
pub struct TrustScore {
    pub alpha: u32,  // Positive experiences (successes)
    pub beta: u32,   // Negative experiences (failures)
}

impl TrustScore {
    /// Calculate trust value as α/(α+β), defaults to 0.5 for no data
    pub fn trust_value(&self) -> f64 {
        let total = self.alpha + self.beta;
        if total == 0 {
            0.5  // Prior: neutral trust
        } else {
            self.alpha as f64 / total as f64
        }
    }

    /// Record a trust event with optional weight
    pub fn record(&mut self, is_success: bool, weight: u8) {
        let w = weight.max(1) as u32;
        if is_success {
            self.alpha = self.alpha.saturating_add(w);
        } else {
            self.beta = self.beta.saturating_add(w);
        }
    }
}

/// Per-AI Materialized View
///
/// Contains both statistics (counters) and content caches (actual message/task data).
/// Content caches are ephemeral - rebuilt from event log on startup.
/// Event log remains the source of truth.
pub struct ViewEngine {
    ai_id: String,
    view_dir: PathBuf,
    cursor: u64,
    stats: ViewStats,

    // === Content Caches (ephemeral, rebuilt on startup) ===

    /// Recent DMs to this AI (ring buffer, max 100)
    recent_dms: VecDeque<CachedDM>,

    /// Recent broadcasts by channel (ring buffer per channel, max 100 each)
    recent_broadcasts: HashMap<String, VecDeque<CachedBroadcast>>,

    /// All channel broadcasts combined (ring buffer, max 100)
    all_broadcasts: VecDeque<CachedBroadcast>,

    /// Active and recent dialogues involving this AI
    dialogues: HashMap<u64, DialogueState>,

    /// All tasks
    tasks: HashMap<u64, TaskState>,

    /// All votes
    votes: HashMap<u64, VoteState>,

    /// Active batches (name -> state)
    batches: HashMap<String, BatchState>,

    /// Active file claims (path -> claim)
    file_claims: HashMap<String, FileClaimState>,

    /// AI presences (ai_id -> presence)
    presences: HashMap<String, PresenceState>,

    // locks removed — deprecated (Feb 2026)
    // pheromones removed — deprecated (Feb 2026)

    /// Recent file actions (ring buffer, max 100)
    file_actions: VecDeque<FileActionState>,

    /// Active rooms
    rooms: HashMap<u64, RoomState>,

    /// All projects (id -> state)
    projects: HashMap<u64, ProjectState>,

    /// All features (id -> state)
    features: HashMap<u64, FeatureState>,

    /// Per-AI trust scores (TIP aggregation)
    ai_trust: HashMap<String, TrustScore>,
}

impl ViewEngine {
    /// Create or open a view for an AI
    ///
    /// Note: Caches start empty. Call warm_cache() after opening to populate
    /// them from the event log.
    pub fn open(ai_id: &str, data_dir: &Path) -> ViewResult<Self> {
        let view_dir = data_dir.join("views");
        fs::create_dir_all(&view_dir)?;

        // Load cursor from simple file (8 bytes)
        let cursor = Self::load_cursor(&view_dir, ai_id);

        Ok(Self {
            ai_id: ai_id.to_string(),
            view_dir,
            cursor,
            stats: ViewStats::default(),

            // All caches start empty - populated by warm_cache()
            recent_dms: VecDeque::new(),
            recent_broadcasts: HashMap::new(),
            all_broadcasts: VecDeque::new(),
            dialogues: HashMap::new(),
            tasks: HashMap::new(),
            votes: HashMap::new(),
            batches: HashMap::new(),
            file_claims: HashMap::new(),
            presences: HashMap::new(),
            file_actions: VecDeque::new(),
            rooms: HashMap::new(),
            projects: HashMap::new(),
            features: HashMap::new(),
            ai_trust: HashMap::new(),
        })
    }

    /// Load cursor from simple file
    fn load_cursor(view_dir: &Path, ai_id: &str) -> u64 {
        let path = view_dir.join(format!("{}.cursor", ai_id));
        fs::read(&path)
            .ok()
            .and_then(|b| b.try_into().ok())
            .map(u64::from_le_bytes)
            .unwrap_or(0)
    }

    /// Persist cursor to simple file (8 bytes, atomic on most filesystems)
    fn persist_cursor(&self) -> io::Result<()> {
        let path = self.view_dir.join(format!("{}.cursor", self.ai_id));
        fs::write(&path, &self.cursor.to_le_bytes())
    }

    /// Get current cursor position
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    /// Get view statistics
    pub fn stats(&self) -> &ViewStats {
        &self.stats
    }

    /// Sync view with event log
    pub fn sync(&mut self, event_log: &mut EventLogReader) -> ViewResult<u64> {
        let mut events_applied = 0u64;
        let current_head = event_log.head_sequence();

        if current_head <= self.cursor {
            return Ok(0);
        }

        // Seek to our cursor position
        if self.cursor > 0 {
            event_log.seek_to_sequence(self.cursor)
                .map_err(|e| ViewError::EventLog(e.to_string()))?;
        }

        // Read and apply events, skipping any corrupted entries
        // (corruption can happen from rkyv enum ordering changes)
        loop {
            match event_log.try_read() {
                Ok(Some(event)) => {
                    // Skip already-processed events: seek_to_sequence(cursor)
                    // positions the reader AT the cursor event, so the first
                    // try_read() re-reads it. Without this guard, append-only
                    // caches (broadcasts, DMs) get duplicates on every sync.
                    if event.header.sequence <= self.cursor {
                        continue;
                    }
                    self.apply_event(&event)?;
                    events_applied += 1;
                    self.cursor = event.header.sequence;
                }
                Ok(None) => break,
                Err(_) => {
                    // Skip corrupted event - reader already advanced past the bad bytes
                    continue;
                }
            }
        }

        // Persist cursor
        self.persist_cursor()?;
        self.stats.events_applied += events_applied;

        Ok(events_applied)
    }

    /// Apply a single event to the view
    ///
    /// Updates both statistics (counters) AND content caches.
    /// This is the core of the materialized view pattern.
    pub fn apply_event(&mut self, event: &Event) -> ViewResult<()> {
        let header = &event.header;
        let source_ai = header.source_ai_str().to_string();
        let timestamp = header.timestamp;
        let sequence = header.sequence;

        // Update last_seen for this AI (presence tracking)
        self.update_last_seen(&source_ai, timestamp);

        match header.event_type {
            // ============== MESSAGING ==============
            event_type::DIRECT_MESSAGE => {
                if let EventPayload::DirectMessage(payload) = &event.payload {
                    // Update stats
                    if payload.to_ai == self.ai_id && source_ai != self.ai_id {
                        self.stats.unread_dms += 1;
                    }

                    // Cache DMs TO this AI
                    if payload.to_ai == self.ai_id {
                        let dm = CachedDM {
                            id: sequence,
                            from_ai: source_ai.clone(),
                            to_ai: payload.to_ai.clone(),
                            content: payload.content.clone(),
                            timestamp,
                            read: false,
                        };
                        self.recent_dms.push_back(dm);
                        // Ring buffer eviction
                        while self.recent_dms.len() > MAX_CACHED_DMS {
                            self.recent_dms.pop_front();
                        }
                    }
                }
            }

            event_type::BROADCAST => {
                if let EventPayload::Broadcast(payload) = &event.payload {
                    let broadcast = CachedBroadcast {
                        id: sequence,
                        from_ai: source_ai.clone(),
                        channel: payload.channel.clone(),
                        content: payload.content.clone(),
                        timestamp,
                    };

                    // Cache by channel
                    let channel_queue = self.recent_broadcasts
                        .entry(payload.channel.clone())
                        .or_insert_with(VecDeque::new);
                    channel_queue.push_back(broadcast.clone());
                    while channel_queue.len() > MAX_CACHED_BROADCASTS_PER_CHANNEL {
                        channel_queue.pop_front();
                    }

                    // Also cache in all_broadcasts
                    self.all_broadcasts.push_back(broadcast);
                    while self.all_broadcasts.len() > MAX_CACHED_BROADCASTS_PER_CHANNEL {
                        self.all_broadcasts.pop_front();
                    }
                }
            }

            event_type::DM_READ => {
                if let EventPayload::DmRead(payload) = &event.payload {
                    // Mark the DM as read in our cache
                    for dm in self.recent_dms.iter_mut() {
                        if dm.id == payload.dm_id && !dm.read {
                            dm.read = true;
                            if self.stats.unread_dms > 0 {
                                self.stats.unread_dms -= 1;
                            }
                            break;
                        }
                    }
                }
            }

            // ============== DIALOGUES ==============
            event_type::DIALOGUE_START => {
                if let EventPayload::DialogueStart(payload) = &event.payload {
                    // Build ordered participant list: initiator first, then others from payload.
                    let mut participants: Vec<String> = vec![source_ai.clone()];
                    for p in &payload.participants {
                        let p = p.trim();
                        if !p.is_empty() && p != source_ai.as_str() {
                            participants.push(p.to_string());
                        }
                    }
                    let first_responder = participants.get(1).cloned()
                        .unwrap_or_else(|| participants[0].clone());

                    // Update stats for dialogues involving this AI
                    if participants.contains(&self.ai_id) {
                        self.stats.active_dialogues += 1;
                    }

                    // Cache dialogue state
                    let mut messages = VecDeque::new();
                    messages.push_back(DialogueMessage {
                        sequence,
                        from_ai: source_ai.clone(),
                        content: payload.topic.clone(),  // Topic is first "message"
                        timestamp,
                    });

                    // Key by timestamp (= ID returned to caller by v2_client.start_dialogue).
                    // All update events (DIALOGUE_RESPOND, DIALOGUE_END, DIALOGUE_MERGE)
                    // carry the caller-facing ID which is the creation timestamp, not the
                    // event log sequence number.
                    self.dialogues.insert(timestamp, DialogueState {
                        id: timestamp,
                        initiator: source_ai.clone(),
                        responder: first_responder.clone(),
                        participants,
                        topic: payload.topic.clone(),
                        status: "active".to_string(),
                        current_turn: first_responder,  // First non-initiator goes first
                        messages,
                        created_at: timestamp,
                        updated_at: timestamp,
                    });
                }
            }

            event_type::DIALOGUE_RESPOND => {
                if let EventPayload::DialogueRespond(payload) = &event.payload {
                    if let Some(dialogue) = self.dialogues.get_mut(&payload.dialogue_id) {
                        // Add message
                        dialogue.messages.push_back(DialogueMessage {
                            sequence,
                            from_ai: source_ai.clone(),
                            content: payload.content.clone(),
                            timestamp,
                        });

                        // Round-robin turn: find current speaker, advance to next
                        let current_idx = dialogue.participants.iter()
                            .position(|p| p == &source_ai)
                            .unwrap_or(0);
                        let next_idx = (current_idx + 1) % dialogue.participants.len();
                        dialogue.current_turn = dialogue.participants[next_idx].clone();
                        dialogue.updated_at = timestamp;
                    }
                }
            }

            event_type::DIALOGUE_END => {
                if let EventPayload::DialogueEnd(payload) = &event.payload {
                    // Update stats
                    if self.stats.active_dialogues > 0 {
                        self.stats.active_dialogues -= 1;
                    }

                    // Update dialogue cache
                    if let Some(dialogue) = self.dialogues.get_mut(&payload.dialogue_id) {
                        dialogue.status = payload.status.clone();
                        dialogue.current_turn = String::new();
                        dialogue.updated_at = timestamp;
                    }
                }
            }

            event_type::DIALOGUE_MERGE => {
                if let EventPayload::DialogueMerge(payload) = &event.payload {
                    // Update stats
                    if self.stats.active_dialogues > 0 {
                        self.stats.active_dialogues -= 1;
                    }

                    // Mark source dialogue as merged
                    if let Some(dialogue) = self.dialogues.get_mut(&payload.source_id) {
                        dialogue.status = format!("merged:{}", payload.target_id);
                        dialogue.current_turn = String::new();
                        dialogue.updated_at = timestamp;
                    }
                }
            }

            // ============== VOTES ==============
            event_type::VOTE_CREATE => {
                if let EventPayload::VoteCreate(payload) = &event.payload {
                    self.stats.pending_votes += 1;

                    // Key by timestamp — consistent with tasks/rooms/dialogues.
                    // VOTE_CAST and VOTE_CLOSE payloads carry the caller-facing timestamp ID.
                    self.votes.insert(timestamp, VoteState {
                        id: timestamp,
                        creator: source_ai.clone(),
                        topic: payload.topic.clone(),
                        options: payload.options.clone(),
                        status: "open".to_string(),
                        casts: Vec::new(),
                        total_voters: payload.required_voters,
                        created_at: timestamp,
                    });
                }
            }

            event_type::VOTE_CAST => {
                if let EventPayload::VoteCast(payload) = &event.payload {
                    if source_ai == self.ai_id && self.stats.pending_votes > 0 {
                        self.stats.pending_votes -= 1;
                    }

                    if let Some(vote) = self.votes.get_mut(&payload.vote_id) {
                        vote.casts.push((source_ai.clone(), payload.choice.clone()));
                    }
                }
            }

            event_type::VOTE_CLOSE => {
                if let EventPayload::VoteClose(payload) = &event.payload {
                    if let Some(vote) = self.votes.get_mut(&payload.vote_id) {
                        vote.status = "closed".to_string();
                    }
                }
            }

            // Locks deprecated — LOCK_ACQUIRE/LOCK_RELEASE events ignored (Feb 2026)
            event_type::LOCK_ACQUIRE => {}
            event_type::LOCK_RELEASE => {}

            // ============== FILE CLAIMS ==============
            event_type::FILE_CLAIM => {
                if let EventPayload::FileClaim(payload) = &event.payload {
                    self.file_claims.insert(payload.path.clone(), FileClaimState {
                        path: payload.path.clone(),
                        holder: source_ai.clone(),
                        working_on: payload.working_on.clone(),
                        duration_seconds: payload.duration_seconds,
                        claimed_at: timestamp,
                    });
                }
            }

            event_type::FILE_RELEASE => {
                if let EventPayload::FileRelease(payload) = &event.payload {
                    self.file_claims.remove(&payload.path);
                }
            }

            // ============== TASKS ==============
            event_type::TASK_CREATE => {
                if let EventPayload::TaskCreate(payload) = &event.payload {
                    // Key by timestamp — TASK_CLAIM/START/COMPLETE/BLOCK/UNBLOCK payloads
                    // all carry the caller-facing timestamp ID, not the event log sequence.
                    self.tasks.insert(timestamp, TaskState {
                        id: timestamp,
                        description: payload.description.clone(),
                        priority: payload.priority,
                        status: "pending".to_string(),
                        assignee: None,
                        tags: payload.tags.clone().unwrap_or_default(),
                        block_reason: None,
                        created_at: timestamp,
                        updated_at: timestamp,
                    });
                }
            }

            event_type::TASK_CLAIM => {
                if let EventPayload::TaskClaim(payload) = &event.payload {
                    if source_ai == self.ai_id {
                        self.stats.my_tasks += 1;
                    }

                    if let Some(task) = self.tasks.get_mut(&payload.task_id) {
                        task.status = "claimed".to_string();
                        task.assignee = Some(source_ai.clone());
                        task.updated_at = timestamp;
                    }
                }
            }

            event_type::TASK_START => {
                if let EventPayload::TaskStart(payload) = &event.payload {
                    if let Some(task) = self.tasks.get_mut(&payload.task_id) {
                        task.status = "in_progress".to_string();
                        task.updated_at = timestamp;
                    }
                }
            }

            event_type::TASK_COMPLETE => {
                if let EventPayload::TaskComplete(payload) = &event.payload {
                    if source_ai == self.ai_id && self.stats.my_tasks > 0 {
                        self.stats.my_tasks -= 1;
                    }

                    if let Some(task) = self.tasks.get_mut(&payload.task_id) {
                        task.status = "completed".to_string();
                        task.updated_at = timestamp;
                    }
                }
            }

            event_type::TASK_BLOCK => {
                if let EventPayload::TaskBlock(payload) = &event.payload {
                    if let Some(task) = self.tasks.get_mut(&payload.task_id) {
                        task.status = "blocked".to_string();
                        task.block_reason = Some(payload.reason.clone());
                        task.updated_at = timestamp;
                    }
                }
            }

            event_type::TASK_UNBLOCK => {
                if let EventPayload::TaskUnblock(payload) = &event.payload {
                    if let Some(task) = self.tasks.get_mut(&payload.task_id) {
                        task.status = "pending".to_string();
                        task.block_reason = None;
                        task.updated_at = timestamp;
                    }
                }
            }

            // ============== BATCHES ==============
            event_type::BATCH_CREATE => {
                if let EventPayload::BatchCreate(payload) = &event.payload {
                    // Parse tasks: "1:desc|2:desc" -> [("1", "desc"), ...]
                    let tasks: Vec<(String, String)> = payload.tasks
                        .split('|')
                        .filter_map(|t| {
                            let parts: Vec<&str> = t.splitn(2, ':').collect();
                            if parts.len() == 2 {
                                Some((parts[0].trim().to_string(), parts[1].trim().to_string()))
                            } else {
                                None
                            }
                        })
                        .collect();

                    self.batches.insert(payload.name.clone(), BatchState {
                        name: payload.name.clone(),
                        creator: source_ai.clone(),
                        tasks,
                        done: HashSet::new(),
                        is_closed: false,
                        created_at: timestamp,
                    });
                }
            }

            event_type::BATCH_TASK_DONE => {
                if let EventPayload::BatchTaskDone(payload) = &event.payload {
                    if let Some(batch) = self.batches.get_mut(&payload.batch_name) {
                        batch.done.insert(payload.label.clone());
                    }
                }
            }

            event_type::BATCH_CLOSE => {
                if let EventPayload::BatchClose(payload) = &event.payload {
                    if let Some(batch) = self.batches.get_mut(&payload.batch_name) {
                        batch.is_closed = true;
                    }
                }
            }

            // ============== PRESENCE ==============
            event_type::PRESENCE_UPDATE => {
                if let EventPayload::PresenceUpdate(payload) = &event.payload {
                    let presence = self.presences.entry(source_ai.clone())
                        .or_insert_with(|| PresenceState {
                            ai_id: source_ai.clone(),
                            status: String::new(),
                            current_task: String::new(),
                            last_seen: 0,
                            last_presence_update: 0,
                        });
                    presence.status = payload.status.clone();
                    presence.current_task = payload.current_task.clone().unwrap_or_default();
                    presence.last_presence_update = timestamp;
                    presence.last_seen = timestamp;
                }
            }

            // ============== ROOMS ==============
            event_type::ROOM_CREATE => {
                if let EventPayload::RoomCreate(payload) = &event.payload {
                    // Key by timestamp — all ROOM_* update events carry room_id as a string
                    // which is the caller-facing timestamp ID from create_room().
                    self.rooms.insert(timestamp, RoomState {
                        id: timestamp,
                        name: payload.name.clone(),
                        topic: payload.topic.clone().unwrap_or_default(),
                        members: vec![source_ai.clone()],
                        messages: VecDeque::new(),
                        created_at: timestamp,
                        is_closed: false,
                        mutes: HashMap::new(),
                        conclusion: None,
                        pinned_messages: Vec::new(),
                    });
                }
            }

            event_type::ROOM_JOIN => {
                if let EventPayload::RoomJoin(payload) = &event.payload {
                    if let Ok(room_id) = payload.room_id.parse::<u64>() {
                        if let Some(room) = self.rooms.get_mut(&room_id) {
                            if !room.members.contains(&source_ai) {
                                room.members.push(source_ai.clone());
                            }
                        }
                    }
                }
            }

            event_type::ROOM_LEAVE => {
                if let EventPayload::RoomLeave(payload) = &event.payload {
                    if let Ok(room_id) = payload.room_id.parse::<u64>() {
                        if let Some(room) = self.rooms.get_mut(&room_id) {
                            room.members.retain(|m| m != &source_ai);
                        }
                    }
                }
            }

            event_type::ROOM_MESSAGE => {
                if let EventPayload::RoomMessage(payload) = &event.payload {
                    if let Ok(room_id) = payload.room_id.parse::<u64>() {
                        if let Some(room) = self.rooms.get_mut(&room_id) {
                            room.messages.push_back((
                                sequence,
                                source_ai.clone(),
                                payload.content.clone(),
                                timestamp,
                            ));
                            // Limit room messages
                            while room.messages.len() > 100 {
                                room.messages.pop_front();
                            }
                        }
                    }
                }
            }

            event_type::ROOM_CLOSE => {
                if let EventPayload::RoomClose(payload) = &event.payload {
                    if let Ok(room_id) = payload.room_id.parse::<u64>() {
                        if let Some(room) = self.rooms.get_mut(&room_id) {
                            room.is_closed = true;
                        }
                    }
                }
            }

            event_type::ROOM_MUTE => {
                if let EventPayload::RoomMute(payload) = &event.payload {
                    if let Ok(room_id) = payload.room_id.parse::<u64>() {
                        if let Some(room) = self.rooms.get_mut(&room_id) {
                            let expires_at = timestamp / 1000 + (payload.minutes as u64 * 60_000);
                            room.mutes.insert(payload.target_ai.clone(), expires_at);
                        }
                    }
                }
            }

            event_type::ROOM_CONCLUDE => {
                if let EventPayload::RoomConclude(payload) = &event.payload {
                    if let Ok(room_id) = payload.room_id.parse::<u64>() {
                        if let Some(room) = self.rooms.get_mut(&room_id) {
                            room.conclusion = payload.conclusion.clone();
                            room.is_closed = true;
                        }
                    }
                }
            }

            event_type::ROOM_PIN_MESSAGE => {
                if let EventPayload::RoomPinMessage(payload) = &event.payload {
                    if let Ok(room_id) = payload.room_id.parse::<u64>() {
                        if let Some(room) = self.rooms.get_mut(&room_id) {
                            if !room.pinned_messages.contains(&payload.msg_seq_id) {
                                room.pinned_messages.push(payload.msg_seq_id);
                            }
                        }
                    }
                }
            }

            event_type::ROOM_UNPIN_MESSAGE => {
                if let EventPayload::RoomUnpinMessage(payload) = &event.payload {
                    if let Ok(room_id) = payload.room_id.parse::<u64>() {
                        if let Some(room) = self.rooms.get_mut(&room_id) {
                            room.pinned_messages.retain(|&id| id != payload.msg_seq_id);
                        }
                    }
                }
            }

            // ============== FILE ACTIONS ==============
            event_type::FILE_ACTION => {
                if let EventPayload::FileAction(payload) = &event.payload {
                    self.file_actions.push_back(FileActionState {
                        id: sequence,
                        ai_id: source_ai.clone(),
                        path: payload.path.clone(),
                        action: payload.action.clone(),
                        timestamp,
                    });
                    // Ring buffer eviction
                    while self.file_actions.len() > MAX_CACHED_FILE_ACTIONS {
                        self.file_actions.pop_front();
                    }
                }
            }

            // Stigmergy deprecated — PHEROMONE_DEPOSIT events ignored (Feb 2026)
            event_type::PHEROMONE_DEPOSIT => {}

            // ============== PROJECTS ==============
            event_type::PROJECT_CREATE => {
                if let EventPayload::ProjectCreate(payload) = &event.payload {
                    self.projects.insert(timestamp, ProjectState {
                        id: timestamp,
                        name: payload.name.clone(),
                        goal: payload.goal.clone(),
                        root_directory: payload.root_directory.clone(),
                        status: "active".to_string(),
                        is_deleted: false,
                        created_at: timestamp,
                    });
                }
            }

            event_type::PROJECT_UPDATE => {
                if let EventPayload::ProjectUpdate(payload) = &event.payload {
                    if let Some(project) = self.projects.get_mut(&payload.project_id) {
                        if let Some(goal) = &payload.goal {
                            project.goal = goal.clone();
                        }
                        if let Some(status) = &payload.status {
                            project.status = status.clone();
                        }
                    }
                }
            }

            event_type::PROJECT_DELETE => {
                if let EventPayload::ProjectDelete(payload) = &event.payload {
                    if let Some(project) = self.projects.get_mut(&payload.project_id) {
                        project.is_deleted = true;
                    }
                }
            }

            event_type::PROJECT_RESTORE => {
                if let EventPayload::ProjectRestore(payload) = &event.payload {
                    if let Some(project) = self.projects.get_mut(&payload.project_id) {
                        project.is_deleted = false;
                    }
                }
            }

            // ============== FEATURES ==============
            event_type::FEATURE_CREATE => {
                if let EventPayload::FeatureCreate(payload) = &event.payload {
                    self.features.insert(timestamp, FeatureState {
                        id: timestamp,
                        project_id: payload.project_id,
                        name: payload.name.clone(),
                        overview: payload.overview.clone(),
                        directory: payload.directory.clone(),
                        is_deleted: false,
                        created_at: timestamp,
                    });
                }
            }

            event_type::FEATURE_UPDATE => {
                if let EventPayload::FeatureUpdate(payload) = &event.payload {
                    if let Some(feature) = self.features.get_mut(&payload.feature_id) {
                        if let Some(name) = &payload.name {
                            feature.name = name.clone();
                        }
                        if let Some(overview) = &payload.overview {
                            feature.overview = overview.clone();
                        }
                        if payload.directory.is_some() {
                            feature.directory = payload.directory.clone();
                        }
                    }
                }
            }

            event_type::FEATURE_DELETE => {
                if let EventPayload::FeatureDelete(payload) = &event.payload {
                    if let Some(feature) = self.features.get_mut(&payload.feature_id) {
                        feature.is_deleted = true;
                    }
                }
            }

            event_type::FEATURE_RESTORE => {
                if let EventPayload::FeatureRestore(payload) = &event.payload {
                    if let Some(feature) = self.features.get_mut(&payload.feature_id) {
                        feature.is_deleted = false;
                    }
                }
            }

            // ============== TRUST ==============
            event_type::TRUST_RECORD => {
                if let EventPayload::TrustRecord(payload) = &event.payload {
                    // Only record trust events where I am the rater
                    if source_ai == self.ai_id {
                        let score = self.ai_trust
                            .entry(payload.target_ai.clone())
                            .or_default();
                        score.record(payload.is_success, payload.weight);
                    }
                }
            }

            _ => {}
        }

        Ok(())
    }

    /// Update last_seen timestamp for an AI (called on every event)
    fn update_last_seen(&mut self, ai_id: &str, timestamp: u64) {
        if ai_id.is_empty() || ai_id == "unknown" {
            return;
        }

        let presence = self.presences.entry(ai_id.to_string())
            .or_insert_with(|| PresenceState {
                ai_id: ai_id.to_string(),
                status: "active".to_string(),
                current_task: String::new(),
                last_seen: 0,
                last_presence_update: 0,
            });
        presence.last_seen = presence.last_seen.max(timestamp);
    }

    /// Query methods
    pub fn unread_dm_count(&self) -> u64 {
        self.stats.unread_dms
    }

    pub fn active_dialogue_count(&self) -> u64 {
        self.stats.active_dialogues
    }

    pub fn pending_vote_count(&self) -> u64 {
        self.stats.pending_votes
    }

    // my_lock_count() removed — locks deprecated (Feb 2026)

    pub fn my_task_count(&self) -> u64 {
        self.stats.my_tasks
    }

    pub fn mark_dm_read(&mut self) {
        if self.stats.unread_dms > 0 {
            self.stats.unread_dms -= 1;
        }
    }

    /// Get all unread DMs (where read == false)
    pub fn get_unread_dms(&self) -> Vec<CachedDM> {
        self.recent_dms
            .iter()
            .filter(|dm| !dm.read)
            .cloned()
            .collect()
    }

    /// Mark a specific DM as read by its ID (sequence number)
    /// Returns true if the DM was found and marked, false otherwise
    pub fn mark_dm_read_by_id(&mut self, dm_id: u64) -> bool {
        for dm in self.recent_dms.iter_mut() {
            if dm.id == dm_id && !dm.read {
                dm.read = true;
                if self.stats.unread_dms > 0 {
                    self.stats.unread_dms -= 1;
                }
                return true;
            }
        }
        false
    }

    /// Mark multiple DMs as read by their IDs
    pub fn mark_dms_read_by_ids(&mut self, dm_ids: &[u64]) {
        for dm in self.recent_dms.iter_mut() {
            if dm_ids.contains(&dm.id) && !dm.read {
                dm.read = true;
                if self.stats.unread_dms > 0 {
                    self.stats.unread_dms -= 1;
                }
            }
        }
    }

    // === TIP Trust Query Methods ===

    /// Get raw trust scores (α, β) for a specific AI
    pub fn get_ai_trust(&self, target_ai: &str) -> (u32, u32) {
        self.ai_trust
            .get(target_ai)
            .map(|s| (s.alpha, s.beta))
            .unwrap_or((0, 0))
    }

    /// Get calculated trust value for a specific AI (0.0 to 1.0)
    pub fn get_ai_trust_value(&self, target_ai: &str) -> f64 {
        self.ai_trust
            .get(target_ai)
            .map(|s| s.trust_value())
            .unwrap_or(0.5)  // Neutral prior
    }

    /// Get all trust scores (for aggregation/decay calculations)
    pub fn get_all_trust_scores(&self) -> &HashMap<String, TrustScore> {
        &self.ai_trust
    }

    /// Get number of AIs we have trust data for
    pub fn trust_count(&self) -> usize {
        self.ai_trust.len()
    }

    // ============== CONTENT CACHE QUERY METHODS ==============
    // These provide O(1) or O(k) access to cached data instead of O(n) log scans

    // === DMs ===

    /// Get recent DMs (newest first, up to limit)
    pub fn get_recent_dms(&self, limit: usize) -> Vec<&CachedDM> {
        self.recent_dms.iter().rev().take(limit).collect()
    }

    /// Get all cached DMs (for conversion to external types)
    pub fn get_all_cached_dms(&self) -> &VecDeque<CachedDM> {
        &self.recent_dms
    }

    /// Get pending DM senders (senders with unreplied DMs)
    /// A sender is pending if we have a DM from them and we haven't sent one back more recently
    pub fn get_pending_dm_senders(&self) -> Vec<String> {
        // Track last DM timestamp per sender
        let mut last_from: HashMap<&str, u64> = HashMap::new();

        // From our cached DMs (these are DMs TO us)
        for dm in &self.recent_dms {
            let entry = last_from.entry(&dm.from_ai).or_insert(0);
            *entry = (*entry).max(dm.timestamp);
        }

        // We don't track sent DMs in cache, so we can't perfectly determine this
        // from cache alone. Return all senders we have DMs from.
        // The full implementation requires scanning the log for our sent DMs.
        last_from.keys().map(|s| s.to_string()).collect()
    }

    // === Broadcasts ===

    /// Get recent broadcasts (all channels, newest first)
    pub fn get_recent_broadcasts(&self, limit: usize) -> Vec<&CachedBroadcast> {
        self.all_broadcasts.iter().rev().take(limit).collect()
    }

    /// Get recent broadcasts for a specific channel
    pub fn get_channel_broadcasts(&self, channel: &str, limit: usize) -> Vec<&CachedBroadcast> {
        self.recent_broadcasts
            .get(channel)
            .map(|q| q.iter().rev().take(limit).collect())
            .unwrap_or_default()
    }

    // === Dialogues ===

    /// Get a dialogue by ID
    /// Get a dialogue by ID (tries sequence key first, then timestamp fallback)
    /// This handles the ID mismatch between outbox-returned timestamps
    /// and view-assigned global sequence numbers.
    pub fn get_dialogue(&self, id: u64) -> Option<&DialogueState> {
        self.dialogues.get(&id)
            .or_else(|| self.dialogues.values().find(|d| d.created_at == id))
    }

    /// Get all dialogues
    pub fn get_all_dialogues(&self) -> &HashMap<u64, DialogueState> {
        &self.dialogues
    }

    /// Get dialogue messages
    pub fn get_dialogue_messages(&self, id: u64) -> Vec<&DialogueMessage> {
        self.dialogues
            .get(&id)
            .map(|d| d.messages.iter().collect())
            .unwrap_or_default()
    }

    /// Get dialogues where it's this AI's turn
    pub fn get_my_turn_dialogues(&self) -> Vec<&DialogueState> {
        self.dialogues
            .values()
            .filter(|d| d.status == "active" && d.current_turn == self.ai_id)
            .collect()
    }

    /// Get dialogue invites — dialogues where it's currently my turn.
    ///
    /// For n-party round-robin dialogues, "invite" means "it's your turn now",
    /// regardless of participant position or how many messages have been exchanged.
    /// The old bilateral conditions (responder check, messages.len()==1) are wrong
    /// for 3+ participant dialogues where any participant can be mid-dialogue.
    pub fn get_dialogue_invites(&self) -> Vec<&DialogueState> {
        self.dialogues
            .values()
            .filter(|d| d.status == "active" && d.current_turn == self.ai_id)
            .collect()
    }

    /// Get active dialogues involving this AI
    pub fn get_active_dialogues(&self) -> Vec<&DialogueState> {
        self.dialogues
            .values()
            .filter(|d| {
                d.status == "active"
                    && (d.initiator == self.ai_id || d.responder == self.ai_id)
            })
            .collect()
    }

    // === Tasks ===

    /// Get a task by ID (tries sequence key first, then timestamp fallback)
    /// This handles the ID mismatch between outbox-returned timestamps
    /// and view-assigned global sequence numbers.
    pub fn get_task(&self, id: u64) -> Option<&TaskState> {
        self.tasks.get(&id)
            .or_else(|| self.tasks.values().find(|t| t.created_at == id))
    }

    /// Get all tasks
    pub fn get_all_tasks(&self) -> &HashMap<u64, TaskState> {
        &self.tasks
    }

    /// Get tasks by status
    pub fn get_tasks_by_status(&self, status: &str) -> Vec<&TaskState> {
        self.tasks
            .values()
            .filter(|t| t.status == status)
            .collect()
    }

    /// Get tasks assigned to this AI
    pub fn get_my_tasks(&self) -> Vec<&TaskState> {
        self.tasks
            .values()
            .filter(|t| t.assignee.as_ref() == Some(&self.ai_id))
            .collect()
    }

    // === Votes ===

    /// Get a vote by ID
    pub fn get_vote(&self, id: u64) -> Option<&VoteState> {
        self.votes.get(&id)
    }

    /// Get all votes
    pub fn get_all_votes(&self) -> &HashMap<u64, VoteState> {
        &self.votes
    }

    /// Get open votes
    pub fn get_open_votes(&self) -> Vec<&VoteState> {
        self.votes
            .values()
            .filter(|v| v.status == "open")
            .collect()
    }

    // === Batches ===

    /// Get a batch by name
    pub fn get_batch(&self, name: &str) -> Option<&BatchState> {
        self.batches.get(name)
    }

    /// Get all batches
    pub fn get_all_batches(&self) -> &HashMap<String, BatchState> {
        &self.batches
    }

    /// Get open (non-closed) batches
    pub fn get_open_batches(&self) -> Vec<&BatchState> {
        self.batches
            .values()
            .filter(|b| !b.is_closed)
            .collect()
    }

    // === File Claims ===

    /// Get all active file claims (filters expired claims)
    pub fn get_active_claims(&self) -> Vec<&FileClaimState> {
        let now_micros = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);

        self.file_claims
            .values()
            .filter(|c| {
                let expires = c.claimed_at + (c.duration_seconds as u64 * 1_000_000);
                expires > now_micros
            })
            .collect()
    }

    /// Check if a file is claimed
    pub fn check_claim(&self, path: &str) -> Option<&FileClaimState> {
        let now_micros = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);

        self.file_claims.get(path).filter(|c| {
            let expires = c.claimed_at + (c.duration_seconds as u64 * 1_000_000);
            expires > now_micros
        })
    }

    // === Presences ===

    /// Get all presences with recent activity (within threshold)
    pub fn get_online_presences(&self, threshold_micros: u64) -> Vec<&PresenceState> {
        let now_micros = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);

        self.presences
            .values()
            .filter(|p| now_micros.saturating_sub(p.last_seen) < threshold_micros)
            .collect()
    }

    /// Get presence for a specific AI
    pub fn get_presence(&self, ai_id: &str) -> Option<&PresenceState> {
        self.presences.get(ai_id)
    }

    /// Get all cached presences
    pub fn get_all_presences(&self) -> &HashMap<String, PresenceState> {
        &self.presences
    }

    // Locks query methods removed — deprecated (Feb 2026)

    // === File Actions ===

    /// Get recent file actions (newest first)
    pub fn get_recent_file_actions(&self, limit: usize) -> Vec<&FileActionState> {
        self.file_actions.iter().rev().take(limit).collect()
    }

    // Pheromone query methods removed — stigmergy deprecated (Feb 2026)

    // === Rooms ===

    /// Get a room by ID
    pub fn get_room(&self, id: u64) -> Option<&RoomState> {
        self.rooms.get(&id)
    }

    /// Get all rooms
    pub fn get_all_rooms(&self) -> &HashMap<u64, RoomState> {
        &self.rooms
    }

    /// Get room messages
    pub fn get_room_messages(&self, room_id: u64, limit: usize) -> Vec<(u64, String, String, u64)> {
        self.rooms
            .get(&room_id)
            .map(|r| {
                r.messages
                    .iter()
                    .rev()
                    .take(limit)
                    .map(|(seq, ai, content, ts)| (*seq, ai.clone(), content.clone(), *ts))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get open (not closed) rooms
    pub fn get_open_rooms(&self) -> Vec<&RoomState> {
        self.rooms
            .values()
            .filter(|r| !r.is_closed)
            .collect()
    }

    // === Projects ===

    /// Get all projects (including deleted — callers filter is_deleted as needed)
    pub fn get_all_projects(&self) -> &HashMap<u64, ProjectState> {
        &self.projects
    }

    /// Get a project by its canonical ID (timestamp)
    pub fn get_project(&self, id: u64) -> Option<&ProjectState> {
        self.projects.get(&id)
    }

    /// Get all features for a project
    pub fn get_features_for_project(&self, project_id: u64) -> Vec<&FeatureState> {
        self.features
            .values()
            .filter(|f| f.project_id == project_id)
            .collect()
    }

    /// Get a feature by its canonical ID (timestamp)
    pub fn get_feature(&self, id: u64) -> Option<&FeatureState> {
        self.features.get(&id)
    }

    // ============== END CONTENT CACHE QUERY METHODS ==============

    /// Rebuild view from scratch (clears caches and replays from beginning)
    pub fn rebuild(&mut self, event_log: &mut EventLogReader) -> ViewResult<u64> {
        self.clear_caches();
        self.cursor = 0;
        self.stats = ViewStats::default();
        event_log.seek_to_sequence(0)
            .map_err(|e| ViewError::EventLog(e.to_string()))?;
        self.sync(event_log)
    }

    /// Clear all content caches
    fn clear_caches(&mut self) {
        self.recent_dms.clear();
        self.recent_broadcasts.clear();
        self.all_broadcasts.clear();
        self.dialogues.clear();
        self.tasks.clear();
        self.votes.clear();
        self.batches.clear();
        self.file_claims.clear();
        self.presences.clear();
        self.file_actions.clear();
        self.rooms.clear();
        self.projects.clear();
        self.features.clear();
        self.ai_trust.clear();
    }

    /// Warm cache by replaying last N events from event log
    ///
    /// This populates all content caches so query methods return O(1) results
    /// instead of scanning the entire event log.
    pub fn warm_cache(&mut self, event_log: &mut EventLogReader) -> ViewResult<u64> {
        let head = event_log.head_sequence();

        // ALWAYS warm from at least WARMUP_EVENT_COUNT events to populate caches
        // This ensures recent DMs, dialogues, etc. are in cache even on restart
        let warmup_start = head.saturating_sub(WARMUP_EVENT_COUNT);

        // Clear existing caches before warming (they might have stale data from partial warmup)
        self.clear_caches();

        if warmup_start > 0 {
            event_log.seek_to_sequence(warmup_start)
                .map_err(|e| ViewError::EventLog(e.to_string()))?;
        }

        let mut events_applied = 0u64;
        while let Ok(Some(event)) = event_log.try_read() {
            self.apply_event(&event)?;
            events_applied += 1;
            self.cursor = event.header.sequence;
        }

        self.persist_cursor()?;
        self.stats.events_applied += events_applied;

        Ok(events_applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Event;
    use tempfile::tempdir;

    #[test]
    fn test_view_open() {
        let dir = tempdir().unwrap();
        let view = ViewEngine::open("test-ai", dir.path()).unwrap();
        assert_eq!(view.cursor(), 0);
        assert_eq!(view.ai_id, "test-ai");
    }

    #[test]
    fn test_apply_dm_to_me() {
        let dir = tempdir().unwrap();
        let mut view = ViewEngine::open("lyra-584", dir.path()).unwrap();

        let event = Event::direct_message("sage-724", "lyra-584", "Hello!");
        view.apply_event(&event).unwrap();

        assert_eq!(view.unread_dm_count(), 1);
    }

    #[test]
    fn test_apply_dm_not_to_me() {
        let dir = tempdir().unwrap();
        let mut view = ViewEngine::open("lyra-584", dir.path()).unwrap();

        let event = Event::direct_message("sage-724", "cascade-230", "Hello!");
        view.apply_event(&event).unwrap();

        assert_eq!(view.unread_dm_count(), 0);
    }

    #[test]
    fn test_apply_dialogue_start() {
        let dir = tempdir().unwrap();
        let mut view = ViewEngine::open("lyra-584", dir.path()).unwrap();

        let event = Event::dialogue_start("sage-724", &["sage-724".to_string(), "lyra-584".to_string()], "API review", true);
        view.apply_event(&event).unwrap();

        assert_eq!(view.active_dialogue_count(), 1);
    }

    #[test]
    fn test_apply_vote() {
        let dir = tempdir().unwrap();
        let mut view = ViewEngine::open("lyra-584", dir.path()).unwrap();

        let event = Event::vote_create("sage-724", "Use REST?", vec!["Yes".to_string(), "No".to_string()], 3);
        view.apply_event(&event).unwrap();
        assert_eq!(view.pending_vote_count(), 1);

        let vote = Event::vote_cast("lyra-584", 1, "Yes");
        view.apply_event(&vote).unwrap();
        assert_eq!(view.pending_vote_count(), 0);
    }

    // test_apply_lock removed — locks deprecated (Feb 2026)

    #[test]
    fn test_sync_no_duplicate_on_incremental_sync() {
        // Regression test for Bug 4: seek_to_sequence(cursor) positions the reader
        // AT the cursor event, so without the `sequence <= cursor` guard the cursor
        // event gets re-applied on every subsequent sync, duplicating append-only
        // caches (broadcasts, DMs, etc.).
        use crate::event_log::EventLogWriter;
        let dir = tempdir().unwrap();

        // Write 2 broadcasts and do an initial sync
        {
            let mut writer = EventLogWriter::open(Some(dir.path())).unwrap();
            writer.append(&Event::broadcast("sage-724", "general", "msg-1")).unwrap();
            writer.append(&Event::broadcast("sage-724", "general", "msg-2")).unwrap();
            writer.sync().unwrap();
        }

        let mut view = ViewEngine::open("test-ai", dir.path()).unwrap();
        let mut reader = EventLogReader::open(Some(dir.path())).unwrap();
        let applied = view.sync(&mut reader).unwrap();
        assert_eq!(applied, 2, "Initial sync should apply both events");
        assert_eq!(view.cursor(), 2);

        // Write a third broadcast and sync again
        {
            let mut writer = EventLogWriter::open(Some(dir.path())).unwrap();
            writer.append(&Event::broadcast("sage-724", "general", "msg-3")).unwrap();
            writer.sync().unwrap();
        }

        let mut reader2 = EventLogReader::open(Some(dir.path())).unwrap();
        let applied2 = view.sync(&mut reader2).unwrap();
        // Without the cursor guard, seek_to_sequence(2) positions AT seq-2, which
        // try_read() then re-applies — returning 2 instead of the correct 1.
        assert_eq!(applied2, 1, "Incremental sync must NOT re-apply the cursor event");
        assert_eq!(view.cursor(), 3);
    }

    #[test]
    fn test_cursor_persistence() {
        let dir = tempdir().unwrap();

        {
            let mut view = ViewEngine::open("test-ai", dir.path()).unwrap();
            view.cursor = 42;
            view.persist_cursor().unwrap();
        }

        {
            let view = ViewEngine::open("test-ai", dir.path()).unwrap();
            assert_eq!(view.cursor(), 42);
        }
    }

    #[test]
    fn test_trust_aggregation() {
        let dir = tempdir().unwrap();
        let mut view = ViewEngine::open("cascade-230", dir.path()).unwrap();

        // Record positive trust for sage-724
        let event = Event::trust_record("cascade-230", "sage-724", true, "helpful answer", 5);
        view.apply_event(&event).unwrap();

        // Check trust score
        let (alpha, beta) = view.get_ai_trust("sage-724");
        assert_eq!(alpha, 5);
        assert_eq!(beta, 0);
        assert!((view.get_ai_trust_value("sage-724") - 1.0).abs() < 0.001);

        // Record negative trust
        let event2 = Event::trust_record("cascade-230", "sage-724", false, "bad advice", 2);
        view.apply_event(&event2).unwrap();

        let (alpha, beta) = view.get_ai_trust("sage-724");
        assert_eq!(alpha, 5);
        assert_eq!(beta, 2);
        // Trust = 5/(5+2) = 0.714...
        let trust = view.get_ai_trust_value("sage-724");
        assert!(trust > 0.7 && trust < 0.72);
    }

    #[test]
    fn test_trust_only_records_own_events() {
        let dir = tempdir().unwrap();
        let mut view = ViewEngine::open("cascade-230", dir.path()).unwrap();

        // Event from another AI should NOT affect my view
        let event = Event::trust_record("lyra-584", "sage-724", true, "helpful", 5);
        view.apply_event(&event).unwrap();

        // Should have no trust data since I didn't rate
        assert_eq!(view.trust_count(), 0);
    }
}
