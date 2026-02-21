//! V2 Client - High-level API for TeamEngram V2
//!
//! Provides a clean interface for CLI and MCP to interact with the V2
//! event sourcing architecture. Handles:
//! - Writing events to outbox
//! - Reading from event log
//! - Syncing local view
//!
//! Usage:
//! ```ignore
//! let client = V2Client::open("ai-1", None)?;
//! client.broadcast("general", "Hello team!")?;
//! let messages = client.recent_broadcasts(10)?;
//! ```

use std::path::{Path, PathBuf};

use crate::event::{Event, EventPayload, event_type};
use crate::outbox::{OutboxProducer, OutboxConsumer};
use crate::event_log::{EventLogReader, EventLogWriter};
use crate::view::ViewEngine;
use crate::compat_types::{Message, MessageType};
use crate::is_ai_online;

/// V2 Client error types
#[derive(Debug)]
pub enum V2Error {
    Outbox(String),
    EventLog(String),
    View(String),
    NotFound(String),
    InvalidState(String),
    InvalidStatus(String),
}

impl std::fmt::Display for V2Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            V2Error::Outbox(e) => write!(f, "Outbox error: {}", e),
            V2Error::EventLog(e) => write!(f, "Event log error: {}", e),
            V2Error::View(e) => write!(f, "View error: {}", e),
            V2Error::NotFound(e) => write!(f, "Not found: {}", e),
            V2Error::InvalidState(e) => write!(f, "Invalid state: {}", e),
            V2Error::InvalidStatus(e) => write!(f, "Invalid status: {}", e),
        }
    }
}

impl std::error::Error for V2Error {}

pub type V2Result<T> = Result<T, V2Error>;

/// High-level client for V2 event sourcing
pub struct V2Client {
    ai_id: String,
    base_dir: PathBuf,
    outbox: OutboxProducer,
    view: ViewEngine,
    // Event log reader for queries
    reader: EventLogReader,
}

