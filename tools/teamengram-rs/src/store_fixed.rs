//! TeamEngram Store - High-level API for team coordination data
//!
//! Provides typed storage for:
//! - Direct Messages (DMs)
//! - Broadcasts
//! - Presence
//! - Dialogues
//! - Tasks

use crate::btree::BTree;
use crate::shadow::ShadowAllocator;
use crate::{NotifyCallback, NotifyType, NoOpNotify};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Record types stored in TeamEngram
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum RecordType {
    DirectMessage = 1,
    Broadcast = 2,
    Presence = 3,
    Dialogue = 4,
    Task = 5,
    Vote = 6,
    FileClaim = 7,
    Room = 8,
    Lock = 9,
    FileAction = 10,      // Track file created/modified/deleted
    DirectoryAccess = 11, // Track directory accessed
    Project = 12,         // Project management
    Feature = 13,         // Feature within a project
    VaultEntry = 14,      // Shared key-value storage
}

impl RecordType {
    fn prefix(&self) -> &'static [u8] {
        match self {
            RecordType::DirectMessage => b"dm:",
            RecordType::Broadcast => b"bc:",
            RecordType::Presence => b"pr:",
            RecordType::Dialogue => b"dg:",
            RecordType::Task => b"tk:",
            RecordType::Vote => b"vt:",
            RecordType::FileClaim => b"fc:",
            RecordType::Room => b"rm:",
            RecordType::Lock => b"lk:",
            RecordType::FileAction => b"fa:",
            RecordType::DirectoryAccess => b"da:",
            RecordType::Project => b"pj:",
            RecordType::Feature => b"ft:",
            RecordType::VaultEntry => b"vl:",
        }
    }
}

/// Dialogue status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum DialogueStatus {
    Active = 0,
    Completed = 1,
    Expired = 2,
    Cancelled = 3,
}

/// Vote status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum VoteStatus {
    Open = 0,
    Closed = 1,
    Cancelled = 2,
}

/// Task status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum TaskStatus {
    Pending = 0,
    Claimed = 1,
    InProgress = 2,
    Completed = 3,
    Failed = 4,
    Cancelled = 5,
}

/// Task priority
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum TaskPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Urgent = 3,
}

/// A stored record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: u64,
    pub record_type: RecordType,
    pub created_at: u64,
    pub data: RecordData,
}

/// Record-specific data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecordData {
    DirectMessage(DirectMessage),
    Broadcast(Broadcast),
    Presence(Presence),
    Task(Task),
    Vote(Vote),
    Dialogue(Dialogue),
    FileClaim(FileClaim),
    Room(Room),
    Lock(Lock),
    FileAction(FileAction),
    DirectoryAccess(DirectoryAccess),
    Project(Project),
    Feature(Feature),
    VaultEntry(VaultEntry),
}

/// Direct message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectMessage {
    pub from_ai: String,
    pub to_ai: String,
    pub content: String,
    pub read: bool,
}

/// Broadcast message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Broadcast {
    pub from_ai: String,
    pub channel: String,
    pub content: String,
}

/// Presence record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Presence {
    pub ai_id: String,
    pub status: String,
    pub current_task: String,
    pub last_seen: u64,
}

/// Task record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub created_by: String,
    pub claimed_by: Option<String>,
    pub description: String,
    pub tags: String,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub result: Option<String>,
    pub claimed_at: Option<u64>,
    pub completed_at: Option<u64>,
}

/// Vote record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    pub topic: String,
    pub options: Vec<String>,
    pub votes: Vec<(String, String)>, // (ai_id, option)
    pub status: VoteStatus,
    pub created_by: String,
    pub closes_at: u64,
}

/// Dialogue record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dialogue {
    pub initiator: String,
    pub responder: String,
    pub topic: String,
    pub status: DialogueStatus,
    pub turn: u8,  // 0 = initiator's turn, 1 = responder's turn
    pub message_count: u32,
    pub turn_timeout_secs: u32,
    pub updated_at: u64,
}

/// File claim record (prevents conflicts)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileClaim {
    pub claimer: String,
    pub path: String,
    pub working_on: String,
    pub expires_at: u64,
}

