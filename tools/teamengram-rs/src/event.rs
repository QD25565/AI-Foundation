//! TeamEngram V2 Event Types - Zero-Copy Serialization
//!
//! All mutations in TeamEngram are represented as immutable events.
//! Events are stored in an append-only log and applied to local views.
//!
//! Design principles:
//! - Zero-copy reads from shared memory (rkyv)
//! - Fixed-size header for fast parsing
//! - Variable payload for flexibility
//! - Checksum for integrity verification

use rkyv::{Archive, Deserialize, Serialize, rancor::Error as RkyvError};
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================================
// Event Type Constants
// ============================================================================

/// Event type discriminants (u16)
/// Grouped by category for easy identification
pub mod event_type {
    // Coordination Events (0x00XX)
    pub const BROADCAST: u16 = 0x0001;
    pub const DIRECT_MESSAGE: u16 = 0x0002;
    pub const PRESENCE_UPDATE: u16 = 0x0003;
    pub const DM_READ: u16 = 0x0004;

    // Dialogue Events (0x01XX)
    pub const DIALOGUE_START: u16 = 0x0100;
    pub const DIALOGUE_RESPOND: u16 = 0x0101;
    pub const DIALOGUE_END: u16 = 0x0102;
    pub const DIALOGUE_MERGE: u16 = 0x0103;

    // Vote Events (0x02XX)
    pub const VOTE_CREATE: u16 = 0x0200;
    pub const VOTE_CAST: u16 = 0x0201;
    pub const VOTE_CLOSE: u16 = 0x0202;

    // Room Events (0x03XX)
    pub const ROOM_CREATE: u16 = 0x0300;
    pub const ROOM_JOIN: u16 = 0x0301;
    pub const ROOM_LEAVE: u16 = 0x0302;
    pub const ROOM_MESSAGE: u16 = 0x0303;
    pub const ROOM_CLOSE: u16 = 0x0304;

    // Lock Events (0x04XX) — DEPRECATED: Locks removed (Feb 2026, QD directive)
    // Constants kept for backward compatibility with existing events in the log
    pub const LOCK_ACQUIRE: u16 = 0x0400;
    pub const LOCK_RELEASE: u16 = 0x0401;

    // File Events (0x05XX)
    pub const FILE_ACTION: u16 = 0x0500;
    pub const FILE_CLAIM: u16 = 0x0501;
    pub const FILE_RELEASE: u16 = 0x0502;

    // Task Events (0x06XX)
    pub const TASK_CREATE: u16 = 0x0600;
    pub const TASK_CLAIM: u16 = 0x0601;
    pub const TASK_START: u16 = 0x0602;
    pub const TASK_COMPLETE: u16 = 0x0603;
    pub const TASK_BLOCK: u16 = 0x0604;
    pub const TASK_UNBLOCK: u16 = 0x0605;

    // Stigmergy Events (0x07XX) — DEPRECATED: Stigmergy removed (Feb 2026, QD directive)
    // Constant kept for backward compatibility with existing events in the log
    pub const PHEROMONE_DEPOSIT: u16 = 0x0700;

    // Project Events (0x08XX)
    pub const PROJECT_CREATE: u16 = 0x0800;
    pub const PROJECT_UPDATE: u16 = 0x0801;
    pub const PROJECT_DELETE: u16 = 0x0802;
    pub const PROJECT_RESTORE: u16 = 0x0803;

    // Feature Events (0x09XX)
    pub const FEATURE_CREATE: u16 = 0x0900;
    pub const FEATURE_UPDATE: u16 = 0x0901;
    pub const FEATURE_DELETE: u16 = 0x0902;
    pub const FEATURE_RESTORE: u16 = 0x0903;

    // Learning Events (0x0AXX) - Shared team insights ("muscle memory")
    pub const LEARNING_CREATE: u16 = 0x0A00;
    pub const LEARNING_UPDATE: u16 = 0x0A01;
    pub const LEARNING_DELETE: u16 = 0x0A02;

    // Trust Events (0x0BXX) - TIP: Trust Inference and Propagation
    pub const TRUST_RECORD: u16 = 0x0B00;

    // Batch Events (0x0CXX) - Simple grouped tasks with AI-chosen labels
    pub const BATCH_CREATE: u16 = 0x0C00;
    pub const BATCH_TASK_DONE: u16 = 0x0C01;
    pub const BATCH_CLOSE: u16 = 0x0C02;

    /// Get category name from event type
    pub fn category(event_type: u16) -> &'static str {
        match event_type >> 8 {
            0x00 => "coordination",
            0x01 => "dialogue",
            0x02 => "vote",
            0x03 => "room",
            0x04 => "lock",
            0x05 => "file",
            0x06 => "task",
            0x07 => "stigmergy",
            0x08 => "project",
            0x09 => "feature",
            0x0A => "learning",
            0x0B => "trust",
            0x0C => "batch",
            _ => "unknown",
        }
    }
}

