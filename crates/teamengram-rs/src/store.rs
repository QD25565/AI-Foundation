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

// Presence uses OS-level mutex detection - see wake::is_ai_online()

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

/// Result of attempting to join a room
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinRoomResult {
    Joined,
    NotFound,
    Closed,
    AlreadyMember,
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

    /// Load next_id from stored value, with fallback scan for max ID
    fn load_next_id(allocator: &mut ShadowAllocator) -> Result<u64> {
        let tree = BTree::new(allocator);

        // Try to load from meta key
        let persisted_id = if let Some(value) = tree.get(b"meta:next_id")? {
            if value.len() >= 8 {
                u64::from_le_bytes(value[..8].try_into().unwrap())
            } else {
                0
            }
        } else {
            0
        };

        // Scan for actual max ID to prevent duplicates after unclean shutdown
        let mut max_found: u64 = 0;
        let prefixes: [&[u8]; 14] = [b"dm:id:", b"bc:id:", b"pr:id:", b"dg:id:", b"tk:id:", b"vt:id:", b"fc:id:", b"rm:id:", b"lk:id:", b"fa:id:", b"da:id:", b"pj:id:", b"ft:id:", b"vl:id:"];
        
        let mut iter = tree.iter()?;
        while let Some((key, _value)) = iter.next()? {
            // Check if key matches any of our ID prefixes
            for prefix in &prefixes {
                if key.starts_with(*prefix) && key.len() >= prefix.len() + 16 {
                    // Key format is "prefix{:016x}" - extract ID from hex
                    if let Ok(hex_str) = std::str::from_utf8(&key[prefix.len()..prefix.len()+16]) {
                        if let Ok(id) = u64::from_str_radix(hex_str, 16) {
                            max_found = max_found.max(id);
                        }
                    }
                    break;
                }
            }
        }

        // Use the higher of persisted_id or max_found + 1
        let next_id = persisted_id.max(max_found.saturating_add(1));
        Ok(next_id)
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

    /// Get per-AI store path - PREFERRED for multi-AI setups
    /// Each AI gets isolated storage: teamengram_{ai_id}.engram
    /// This eliminates cross-AI concurrency issues entirely
    pub fn path_for_ai(ai_id: &str) -> PathBuf {
        let base = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let safe_id = ai_id.chars().map(|c| if c == '/' || c == '\\' || c == ':' { '_' } else { c }).collect::<String>();
        let filename = format!("teamengram_{}.engram", safe_id);
        base.join(".ai-foundation").join(filename)
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

        // Create all keys for the DM
        let dm_id_key = format!("dm:id:{:016x}", id);
        let recipient_key = format!("dm:to:{}:{:016x}", to, id);

        // CRITICAL: Use batch_insert to insert both keys in a single transaction
        let entries: Vec<(&[u8], &[u8])> = vec![
            (dm_id_key.as_bytes(), &value),
            (recipient_key.as_bytes(), &value),
        ];
        let mut tree = BTree::new(&mut self.allocator);
        tree.batch_insert(&entries)?;

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


    /// Get UNREAD DMs for a recipient (only incoming, not sent by this AI)
    pub fn get_unread_dms(&mut self, to_ai: &str, limit: usize) -> Result<Vec<Record>> {
        let prefix = format!("dm:to:{}:", to_ai);
        let all_dms = self.query_by_prefix(&prefix, limit * 2)?; // Fetch extra to filter

        Ok(all_dms.into_iter()
            .filter(|r| {
                if let RecordData::DirectMessage(dm) = &r.data {
                    // Only unread AND not sent by this AI (incoming only)
                    !dm.read && dm.from_ai != to_ai
                } else {
                    false
                }
            })
            .take(limit)
            .collect())
    }

    /// Mark a DM as read
    pub fn mark_dm_read(&mut self, id: u64) -> Result<bool> {
        let key = format!("dm:id:{:016x}", id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::DirectMessage(ref mut dm) = record.data {
                if dm.read {
                    return Ok(false); // Already read
                }
                dm.read = true;

                // Extract to_ai before serializing to avoid borrow conflict
                let to_ai = dm.to_ai.clone();

                // Update main record
                let new_value = bincode::serialize(&record)?;
                let mut tree = BTree::new(&mut self.allocator);
                tree.insert(key.as_bytes(), &new_value)?;

                // Also update the recipient index
                let recipient_key = format!("dm:to:{}:{:016x}", to_ai, id);
                tree.insert(recipient_key.as_bytes(), &new_value)?;

                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Mark multiple DMs as read (batch operation)
    pub fn mark_dms_read(&mut self, ids: &[u64]) -> Result<usize> {
        let mut count = 0;
        for &id in ids {
            if self.mark_dm_read(id)? {
                count += 1;
            }
        }
        Ok(count)
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

        // Create all keys for the broadcast
        let bc_id_key = format!("bc:id:{:016x}", id);
        let channel_key = format!("bc:ch:{}:{:016x}", channel, id);

        // CRITICAL: Use batch_insert to insert both keys in a single transaction
        let entries: Vec<(&[u8], &[u8])> = vec![
            (bc_id_key.as_bytes(), &value),
            (channel_key.as_bytes(), &value),
        ];
        let mut tree = BTree::new(&mut self.allocator);
        tree.batch_insert(&entries)?;

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

    /// Get all stored presences (no TTL filtering - caller should use wake::is_ai_online())
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

    /// Get deduplicated presences (one per AI, most recent)
    pub fn get_unique_presences(&mut self) -> Result<Vec<Presence>> {
        let all = self.get_all_presences()?;
        let mut map: std::collections::HashMap<String, Presence> = std::collections::HashMap::new();

        for p in all {
            map.entry(p.ai_id.clone())
                .and_modify(|existing| {
                    if p.last_seen > existing.last_seen {
                        *existing = p.clone();
                    }
                })
                .or_insert(p);
        }

        Ok(map.into_values().collect())
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

        // Create all keys for the dialogue
        let dg_id_key = format!("dg:id:{:016x}", id);
        let init_key = format!("dg:ai:{}:{:016x}", initiator, id);
        let resp_key = format!("dg:ai:{}:{:016x}", responder, id);

        // CRITICAL: Use batch_insert to insert all 3 keys in a single transaction
        // This fixes the bug where separate transactions caused 2nd/3rd keys to be lost
        let entries: Vec<(&[u8], &[u8])> = vec![
            (dg_id_key.as_bytes(), &value),
            (init_key.as_bytes(), &value),
            (resp_key.as_bytes(), &value),
        ];
        let mut tree = BTree::new(&mut self.allocator);
        tree.batch_insert(&entries)?;

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

    /// Force delete a dialogue by ID (for cleanup of corrupted dialogues)
    /// This deletes dg:ai: index keys even when dg:id: key is corrupted
    pub fn delete_dialogue_force(&mut self, id: u64) -> Result<u32> {
        let hex_id = format!("{:016x}", id);
        let mut deleted_count = 0u32;

        // First, try to find the dialogue via prefix scan to get initiator/responder
        let all_dialogues = self.query_by_prefix("dg:ai:", 1000)?;
        for record in all_dialogues {
            if record.id == id {
                if let RecordData::Dialogue(d) = &record.data {
                    // Delete initiator index key
                    let init_key = format!("dg:ai:{}:{}", d.initiator, hex_id);
                    let mut tree = BTree::new(&mut self.allocator);
                    if tree.delete(init_key.as_bytes()).unwrap_or(false) {
                        deleted_count += 1;
                    }

                    // Delete responder index key
                    let resp_key = format!("dg:ai:{}:{}", d.responder, hex_id);
                    let mut tree = BTree::new(&mut self.allocator);
                    if tree.delete(resp_key.as_bytes()).unwrap_or(false) {
                        deleted_count += 1;
                    }
                }
                break;
            }
        }

        // Try to delete the main dg:id: key (might fail if corrupted)
        let id_key = format!("dg:id:{}", hex_id);
        let mut tree = BTree::new(&mut self.allocator);
        if tree.delete(id_key.as_bytes()).unwrap_or(false) {
            deleted_count += 1;
        }

        Ok(deleted_count)
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

        // Create all keys for the file claim
        let fc_id_key = format!("fc:id:{:016x}", id);
        let path_key = format!("fc:path:{}", path);

        // CRITICAL: Use batch_insert to insert both keys in a single transaction
        let entries: Vec<(&[u8], &[u8])> = vec![
            (fc_id_key.as_bytes(), &value),
            (path_key.as_bytes(), &value),
        ];
        let mut tree = BTree::new(&mut self.allocator);
        tree.batch_insert(&entries)?;

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
    pub fn join_room(&mut self, room_id: u64, ai_id: &str) -> Result<JoinRoomResult> {
        let key = format!("rm:id:{:016x}", room_id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::Room(ref mut room) = record.data {
                // Check if already a member
                if room.participants.contains(&ai_id.to_string()) {
                    return Ok(JoinRoomResult::AlreadyMember);
                }
                // Check if room is open
                if !room.is_open {
                    return Ok(JoinRoomResult::Closed);
                }
                // Join the room
                room.participants.push(ai_id.to_string());
                self.insert_record(&record)?;
                return Ok(JoinRoomResult::Joined);
            }
        }
        Ok(JoinRoomResult::NotFound)
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

        // Create all keys for the task
        let tk_id_key = format!("tk:id:{:016x}", id);
        let status_key = format!("tk:status:pending:{:016x}", id);

        // CRITICAL: Use batch_insert to insert both keys in a single transaction
        // This fixes the bug where separate transactions caused keys to be lost
        let entries: Vec<(&[u8], &[u8])> = vec![
            (tk_id_key.as_bytes(), &value),
            (status_key.as_bytes(), &value),
        ];
        let mut tree = BTree::new(&mut self.allocator);
        tree.batch_insert(&entries)?;

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

        // Create all keys for the lock
        let lk_id_key = format!("lk:id:{:016x}", id);
        let holder_key = format!("lk:holder:{}:{:016x}", holder, id);

        // CRITICAL: Use batch_insert to insert all 3 keys in a single transaction
        let entries: Vec<(&[u8], &[u8])> = vec![
            (lk_id_key.as_bytes(), &value),
            (resource_key.as_bytes(), &value),
            (holder_key.as_bytes(), &value),
        ];
        let mut tree = BTree::new(&mut self.allocator);
        tree.batch_insert(&entries)?;

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

                // Update resource index
                let mut tree = BTree::new(&mut self.allocator);
                tree.insert(resource_key.as_bytes(), &new_value)?;

                // Also update the ID index (for list_all_locks bulletin awareness)
                let id_key = format!("lk:id:{:016x}", record.id);
                let mut tree = BTree::new(&mut self.allocator);
                tree.insert(id_key.as_bytes(), &new_value)?;

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

    /// List all active locks (for BulletinBoard awareness)
    pub fn list_all_locks(&mut self, limit: usize) -> Result<Vec<(u64, Lock)>> {
        let records = self.query_by_prefix("lk:id:", limit)?;
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

        // NOTE: persist_next_id() REMOVED - ROOT CAUSE of B+tree corruption!
        // See flush() method instead - interleaved txns caused Invalid page ID errors

        Ok(())
    }

    /// Persist next_id to disk. Call periodically or before shutdown.
    pub fn flush(&mut self) -> Result<()> {
        self.persist_next_id()
    }

    /// Query records by key prefix
    fn query_by_prefix(&mut self, prefix: &str, limit: usize) -> Result<Vec<Record>> {
        let tree = BTree::new(&mut self.allocator);
        let mut results = Vec::new();

        // For now, scan all and filter
        // TODO: Implement proper prefix iteration
        let mut iter = tree.iter()?;
        while let Some((key, value)) = iter.next()? {
            if key.starts_with(prefix.as_bytes()) {
                if let Ok(record) = bincode::deserialize::<Record>(&value) {
                    results.push(record);
                    if results.len() >= limit {
                        break;
                    }
                }
            }
        }

        // Sort by created_at descending (newest first)
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        results.truncate(limit);

        Ok(results)
    }

    /// Get store statistics
    pub fn stats(&self) -> StoreStats {
        let alloc_stats = self.allocator.stats();
        StoreStats {
            file_size: alloc_stats.file_size,
            total_pages: alloc_stats.total_pages,
            used_pages: alloc_stats.used_pages,
            txn_id: alloc_stats.txn_id,
            next_id: self.next_id,
        }
    }

    // ========================================================================
    // ADDITIONAL METHODS FOR 100% PARITY
    // ========================================================================

    /// Get dialogue invites (dialogues where ai_id is responder and it's their turn)
    pub fn get_dialogue_invites(&mut self, ai_id: &str, limit: usize) -> Result<Vec<(u64, Dialogue)>> {
        let records = self.query_by_prefix("dg:id:", limit * 2)?;
        let now = now_millis();
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::Dialogue(d) = r.data {
                // Invite = responder hasn't responded yet (turn 0, status active)
                if d.responder == ai_id && d.turn == 0 && d.status == DialogueStatus::Active {
                    // Check not expired (24 hour default)
                    if r.created_at + 86400000 > now {
                        return Some((r.id, d));
                    }
                }
            }
            None
        }).take(limit).collect())
    }

    /// Get dialogues where it's this AI's turn
    pub fn get_my_turn_dialogues(&mut self, ai_id: &str, limit: usize) -> Result<Vec<(u64, Dialogue)>> {
        let records = self.query_by_prefix("dg:id:", limit * 2)?;
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::Dialogue(d) = r.data {
                if d.status == DialogueStatus::Active {
                    // Even turns = initiator's turn, odd turns = responder's turn
                    let is_my_turn = (d.turn % 2 == 0 && d.initiator == ai_id) ||
                                     (d.turn % 2 == 1 && d.responder == ai_id);
                    if is_my_turn {
                        return Some((r.id, d));
                    }
                }
            }
            None
        }).take(limit).collect())
    }

    /// Close a vote (mark as closed)
    pub fn close_vote(&mut self, vote_id: u64, ai_id: &str) -> Result<bool> {
        let key = format!("vt:id:{:016x}", vote_id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::Vote(ref mut vote) = record.data {
                // Only creator can close
                if vote.created_by != ai_id {
                    return Ok(false);
                }
                vote.status = VoteStatus::Closed;
                let new_value = bincode::serialize(&record)?;
                let mut tree = BTree::new(&mut self.allocator);
                tree.insert(key.as_bytes(), &new_value)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// List all file claims
    pub fn list_file_claims(&mut self, limit: usize) -> Result<Vec<(u64, FileClaim)>> {
        let records = self.query_by_prefix("fc:id:", limit)?;
        let now = now_millis();
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::FileClaim(claim) = r.data {
                if claim.expires_at > now {
                    Some((r.id, claim))
                } else {
                    None
                }
            } else {
                None
            }
        }).collect())
    }

    /// Leave a room
    pub fn leave_room(&mut self, room_id: u64, ai_id: &str) -> Result<bool> {
        let key = format!("rm:id:{:016x}", room_id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::Room(ref mut room) = record.data {
                if let Some(pos) = room.participants.iter().position(|p| p == ai_id) {
                    room.participants.remove(pos);
                    let new_value = bincode::serialize(&record)?;
                    let mut tree = BTree::new(&mut self.allocator);
                    tree.insert(key.as_bytes(), &new_value)?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Close a room (creator only)
    pub fn close_room(&mut self, room_id: u64, ai_id: &str) -> Result<bool> {
        let key = format!("rm:id:{:016x}", room_id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::Room(ref mut room) = record.data {
                if room.creator != ai_id {
                    return Ok(false);
                }
                room.is_open = false;
                let new_value = bincode::serialize(&record)?;
                let mut tree = BTree::new(&mut self.allocator);
                tree.insert(key.as_bytes(), &new_value)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Get task queue statistics
    pub fn task_stats(&mut self) -> Result<TaskStats> {
        let records = self.query_by_prefix("tk:id:", 1000)?;
        let mut stats = TaskStats::default();

        for r in records {
            if let RecordData::Task(task) = r.data {
                match task.status {
                    TaskStatus::Pending => stats.pending += 1,
                    TaskStatus::Claimed => stats.claimed += 1,
                    TaskStatus::InProgress => stats.in_progress += 1,
                    TaskStatus::Completed => stats.completed += 1,
                    TaskStatus::Failed => stats.failed += 1,
                    TaskStatus::Cancelled => stats.cancelled += 1,
                }
                stats.total += 1;
            }
        }
        Ok(stats)
    }

    // ========================================================================
    // FILE ACTION OPERATIONS (Awareness tracking)
    // ========================================================================

    /// Log a file action (created, modified, deleted, reviewed)
    pub fn log_file_action(&mut self, ai_id: &str, path: &str, action: &str) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;

        let now = now_millis();
        let record = Record {
            id,
            record_type: RecordType::FileAction,
            created_at: now,
            data: RecordData::FileAction(FileAction {
                ai_id: ai_id.to_string(),
                path: path.to_string(),
                action: action.to_string(),
                timestamp: now,
            }),
        };

        let value = bincode::serialize(&record)?;

        // Create all keys for the file action
        let fa_id_key = format!("fa:id:{:016x}", id);
        let ai_key = format!("fa:ai:{}:{:016x}", ai_id, id);

        // CRITICAL: Use batch_insert to insert both keys in a single transaction
        let entries: Vec<(&[u8], &[u8])> = vec![
            (fa_id_key.as_bytes(), &value),
            (ai_key.as_bytes(), &value),
        ];
        let mut tree = BTree::new(&mut self.allocator);
        tree.batch_insert(&entries)?;

        Ok(id)
    }

    /// Get recent file actions for an AI
    pub fn get_file_actions(&mut self, ai_id: &str, limit: usize) -> Result<Vec<FileAction>> {
        let prefix = format!("fa:ai:{}:", ai_id);
        let records = self.query_by_prefix(&prefix, limit)?;
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::FileAction(fa) = r.data {
                Some(fa)
            } else {
                None
            }
        }).collect())
    }

    /// Get all recent file actions (team-wide)
    pub fn get_recent_file_actions(&mut self, limit: usize) -> Result<Vec<(u64, FileAction)>> {
        let records = self.query_by_prefix("fa:id:", limit)?;
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::FileAction(fa) = r.data {
                Some((r.id, fa))
            } else {
                None
            }
        }).collect())
    }

    // ========================================================================
    // DIRECTORY ACCESS OPERATIONS (Awareness tracking)
    // ========================================================================

    /// Track directory access
    pub fn track_directory(&mut self, ai_id: &str, directory: &str, access_type: &str) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;

        let now = now_millis();
        let record = Record {
            id,
            record_type: RecordType::DirectoryAccess,
            created_at: now,
            data: RecordData::DirectoryAccess(DirectoryAccess {
                ai_id: ai_id.to_string(),
                directory: directory.to_string(),
                access_type: access_type.to_string(),
                timestamp: now,
            }),
        };

        let value = bincode::serialize(&record)?;

        // Create all keys for the directory access
        let da_id_key = format!("da:id:{:016x}", id);
        let ai_key = format!("da:ai:{}:{:016x}", ai_id, id);

        // CRITICAL: Use batch_insert to insert both keys in a single transaction
        let entries: Vec<(&[u8], &[u8])> = vec![
            (da_id_key.as_bytes(), &value),
            (ai_key.as_bytes(), &value),
        ];
        let mut tree = BTree::new(&mut self.allocator);
        tree.batch_insert(&entries)?;

        Ok(id)
    }

    /// Get recent directories accessed by an AI
    pub fn get_recent_directories(&mut self, ai_id: &str, limit: usize) -> Result<Vec<DirectoryAccess>> {
        let prefix = format!("da:ai:{}:", ai_id);
        let records = self.query_by_prefix(&prefix, limit)?;
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::DirectoryAccess(da) = r.data {
                Some(da)
            } else {
                None
            }
        }).collect())
    }

    // ========================================================================
    // PROJECT OPERATIONS (Team coordination)
    // ========================================================================

    /// Create a new project
    pub fn create_project(&mut self, name: &str, goal: &str, root_directory: &str, created_by: &str) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;

        let now = now_millis();
        let record = Record {
            id,
            record_type: RecordType::Project,
            created_at: now,
            data: RecordData::Project(Project {
                name: name.to_string(),
                goal: goal.to_string(),
                root_directory: root_directory.to_string(),
                created_by: created_by.to_string(),
                status: "active".to_string(),
                created_at: now,
                updated_at: now,
            }),
        };

        self.insert_record(&record)?;
        Ok(id)
    }

    /// Get a project by ID
    pub fn get_project(&mut self, id: u64) -> Result<Option<Project>> {
        let key = format!("pj:id:{:016x}", id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let record: Record = bincode::deserialize(&value)?;
            if let RecordData::Project(project) = record.data {
                return Ok(Some(project));
            }
        }
        Ok(None)
    }

    /// List all active projects
    pub fn list_projects(&mut self, limit: usize) -> Result<Vec<(u64, Project)>> {
        let records = self.query_by_prefix("pj:id:", limit)?;
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::Project(project) = r.data {
                if project.status != "deleted" {
                    Some((r.id, project))
                } else {
                    None
                }
            } else {
                None
            }
        }).collect())
    }

    /// Update a project
    pub fn update_project(&mut self, id: u64, goal: Option<&str>, status: Option<&str>) -> Result<bool> {
        let key = format!("pj:id:{:016x}", id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::Project(ref mut project) = record.data {
                let mut changed = false;
                if let Some(g) = goal {
                    project.goal = g.to_string();
                    changed = true;
                }
                if let Some(s) = status {
                    project.status = s.to_string();
                    changed = true;
                }
                if changed {
                    project.updated_at = now_millis();
                    let new_value = bincode::serialize(&record)?;
                    let mut tree = BTree::new(&mut self.allocator);
                    tree.insert(key.as_bytes(), &new_value)?;
                    self.notify.notify(NotifyType::Project, "", "", "");
                }
                return Ok(changed);
            }
        }
        Ok(false)
    }

    /// Soft delete a project (set status to "deleted")
    pub fn soft_delete_project(&mut self, id: u64) -> Result<bool> {
        self.update_project(id, None, Some("deleted"))
    }

    /// Restore a project (set status back to "active")
    pub fn restore_project(&mut self, id: u64) -> Result<bool> {
        self.update_project(id, None, Some("active"))
    }

    // ========================================================================
    // FEATURE OPERATIONS (Project components)
    // ========================================================================

    /// Create a feature within a project
    pub fn create_feature(&mut self, project_id: u64, name: &str, overview: &str, directory: Option<&str>, created_by: &str) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;

        let now = now_millis();
        let record = Record {
            id,
            record_type: RecordType::Feature,
            created_at: now,
            data: RecordData::Feature(Feature {
                project_id,
                name: name.to_string(),
                overview: overview.to_string(),
                directory: directory.map(|s| s.to_string()),
                created_by: created_by.to_string(),
                status: "active".to_string(),
                created_at: now,
                updated_at: now,
            }),
        };

        let value = bincode::serialize(&record)?;

        // Create all keys for the feature
        let ft_id_key = format!("ft:id:{:016x}", id);
        let proj_key = format!("ft:proj:{}:{:016x}", project_id, id);

        // CRITICAL: Use batch_insert to insert both keys in a single transaction
        let entries: Vec<(&[u8], &[u8])> = vec![
            (ft_id_key.as_bytes(), &value),
            (proj_key.as_bytes(), &value),
        ];
        let mut tree = BTree::new(&mut self.allocator);
        tree.batch_insert(&entries)?;

        Ok(id)
    }

    /// Get a feature by ID
    pub fn get_feature(&mut self, id: u64) -> Result<Option<Feature>> {
        let key = format!("ft:id:{:016x}", id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let record: Record = bincode::deserialize(&value)?;
            if let RecordData::Feature(feature) = record.data {
                return Ok(Some(feature));
            }
        }
        Ok(None)
    }

    /// List features in a project
    pub fn list_features(&mut self, project_id: u64, limit: usize) -> Result<Vec<(u64, Feature)>> {
        let prefix = format!("ft:proj:{}:", project_id);
        let records = self.query_by_prefix(&prefix, limit)?;
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::Feature(feature) = r.data {
                if feature.status != "deleted" {
                    Some((r.id, feature))
                } else {
                    None
                }
            } else {
                None
            }
        }).collect())
    }

    /// Update a feature
    pub fn update_feature(&mut self, id: u64, name: Option<&str>, overview: Option<&str>, directory: Option<&str>) -> Result<bool> {
        let key = format!("ft:id:{:016x}", id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::Feature(ref mut feature) = record.data {
                let mut changed = false;
                if let Some(n) = name {
                    feature.name = n.to_string();
                    changed = true;
                }
                if let Some(o) = overview {
                    feature.overview = o.to_string();
                    changed = true;
                }
                if let Some(d) = directory {
                    feature.directory = Some(d.to_string());
                    changed = true;
                }
                if changed {
                    feature.updated_at = now_millis();
                    let new_value = bincode::serialize(&record)?;
                    let mut tree = BTree::new(&mut self.allocator);
                    tree.insert(key.as_bytes(), &new_value)?;
                    self.notify.notify(NotifyType::Feature, "", "", "");
                }
                return Ok(changed);
            }
        }
        Ok(false)
    }

    /// Soft delete a feature
    pub fn soft_delete_feature(&mut self, id: u64) -> Result<bool> {
        let key = format!("ft:id:{:016x}", id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::Feature(ref mut feature) = record.data {
                feature.status = "deleted".to_string();
                feature.updated_at = now_millis();
                let new_value = bincode::serialize(&record)?;
                let mut tree = BTree::new(&mut self.allocator);
                tree.insert(key.as_bytes(), &new_value)?;
                self.notify.notify(NotifyType::Feature, "", "", "");
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Restore a feature
    pub fn restore_feature(&mut self, id: u64) -> Result<bool> {
        let key = format!("ft:id:{:016x}", id);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(key.as_bytes())? {
            let mut record: Record = bincode::deserialize(&value)?;
            if let RecordData::Feature(ref mut feature) = record.data {
                feature.status = "active".to_string();
                feature.updated_at = now_millis();
                let new_value = bincode::serialize(&record)?;
                let mut tree = BTree::new(&mut self.allocator);
                tree.insert(key.as_bytes(), &new_value)?;
                self.notify.notify(NotifyType::Feature, "", "", "");
                return Ok(true);
            }
        }
        Ok(false)
    }

    // ========================================================================
    // VAULT OPERATIONS (Shared key-value storage)
    // ========================================================================

    /// Store a value in the shared vault
    pub fn vault_store(&mut self, key: &str, value: &str, updated_by: &str) -> Result<()> {
        let vault_key = format!("vl:key:{}", key);
        let now = now_millis();

        let record = Record {
            id: 0, // Vault uses key as identifier
            record_type: RecordType::VaultEntry,
            created_at: now,
            data: RecordData::VaultEntry(VaultEntry {
                key: key.to_string(),
                value: value.to_string(),
                updated_by: updated_by.to_string(),
                updated_at: now,
            }),
        };

        let serialized = bincode::serialize(&record)?;
        let mut tree = BTree::new(&mut self.allocator);
        tree.insert(vault_key.as_bytes(), &serialized)?;
        self.notify.notify(NotifyType::Vault, "", "", "");
        Ok(())
    }

    /// Get a value from the shared vault
    pub fn vault_get(&mut self, key: &str) -> Result<Option<String>> {
        let vault_key = format!("vl:key:{}", key);
        let tree = BTree::new(&mut self.allocator);

        if let Some(value) = tree.get(vault_key.as_bytes())? {
            let record: Record = bincode::deserialize(&value)?;
            if let RecordData::VaultEntry(entry) = record.data {
                return Ok(Some(entry.value));
            }
        }
        Ok(None)
    }

    /// List all vault keys
    pub fn vault_list(&mut self) -> Result<Vec<String>> {
        let records = self.query_by_prefix("vl:key:", 1000)?;
        Ok(records.into_iter().filter_map(|r| {
            if let RecordData::VaultEntry(entry) = r.data {
                Some(entry.key)
            } else {
                None
            }
        }).collect())
    }
}

/// Task queue statistics
#[derive(Debug, Clone, Default)]
pub struct TaskStats {
    pub total: usize,
    pub pending: usize,
    pub claimed: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
}

/// Store statistics
#[derive(Debug, Clone)]
pub struct StoreStats {
    pub file_size: u64,
    pub total_pages: u64,
    pub used_pages: u64,
    pub txn_id: u64,
    pub next_id: u64,
}

/// Get current time in milliseconds
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_dm_insert_and_get() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.teamengram");

        let mut store = TeamEngram::open(&path).unwrap();

        let id = store.insert_dm("ai-1", "ai-2", "Hello Sage!").unwrap();
        assert_eq!(id, 1);

        let dms = store.get_dms("ai-2", 10).unwrap();
        assert_eq!(dms.len(), 1);

        if let RecordData::DirectMessage(dm) = &dms[0].data {
            assert_eq!(dm.from_ai, "ai-1");
            assert_eq!(dm.to_ai, "ai-2");
            assert_eq!(dm.content, "Hello Sage!");
        } else {
            panic!("Expected DirectMessage");
        }
    }

    #[test]
    fn test_broadcast() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.teamengram");

        let mut store = TeamEngram::open(&path).unwrap();

        store.insert_broadcast("ai-3", "general", "Team update!").unwrap();

        let broadcasts = store.get_broadcasts("general", 10).unwrap();
        assert_eq!(broadcasts.len(), 1);
    }

    #[test]
    fn test_presence() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.teamengram");

        let mut store = TeamEngram::open(&path).unwrap();

        store.update_presence("ai-2", "active", "Working on TeamEngram").unwrap();

        let presence = store.get_presence("ai-2").unwrap().unwrap();
        assert_eq!(presence.ai_id, "ai-2");
        assert_eq!(presence.status, "active");
    }

    #[test]
    fn test_task_queue_claim_complete() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.teamengram");

        let mut store = TeamEngram::open(&path).unwrap();

        // Queue a task
        let task_id = store.queue_task("ai-2", "Review the code", TaskPriority::Normal, "review").unwrap();
        assert_eq!(task_id, 1);

        // Verify task exists by listing
        let tasks = store.list_tasks(10).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].0, task_id);

        // Claim the task
        let claimed = store.claim_task(task_id, "ai-1").unwrap();
        assert!(claimed, "Task should be claimable");

        // Complete the task
        let completed = store.complete_task(task_id, "ai-1", "Looks good!").unwrap();
        assert!(completed, "Task should be completable");

        // Verify final state
        let stats = store.task_stats().unwrap();
        assert_eq!(stats.total, 1);
        assert_eq!(stats.completed, 1);
    }
}