/// Room record (collaboration space)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub name: String,
    pub creator: String,
    pub participants: Vec<String>,
    pub topic: String,
    pub is_open: bool,
}

/// Resource lock (for coordination)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lock {
    pub holder: String,
    pub resource: String,
    pub working_on: String,
    pub acquired_at: u64,
    pub expires_at: u64,
}

/// File action record (for awareness/SessionStart)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAction {
    pub ai_id: String,
    pub path: String,
    pub action: String, // created, modified, deleted, reviewed
    pub timestamp: u64,
}

/// Directory access record (for awareness/SessionStart)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryAccess {
    pub ai_id: String,
    pub directory: String,
    pub access_type: String, // read, write, search
    pub timestamp: u64,
}

/// Project record (multi-project management)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub goal: String,
    pub root_directory: String,
    pub created_by: String,
    pub status: String, // active, archived, deleted
    pub created_at: u64,
    pub updated_at: u64,
}

/// Feature record (component within a project)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feature {
    pub project_id: u64,
    pub name: String,
    pub overview: String,
    pub directory: Option<String>,
    pub created_by: String,
    pub status: String, // active, archived, deleted
    pub created_at: u64,
    pub updated_at: u64,
}

/// Shared vault entry (team-wide key-value storage)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultEntry {
    pub key: String,
    pub value: String,
    pub updated_by: String,
    pub updated_at: u64,
}

/// TeamEngram store
pub struct TeamEngram {
    allocator: ShadowAllocator,
    #[allow(dead_code)]
    path: PathBuf,
    next_id: u64,
    /// IPC notification callback (fires after writes)
    notify: Arc<dyn NotifyCallback>,
}

impl TeamEngram {
    /// Open or create a TeamEngram store
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut allocator = ShadowAllocator::open(&path)
            .context("Failed to open TeamEngram store")?;

        // Load next_id from persisted value
        let next_id = Self::load_next_id(&mut allocator)?;