impl V2Client {
    /// Open or create a V2 client for an AI
    pub fn open(ai_id: &str, base_dir: Option<&Path>) -> V2Result<Self> {
        let base = base_dir
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".ai-foundation")
                    .join("v2")
            });

        std::fs::create_dir_all(&base).map_err(|e| V2Error::EventLog(e.to_string()))?;

        let outbox = OutboxProducer::open(ai_id, Some(&base))
            .map_err(|e| V2Error::Outbox(e.to_string()))?;

        let mut view = ViewEngine::open(ai_id, &base)
            .map_err(|e| V2Error::View(e.to_string()))?;

        // Ensure event log exists by creating a writer first (which initializes the file)
        // Then drop it and open a reader
        {
            let _writer = EventLogWriter::open(Some(&base))
                .map_err(|e| V2Error::EventLog(e.to_string()))?;
        }

        let mut reader = EventLogReader::open(Some(&base))
            .map_err(|e| V2Error::EventLog(e.to_string()))?;

        // WARM CACHE on startup - populate content caches from event log
        // This enables O(1) queries instead of O(n) log scans
        view.warm_cache(&mut reader)
            .map_err(|e| V2Error::View(e.to_string()))?;

        Ok(Self {
            ai_id: ai_id.to_string(),
            base_dir: base,
            outbox,
            view,
            reader,
        })
    }

    /// Get the AI ID
    pub fn ai_id(&self) -> &str {
        &self.ai_id
    }

    /// Sync view with event log (call periodically or before queries)
    ///
    /// IMPORTANT: We must refresh the mmap first to see events written by other
    /// processes (e.g., the sequencer daemon). Without this, the mmap would only
    /// reflect the state when it was originally opened.
    pub fn sync(&mut self) -> V2Result<u64> {
        // Refresh mmap to see new events from sequencer
        self.reader.refresh()
            .map_err(|e| V2Error::EventLog(e.to_string()))?;

        self.view.sync(&mut self.reader)
            .map_err(|e| V2Error::View(e.to_string()))
    }

    // ========== MESSAGING ==========

    /// Send a broadcast message
    pub fn broadcast(&mut self, channel: &str, content: &str) -> V2Result<u64> {
        let event = Event::broadcast(&self.ai_id, channel, content);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Send a direct message
    pub fn direct_message(&mut self, to_ai: &str, content: &str) -> V2Result<u64> {
        let event = Event::direct_message(&self.ai_id, to_ai, content);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Get recent broadcasts from ViewEngine cache (O(k) instead of O(n))
    pub fn recent_broadcasts(&mut self, limit: usize, channel: Option<&str>) -> V2Result<Vec<Message>> {
        self.sync()?;

        // Use ViewEngine cache for O(k) access
        let cached = match channel {
            Some(ch) => self.view.get_channel_broadcasts(ch, limit),
            None => self.view.get_recent_broadcasts(limit),
        };

        let messages: Vec<Message> = cached.into_iter()
            .map(|b| {
                let ts_secs = (b.timestamp / 1_000_000) as i64;
                let ts_nanos = ((b.timestamp % 1_000_000) * 1000) as u32;
                let timestamp = chrono::DateTime::from_timestamp(ts_secs, ts_nanos)
                    .unwrap_or_else(chrono::Utc::now);

                Message {
                    id: b.id as i32,
                    from_ai: b.from_ai.clone(),
                    to_ai: None,
                    content: b.content.clone(),
                    message_type: MessageType::Broadcast,
                    channel: b.channel.clone(),
                    timestamp,
                }
            })
            .collect();

        Ok(messages)
    }

    /// Get recent DMs to this AI from ViewEngine cache (O(k) instead of O(n))
    pub fn recent_dms(&mut self, limit: usize) -> V2Result<Vec<Message>> {
        self.sync()?;

        // Use ViewEngine cache for O(k) access
        let cached = self.view.get_recent_dms(limit);

        let messages: Vec<Message> = cached.into_iter()
            .map(|dm| {
                let ts_secs = (dm.timestamp / 1_000_000) as i64;
                let ts_nanos = ((dm.timestamp % 1_000_000) * 1000) as u32;
                let timestamp = chrono::DateTime::from_timestamp(ts_secs, ts_nanos)
                    .unwrap_or_else(chrono::Utc::now);

                Message {
                    id: dm.id as i32,
                    from_ai: dm.from_ai.clone(),
                    to_ai: Some(dm.to_ai.clone()),
                    content: dm.content.clone(),
                    message_type: MessageType::Direct,
                    channel: String::new(),
                    timestamp,
                }
            })
            .collect();

        Ok(messages)
    }


    /// Get senders who have pending (unreplied) DMs
    /// A sender is "pending" if their last DM to me is newer than my last DM to them
    /// This is derived from events - no separate state tracking needed
    pub fn get_pending_dm_senders(&mut self) -> V2Result<Vec<String>> {
        self.sync()?;
        
        use std::collections::HashMap;
        
        // Track last DM timestamp per sender (to me) and per recipient (from me)
        let mut last_dm_to_me: HashMap<String, u64> = HashMap::new();
        let mut last_dm_from_me: HashMap<String, u64> = HashMap::new();
        
        let mut temp_reader = EventLogReader::open(Some(&self.base_dir))
            .map_err(|e| V2Error::EventLog(e.to_string()))?;
        
        loop {
        
            let event = match temp_reader.try_read() {
        
                Ok(Some(e)) => e,
        
                Ok(None) => break,
        
                Err(_) => continue, // Skip events that fail to deserialize
        
            };
            if event.header.event_type == event_type::DIRECT_MESSAGE {
                if let EventPayload::DirectMessage(payload) = &event.payload {
                    let sender = event.header.source_ai_str().to_string();
                    let recipient = &payload.to_ai;
                    let timestamp = event.header.timestamp;
                    
                    if recipient == &self.ai_id {
                        // DM to me - track sender's last message
                        last_dm_to_me.entry(sender)
                            .and_modify(|t| *t = (*t).max(timestamp))
                            .or_insert(timestamp);
                    } else if sender == self.ai_id {
                        // DM from me - track my last message to this recipient
                        last_dm_from_me.entry(recipient.clone())
                            .and_modify(|t| *t = (*t).max(timestamp))
                            .or_insert(timestamp);
                    }
                }
            }
        }
        
        // Find senders where their last DM to me is:
        // 1. Newer than my last DM to them (not yet replied)
        // 2. Within the TTL window (not stale)
        // TTL: 6 hours in microseconds (timestamps are in μs)
        const DM_PENDING_TTL_US: u64 = 6 * 60 * 60 * 1_000_000;
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;

        let pending: Vec<String> = last_dm_to_me
            .into_iter()
            .filter(|(sender, their_last)| {
                // Check TTL first - if DM is older than 6 hours, auto-expire
                if now_us.saturating_sub(*their_last) > DM_PENDING_TTL_US {
                    return false; // Expired - too old
                }
                // Check if replied
                match last_dm_from_me.get(sender) {
                    Some(my_last) => their_last > my_last,
                    None => true, // Never replied to them
                }
            })
            .map(|(sender, _)| sender)
            .collect();

        Ok(pending)
    }

    // ========== DIALOGUES ==========

    /// Start a dialogue with another AI
    pub fn start_dialogue(&mut self, responder: &str, topic: &str) -> V2Result<u64> {
        let event = Event::dialogue_start(&self.ai_id, responder, topic);
        let timestamp = event.header.timestamp; // Use as dialogue ID
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Respond to a dialogue
    pub fn respond_dialogue(&mut self, dialogue_id: u64, response: &str) -> V2Result<u64> {
        let event = Event::dialogue_respond(&self.ai_id, dialogue_id, response);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// End a dialogue
    pub fn end_dialogue(&mut self, dialogue_id: u64, status: &str) -> V2Result<u64> {
        let event = Event::dialogue_end(&self.ai_id, dialogue_id, status);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// End a dialogue with optional summary
    pub fn end_dialogue_with_summary(&mut self, dialogue_id: u64, status: &str, summary: Option<&str>) -> V2Result<u64> {
        let event = Event::dialogue_end_with_summary(&self.ai_id, dialogue_id, status, summary);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Merge two dialogues - source dialogue is marked as merged into target
    /// Use when two AIs create dialogues with each other about the same topic
    pub fn merge_dialogues(&mut self, source_id: u64, target_id: u64) -> V2Result<u64> {
        let event = Event::dialogue_merge(&self.ai_id, source_id, target_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    // ========== VOTES ==========

    /// Create a vote
    pub fn create_vote(&mut self, topic: &str, options: Vec<String>, total_voters: u32) -> V2Result<u64> {
        let event = Event::vote_create(&self.ai_id, topic, options, total_voters);
        let timestamp = event.header.timestamp; // Use as vote ID
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Cast a vote
    pub fn cast_vote(&mut self, vote_id: u64, choice: &str) -> V2Result<u64> {
        let event = Event::vote_cast(&self.ai_id, vote_id, choice);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Close a vote
    pub fn close_vote(&mut self, vote_id: u64) -> V2Result<u64> {
        let event = Event::vote_close(&self.ai_id, vote_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    // Locks removed — deprecated (Feb 2026, architecture decision)

    // ========== FILE CLAIMS ==========

    /// Claim a file for exclusive work
    pub fn claim_file(&mut self, path: &str, duration_secs: u32) -> V2Result<u64> {
        let event = Event::file_claim(&self.ai_id, path, duration_secs);
        let timestamp = event.header.timestamp; // Use as claim ID
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Release a file claim
    pub fn release_file(&mut self, path: &str) -> V2Result<u64> {
        let event = Event::file_release(&self.ai_id, path);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Get active file claims from ViewEngine cache (O(k) instead of O(n))
    pub fn get_claims(&mut self) -> V2Result<Vec<(String, String, u64, u32)>> {
        self.sync()?;

        // Use ViewEngine cache for O(k) access (already filters expired)
        let cached = self.view.get_active_claims();

        Ok(cached.into_iter()
            .map(|c| (c.path.clone(), c.holder.clone(), c.claimed_at, c.duration_seconds))
            .collect())
    }

    /// Check if a specific file is claimed
    pub fn check_claim(&mut self, path: &str) -> V2Result<Option<(String, u64, u32)>> {
        let claims = self.get_claims()?;
        Ok(claims.into_iter()
            .find(|(p, _, _, _)| p == path)
            .map(|(_, ai, ts, duration)| (ai, ts, duration)))
    }

    // ========== TASKS ==========

    /// Add a task
    ///
    /// Returns the task's timestamp which serves as a temporary ID until
    /// the sequencer assigns a real sequence number. The timestamp ID
    /// can be used with get_task() immediately (read-your-own-writes).
    pub fn add_task(&mut self, description: &str, priority: u32, tags: &str) -> V2Result<u64> {
        let event = Event::task_add(&self.ai_id, description, priority, tags);
        let timestamp = event.header.timestamp; // Use as temp task ID
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Claim a task
    pub fn claim_task(&mut self, task_id: u64) -> V2Result<u64> {
        let event = Event::task_claim(&self.ai_id, task_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Complete a task
    pub fn complete_task(&mut self, task_id: u64, result: &str) -> V2Result<u64> {
        let event = Event::task_complete(&self.ai_id, task_id, result);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Start working on a task
    pub fn start_task(&mut self, task_id: u64) -> V2Result<u64> {
        let event = Event::task_start(&self.ai_id, task_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Block a task with reason
    pub fn block_task(&mut self, task_id: u64, reason: &str) -> V2Result<u64> {
        let event = Event::task_block(&self.ai_id, task_id, reason);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Unblock a task
    pub fn unblock_task(&mut self, task_id: u64) -> V2Result<u64> {
        let event = Event::task_unblock(&self.ai_id, task_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Update task status (generic status change)
    pub fn update_task_status(&mut self, task_id: u64, status: &str) -> V2Result<u64> {
        // Map status strings to appropriate events
        match status.to_lowercase().as_str() {
            "started" | "in_progress" | "in-progress" => self.start_task(task_id),
            "completed" | "done" | "finished" => self.complete_task(task_id, "completed"),
            "blocked" | "paused" => self.block_task(task_id, "blocked via status update"),
            "unblocked" | "resumed" => self.unblock_task(task_id),
            _ => Err(V2Error::InvalidStatus(format!("Unknown status: {}", status))),
        }
    }

    // ========== BATCHES (Simple grouped tasks) ==========

    /// Create a batch with inline tasks
    /// tasks format: "1:Fix login,2:Fix logout,3:Test both" or "a:Header,b:Footer"
    pub fn batch_create(&mut self, name: &str, tasks: &str) -> V2Result<u64> {
        let event = Event::batch_create(&self.ai_id, name, tasks);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Mark a task in a batch as done
    pub fn batch_task_done(&mut self, batch_name: &str, label: &str) -> V2Result<u64> {
        let event = Event::batch_task_done(&self.ai_id, batch_name, label);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Close a batch (marks all remaining tasks as done)
    pub fn batch_close(&mut self, batch_name: &str) -> V2Result<u64> {
        let event = Event::batch_close(&self.ai_id, batch_name);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Get all batches with their status from ViewEngine cache (O(k) instead of O(n))
    /// Returns: Vec<(name, creator, total_tasks, done_count, is_closed)>
    pub fn get_batches(&mut self) -> V2Result<Vec<(String, String, usize, usize, bool)>> {
        self.sync()?;

        // Use ViewEngine cache - get open batches only
        let cached = self.view.get_open_batches();

        Ok(cached.into_iter()
            .map(|b| (
                b.name.clone(),
                b.creator.clone(),
                b.tasks.len(),
                b.done.len(),
                b.is_closed,
            ))
            .collect())
    }

    /// Get tasks in a specific batch from ViewEngine cache (O(1) lookup instead of O(n))
    /// Returns: Vec<(label, description, is_done)>
    pub fn get_batch(&mut self, batch_name: &str) -> V2Result<Option<(String, Vec<(String, String, bool)>)>> {
        self.sync()?;

        // Use ViewEngine cache for O(1) lookup
        match self.view.get_batch(batch_name) {
            Some(b) => {
                let result: Vec<(String, String, bool)> = b.tasks.iter()
                    .map(|(label, desc)| {
                        let is_done = b.is_closed || b.done.contains(label);
                        (label.clone(), desc.clone(), is_done)
                    })
                    .collect();
                Ok(Some((b.creator.clone(), result)))
            }
            None => Ok(None),
        }
    }

    // ========== PRESENCE ==========

    /// Update presence
    pub fn update_presence(&mut self, status: &str, current_task: Option<&str>) -> V2Result<u64> {
        let event = Event::presence_update(&self.ai_id, status, current_task);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    // ========== ROOMS ==========

    /// Create a room
    pub fn create_room(&mut self, name: &str, topic: &str) -> V2Result<u64> {
        let event = Event::room_create(&self.ai_id, name, topic);
        let timestamp = event.header.timestamp; // Use as room ID
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Join a room
    pub fn join_room(&mut self, room_id: &str) -> V2Result<u64> {
        let event = Event::room_join(&self.ai_id, room_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Leave a room
    pub fn leave_room(&mut self, room_id: &str) -> V2Result<u64> {
        let event = Event::room_leave(&self.ai_id, room_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Close a room
    pub fn close_room(&mut self, room_id: &str) -> V2Result<u64> {
        let event = Event::room_close(&self.ai_id, room_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Send a message to a room
    pub fn send_room_message(&mut self, room_id: &str, content: &str) -> V2Result<u64> {
        let event = Event::room_message(&self.ai_id, room_id, content);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    // ========== FILE ACTIONS ==========

    /// Log a file action
    pub fn log_file_action(&mut self, path: &str, action: &str) -> V2Result<u64> {
        let event = Event::file_action(&self.ai_id, path, action);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }
    // deposit_pheromone() removed — stigmergy deprecated (Feb 2026, architecture decision)

    // ========== STATS ==========

    /// Get view statistics
    pub fn stats(&self) -> &crate::view::ViewStats {
        self.view.stats()
    }

    /// Get unread DM count
    pub fn unread_dm_count(&self) -> u64 {
        self.view.unread_dm_count()
    }

    /// Get all unread DMs
    pub fn get_unread_dms(&self) -> Vec<crate::view::CachedDM> {
        self.view.get_unread_dms()
    }

    /// Mark a specific DM as read by ID (in-memory only - DEPRECATED)
    /// Use emit_dm_read() instead for persistent read marks
    pub fn mark_dm_read_by_id(&mut self, dm_id: u64) -> bool {
        self.view.mark_dm_read_by_id(dm_id)
    }

    /// Mark multiple DMs as read (in-memory only - DEPRECATED)
    /// Use emit_dm_read() instead for persistent read marks
    pub fn mark_dms_read_by_ids(&mut self, dm_ids: &[u64]) {
        self.view.mark_dms_read_by_ids(dm_ids)
    }

    /// Mark a DM as read (event-sourced - persists across CLI invocations)
    /// This emits a DM_READ event that gets processed by view.rs on rebuild
    pub fn emit_dm_read(&mut self, dm_id: u64) -> V2Result<u64> {
        let event = Event::dm_read(&self.ai_id, dm_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Mark multiple DMs as read (event-sourced - persists across CLI invocations)
    pub fn emit_dms_read(&mut self, dm_ids: &[u64]) -> V2Result<()> {
        for dm_id in dm_ids {
            self.emit_dm_read(*dm_id)?;
        }
        Ok(())
    }

    /// Get active dialogue count
    pub fn active_dialogue_count(&self) -> u64 {
        self.view.active_dialogue_count()
    }

    /// Get pending vote count
    pub fn pending_vote_count(&self) -> u64 {
        self.view.pending_vote_count()
    }

    // my_lock_count() removed — locks deprecated (Feb 2026, architecture decision)

    /// Get my task count
    pub fn my_task_count(&self) -> u64 {
        self.view.my_task_count()
    }

    // ========== QUERY METHODS ==========

    /// Get all current presences (latest presence per AI) from ViewEngine cache (O(k) instead of O(n))
    /// Returns Vec of (ai_id, status, current_task)
    /// Filters to only include AIs with recent event log activity (last 10 minutes)
    pub fn get_presences(&mut self) -> V2Result<Vec<(String, String, String)>> {
        self.sync()?;

        // 3 minutes in microseconds - if no tool activity for 3 min, AI is "offline"
        // (API connections timeout at 3 min, so this matches that boundary)
        const ONLINE_THRESHOLD_MICROS: u64 = 3 * 60 * 1_000_000;

        // Use ViewEngine cache for O(k) access
        let cached = self.view.get_online_presences(ONLINE_THRESHOLD_MICROS);

        Ok(cached.into_iter()
            .map(|p| (p.ai_id.clone(), p.status.clone(), p.current_task.clone()))
            .collect())
    }

    /// Get all dialogues (active and ended) from ViewEngine cache (O(k) instead of O(n))
    /// Returns Vec of (dialogue_id, initiator, responder, topic, status, current_turn)
    pub fn get_dialogues(&mut self) -> V2Result<Vec<(u64, String, String, String, String, String)>> {
        self.sync()?;

        // Use ViewEngine cache for O(k) access
        let cached = self.view.get_all_dialogues();

        Ok(cached.values()
            .map(|d| (
                d.id,
                d.initiator.clone(),
                d.responder.clone(),
                d.topic.clone(),
                d.status.clone(),
                d.current_turn.clone(),
            ))
            .collect())
    }

    /// Get a single dialogue by ID from ViewEngine cache (O(1) lookup)
    /// Returns (dialogue_id, initiator, responder, topic, status, current_turn)
    /// Get a dialogue by ID
    /// Handles the ID mismatch between outbox-returned timestamps and
    /// view-assigned global sequence numbers via dual lookup.
    /// Also scans the local outbox for pending events (read-your-own-writes).
    /// Returns (dialogue_id, initiator, responder, topic, status, current_turn)
    pub fn get_dialogue(&mut self, dialogue_id: u64) -> V2Result<Option<(u64, String, String, String, String, String)>> {
        self.sync()?;

        // Try view lookup (with timestamp fallback in ViewEngine)
        if let Some(d) = self.view.get_dialogue(dialogue_id) {
            return Ok(Some((
                dialogue_id, // Return the ID the caller knows
                d.initiator.clone(),
                d.responder.clone(),
                d.topic.clone(),
                d.status.clone(),
                d.current_turn.clone(),
            )));
        }

        // Fallback: scan outbox for pending DIALOGUE_START with matching timestamp
        if let Ok(consumer) = OutboxConsumer::open(&self.ai_id, Some(&self.base_dir)) {
            for event_result in consumer.peek_all_pending() {
                if let Ok(event) = event_result {
                    if event.header.event_type == event_type::DIALOGUE_START
                        && event.header.timestamp == dialogue_id
                    {
                        if let EventPayload::DialogueStart(payload) = &event.payload {
                            return Ok(Some((
                                dialogue_id,
                                self.ai_id.clone(),
                                payload.responder.clone(),
                                payload.topic.clone(),
                                "active".to_string(),
                                payload.responder.clone(),
                            )));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Get dialogue invites (dialogues where I am the responder and it's my turn - new invites)
    /// Returns Vec of (dialogue_id, initiator, responder, topic, status, current_turn)
    pub fn get_dialogue_invites(&mut self) -> V2Result<Vec<(u64, String, String, String, String, String)>> {
        let dialogues = self.get_dialogues()?;
        Ok(dialogues
            .into_iter()
            .filter(|(_, _, responder, _, status, turn)| {
                responder == &self.ai_id && status == "active" && turn == &self.ai_id
            })
            .collect())
    }

    /// Get dialogues where it's my turn to respond
    /// Returns Vec of (dialogue_id, initiator, responder, topic, status, current_turn)
    pub fn get_dialogue_my_turn(&mut self) -> V2Result<Vec<(u64, String, String, String, String, String)>> {
        let dialogues = self.get_dialogues()?;
        Ok(dialogues
            .into_iter()
            .filter(|(_, initiator, responder, _, status, turn)| {
                status == "active" && turn == &self.ai_id && (initiator == &self.ai_id || responder == &self.ai_id)
            })
            .collect())
    }

    /// Get all messages in a dialogue from ViewEngine cache (O(k) instead of O(n))
    /// Returns Vec of (sequence, source_ai, content, timestamp_micros)
    pub fn get_dialogue_messages(&mut self, dialogue_id: u64) -> V2Result<Vec<(u64, String, String, u64)>> {
        self.sync()?;

        // Use ViewEngine cache for O(k) access
        let cached = self.view.get_dialogue_messages(dialogue_id);

        Ok(cached.into_iter()
            .map(|m| (m.sequence, m.from_ai.clone(), m.content.clone(), m.timestamp))
            .collect())
    }

    /// Get all votes with their current state
    /// Returns Vec of (vote_id, creator, topic, options, status, casts)
    /// Get all votes from ViewEngine cache (O(k) instead of O(n))
    pub fn get_votes(&mut self) -> V2Result<Vec<(u64, String, String, Vec<String>, String, Vec<(String, String)>)>> {
        self.sync()?;

        // Use ViewEngine cache for O(k) access
        let cached = self.view.get_all_votes();

        Ok(cached.values()
            .map(|v| (
                v.id,
                v.creator.clone(),
                v.topic.clone(),
                v.options.clone(),
                v.status.clone(),
                v.casts.clone(),
            ))
            .collect())
    }

    /// Get a single vote by ID from ViewEngine cache (O(1) lookup)
    /// Returns (vote_id, creator, topic, options, status, casts)
    pub fn get_vote(&mut self, vote_id: u64) -> V2Result<Option<(u64, String, String, Vec<String>, String, Vec<(String, String)>)>> {
        self.sync()?;

        Ok(self.view.get_vote(vote_id)
            .map(|v| (
                v.id,
                v.creator.clone(),
                v.topic.clone(),
                v.options.clone(),
                v.status.clone(),
                v.casts.clone(),
            )))
    }

    /// Get a single task by ID from ViewEngine cache
    /// Handles the ID mismatch between outbox-returned timestamps and
    /// view-assigned global sequence numbers via dual lookup.
    /// Also scans the local outbox for pending events (read-your-own-writes).
    /// Returns (task_id, description, priority, status, assignee)
    pub fn get_task(&mut self, task_id: u64) -> V2Result<Option<(u64, String, i32, String, Option<String>)>> {
        self.sync()?;

        // Try view lookup (with timestamp fallback in ViewEngine)
        if let Some(t) = self.view.get_task(task_id) {
            return Ok(Some((
                task_id, // Return the ID the caller knows, not the internal key
                t.description.clone(),
                t.priority,
                t.status.clone(),
                t.assignee.clone(),
            )));
        }

        // Fallback: scan outbox for pending TASK_CREATE with matching timestamp
        // (event hasn't been processed by daemon yet)
        if let Ok(consumer) = OutboxConsumer::open(&self.ai_id, Some(&self.base_dir)) {
            for event_result in consumer.peek_all_pending() {
                if let Ok(event) = event_result {
                    if event.header.event_type == event_type::TASK_CREATE
                        && event.header.timestamp == task_id
                    {
                        if let EventPayload::TaskCreate(payload) = &event.payload {
                            return Ok(Some((
                                task_id,
                                payload.description.clone(),
                                payload.priority,
                                "pending".to_string(),
                                None,
                            )));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Get all tasks with their current state from ViewEngine cache (O(k) instead of O(n))
    /// Returns Vec of (task_id, description, priority, status, assignee)
    ///
    /// NOTE: Also scans the local outbox for pending TASK_CREATE events that
    /// haven't been merged yet (read-your-own-writes).
    pub fn get_tasks(&mut self) -> V2Result<Vec<(u64, String, i32, String, Option<String>)>> {
        self.sync()?;

        // Use ViewEngine cache for O(k) access
        let cached = self.view.get_all_tasks();

        let mut tasks: Vec<(u64, String, i32, String, Option<String>)> = cached.values()
            .map(|t| (
                t.id,
                t.description.clone(),
                t.priority,
                t.status.clone(),
                t.assignee.clone(),
            ))
            .collect();

        // Also scan local outbox for pending TASK_CREATE events (read-your-own-writes)
        // These haven't been merged by the sequencer yet, so use timestamp as temp ID
        match OutboxConsumer::open(&self.ai_id, Some(&self.base_dir)) {
            Ok(consumer) => {
                let pending = consumer.peek_all_pending();
                for event_result in pending {
                    match event_result {
                        Ok(event) => {
                            if event.header.event_type == event_type::TASK_CREATE {
                                if let EventPayload::TaskCreate(payload) = &event.payload {
                                    let temp_id = event.header.timestamp;
                                    // Only add if not already in tasks (sequencer may have merged it)
                                    if !tasks.iter().any(|(id, ..)| *id == temp_id) {
                                        tasks.push((
                                            temp_id,
                                            payload.description.clone(),
                                            payload.priority,
                                            "pending".to_string(),
                                            None,
                                        ));
                                    }
                                }
                            }
                        }
                        Err(_) => {}
                    }
                }
            }
            Err(_) => {}
        }

        Ok(tasks)
    }

    /// Get task statistics
    /// Returns (total, pending, claimed, in_progress, completed, failed, cancelled)
    pub fn get_task_stats(&mut self) -> V2Result<(u64, u64, u64, u64, u64, u64, u64)> {
        let tasks = self.get_tasks()?;
        let mut pending = 0u64;
        let mut claimed = 0u64;
        let mut in_progress = 0u64;
        let mut completed = 0u64;
        let mut failed = 0u64;
        let mut cancelled = 0u64;

        for (_, _, _, status, _) in &tasks {
            match status.as_str() {
                "pending" => pending += 1,
                "claimed" => claimed += 1,
                "in_progress" => in_progress += 1,
                "completed" => completed += 1,
                "failed" => failed += 1,
                "cancelled" => cancelled += 1,
                _ => {}
            }
        }

        let total = tasks.len() as u64;
        Ok((total, pending, claimed, in_progress, completed, failed, cancelled))
    }

    /// Get all rooms from ViewEngine cache (O(k) instead of O(n))
    /// Returns Vec of (room_id, name, topic, members)
    pub fn get_rooms(&mut self) -> V2Result<Vec<(u64, String, String, Vec<String>)>> {
        self.sync()?;

        // Use ViewEngine cache for O(k) access - only return open rooms
        let cached = self.view.get_open_rooms();

        Ok(cached.into_iter()
            .map(|r| (r.id, r.name.clone(), r.topic.clone(), r.members.clone()))
            .collect())
    }

    /// Get a single room by ID from ViewEngine cache (O(1) lookup)
    /// Returns (room_id, name, topic, members)
    pub fn get_room(&mut self, room_id: u64) -> V2Result<Option<(u64, String, String, Vec<String>)>> {
        self.sync()?;

        Ok(self.view.get_room(room_id)
            .filter(|r| !r.is_closed)
            .map(|r| (r.id, r.name.clone(), r.topic.clone(), r.members.clone())))
    }

    /// Get messages for a room from ViewEngine cache (O(k) instead of O(n))
    /// Returns Vec of (seq, from_ai, content, timestamp)
    pub fn get_room_messages(&mut self, room_id: &str, limit: usize) -> V2Result<Vec<(u64, String, String, u64)>> {
        self.sync()?;

        // Parse room_id as u64
        let room_id_u64 = room_id.parse::<u64>()
            .map_err(|_| V2Error::InvalidState(format!("Invalid room ID: {}", room_id)))?;

        // Use ViewEngine cache
        Ok(self.view.get_room_messages(room_id_u64, limit))
    }

    /// Get recent file actions from ViewEngine cache (O(k) instead of O(n))
    /// Returns Vec of (ai_id, action, path, timestamp_micros)
    pub fn get_file_actions(&mut self, limit: usize) -> V2Result<Vec<(String, String, String, u64)>> {
        self.sync()?;

        // Use ViewEngine cache for O(k) access
        let cached = self.view.get_recent_file_actions(limit);

        Ok(cached.into_iter()
            .map(|a| (a.ai_id.clone(), a.action.clone(), a.path.clone(), a.timestamp))
            .collect())
    }
    // get_pheromones() removed — stigmergy deprecated (Feb 2026, architecture decision)
    // check_lock() removed — locks deprecated (Feb 2026, architecture decision)

    // ===== Project Methods =====

    /// Create a project
    pub fn create_project(&mut self, name: &str, goal: &str, root_directory: &str) -> V2Result<u64> {
        let event = Event::project_create(&self.ai_id, name, goal, root_directory);
        let timestamp = event.header.timestamp; // Use as project ID
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Update a project
    pub fn update_project(&mut self, project_id: u64, goal: Option<&str>, status: Option<&str>) -> V2Result<u64> {
        let event = Event::project_update(&self.ai_id, project_id, goal, status);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Delete a project (soft delete)
    pub fn delete_project(&mut self, project_id: u64) -> V2Result<u64> {
        let event = Event::project_delete(&self.ai_id, project_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Restore a deleted project
    pub fn restore_project(&mut self, project_id: u64) -> V2Result<u64> {
        let event = Event::project_restore(&self.ai_id, project_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// List all projects (scans event log, applies create/delete/restore events)
    ///
    /// CANONICAL ID = timestamp (set at event creation, returned by create_project).
    /// Builds a sequence→timestamp backward compat map so old events that reference
    /// projects by sequence number still resolve correctly.
    /// Also scans the outbox for pending events (read-your-own-writes).
    pub fn list_projects(&mut self) -> V2Result<Vec<(u64, String, String, String, String, bool)>> {
        // Returns: (project_id, name, goal, root_directory, status, is_deleted)
        self.sync()?;

        use std::collections::HashMap;
        // Key = timestamp (canonical ID). Value = (name, goal, root_directory, status, is_deleted)
        let mut projects: HashMap<u64, (String, String, String, String, bool)> = HashMap::new();
        // Backward compat: sequence → timestamp for old events that used sequence as project_id
        let mut seq_to_ts: HashMap<u64, u64> = HashMap::new();

        let mut temp_reader = EventLogReader::open(Some(&self.base_dir))
            .map_err(|e| V2Error::EventLog(e.to_string()))?;

        loop {
            match temp_reader.try_read() {
                Ok(Some(event)) => {
                    match event.header.event_type {
                        event_type::PROJECT_CREATE => {
                            if let EventPayload::ProjectCreate(payload) = &event.payload {
                                let id = event.header.timestamp;
                                seq_to_ts.insert(event.header.sequence, id);
                                projects.insert(id, (
                                    payload.name.clone(),
                                    payload.goal.clone(),
                                    payload.root_directory.clone(),
                                    "active".to_string(),
                                    false,
                                ));
                            }
                        }
                        event_type::PROJECT_UPDATE => {
                            if let EventPayload::ProjectUpdate(payload) = &event.payload {
                                // Resolve: payload.project_id could be timestamp (new) or sequence (old)
                                let canonical_id = seq_to_ts.get(&payload.project_id)
                                    .copied()
                                    .unwrap_or(payload.project_id);
                                if let Some(p) = projects.get_mut(&canonical_id) {
                                    if let Some(ref g) = payload.goal { p.1 = g.clone(); }
                                    if let Some(ref s) = payload.status { p.3 = s.clone(); }
                                }
                            }
                        }
                        event_type::PROJECT_DELETE => {
                            if let EventPayload::ProjectDelete(payload) = &event.payload {
                                let canonical_id = seq_to_ts.get(&payload.project_id)
                                    .copied()
                                    .unwrap_or(payload.project_id);
                                if let Some(p) = projects.get_mut(&canonical_id) {
                                    p.4 = true;
                                }
                            }
                        }
                        event_type::PROJECT_RESTORE => {
                            if let EventPayload::ProjectRestore(payload) = &event.payload {
                                let canonical_id = seq_to_ts.get(&payload.project_id)
                                    .copied()
                                    .unwrap_or(payload.project_id);
                                if let Some(p) = projects.get_mut(&canonical_id) {
                                    p.4 = false;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ok(None) => break,
                Err(_) => continue,
            }
        }

        // Read-your-own-writes: scan outbox for pending PROJECT_CREATE events
        if let Ok(consumer) = OutboxConsumer::open(&self.ai_id, Some(&self.base_dir)) {
            for event_result in consumer.peek_all_pending() {
                if let Ok(event) = event_result {
                    if event.header.event_type == event_type::PROJECT_CREATE {
                        if let EventPayload::ProjectCreate(payload) = &event.payload {
                            let id = event.header.timestamp;
                            if !projects.contains_key(&id) {
                                projects.insert(id, (
                                    payload.name.clone(),
                                    payload.goal.clone(),
                                    payload.root_directory.clone(),
                                    "active".to_string(),
                                    false,
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Return only non-deleted projects
        let result: Vec<_> = projects.into_iter()
            .filter(|(_, (_, _, _, _, deleted))| !deleted)
            .map(|(id, (name, goal, dir, status, deleted))| (id, name, goal, dir, status, deleted))
            .collect();

        Ok(result)
    }

    /// Get a specific project by ID (accepts either timestamp or legacy sequence)
    pub fn get_project(&mut self, project_id: u64) -> V2Result<Option<(u64, String, String, String, String, bool)>> {
        let projects = self.list_projects()?;

        // Direct match — works for timestamps (canonical ID going forward)
        if let Some(p) = projects.iter().find(|(id, ..)| *id == project_id).cloned() {
            return Ok(Some(p));
        }

        // Backward compat: project_id might be a legacy sequence number
        // Quick scan to resolve sequence → timestamp
        let mut temp_reader = EventLogReader::open(Some(&self.base_dir))
            .map_err(|e| V2Error::EventLog(e.to_string()))?;
        loop {
            match temp_reader.try_read() {
                Ok(Some(event)) => {
                    if event.header.event_type == event_type::PROJECT_CREATE
                        && event.header.sequence == project_id
                    {
                        let ts = event.header.timestamp;
                        return Ok(projects.into_iter().find(|(id, ..)| *id == ts));
                    }
                }
                Ok(None) => break,
                Err(_) => continue,
            }
        }

        Ok(None)
    }

    // ===== Feature Methods =====

    /// Create a feature
    pub fn create_feature(&mut self, project_id: u64, name: &str, overview: &str, directory: Option<&str>) -> V2Result<u64> {
        let event = Event::feature_create(&self.ai_id, project_id, name, overview, directory);
        let timestamp = event.header.timestamp; // Use as feature ID
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Update a feature
    pub fn update_feature(&mut self, feature_id: u64, name: Option<&str>, overview: Option<&str>, directory: Option<&str>) -> V2Result<u64> {
        let event = Event::feature_update(&self.ai_id, feature_id, name, overview, directory);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Delete a feature (soft delete)
    pub fn delete_feature(&mut self, feature_id: u64) -> V2Result<u64> {
        let event = Event::feature_delete(&self.ai_id, feature_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Restore a deleted feature
    pub fn restore_feature(&mut self, feature_id: u64) -> V2Result<u64> {
        let event = Event::feature_restore(&self.ai_id, feature_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event)
            .map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// List features for a project
    ///
    /// CANONICAL ID = timestamp. Accepts project_id as either timestamp or legacy sequence.
    /// Builds sequence→timestamp maps for both projects and features so old events
    /// that reference entities by sequence number still resolve correctly.
    /// Also scans the outbox for pending events (read-your-own-writes).
    pub fn list_features(&mut self, project_id: u64) -> V2Result<Vec<(u64, u64, String, String, Option<String>, bool)>> {
        // Returns: (feature_id, project_id, name, overview, directory, is_deleted)
        self.sync()?;

        use std::collections::HashMap;
        // Backward compat maps: sequence → timestamp
        let mut project_seq_to_ts: HashMap<u64, u64> = HashMap::new();
        let mut feature_seq_to_ts: HashMap<u64, u64> = HashMap::new();
        // Key = timestamp (canonical). Value = (project_id, name, overview, directory, is_deleted)
        let mut features: HashMap<u64, (u64, String, String, Option<String>, bool)> = HashMap::new();
        // Resolve input project_id: might be a legacy sequence number
        let mut resolved_project_id = project_id;

        let mut temp_reader = EventLogReader::open(Some(&self.base_dir))
            .map_err(|e| V2Error::EventLog(e.to_string()))?;

        loop {
            match temp_reader.try_read() {
                Ok(Some(event)) => {
                    match event.header.event_type {
                        event_type::PROJECT_CREATE => {
                            // Build seq→ts map for resolving feature payload.project_id
                            project_seq_to_ts.insert(event.header.sequence, event.header.timestamp);
                            // If input project_id matches this project's sequence, resolve to timestamp
                            if event.header.sequence == project_id && event.header.timestamp != project_id {
                                resolved_project_id = event.header.timestamp;
                            }
                        }
                        event_type::FEATURE_CREATE => {
                            if let EventPayload::FeatureCreate(payload) = &event.payload {
                                // Resolve payload.project_id: could be timestamp (new) or sequence (old)
                                let canonical_proj_id = project_seq_to_ts.get(&payload.project_id)
                                    .copied()
                                    .unwrap_or(payload.project_id);

                                if canonical_proj_id == resolved_project_id {
                                    let id = event.header.timestamp;
                                    feature_seq_to_ts.insert(event.header.sequence, id);
                                    features.insert(id, (
                                        resolved_project_id,
                                        payload.name.clone(),
                                        payload.overview.clone(),
                                        payload.directory.clone(),
                                        false,
                                    ));
                                }
                            }
                        }
                        event_type::FEATURE_UPDATE => {
                            if let EventPayload::FeatureUpdate(payload) = &event.payload {
                                // Resolve feature_id: could be timestamp (new) or sequence (old)
                                let canonical_feat_id = feature_seq_to_ts.get(&payload.feature_id)
                                    .copied()
                                    .unwrap_or(payload.feature_id);
                                if let Some(f) = features.get_mut(&canonical_feat_id) {
                                    if let Some(ref n) = payload.name { f.1 = n.clone(); }
                                    if let Some(ref o) = payload.overview { f.2 = o.clone(); }
                                    if payload.directory.is_some() { f.3 = payload.directory.clone(); }
                                }
                            }
                        }
                        event_type::FEATURE_DELETE => {
                            if let EventPayload::FeatureDelete(payload) = &event.payload {
                                let canonical_feat_id = feature_seq_to_ts.get(&payload.feature_id)
                                    .copied()
                                    .unwrap_or(payload.feature_id);
                                if let Some(f) = features.get_mut(&canonical_feat_id) {
                                    f.4 = true;
                                }
                            }
                        }
                        event_type::FEATURE_RESTORE => {
                            if let EventPayload::FeatureRestore(payload) = &event.payload {
                                let canonical_feat_id = feature_seq_to_ts.get(&payload.feature_id)
                                    .copied()
                                    .unwrap_or(payload.feature_id);
                                if let Some(f) = features.get_mut(&canonical_feat_id) {
                                    f.4 = false;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ok(None) => break,
                Err(_) => continue,
            }
        }

        // Read-your-own-writes: scan outbox for pending FEATURE_CREATE events
        if let Ok(consumer) = OutboxConsumer::open(&self.ai_id, Some(&self.base_dir)) {
            for event_result in consumer.peek_all_pending() {
                if let Ok(event) = event_result {
                    if event.header.event_type == event_type::FEATURE_CREATE {
                        if let EventPayload::FeatureCreate(payload) = &event.payload {
                            // Resolve payload.project_id through the map
                            let canonical_proj_id = project_seq_to_ts.get(&payload.project_id)
                                .copied()
                                .unwrap_or(payload.project_id);
                            if canonical_proj_id == resolved_project_id {
                                let id = event.header.timestamp;
                                if !features.contains_key(&id) {
                                    features.insert(id, (
                                        resolved_project_id,
                                        payload.name.clone(),
                                        payload.overview.clone(),
                                        payload.directory.clone(),
                                        false,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Return only non-deleted features
        let result: Vec<_> = features.into_iter()
            .filter(|(_, (_, _, _, _, deleted))| !deleted)
            .map(|(id, (proj_id, name, overview, dir, deleted))| (id, proj_id, name, overview, dir, deleted))
            .collect();

        Ok(result)
    }

    /// Get a specific feature by ID (accepts either timestamp or legacy sequence)
    ///
    /// CANONICAL ID = timestamp. Builds sequence→timestamp maps for backward compat.
    /// Also scans outbox for pending events (read-your-own-writes).
    pub fn get_feature(&mut self, feature_id: u64) -> V2Result<Option<(u64, u64, String, String, Option<String>, bool)>> {
        // Need to scan all features since we don't know the project_id
        self.sync()?;

        use std::collections::HashMap;
        let mut project_seq_to_ts: HashMap<u64, u64> = HashMap::new();
        let mut feature_seq_to_ts: HashMap<u64, u64> = HashMap::new();
        let mut feature: Option<(u64, u64, String, String, Option<String>, bool)> = None;
        // Track the canonical feature_id (resolved from sequence if needed)
        let mut canonical_feature_id = feature_id;

        let mut temp_reader = EventLogReader::open(Some(&self.base_dir))
            .map_err(|e| V2Error::EventLog(e.to_string()))?;

        loop {
            match temp_reader.try_read() {
                Ok(Some(event)) => {
                    match event.header.event_type {
                        event_type::PROJECT_CREATE => {
                            project_seq_to_ts.insert(event.header.sequence, event.header.timestamp);
                        }
                        event_type::FEATURE_CREATE => {
                            if let EventPayload::FeatureCreate(payload) = &event.payload {
                                let ts = event.header.timestamp;
                                feature_seq_to_ts.insert(event.header.sequence, ts);
                                // Match by timestamp (canonical) or sequence (legacy)
                                if ts == feature_id || event.header.sequence == feature_id {
                                    canonical_feature_id = ts;
                                    // Resolve project_id to canonical timestamp
                                    let canonical_proj_id = project_seq_to_ts.get(&payload.project_id)
                                        .copied()
                                        .unwrap_or(payload.project_id);
                                    feature = Some((
                                        ts,
                                        canonical_proj_id,
                                        payload.name.clone(),
                                        payload.overview.clone(),
                                        payload.directory.clone(),
                                        false,
                                    ));
                                }
                            }
                        }
                        event_type::FEATURE_UPDATE => {
                            if let EventPayload::FeatureUpdate(payload) = &event.payload {
                                let canonical_feat_id = feature_seq_to_ts.get(&payload.feature_id)
                                    .copied()
                                    .unwrap_or(payload.feature_id);
                                if canonical_feat_id == canonical_feature_id {
                                    if let Some(ref mut f) = feature {
                                        if let Some(ref n) = payload.name { f.2 = n.clone(); }
                                        if let Some(ref o) = payload.overview { f.3 = o.clone(); }
                                        if payload.directory.is_some() { f.4 = payload.directory.clone(); }
                                    }
                                }
                            }
                        }
                        event_type::FEATURE_DELETE => {
                            if let EventPayload::FeatureDelete(payload) = &event.payload {
                                let canonical_feat_id = feature_seq_to_ts.get(&payload.feature_id)
                                    .copied()
                                    .unwrap_or(payload.feature_id);
                                if canonical_feat_id == canonical_feature_id {
                                    if let Some(ref mut f) = feature {
                                        f.5 = true;
                                    }
                                }
                            }
                        }
                        event_type::FEATURE_RESTORE => {
                            if let EventPayload::FeatureRestore(payload) = &event.payload {
                                let canonical_feat_id = feature_seq_to_ts.get(&payload.feature_id)
                                    .copied()
                                    .unwrap_or(payload.feature_id);
                                if canonical_feat_id == canonical_feature_id {
                                    if let Some(ref mut f) = feature {
                                        f.5 = false;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ok(None) => break,
                Err(_) => continue,
            }
        }

        // Read-your-own-writes: scan outbox for pending FEATURE_CREATE
        if feature.is_none() {
            if let Ok(consumer) = OutboxConsumer::open(&self.ai_id, Some(&self.base_dir)) {
                for event_result in consumer.peek_all_pending() {
                    if let Ok(event) = event_result {
                        if event.header.event_type == event_type::FEATURE_CREATE
                            && event.header.timestamp == feature_id
                        {
                            if let EventPayload::FeatureCreate(payload) = &event.payload {
                                let canonical_proj_id = project_seq_to_ts.get(&payload.project_id)
                                    .copied()
                                    .unwrap_or(payload.project_id);
                                feature = Some((
                                    event.header.timestamp,
                                    canonical_proj_id,
                                    payload.name.clone(),
                                    payload.overview.clone(),
                                    payload.directory.clone(),
                                    false,
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Return None if deleted
        if let Some(ref f) = feature {
            if f.5 { return Ok(None); }
        }

        Ok(feature)
    }

    // ===== Project Resolution =====

    /// Normalize a path for cross-platform comparison.
    /// Handles WSL (/mnt/c/...) ↔ Windows (C:/...) conversion.
    fn normalize_path_for_compare(path: &str) -> String {
        let s = path.replace('\\', "/");
        // WSL → Windows: /mnt/c/Users/... → c:/users/...
        if s.starts_with("/mnt/") && s.len() > 6 && s.as_bytes()[5].is_ascii_alphabetic() && s.as_bytes()[6] == b'/' {
            let drive = (s.as_bytes()[5] as char).to_lowercase().next().unwrap();
            return format!("{}:/{}", drive, &s[7..]).to_lowercase();
        }
        // Windows → lowercase: C:/Users/... → c:/users/...
        s.to_lowercase()
    }

    /// Resolve a file path to its project and (optionally) feature.
    /// Uses longest-match algorithm: the project whose root_directory is the
    /// longest prefix of `file_path` wins. Within that project, the feature
    /// whose directory is the longest prefix wins.
    ///
    /// Returns: Option<(project_id, name, goal, root_dir, Option<(feature_id, name, overview, directory)>)>
    pub fn resolve_project_for_file(&mut self, file_path: &str)
        -> V2Result<Option<(u64, String, String, String, Option<(u64, String, String, String)>)>>
    {
        // Normalize path for cross-platform comparison (WSL ↔ Windows)
        let normalized_lower = Self::normalize_path_for_compare(file_path);

        // Get all active projects
        let projects = self.list_projects()?;

        // Find project with longest matching root_directory prefix
        let mut best_project: Option<(u64, String, String, String)> = None;
        let mut best_len: usize = 0;

        for (id, name, goal, root_dir, status, _deleted) in &projects {
            if status != "active" { continue; }
            let norm_root = Self::normalize_path_for_compare(root_dir);
            if normalized_lower.starts_with(&norm_root) && norm_root.len() > best_len {
                best_len = norm_root.len();
                best_project = Some((*id, name.clone(), goal.clone(), root_dir.clone()));
            }
        }

        let (proj_id, proj_name, proj_goal, proj_dir) = match best_project {
            Some(p) => p,
            None => return Ok(None),
        };

        // Get features for this project, find longest matching directory
        let features = self.list_features(proj_id)?;
        let mut best_feature: Option<(u64, String, String, String)> = None;
        let mut best_feat_len: usize = 0;

        for (feat_id, _proj_id, feat_name, overview, directory, _deleted) in &features {
            if let Some(ref dir) = directory {
                // Feature directory can be relative (to project root) or absolute
                let feat_path = if dir.contains(':') || dir.starts_with('/') {
                    // Absolute path
                    Self::normalize_path_for_compare(dir)
                } else {
                    // Relative to project root — join with project's normalized root
                    let base = Self::normalize_path_for_compare(&proj_dir);
                    let base = base.trim_end_matches('/');
                    format!("{}/{}", base, dir.replace('\\', "/").to_lowercase())
                };

                if normalized_lower.starts_with(&feat_path) && feat_path.len() > best_feat_len {
                    best_feat_len = feat_path.len();
                    best_feature = Some((*feat_id, feat_name.clone(), overview.clone(), dir.clone()));
                }
            }
        }

        Ok(Some((proj_id, proj_name, proj_goal, proj_dir, best_feature)))
    }

    // ========================================================================
    // Learning Operations (Shared Team Insights - "Muscle Memory")
    // ========================================================================

    /// Create a new learning (shared insight)
    /// Returns the sequence number (learning_id)
    pub fn create_learning(&mut self, content: &str, tags: &str, importance: u8) -> Result<u64, V2Error> {
        let event = Event::learning_create(&self.ai_id, content, tags, importance);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event).map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Update an existing learning
    pub fn update_learning(&mut self, learning_id: u64, content: Option<&str>, tags: Option<&str>, importance: Option<u8>) -> Result<u64, V2Error> {
        let event = Event::learning_update(&self.ai_id, learning_id, content, tags, importance);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event).map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Delete a learning
    pub fn delete_learning(&mut self, learning_id: u64) -> Result<u64, V2Error> {
        let event = Event::learning_delete(&self.ai_id, learning_id);
        let timestamp = event.header.timestamp;
        self.outbox.write_event(&event).map_err(|e| V2Error::Outbox(e.to_string()))?;
        Ok(timestamp)
    }

    /// Get all learnings for a specific AI (their playbook)
    /// Returns: Vec<(learning_id, ai_id, content, tags, importance, deleted)>
    pub fn get_ai_learnings(&mut self, target_ai: &str) -> Result<Vec<(u64, String, String, String, u8, bool)>, V2Error> {
        // Open event log reader - returns empty if no log exists
        let mut temp_reader = match EventLogReader::open(Some(&self.base_dir)) {
            Ok(r) => r,
            Err(_) => return Ok(Vec::new()),
        };
        // HashMap: learning_id -> (ai_id, content, tags, importance, deleted)
        let mut learnings: std::collections::HashMap<u64, (String, String, String, u8, bool)> = std::collections::HashMap::new();

        loop {
            match temp_reader.try_read() {
                Ok(Some(event)) => {
                    match event.header.event_type {
                        event_type::LEARNING_CREATE => {
                            if let EventPayload::LearningCreate(payload) = &event.payload {
                                if event.header.source_ai_str() == target_ai {
                                    let id = event.header.sequence;
                                    learnings.insert(id, (
                                        event.header.source_ai_str().to_string(),
                                        payload.content.clone(),
                                        payload.tags.clone(),
                                        payload.importance,
                                        false,
                                    ));
                                }
                            }
                        }
                        event_type::LEARNING_UPDATE => {
                            if let EventPayload::LearningUpdate(payload) = &event.payload {
                                if let Some(l) = learnings.get_mut(&payload.learning_id) {
                                    if let Some(ref c) = payload.content { l.1 = c.clone(); }
                                    if let Some(ref t) = payload.tags { l.2 = t.clone(); }
                                    if let Some(i) = payload.importance { l.3 = i; }
                                }
                            }
                        }
                        event_type::LEARNING_DELETE => {
                            if let EventPayload::LearningDelete(payload) = &event.payload {
                                if let Some(l) = learnings.get_mut(&payload.learning_id) {
                                    l.4 = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ok(None) => break,
                Err(_) => continue,
            }
        }

        // Filter out deleted and return as vec with id
        let result: Vec<_> = learnings.into_iter()
            .filter(|(_, l)| !l.4)
            .map(|(id, l)| (id, l.0, l.1, l.2, l.3, l.4))
            .collect();

        Ok(result)
    }

    /// Get my learnings (my playbook)
    pub fn get_my_learnings(&mut self) -> Result<Vec<(u64, String, String, String, u8, bool)>, V2Error> {
        self.get_ai_learnings(&self.ai_id.clone())
    }

    /// Get team playbook - top learnings from all AIs
    /// Returns learnings sorted by importance (highest first), limited to `limit`
    pub fn get_team_playbook(&mut self, limit: usize) -> Result<Vec<(u64, String, String, String, u8)>, V2Error> {
        // Open event log reader - returns empty if no log exists
        let mut temp_reader = match EventLogReader::open(Some(&self.base_dir)) {
            Ok(r) => r,
            Err(_) => return Ok(Vec::new()),
        };
        // HashMap: learning_id -> (ai_id, content, tags, importance, deleted)
        let mut learnings: std::collections::HashMap<u64, (String, String, String, u8, bool)> = std::collections::HashMap::new();

        loop {
            match temp_reader.try_read() {
                Ok(Some(event)) => {
                    match event.header.event_type {
                        event_type::LEARNING_CREATE => {
                            if let EventPayload::LearningCreate(payload) = &event.payload {
                                let id = event.header.sequence;
                                learnings.insert(id, (
                                    event.header.source_ai_str().to_string(),
                                    payload.content.clone(),
                                    payload.tags.clone(),
                                    payload.importance,
                                    false,
                                ));
                            }
                        }
                        event_type::LEARNING_UPDATE => {
                            if let EventPayload::LearningUpdate(payload) = &event.payload {
                                if let Some(l) = learnings.get_mut(&payload.learning_id) {
                                    if let Some(ref c) = payload.content { l.1 = c.clone(); }
                                    if let Some(ref t) = payload.tags { l.2 = t.clone(); }
                                    if let Some(i) = payload.importance { l.3 = i; }
                                }
                            }
                        }
                        event_type::LEARNING_DELETE => {
                            if let EventPayload::LearningDelete(payload) = &event.payload {
                                if let Some(l) = learnings.get_mut(&payload.learning_id) {
                                    l.4 = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ok(None) => break,
                Err(_) => continue,
            }
        }

        // Filter out deleted, sort by importance (desc), take limit
        let mut result: Vec<_> = learnings.into_iter()
            .filter(|(_, l)| !l.4)
            .map(|(id, l)| (id, l.0, l.1, l.2, l.3))
            .collect();

        // Sort by importance (index 4) descending
        result.sort_by(|a, b| b.4.cmp(&a.4));
        result.truncate(limit);

        Ok(result)
    }

    /// Count learnings for an AI (for enforcing the 15 limit)
    pub fn count_learnings(&mut self, target_ai: &str) -> Result<usize, V2Error> {
        let learnings = self.get_ai_learnings(target_ai)?;
        Ok(learnings.len())
    }

    // ========================================================================
    // Trust Methods (TIP: Trust Inference and Propagation)
    // ========================================================================

    /// Record trust feedback about another AI
    /// is_success: true = positive interaction, false = negative
    /// weight: 1-10 significance (default 1)
    pub fn record_trust(&mut self, target_ai: &str, is_success: bool, context: &str, weight: u8) -> Result<u64, V2Error> {
        let event = Event::trust_record(&self.ai_id, target_ai, is_success, context, weight);
        self.outbox.write_event(&event).map_err(|e| V2Error::Outbox(e.to_string()))
    }

    /// Get all trust records from the event log
    /// Returns: Vec<(rater_ai, target_ai, is_success, context, weight, timestamp)>
    pub fn get_trust_records(&mut self) -> Result<Vec<(String, String, bool, String, u8, u64)>, V2Error> {
        let mut temp_reader = match EventLogReader::open(Some(&self.base_dir)) {
            Ok(r) => r,
            Err(_) => return Ok(Vec::new()),
        };

        let mut records = Vec::new();

        loop {
            let event = match temp_reader.try_read() {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(_) => continue, // Skip events that fail to deserialize
            };

            if event.header.event_type == event_type::TRUST_RECORD {
                if let EventPayload::TrustRecord(payload) = &event.payload {
                    records.push((
                        event.header.source_ai_str().to_string(),
                        payload.target_ai.clone(),
                        payload.is_success,
                        payload.context.clone(),
                        payload.weight,
                        event.header.timestamp,
                    ));
                }
            }
        }

        Ok(records)
    }

    /// Calculate decay factor for a trust record based on age
    /// Uses half-life decay: factor = 0.5^(elapsed_days / half_life_days)
    fn decay_factor(timestamp_micros: u64, half_life_days: f64) -> f64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now_micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        let elapsed_micros = now_micros.saturating_sub(timestamp_micros);
        let elapsed_days = elapsed_micros as f64 / (1_000_000.0 * 60.0 * 60.0 * 24.0);
        0.5_f64.powf(elapsed_days / half_life_days)
    }

    /// Compute trust score for a specific AI from my perspective
    /// Uses Beta distribution: Trust = α/(α+β) where α=successes+1, β=failures+1
    /// Returns: Option<(trust_score, alpha, beta, variance)>
    pub fn get_trust_score(&mut self, target_ai: &str) -> Result<Option<(f64, u32, u32, f64)>, V2Error> {
        let records = self.get_trust_records()?;

        // Filter to only my ratings of the target
        let my_id = self.ai_id.clone();
        let mut alpha: u32 = 1; // Prior: start with Beta(1,1) = uniform
        let mut beta: u32 = 1;

        for (rater, target, is_success, _, weight, _) in records {
            if rater == my_id && target == target_ai {
                let w = weight as u32;
                if is_success {
                    alpha += w;
                } else {
                    beta += w;
                }
            }
        }

        // If only prior (no actual interactions), return None
        if alpha == 1 && beta == 1 {
            return Ok(None);
        }

        let trust = alpha as f64 / (alpha + beta) as f64;
        let total = (alpha + beta) as f64;
        let variance = (alpha as f64 * beta as f64) / (total * total * (total + 1.0));

        Ok(Some((trust, alpha, beta, variance)))
    }

    /// Compute decayed trust score for a specific AI from my perspective
    /// Applies half-life decay to older interactions (default 90 days)
    /// Returns: Option<(trust_score, effective_alpha, effective_beta)>
    pub fn get_decayed_trust_score(&mut self, target_ai: &str, half_life_days: Option<f64>) -> Result<Option<(f64, f64, f64)>, V2Error> {
        let half_life = half_life_days.unwrap_or(90.0); // Default 90 days (~3 months)
        let records = self.get_trust_records()?;
        let my_id = self.ai_id.clone();

        let mut alpha: f64 = 1.0; // Prior
        let mut beta: f64 = 1.0;

        for (rater, target, is_success, _, weight, timestamp) in records {
            if rater == my_id && target == target_ai {
                let decay = Self::decay_factor(timestamp, half_life);
                let decayed_weight = weight as f64 * decay;
                if is_success {
                    alpha += decayed_weight;
                } else {
                    beta += decayed_weight;
                }
            }
        }

        // If only prior (no meaningful interactions after decay), return None
        if alpha < 1.01 && beta < 1.01 {
            return Ok(None);
        }

        let trust = alpha / (alpha + beta);
        Ok(Some((trust, alpha, beta)))
    }

    /// Compute all decayed trust scores from my perspective
    /// Returns: Vec<(target_ai, trust_score, effective_alpha, effective_beta)>
    pub fn get_all_decayed_trust_scores(&mut self, half_life_days: Option<f64>) -> Result<Vec<(String, f64, f64, f64)>, V2Error> {
        let half_life = half_life_days.unwrap_or(90.0);
        let records = self.get_trust_records()?;
        let my_id = self.ai_id.clone();

        let mut scores: std::collections::HashMap<String, (f64, f64)> = std::collections::HashMap::new();

        for (rater, target, is_success, _, weight, timestamp) in records {
            if rater == my_id {
                let entry = scores.entry(target).or_insert((1.0, 1.0)); // Prior
                let decay = Self::decay_factor(timestamp, half_life);
                let decayed_weight = weight as f64 * decay;
                if is_success {
                    entry.0 += decayed_weight;
                } else {
                    entry.1 += decayed_weight;
                }
            }
        }

        let mut result: Vec<(String, f64, f64, f64)> = scores
            .into_iter()
            .filter(|(_, (a, b))| *a > 1.01 || *b > 1.01)
            .map(|(target, (alpha, beta))| {
                let trust = alpha / (alpha + beta);
                (target, trust, alpha, beta)
            })
            .collect();

        result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(result)
    }

    /// Compute all trust scores from my perspective
    /// Returns: Vec<(target_ai, trust_score, alpha, beta, variance)>
    pub fn get_all_trust_scores(&mut self) -> Result<Vec<(String, f64, u32, u32, f64)>, V2Error> {
        let records = self.get_trust_records()?;
        let my_id = self.ai_id.clone();

        // Aggregate by target_ai
        let mut scores: std::collections::HashMap<String, (u32, u32)> = std::collections::HashMap::new();

        for (rater, target, is_success, _, weight, _) in records {
            if rater == my_id {
                let entry = scores.entry(target).or_insert((1, 1)); // Prior
                let w = weight as u32;
                if is_success {
                    entry.0 += w;
                } else {
                    entry.1 += w;
                }
            }
        }

        let mut result: Vec<(String, f64, u32, u32, f64)> = scores
            .into_iter()
            .filter(|(_, (a, b))| *a > 1 || *b > 1) // Only include if we have actual data
            .map(|(target, (alpha, beta))| {
                let trust = alpha as f64 / (alpha + beta) as f64;
                let total = (alpha + beta) as f64;
                let variance = (alpha as f64 * beta as f64) / (total * total * (total + 1.0));
                (target, trust, alpha, beta, variance)
            })
            .collect();

        // Sort by trust score descending
        result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_client_open() {
        let dir = tempdir().unwrap();
        let client = V2Client::open("test-ai", Some(dir.path())).unwrap();
        assert_eq!(client.ai_id(), "test-ai");
    }

    #[test]
    fn test_client_broadcast() {
        let dir = tempdir().unwrap();
        let mut client = V2Client::open("test-ai", Some(dir.path())).unwrap();
        // write_event returns local outbox position (can be 0)
        // Global sequence assigned by sequencer later
        let _seq = client.broadcast("general", "Hello world!").unwrap();
    }

    #[test]
    fn test_client_dm() {
        let dir = tempdir().unwrap();
        let mut client = V2Client::open("test-ai", Some(dir.path())).unwrap();
        // write_event returns local outbox position (can be 0)
        let _seq = client.direct_message("other-ai", "Hello!").unwrap();
    }
}