// ============================================================================
// Event Header (Fixed Size - 64 bytes, cache-line aligned)
// ============================================================================

/// Fixed-size event header for the master log
///
/// Layout is cache-line aligned for optimal memory access.
/// All fields are little-endian for cross-platform compatibility.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventHeader {
    /// Global sequence number (monotonically increasing)
    pub sequence: u64,
    /// Timestamp in microseconds since UNIX epoch
    pub timestamp: u64,
    /// Source AI ID (32 bytes, null-padded)
    pub source_ai: [u8; 32],
    /// Event type discriminant
    pub event_type: u16,
    /// Payload length in bytes
    pub payload_len: u16,
    /// Reserved for future use
    pub flags: u16,
    /// Reserved
    pub _reserved: u16,
    /// CRC32 checksum of header (excluding this field) + payload
    pub checksum: u32,
}

impl EventHeader {
    /// Size of header in bytes
    pub const SIZE: usize = 64;

    /// Create a new event header
    pub fn new(source_ai: &str, event_type: u16, payload_len: u16) -> Self {
        let mut ai_bytes = [0u8; 32];
        let src = source_ai.as_bytes();
        let len = src.len().min(32);
        ai_bytes[..len].copy_from_slice(&src[..len]);

        Self {
            sequence: 0, // Set by Sequencer
            timestamp: current_timestamp_micros(),
            source_ai: ai_bytes,
            event_type,
            payload_len,
            flags: 0,
            _reserved: 0,
            checksum: 0, // Calculated after serialization
        }
    }

    /// Get source AI as string
    pub fn source_ai_str(&self) -> &str {
        let end = self.source_ai.iter().position(|&b| b == 0).unwrap_or(32);
        std::str::from_utf8(&self.source_ai[..end]).unwrap_or("")
    }

    /// Serialize header to bytes
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        unsafe { std::mem::transmute_copy(self) }
    }

    /// Deserialize header from bytes
    pub fn from_bytes(bytes: &[u8; Self::SIZE]) -> Self {
        unsafe { std::mem::transmute_copy(bytes) }
    }

    /// Calculate CRC32 checksum
    ///
    /// Layout: sequence(8) + timestamp(8) + source_ai(32) + event_type(2) +
    ///         payload_len(2) + flags(2) + _reserved(2) + checksum(4) + padding(4)
    /// Checksum is at offset 56, so we hash bytes 0-56 + payload
    pub fn calculate_checksum(&self, payload: &[u8]) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        // Hash header bytes except checksum field (bytes 56-60) and padding (60-64)
        let header_bytes = self.to_bytes();
        hasher.update(&header_bytes[..56]);
        hasher.update(payload);
        hasher.finalize()
    }
}

// ============================================================================
// Event Payloads (rkyv zero-copy)
// ============================================================================

/// Broadcast event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct BroadcastPayload {
    pub channel: String,
    pub content: String,
}

/// Direct message event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct DirectMessagePayload {
    pub to_ai: String,
    pub content: String,
}

/// Presence update event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct PresenceUpdatePayload {
    pub status: String,
    pub current_task: Option<String>,
}

/// DM read event payload - marks a DM as read (persisted via event sourcing)
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct DmReadPayload {
    pub dm_id: u64,
}

/// Dialogue start event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct DialogueStartPayload {
    pub responder: String,
    pub topic: String,
    pub timeout_seconds: u32,
}

/// Dialogue respond event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct DialogueRespondPayload {
    pub dialogue_id: u64,
    pub content: String,
}

/// Dialogue end event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct DialogueEndPayload {
    pub dialogue_id: u64,
    pub status: String,
    /// Optional summary of the dialogue outcome
    pub summary: Option<String>,
}

/// Dialogue merge event payload - combines two dialogues into one
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct DialogueMergePayload {
    /// The dialogue to be merged (will be marked as merged)
    pub source_id: u64,
    /// The dialogue to merge into (will remain active)
    pub target_id: u64,
}

/// Vote create event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct VoteCreatePayload {
    pub topic: String,
    pub options: Vec<String>,
    pub required_voters: u32,
}

/// Vote cast event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct VoteCastPayload {
    pub vote_id: u64,
    pub choice: String,
}

/// Vote close event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct VoteClosePayload {
    pub vote_id: u64,
}

/// Room create event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct RoomCreatePayload {
    pub name: String,
    pub topic: Option<String>,
}

/// Room join event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct RoomJoinPayload {
    pub room_id: String,
}

/// Room leave event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct RoomLeavePayload {
    pub room_id: String,
}

/// Room message event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct RoomMessagePayload {
    pub room_id: String,
    pub content: String,
}