        Ok(Self {
            allocator,
            path,
            next_id,
            notify: Arc::new(NoOpNotify),
        })
    }

    /// Open with a custom notification callback
    pub fn open_with_notify(path: impl AsRef<Path>, notify: Arc<dyn NotifyCallback>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut allocator = ShadowAllocator::open(&path)
            .context("Failed to open TeamEngram store")?;

        // Load next_id from persisted value
        let next_id = Self::load_next_id(&mut allocator)?;

        Ok(Self {
            allocator,
            path,
            next_id,
            notify,
        })
    }

    /// Load next_id from stored value, or scan for max ID
    fn load_next_id(allocator: &mut ShadowAllocator) -> Result<u64> {
        let tree = BTree::new(allocator);

        // Try to load from meta key
        if let Some(value) = tree.get(b"meta:next_id")? {
            if value.len() >= 8 {
                let next_id = u64::from_le_bytes(value[..8].try_into().unwrap());
                return Ok(next_id);
            }
        }

        // Default to 1 if no persisted value
        Ok(1)
    }

    /// Persist next_id to storage
    fn persist_next_id(&mut self) -> Result<()> {
        let value = self.next_id.to_le_bytes().to_vec();
        let mut tree = BTree::new(&mut self.allocator);
        tree.insert(b"meta:next_id", &value)?;
        Ok(())
    }

    /// Set the notification callback
    pub fn set_notify(&mut self, notify: Arc<dyn NotifyCallback>) {
        self.notify = notify;
    }

    /// Helper to create content preview (first 128 chars)
    fn content_preview(content: &str) -> &str {
        if content.len() <= 128 {
            content
        } else {
            &content[..128]
        }
    }

    /// Get the default store path
    pub fn default_path() -> PathBuf {
        let base = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join(".ai-foundation").join("teamengram.engram")
    }

    /// Insert a direct message
    pub fn insert_dm(&mut self, from: &str, to: &str, content: &str) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;

        let record = Record {
            id,
            record_type: RecordType::DirectMessage,
            created_at: now_millis(),
            data: RecordData::DirectMessage(DirectMessage {
                from_ai: from.to_string(),
                to_ai: to.to_string(),
                content: content.to_string(),
                read: false,
            }),
        };

        // Serialize once, store at both locations
        let value = bincode::serialize(&record)?;

        // Store main record
        self.insert_record(&record)?;

        // Also index by recipient for fast lookup (store full record for easy retrieval)
        let recipient_key = format!("dm:to:{}:{:016x}", to, id);
        let mut tree = BTree::new(&mut self.allocator);
        tree.insert(recipient_key.as_bytes(), &value)?;

        // Fire IPC notification
        self.notify.notify(
            NotifyType::DirectMessage,
            from,
            to,
            Self::content_preview(content),
        );

        Ok(id)
    }

    /// Get DMs for a recipient
    pub fn get_dms(&mut self, to_ai: &str, limit: usize) -> Result<Vec<Record>> {
        let prefix = format!("dm:to:{}:", to_ai);
        self.query_by_prefix(&prefix, limit)
    }

    /// Insert a broadcast
    pub fn insert_broadcast(&mut self, from: &str, channel: &str, content: &str) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;

        let record = Record {
            id,
            record_type: RecordType::Broadcast,
            created_at: now_millis(),
            data: RecordData::Broadcast(Broadcast {
                from_ai: from.to_string(),
                channel: channel.to_string(),
                content: content.to_string(),
            }),
        };

        // Serialize once, store at both locations
        let value = bincode::serialize(&record)?;

        self.insert_record(&record)?;

        // Index by channel (store full record for easy retrieval)
        let channel_key = format!("bc:ch:{}:{:016x}", channel, id);
        let mut tree = BTree::new(&mut self.allocator);
        tree.insert(channel_key.as_bytes(), &value)?;

        // Fire IPC notification (check for mentions and urgent keywords)
        let notify_type = if content.contains("@") {
            NotifyType::Mention
        } else if content.to_lowercase().contains("urgent")
            || content.to_lowercase().contains("critical")
            || content.to_lowercase().contains("help") {
            NotifyType::Urgent
        } else {
            NotifyType::Broadcast
        };
        self.notify.notify(
            notify_type,
            from,
            "", // broadcast has no specific target
            Self::content_preview(content),
        );

        Ok(id)
    }

    /// Get broadcasts by channel
    pub fn get_broadcasts(&mut self, channel: &str, limit: usize) -> Result<Vec<Record>> {
        let prefix = format!("bc:ch:{}:", channel);
        self.query_by_prefix(&prefix, limit)
    }

    /// Update presence
    pub fn update_presence(&mut self, ai_id: &str, status: &str, task: &str) -> Result<()> {
        let key = format!("pr:{}", ai_id);

        let record = Record {
            id: 0, // Presence uses ai_id as key
            record_type: RecordType::Presence,
            created_at: now_millis(),
            data: RecordData::Presence(Presence {
                ai_id: ai_id.to_string(),
                status: status.to_string(),
                current_task: task.to_string(),
                last_seen: now_millis(),
            }),
        };

        let value = bincode::serialize(&record)?;
        let mut tree = BTree::new(&mut self.allocator);
        tree.insert(key.as_bytes(), &value)?;

        Ok(())
    }

    /// Get presence for an AI
    pub fn get_presence(&mut self, ai_id: &str) -> Result<Option<Presence>> {
        let key = format!("pr:{}", ai_id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let record: Record = bincode::deserialize(&value)?;
            if let RecordData::Presence(p) = record.data {
                return Ok(Some(p));
            }
        }

        Ok(None)
    }

    /// Get all active presences
    pub fn get_all_presences(&mut self) -> Result<Vec<Presence>> {
        self.query_by_prefix("pr:", 100)?
            .into_iter()
            .filter_map(|r| {
                if let RecordData::Presence(p) = r.data {
                    Some(Ok(p))
                } else {
                    None
                }
            })
            .collect()
    }

    // ========================================================================
    // DIALOGUE OPERATIONS
    // ========================================================================

    /// Start a new dialogue
    pub fn start_dialogue(&mut self, initiator: &str, responder: &str, topic: &str) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;

        let now = now_millis();
        let record = Record {
            id,
            record_type: RecordType::Dialogue,
            created_at: now,
            data: RecordData::Dialogue(Dialogue {
                initiator: initiator.to_string(),
                responder: responder.to_string(),
                topic: topic.to_string(),
                status: DialogueStatus::Active,
                turn: 1,  // Responder's turn first (initiator sent first message)
                message_count: 1,
                turn_timeout_secs: 180,  // 3 minutes default
                updated_at: now,
            }),
        };

        let value = bincode::serialize(&record)?;

        // Store main record
        self.insert_record(&record)?;

        // Index by initiator
        let init_key = format!("dg:ai:{}:{:016x}", initiator, id);
        let mut tree = BTree::new(&mut self.allocator);
        tree.insert(init_key.as_bytes(), &value)?;

        // Index by responder
        let resp_key = format!("dg:ai:{}:{:016x}", responder, id);
        let mut tree = BTree::new(&mut self.allocator);
        tree.insert(resp_key.as_bytes(), &value)?;

        // Fire IPC notification
        self.notify.notify(
            NotifyType::Dialogue,
            initiator,
            responder,
            Self::content_preview(topic),
        );

        Ok(id)
    }

    /// Get dialogue by ID
    pub fn get_dialogue(&mut self, id: u64) -> Result<Option<Dialogue>> {
        let key = format!("dg:id:{:016x}", id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let record: Record = bincode::deserialize(&value)?;
            if let RecordData::Dialogue(d) = record.data {
                return Ok(Some(d));
            }
        }
        Ok(None)
    }

    /// Get dialogues for an AI (as initiator or responder)
    pub fn get_dialogues_for_ai(&mut self, ai_id: &str, limit: usize) -> Result<Vec<(u64, Dialogue)>> {
        let prefix = format!("dg:ai:{}:", ai_id);
        let records = self.query_by_prefix(&prefix, limit)?;

        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::Dialogue(d) = r.data {
                Some((r.id, d))
            } else {
                None
            }
        }).collect())
    }

    /// Respond to a dialogue (updates turn)
    pub fn respond_to_dialogue(&mut self, id: u64) -> Result<bool> {
        if let Some(mut dialogue) = self.get_dialogue(id)? {
            dialogue.turn = if dialogue.turn == 0 { 1 } else { 0 };
            dialogue.message_count += 1;
            dialogue.updated_at = now_millis();

            // Re-store the updated dialogue
            let record = Record {
                id,
                record_type: RecordType::Dialogue,
                created_at: dialogue.updated_at,
                data: RecordData::Dialogue(dialogue),
            };
            self.insert_record(&record)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// End a dialogue
    pub fn end_dialogue(&mut self, id: u64, status: DialogueStatus) -> Result<bool> {
        if let Some(mut dialogue) = self.get_dialogue(id)? {
            dialogue.status = status;
            dialogue.updated_at = now_millis();

            let record = Record {
                id,
                record_type: RecordType::Dialogue,
                created_at: dialogue.updated_at,
                data: RecordData::Dialogue(dialogue),
            };
            self.insert_record(&record)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ========================================================================
    // VOTE OPERATIONS
    // ========================================================================

    /// Create a new vote
    pub fn create_vote(&mut self, created_by: &str, topic: &str, options: Vec<String>, duration_mins: u32) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;

        let now = now_millis();
        let record = Record {
            id,
            record_type: RecordType::Vote,
            created_at: now,
            data: RecordData::Vote(Vote {
                topic: topic.to_string(),
                options,
                votes: Vec::new(),
                status: VoteStatus::Open,
                created_by: created_by.to_string(),
                closes_at: now + (duration_mins as u64 * 60 * 1000),
            }),
        };

        self.insert_record(&record)?;

        // Fire IPC notification
        self.notify.notify(
            NotifyType::Vote,
            created_by,
            "", // Vote is broadcast to all
            Self::content_preview(topic),
        );

        Ok(id)
    }

    /// Cast a vote
    pub fn cast_vote(&mut self, vote_id: u64, ai_id: &str, option: &str) -> Result<bool> {
        let key = format!("vt:id:{:016x}", vote_id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::Vote(ref mut vote) = record.data {
                if vote.status != VoteStatus::Open {
                    return Ok(false);
                }
                // Remove any existing vote from this AI
                vote.votes.retain(|(voter, _)| voter != ai_id);
                // Add new vote
                vote.votes.push((ai_id.to_string(), option.to_string()));

                self.insert_record(&record)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Get vote by ID
    pub fn get_vote(&mut self, id: u64) -> Result<Option<Vote>> {
        let key = format!("vt:id:{:016x}", id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let record: Record = bincode::deserialize(&value)?;
            if let RecordData::Vote(v) = record.data {
                return Ok(Some(v));
            }
        }
        Ok(None)
    }

    /// List recent votes
    pub fn list_votes(&mut self, limit: usize) -> Result<Vec<(u64, Vote)>> {
        let records = self.query_by_prefix("vt:id:", limit)?;
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::Vote(v) = r.data {
                Some((r.id, v))
            } else {
                None
            }
        }).collect())
    }

    // ========================================================================
    // FILE CLAIM OPERATIONS
    // ========================================================================

    /// Claim a file
    pub fn claim_file(&mut self, claimer: &str, path: &str, working_on: &str, duration_mins: u32) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;

        let now = now_millis();
        let record = Record {
            id,
            record_type: RecordType::FileClaim,
            created_at: now,
            data: RecordData::FileClaim(FileClaim {
                claimer: claimer.to_string(),
                path: path.to_string(),
                working_on: working_on.to_string(),
                expires_at: now + (duration_mins as u64 * 60 * 1000),
            }),
        };

        let value = bincode::serialize(&record)?;
        self.insert_record(&record)?;

        // Index by path for fast lookup
        let path_key = format!("fc:path:{}", path);
        let mut tree = BTree::new(&mut self.allocator);
        tree.insert(path_key.as_bytes(), &value)?;

        Ok(id)
    }

    /// Check if file is claimed
    pub fn check_file_claim(&mut self, path: &str) -> Result<Option<FileClaim>> {
        let key = format!("fc:path:{}", path);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let record: Record = bincode::deserialize(&value)?;
            if let RecordData::FileClaim(claim) = record.data {
                // Check if expired
                if claim.expires_at > now_millis() {
                    return Ok(Some(claim));
                }
            }
        }
        Ok(None)
    }

    /// Release a file claim
    pub fn release_file(&mut self, path: &str) -> Result<bool> {
        // Find and delete the file claim by path
        // Key format: fc:path:{path}
        let key = format!("fc:path:{}", path);

        // Try to delete from B+Tree
        let mut tree = BTree::new(&mut self.allocator);
        match tree.delete(key.as_bytes()) {
            Ok(deleted) => Ok(deleted),
            Err(e) => {
                // If delete not implemented, fail loudly - no silent "let it expire"
                anyhow::bail!("Failed to release file claim for '{}': {} - delete must be implemented", path, e)
            }
        }
    }

    // ========================================================================
    // ROOM OPERATIONS
    // ========================================================================

    /// Create a room
    pub fn create_room(&mut self, creator: &str, name: &str, topic: &str) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;

        let record = Record {
            id,
            record_type: RecordType::Room,
            created_at: now_millis(),
            data: RecordData::Room(Room {
                name: name.to_string(),
                creator: creator.to_string(),
                participants: vec![creator.to_string()],
                topic: topic.to_string(),
                is_open: true,
            }),
        };

        self.insert_record(&record)?;
        Ok(id)
    }

    /// Join a room
    pub fn join_room(&mut self, room_id: u64, ai_id: &str) -> Result<bool> {
        let key = format!("rm:id:{:016x}", room_id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::Room(ref mut room) = record.data {
                if room.is_open && !room.participants.contains(&ai_id.to_string()) {
                    room.participants.push(ai_id.to_string());
                    self.insert_record(&record)?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Get room by ID
    pub fn get_room(&mut self, id: u64) -> Result<Option<Room>> {
        let key = format!("rm:id:{:016x}", id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let record: Record = bincode::deserialize(&value)?;
            if let RecordData::Room(r) = record.data {
                return Ok(Some(r));
            }
        }
        Ok(None)
    }

    /// List active rooms
    pub fn list_rooms(&mut self, limit: usize) -> Result<Vec<(u64, Room)>> {
        let records = self.query_by_prefix("rm:id:", limit)?;
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::Room(room) = r.data {
                if room.is_open {
                    Some((r.id, room))
                } else {
                    None
                }
            } else {
                None
            }
        }).collect())
    }

    // ========================================================================
    // TASK OPERATIONS
    // ========================================================================

    /// Queue a new task
    pub fn queue_task(&mut self, created_by: &str, description: &str, priority: TaskPriority, tags: &str) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;

        let record = Record {
            id,
            record_type: RecordType::Task,
            created_at: now_millis(),
            data: RecordData::Task(Task {
                created_by: created_by.to_string(),
                claimed_by: None,
                description: description.to_string(),
                tags: tags.to_string(),
                status: TaskStatus::Pending,
                priority,
                result: None,
                claimed_at: None,
                completed_at: None,
            }),
        };

        let value = bincode::serialize(&record)?;

        self.insert_record(&record)?;

        // Index by status for fast queries
        let status_key = format!("tk:status:pending:{:016x}", id);
        let mut tree = BTree::new(&mut self.allocator);
        tree.insert(status_key.as_bytes(), &value)?;

        // Fire IPC notification
        self.notify.notify(
            NotifyType::Task,
            created_by,
            "", // Task broadcast to all
            Self::content_preview(description),
        );

        Ok(id)
    }

    /// Claim a task
    pub fn claim_task(&mut self, task_id: u64, ai_id: &str) -> Result<bool> {
        let key = format!("tk:id:{:016x}", task_id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::Task(ref mut task) = record.data {
                if task.status != TaskStatus::Pending {
                    return Ok(false); // Already claimed or completed
                }
                task.status = TaskStatus::Claimed;
                task.claimed_by = Some(ai_id.to_string());
                task.claimed_at = Some(now_millis());

                // Update record
                let new_value = bincode::serialize(&record)?;
                let mut tree = BTree::new(&mut self.allocator);
                tree.insert(key.as_bytes(), &new_value)?;

                return Ok(true);
            }
        }
        Ok(false)
    }


    /// Start working on a claimed task (Claimed -> InProgress)
    pub fn start_task(&mut self, task_id: u64, ai_id: &str) -> Result<bool> {
        let key = format!("tk:id:{:016x}", task_id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::Task(ref mut task) = record.data {
                if task.claimed_by.as_deref() != Some(ai_id) {
                    return Ok(false);
                }
                if task.status != TaskStatus::Claimed {
                    return Ok(false);
                }
                task.status = TaskStatus::InProgress;
                let new_value = bincode::serialize(&record)?;
                let mut tree = BTree::new(&mut self.allocator);
                tree.insert(key.as_bytes(), &new_value)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Complete a task
    pub fn complete_task(&mut self, task_id: u64, ai_id: &str, result: &str) -> Result<bool> {
        let key = format!("tk:id:{:016x}", task_id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::Task(ref mut task) = record.data {
                // Can only complete if claimed by this AI
                if task.claimed_by.as_deref() != Some(ai_id) {
                    return Ok(false);
                }
                task.status = TaskStatus::Completed;
                task.result = Some(result.to_string());
                task.completed_at = Some(now_millis());

                // Update record
                let new_value = bincode::serialize(&record)?;
                let mut tree = BTree::new(&mut self.allocator);
                tree.insert(key.as_bytes(), &new_value)?;

                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Get a task by ID
    pub fn get_task(&mut self, id: u64) -> Result<Option<Task>> {
        let key = format!("tk:id:{:016x}", id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let record: Record = bincode::deserialize(&value)?;
            if let RecordData::Task(task) = record.data {
                return Ok(Some(task));
            }
        }
        Ok(None)
    }

    /// List pending tasks
    pub fn list_pending_tasks(&mut self, limit: usize) -> Result<Vec<(u64, Task)>> {
        let records = self.query_by_prefix("tk:status:pending:", limit)?;
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::Task(task) = r.data {
                if task.status == TaskStatus::Pending {
                    Some((r.id, task))
                } else {
                    None
                }
            } else {
                None
            }
        }).collect())
    }

    /// List all tasks
    pub fn list_tasks(&mut self, limit: usize) -> Result<Vec<(u64, Task)>> {
        let records = self.query_by_prefix("tk:id:", limit)?;
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::Task(task) = r.data {
                Some((r.id, task))
            } else {
                None
            }
        }).collect())
    }

    // ========================================================================
    // LOCK OPERATIONS (Resource coordination)
    // ========================================================================

    /// Acquire a lock on a resource
    pub fn acquire_lock(&mut self, holder: &str, resource: &str, working_on: &str, duration_mins: u32) -> Result<Option<u64>> {
        // Check if resource is already locked
        let resource_key = format!("lk:res:{}", resource);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(resource_key.as_bytes())? {
            let record: Record = bincode::deserialize(&value)?;
            if let RecordData::Lock(lock) = record.data {
                // Check if lock expired
                if lock.expires_at > now_millis() {
                    return Ok(None); // Resource already locked
                }
                // Lock expired, we can take it
            }
        }

        let id = self.next_id;
        self.next_id += 1;

        let now = now_millis();
        let record = Record {
            id,
            record_type: RecordType::Lock,
            created_at: now,
            data: RecordData::Lock(Lock {
                holder: holder.to_string(),
                resource: resource.to_string(),
                working_on: working_on.to_string(),
                acquired_at: now,
                expires_at: now + (duration_mins as u64 * 60 * 1000),
            }),
        };

        let value = bincode::serialize(&record)?;

        self.insert_record(&record)?;

        // Index by resource for fast lookup
        let mut tree = BTree::new(&mut self.allocator);
        tree.insert(resource_key.as_bytes(), &value)?;

        // Index by holder
        let holder_key = format!("lk:holder:{}:{:016x}", holder, id);
        let mut tree = BTree::new(&mut self.allocator);
        tree.insert(holder_key.as_bytes(), &value)?;

        Ok(Some(id))
    }

    /// Release a lock
    pub fn release_lock(&mut self, resource: &str, holder: &str) -> Result<bool> {
        let resource_key = format!("lk:res:{}", resource);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(resource_key.as_bytes())? {
            let record: Record = bincode::deserialize(&value)?;
            if let RecordData::Lock(ref lock) = record.data {
                // Only holder can release
                if lock.holder != holder {
                    return Ok(false);
                }
                // Remove the lock by setting it to expired
                let mut new_record = record.clone();
                if let RecordData::Lock(ref mut l) = new_record.data {
                    l.expires_at = 0; // Mark as expired
                }
                let new_value = bincode::serialize(&new_record)?;
                let mut tree = BTree::new(&mut self.allocator);
                tree.insert(resource_key.as_bytes(), &new_value)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Check if a resource is locked
    pub fn check_lock(&mut self, resource: &str) -> Result<Option<Lock>> {
        let resource_key = format!("lk:res:{}", resource);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(resource_key.as_bytes())? {
            let record: Record = bincode::deserialize(&value)?;
            if let RecordData::Lock(lock) = record.data {
                // Check if still valid
                if lock.expires_at > now_millis() {
                    return Ok(Some(lock));
                }
            }
        }
        Ok(None)
    }

    /// List locks held by an AI
    pub fn list_locks_by_holder(&mut self, holder: &str, limit: usize) -> Result<Vec<(u64, Lock)>> {
        let prefix = format!("lk:holder:{}:", holder);
        let records = self.query_by_prefix(&prefix, limit)?;
        let now = now_millis();
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::Lock(lock) = r.data {
                if lock.expires_at > now {
                    Some((r.id, lock))
                } else {
                    None
                }
            } else {
                None
            }
        }).collect())
    }

    /// Insert a record with auto-generated key
    fn insert_record(&mut self, record: &Record) -> Result<()> {
        let key = format!("{}{}:{:016x}",
            record.record_type.prefix().iter().map(|&b| b as char).collect::<String>(),
            "id",
            record.id
        );

        let value = bincode::serialize(record)?;
        let mut tree = BTree::new(&mut self.allocator);
        tree.insert(key.as_bytes(), &value)?;

        // Persist next_id after each insert
        self.persist_next_id()?;

        Ok(())
    }