/// Room close event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct RoomClosePayload {
    pub room_id: String,
}

/// Lock acquire event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct LockAcquirePayload {
    pub resource: String,
    pub duration_seconds: u32,
    pub reason: String,
}

/// Lock release event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct LockReleasePayload {
    pub resource: String,
}

/// File action event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct FileActionPayload {
    pub path: String,
    pub action: String, // "read", "write", "delete", etc.
}

/// File claim event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct FileClaimPayload {
    pub path: String,
    pub duration_seconds: u32,
}

/// File release event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct FileReleasePayload {
    pub path: String,
}

/// Task create event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct TaskCreatePayload {
    pub description: String,
    pub priority: i32,
    pub tags: Option<String>,
}

/// Task claim event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct TaskClaimPayload {
    pub task_id: u64,
}

/// Task start event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct TaskStartPayload {
    pub task_id: u64,
}

/// Task complete event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct TaskCompletePayload {
    pub task_id: u64,
    pub result: String,
}

/// Task block event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct TaskBlockPayload {
    pub task_id: u64,
    pub reason: String,
}

/// Task unblock event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct TaskUnblockPayload {
    pub task_id: u64,
}
/// Pheromone deposit event payload (stigmergy - indirect coordination)
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]pub struct PheromoneDepositPayload {
    pub location: String,
    pub pheromone_type: String,
    pub content: String,
    pub intensity: u8,
}

/// Project create event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct ProjectCreatePayload {
    pub name: String,
    pub goal: String,
    pub root_directory: String,
}

/// Project update event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct ProjectUpdatePayload {
    pub project_id: u64,
    pub goal: Option<String>,
    pub status: Option<String>,
}

/// Project delete event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct ProjectDeletePayload {
    pub project_id: u64,
}

/// Project restore event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct ProjectRestorePayload {
    pub project_id: u64,
}

/// Feature create event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct FeatureCreatePayload {
    pub project_id: u64,
    pub name: String,
    pub overview: String,
    pub directory: Option<String>,
}

/// Feature update event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct FeatureUpdatePayload {
    pub feature_id: u64,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub directory: Option<String>,
}

/// Feature delete event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct FeatureDeletePayload {
    pub feature_id: u64,
}

/// Feature restore event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct FeatureRestorePayload {
    pub feature_id: u64,
}

/// Learning create event payload - shared team insight
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct LearningCreatePayload {
    pub content: String,
    pub tags: String,
    pub importance: u8, // 0-100 scale (stored as u8 for efficiency)
}

/// Learning update event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct LearningUpdatePayload {
    pub learning_id: u64,
    pub content: Option<String>,
    pub tags: Option<String>,
    pub importance: Option<u8>,
}

/// Learning delete event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct LearningDeletePayload {
    pub learning_id: u64,
}

/// Trust record event payload (TIP: Trust Inference and Propagation)
/// Uses Beta distribution: Trust = α/(α+β) where α=successes, β=failures
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct TrustRecordPayload {
    pub target_ai: String,      // AI being rated
    pub is_success: bool,       // true = positive interaction, false = negative
    pub context: String,        // What interaction this was about
    pub weight: u8,             // Significance 1-10 (default 1)
}

/// Batch create event payload - simple grouped tasks with AI-chosen labels
/// Tasks format: "1:Fix login,2:Fix logout,3:Test both" or "a:Header,b:Footer"
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct BatchCreatePayload {
    pub name: String,           // Batch name (e.g. "Homepage Redesign")
    pub tasks: String,          // Inline tasks "label:desc,label:desc,..."
}

/// Batch task done event payload
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct BatchTaskDonePayload {
    pub batch_name: String,
    pub label: String,
}

/// Batch close event payload - closes batch, marks remaining tasks done
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub struct BatchClosePayload {
    pub batch_name: String,
}

// ============================================================================
// Unified Event Enum
// ============================================================================

/// All possible TeamEngram events
///
/// This enum covers every mutation that can occur in the system.
/// Each variant maps to a specific event type constant.
#[derive(Archive, Deserialize, Serialize, Debug, Clone)]
#[rkyv(compare(PartialEq))]
pub enum EventPayload {
    // Coordination
    Broadcast(BroadcastPayload),
    DirectMessage(DirectMessagePayload),
    PresenceUpdate(PresenceUpdatePayload),
    // NOTE: DmRead moved to END of enum to preserve rkyv variant indices!
    // Adding variants in the middle breaks deserialization of old events.

    // Dialogues
    DialogueStart(DialogueStartPayload),
    DialogueRespond(DialogueRespondPayload),
    DialogueEnd(DialogueEndPayload),
    DialogueMerge(DialogueMergePayload),

    // Votes
    VoteCreate(VoteCreatePayload),
    VoteCast(VoteCastPayload),
    VoteClose(VoteClosePayload),

    // Rooms
    RoomCreate(RoomCreatePayload),
    RoomJoin(RoomJoinPayload),
    RoomLeave(RoomLeavePayload),
    RoomMessage(RoomMessagePayload),
    RoomClose(RoomClosePayload),

    // Locks
    LockAcquire(LockAcquirePayload),
    LockRelease(LockReleasePayload),

    // Files
    FileAction(FileActionPayload),
    FileClaim(FileClaimPayload),
    FileRelease(FileReleasePayload),

    // Tasks
    TaskCreate(TaskCreatePayload),
    TaskClaim(TaskClaimPayload),
    TaskStart(TaskStartPayload),
    TaskComplete(TaskCompletePayload),
    TaskBlock(TaskBlockPayload),
    TaskUnblock(TaskUnblockPayload),

    // Stigmergy
    PheromoneDeposit(PheromoneDepositPayload),

    // Projects
    ProjectCreate(ProjectCreatePayload),
    ProjectUpdate(ProjectUpdatePayload),
    ProjectDelete(ProjectDeletePayload),
    ProjectRestore(ProjectRestorePayload),

    // Features
    FeatureCreate(FeatureCreatePayload),
    FeatureUpdate(FeatureUpdatePayload),
    FeatureDelete(FeatureDeletePayload),
    FeatureRestore(FeatureRestorePayload),

    // Learnings (shared team insights)
    LearningCreate(LearningCreatePayload),
    LearningUpdate(LearningUpdatePayload),
    LearningDelete(LearningDeletePayload),

    // Trust (TIP: Trust Inference and Propagation)
    TrustRecord(TrustRecordPayload),

    // Batch - simple grouped tasks with AI-chosen labels
    BatchCreate(BatchCreatePayload),
    BatchTaskDone(BatchTaskDonePayload),
    BatchClose(BatchClosePayload),

    // DM Read tracking (added at END to preserve rkyv variant indices)
    // NEVER add new variants in the middle of this enum!
    DmRead(DmReadPayload),
}

impl EventPayload {
    /// Get the event type constant for this payload
    pub fn event_type(&self) -> u16 {
        match self {
            EventPayload::Broadcast(_) => event_type::BROADCAST,
            EventPayload::DirectMessage(_) => event_type::DIRECT_MESSAGE,
            EventPayload::PresenceUpdate(_) => event_type::PRESENCE_UPDATE,
            EventPayload::DmRead(_) => event_type::DM_READ,
            EventPayload::DialogueStart(_) => event_type::DIALOGUE_START,
            EventPayload::DialogueRespond(_) => event_type::DIALOGUE_RESPOND,
            EventPayload::DialogueEnd(_) => event_type::DIALOGUE_END,
            EventPayload::DialogueMerge(_) => event_type::DIALOGUE_MERGE,
            EventPayload::VoteCreate(_) => event_type::VOTE_CREATE,
            EventPayload::VoteCast(_) => event_type::VOTE_CAST,
            EventPayload::VoteClose(_) => event_type::VOTE_CLOSE,
            EventPayload::RoomCreate(_) => event_type::ROOM_CREATE,
            EventPayload::RoomJoin(_) => event_type::ROOM_JOIN,
            EventPayload::RoomLeave(_) => event_type::ROOM_LEAVE,
            EventPayload::RoomMessage(_) => event_type::ROOM_MESSAGE,
            EventPayload::RoomClose(_) => event_type::ROOM_CLOSE,
            EventPayload::LockAcquire(_) => event_type::LOCK_ACQUIRE,
            EventPayload::LockRelease(_) => event_type::LOCK_RELEASE,
            EventPayload::FileAction(_) => event_type::FILE_ACTION,
            EventPayload::FileClaim(_) => event_type::FILE_CLAIM,
            EventPayload::FileRelease(_) => event_type::FILE_RELEASE,
            EventPayload::TaskCreate(_) => event_type::TASK_CREATE,
            EventPayload::TaskClaim(_) => event_type::TASK_CLAIM,
            EventPayload::TaskStart(_) => event_type::TASK_START,
            EventPayload::TaskComplete(_) => event_type::TASK_COMPLETE,
            EventPayload::TaskBlock(_) => event_type::TASK_BLOCK,
            EventPayload::TaskUnblock(_) => event_type::TASK_UNBLOCK,
            EventPayload::PheromoneDeposit(_) => event_type::PHEROMONE_DEPOSIT,
            EventPayload::ProjectCreate(_) => event_type::PROJECT_CREATE,
            EventPayload::ProjectUpdate(_) => event_type::PROJECT_UPDATE,
            EventPayload::ProjectDelete(_) => event_type::PROJECT_DELETE,
            EventPayload::ProjectRestore(_) => event_type::PROJECT_RESTORE,
            EventPayload::FeatureCreate(_) => event_type::FEATURE_CREATE,
            EventPayload::FeatureUpdate(_) => event_type::FEATURE_UPDATE,
            EventPayload::FeatureDelete(_) => event_type::FEATURE_DELETE,
            EventPayload::FeatureRestore(_) => event_type::FEATURE_RESTORE,
            EventPayload::LearningCreate(_) => event_type::LEARNING_CREATE,
            EventPayload::LearningUpdate(_) => event_type::LEARNING_UPDATE,
            EventPayload::LearningDelete(_) => event_type::LEARNING_DELETE,
            EventPayload::TrustRecord(_) => event_type::TRUST_RECORD,
            EventPayload::BatchCreate(_) => event_type::BATCH_CREATE,
            EventPayload::BatchTaskDone(_) => event_type::BATCH_TASK_DONE,
            EventPayload::BatchClose(_) => event_type::BATCH_CLOSE,
        }
    }

    /// Serialize payload to bytes using rkyv
    pub fn to_bytes(&self) -> Vec<u8> {
        rkyv::to_bytes::<RkyvError>(self)
            .expect("Serialization should not fail")
            .to_vec()
    }

    /// Deserialize payload from bytes
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        rkyv::from_bytes::<Self, RkyvError>(data).ok()
    }

    /// Access archived payload without copying (zero-copy)
    pub fn access(data: &[u8]) -> Option<&ArchivedEventPayload> {
        rkyv::access::<ArchivedEventPayload, RkyvError>(data).ok()
    }
}

// ============================================================================
// Full Event (Header + Payload)
// ============================================================================

/// A complete event with header and payload
#[derive(Debug, Clone)]
pub struct Event {
    pub header: EventHeader,
    pub payload: EventPayload,
}

impl Event {
    /// Create a new event
    pub fn new(source_ai: &str, payload: EventPayload) -> Self {
        let payload_bytes = payload.to_bytes();
        let header = EventHeader::new(source_ai, payload.event_type(), payload_bytes.len() as u16);
        Self { header, payload }
    }

    /// Serialize to bytes (header + payload)
    pub fn to_bytes(&self) -> Vec<u8> {
        let payload_bytes = self.payload.to_bytes();
        let mut header = self.header;
        header.payload_len = payload_bytes.len() as u16;
        header.checksum = header.calculate_checksum(&payload_bytes);

        let mut bytes = Vec::with_capacity(EventHeader::SIZE + payload_bytes.len());
        bytes.extend_from_slice(&header.to_bytes());
        bytes.extend_from_slice(&payload_bytes);
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < EventHeader::SIZE {
            return None;
        }

        let header_bytes: [u8; EventHeader::SIZE] = data[..EventHeader::SIZE].try_into().ok()?;
        let header = EventHeader::from_bytes(&header_bytes);

        let payload_end = EventHeader::SIZE + header.payload_len as usize;
        if data.len() < payload_end {
            return None;
        }

        let payload_bytes = &data[EventHeader::SIZE..payload_end];

        // Verify checksum
        let expected_checksum = header.calculate_checksum(payload_bytes);
        if header.checksum != expected_checksum {
            return None; // Corrupted event
        }

        let payload = EventPayload::from_bytes(payload_bytes)?;
        Some(Self { header, payload })
    }

    /// Get the sequence number
    pub fn sequence(&self) -> u64 {
        self.header.sequence
    }

    /// Get the source AI
    pub fn source_ai(&self) -> &str {
        self.header.source_ai_str()
    }

    /// Get the event type
    pub fn event_type(&self) -> u16 {
        self.header.event_type
    }

    /// Get the timestamp in microseconds
    pub fn timestamp(&self) -> u64 {
        self.header.timestamp
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Get current timestamp in microseconds since UNIX epoch
fn current_timestamp_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

// ============================================================================
// Convenience Constructors
// ============================================================================

impl Event {
    /// Create a broadcast event
    pub fn broadcast(source_ai: &str, channel: &str, content: &str) -> Self {
        Self::new(source_ai, EventPayload::Broadcast(BroadcastPayload {
            channel: channel.to_string(),
            content: content.to_string(),
        }))
    }

    /// Create a direct message event
    pub fn direct_message(source_ai: &str, to_ai: &str, content: &str) -> Self {
        Self::new(source_ai, EventPayload::DirectMessage(DirectMessagePayload {
            to_ai: to_ai.to_string(),
            content: content.to_string(),
        }))
    }

    /// Create a presence update event
    pub fn presence_update(source_ai: &str, status: &str, current_task: Option<&str>) -> Self {
        Self::new(source_ai, EventPayload::PresenceUpdate(PresenceUpdatePayload {
            status: status.to_string(),
            current_task: current_task.map(|s| s.to_string()),
        }))
    }

    /// Create a DM read event (marks a DM as read, persisted via event sourcing)
    pub fn dm_read(source_ai: &str, dm_id: u64) -> Self {
        Self::new(source_ai, EventPayload::DmRead(DmReadPayload {
            dm_id,
        }))
    }

    /// Create a dialogue start event
    pub fn dialogue_start(source_ai: &str, responder: &str, topic: &str) -> Self {
        Self::new(source_ai, EventPayload::DialogueStart(DialogueStartPayload {
            responder: responder.to_string(),
            topic: topic.to_string(),
            timeout_seconds: 180, // Default 3 minutes
        }))
    }

    /// Create a dialogue respond event
    pub fn dialogue_respond(source_ai: &str, dialogue_id: u64, content: &str) -> Self {
        Self::new(source_ai, EventPayload::DialogueRespond(DialogueRespondPayload {
            dialogue_id,
            content: content.to_string(),
        }))
    }

    /// Create a vote create event
    pub fn vote_create(source_ai: &str, topic: &str, options: Vec<String>, required_voters: u32) -> Self {
        Self::new(source_ai, EventPayload::VoteCreate(VoteCreatePayload {
            topic: topic.to_string(),
            options,
            required_voters,
        }))
    }

    /// Create a vote cast event
    pub fn vote_cast(source_ai: &str, vote_id: u64, choice: &str) -> Self {
        Self::new(source_ai, EventPayload::VoteCast(VoteCastPayload {
            vote_id,
            choice: choice.to_string(),
        }))
    }

    /// Create a vote close event
    pub fn vote_close(source_ai: &str, vote_id: u64) -> Self {
        Self::new(source_ai, EventPayload::VoteClose(VoteClosePayload {
            vote_id,
        }))
    }

    // lock_acquire() and lock_release() removed — locks deprecated (Feb 2026, QD directive)
    // Enum variants + payload structs kept for rkyv backward compatibility

    /// Create a file action event
    pub fn file_action(source_ai: &str, path: &str, action: &str) -> Self {
        Self::new(source_ai, EventPayload::FileAction(FileActionPayload {
            path: path.to_string(),
            action: action.to_string(),
        }))
    }

    /// Create a file claim event
    pub fn file_claim(source_ai: &str, path: &str, duration_seconds: u32) -> Self {
        Self::new(source_ai, EventPayload::FileClaim(FileClaimPayload {
            path: path.to_string(),
            duration_seconds,
        }))
    }

    /// Create a file release event
    pub fn file_release(source_ai: &str, path: &str) -> Self {
        Self::new(source_ai, EventPayload::FileRelease(FileReleasePayload {
            path: path.to_string(),
        }))
    }

    /// Create a task create event
    pub fn task_create(source_ai: &str, description: &str, priority: i32, tags: Option<&str>) -> Self {
        Self::new(source_ai, EventPayload::TaskCreate(TaskCreatePayload {
            description: description.to_string(),
            priority,
            tags: tags.map(|s| s.to_string()),
        }))
    }

    /// Alias for task_create
    pub fn task_add(source_ai: &str, description: &str, priority: u32, tags: &str) -> Self {
        Self::task_create(source_ai, description, priority as i32, Some(tags))
    }

    /// Create a task claim event
    pub fn task_claim(source_ai: &str, task_id: u64) -> Self {
        Self::new(source_ai, EventPayload::TaskClaim(TaskClaimPayload { task_id }))
    }

    /// Create a task complete event
    pub fn task_complete(source_ai: &str, task_id: u64, result: &str) -> Self {
        Self::new(source_ai, EventPayload::TaskComplete(TaskCompletePayload {
            task_id,
            result: result.to_string(),
        }))
    }

    /// Create a task start event
    pub fn task_start(source_ai: &str, task_id: u64) -> Self {
        Self::new(source_ai, EventPayload::TaskStart(TaskStartPayload { task_id }))
    }

    /// Create a task block event
    pub fn task_block(source_ai: &str, task_id: u64, reason: &str) -> Self {
        Self::new(source_ai, EventPayload::TaskBlock(TaskBlockPayload {
            task_id,
            reason: reason.to_string(),
        }))
    }

    /// Create a task unblock event
    pub fn task_unblock(source_ai: &str, task_id: u64) -> Self {
        Self::new(source_ai, EventPayload::TaskUnblock(TaskUnblockPayload { task_id }))
    }

    /// Create a dialogue end event
    pub fn dialogue_end(source_ai: &str, dialogue_id: u64, status: &str) -> Self {
        Self::new(source_ai, EventPayload::DialogueEnd(DialogueEndPayload {
            dialogue_id,
            status: status.to_string(),
            summary: None,
        }))
    }

    /// Create a dialogue end event with summary
    pub fn dialogue_end_with_summary(source_ai: &str, dialogue_id: u64, status: &str, summary: Option<&str>) -> Self {
        Self::new(source_ai, EventPayload::DialogueEnd(DialogueEndPayload {
            dialogue_id,
            status: status.to_string(),
            summary: summary.map(|s| s.to_string()),
        }))
    }

    /// Create a dialogue merge event - merges source dialogue into target
    pub fn dialogue_merge(source_ai: &str, source_id: u64, target_id: u64) -> Self {
        Self::new(source_ai, EventPayload::DialogueMerge(DialogueMergePayload {
            source_id,
            target_id,
        }))
    }

    /// Create a room create event
    pub fn room_create(source_ai: &str, name: &str, topic: &str) -> Self {
        Self::new(source_ai, EventPayload::RoomCreate(RoomCreatePayload {
            name: name.to_string(),
            topic: Some(topic.to_string()),
        }))
    }

    /// Create a room join event
    pub fn room_join(source_ai: &str, room_id: &str) -> Self {
        Self::new(source_ai, EventPayload::RoomJoin(RoomJoinPayload {
            room_id: room_id.to_string(),
        }))
    }

    /// Create a room leave event
    pub fn room_leave(source_ai: &str, room_id: &str) -> Self {
        Self::new(source_ai, EventPayload::RoomLeave(RoomLeavePayload {
            room_id: room_id.to_string(),
        }))
    }

    /// Create a room close event
    pub fn room_close(source_ai: &str, room_id: &str) -> Self {
        Self::new(source_ai, EventPayload::RoomClose(RoomClosePayload {
            room_id: room_id.to_string(),
        }))
    }

    /// Create a room message event
    pub fn room_message(source_ai: &str, room_id: &str, content: &str) -> Self {
        Self::new(source_ai, EventPayload::RoomMessage(RoomMessagePayload {
            room_id: room_id.to_string(),
            content: content.to_string(),
        }))
    }

    // pheromone_deposit() removed — stigmergy deprecated (Feb 2026, QD directive)
    // Enum variant + payload struct kept for rkyv backward compatibility

    // ===== Project Events =====

    /// Create a project create event
    pub fn project_create(source_ai: &str, name: &str, goal: &str, root_directory: &str) -> Self {
        Self::new(source_ai, EventPayload::ProjectCreate(ProjectCreatePayload {
            name: name.to_string(),
            goal: goal.to_string(),
            root_directory: root_directory.to_string(),
        }))
    }

    /// Create a project update event
    pub fn project_update(source_ai: &str, project_id: u64, goal: Option<&str>, status: Option<&str>) -> Self {
        Self::new(source_ai, EventPayload::ProjectUpdate(ProjectUpdatePayload {
            project_id,
            goal: goal.map(|s| s.to_string()),
            status: status.map(|s| s.to_string()),
        }))
    }

    /// Create a project delete event
    pub fn project_delete(source_ai: &str, project_id: u64) -> Self {
        Self::new(source_ai, EventPayload::ProjectDelete(ProjectDeletePayload {
            project_id,
        }))
    }

    /// Create a project restore event
    pub fn project_restore(source_ai: &str, project_id: u64) -> Self {
        Self::new(source_ai, EventPayload::ProjectRestore(ProjectRestorePayload {
            project_id,
        }))
    }

    // ===== Feature Events =====

    /// Create a feature create event
    pub fn feature_create(source_ai: &str, project_id: u64, name: &str, overview: &str, directory: Option<&str>) -> Self {
        Self::new(source_ai, EventPayload::FeatureCreate(FeatureCreatePayload {
            project_id,
            name: name.to_string(),
            overview: overview.to_string(),
            directory: directory.map(|s| s.to_string()),
        }))
    }

    /// Create a feature update event
    pub fn feature_update(source_ai: &str, feature_id: u64, name: Option<&str>, overview: Option<&str>, directory: Option<&str>) -> Self {
        Self::new(source_ai, EventPayload::FeatureUpdate(FeatureUpdatePayload {
            feature_id,
            name: name.map(|s| s.to_string()),
            overview: overview.map(|s| s.to_string()),
            directory: directory.map(|s| s.to_string()),
        }))
    }

    /// Create a feature delete event
    pub fn feature_delete(source_ai: &str, feature_id: u64) -> Self {
        Self::new(source_ai, EventPayload::FeatureDelete(FeatureDeletePayload {
            feature_id,
        }))
    }

    /// Create a feature restore event
    pub fn feature_restore(source_ai: &str, feature_id: u64) -> Self {
        Self::new(source_ai, EventPayload::FeatureRestore(FeatureRestorePayload {
            feature_id,
        }))
    }

    // ===== Learning Events (Shared Team Insights) =====

    /// Create a learning create event
    pub fn learning_create(source_ai: &str, content: &str, tags: &str, importance: u8) -> Self {
        Self::new(source_ai, EventPayload::LearningCreate(LearningCreatePayload {
            content: content.to_string(),
            tags: tags.to_string(),
            importance,
        }))
    }

    /// Create a learning update event
    pub fn learning_update(source_ai: &str, learning_id: u64, content: Option<&str>, tags: Option<&str>, importance: Option<u8>) -> Self {
        Self::new(source_ai, EventPayload::LearningUpdate(LearningUpdatePayload {
            learning_id,
            content: content.map(|s| s.to_string()),
            tags: tags.map(|s| s.to_string()),
            importance,
        }))
    }

    /// Create a learning delete event
    pub fn learning_delete(source_ai: &str, learning_id: u64) -> Self {
        Self::new(source_ai, EventPayload::LearningDelete(LearningDeletePayload {
            learning_id,
        }))
    }

    /// Create a trust record event (TIP: Trust Inference and Propagation)
    pub fn trust_record(source_ai: &str, target_ai: &str, is_success: bool, context: &str, weight: u8) -> Self {
        Self::new(source_ai, EventPayload::TrustRecord(TrustRecordPayload {
            target_ai: target_ai.to_string(),
            is_success,
            context: context.to_string(),
            weight: weight.clamp(1, 10), // Ensure weight is 1-10
        }))
    }

    /// Create a batch with inline tasks
    /// tasks format: "1:Fix login,2:Fix logout,3:Test both"
    pub fn batch_create(source_ai: &str, name: &str, tasks: &str) -> Self {
        Self::new(source_ai, EventPayload::BatchCreate(BatchCreatePayload {
            name: name.to_string(),
            tasks: tasks.to_string(),
        }))
    }

    /// Mark a task in a batch as done
    pub fn batch_task_done(source_ai: &str, batch_name: &str, label: &str) -> Self {
        Self::new(source_ai, EventPayload::BatchTaskDone(BatchTaskDonePayload {
            batch_name: batch_name.to_string(),
            label: label.to_string(),
        }))
    }

    /// Close a batch (marks all remaining tasks as done)
    pub fn batch_close(source_ai: &str, batch_name: &str) -> Self {
        Self::new(source_ai, EventPayload::BatchClose(BatchClosePayload {
            batch_name: batch_name.to_string(),
        }))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_header_size() {
        assert_eq!(std::mem::size_of::<EventHeader>(), 64);
    }

    #[test]
    fn test_event_roundtrip() {
        let event = Event::broadcast("lyra-584", "general", "Hello, team!");
        let bytes = event.to_bytes();
        let decoded = Event::from_bytes(&bytes).expect("Should decode");

        assert_eq!(decoded.source_ai(), "lyra-584");
        assert_eq!(decoded.event_type(), event_type::BROADCAST);

        if let EventPayload::Broadcast(payload) = &decoded.payload {
            assert_eq!(payload.channel, "general");
            assert_eq!(payload.content, "Hello, team!");
        } else {
            panic!("Wrong payload type");
        }
    }

    #[test]
    fn test_event_checksum_validation() {
        let event = Event::direct_message("sage-724", "lyra-584", "Test message");
        let mut bytes = event.to_bytes();

        // Corrupt the payload
        if bytes.len() > EventHeader::SIZE {
            bytes[EventHeader::SIZE] ^= 0xFF;
        }

        // Should fail checksum validation
        assert!(Event::from_bytes(&bytes).is_none());
    }

    #[test]
    fn test_zero_copy_access() {
        let payload = EventPayload::Broadcast(BroadcastPayload {
            channel: "general".to_string(),
            content: "Zero-copy test".to_string(),
        });
        let bytes = payload.to_bytes();

        // Access without copying
        let archived = EventPayload::access(&bytes).expect("Should access");

        // Can read fields directly from the buffer
        if let ArchivedEventPayload::Broadcast(ref p) = archived {
            assert_eq!(p.channel.as_str(), "general");
            assert_eq!(p.content.as_str(), "Zero-copy test");
        } else {
            panic!("Wrong archived type");
        }
    }

    #[test]
    fn test_all_event_types() {
        // Test each event type can serialize/deserialize
        let events = vec![
            Event::broadcast("ai", "ch", "msg"),
            Event::direct_message("ai", "to", "msg"),
            Event::presence_update("ai", "active", Some("working")),
            Event::dialogue_start("ai", "responder", "topic"),
            Event::dialogue_respond("ai", 1, "response"),
            Event::vote_create("ai", "topic", vec!["a".into(), "b".into()], 3),
            Event::vote_cast("ai", 1, "a"),
            Event::file_action("ai", "/path", "read"),
            Event::task_create("ai", "description", 5, Some("tag1,tag2")),
        ];

        for event in events {
            let bytes = event.to_bytes();
            let decoded = Event::from_bytes(&bytes);
            assert!(decoded.is_some(), "Failed for event type {}", event.event_type());
        }
    }
}
