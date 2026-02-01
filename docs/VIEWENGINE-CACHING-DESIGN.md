# ViewEngine Content Caching Design

## Architecture Clarification

**This design uses PURE EVENT-DRIVEN architecture:**
- **NO BTree** - removed in favor of simple file I/O for cursor
- **NO ShadowAllocator** - not needed for simple cursor persistence
- **Outboxes** - per-AI ring buffers for event delivery (existing)
- **Event Log** - append-only source of truth (existing)
- **Materialized Views** - ephemeral in-memory caches rebuilt from events

The only persistent state is a single 8-byte cursor file per AI.

## Problem Statement

Currently, query methods in `v2_client.rs` (`recent_dms()`, `recent_broadcasts()`, `get_dialogues()`, etc.) create a new `EventLogReader` and scan the **entire event log** from the beginning for every query. With 95K+ events today (and growing), this is architecturally wrong.

The event log is append-only and will grow indefinitely. What works "fine" at 0.3s today becomes 3s at 1M events, then 30s at 10M events. This affects **all AIs** using AI-Foundation infrastructure.

## Design Principles

1. **Event Log = Source of Truth** - Append-only, never modified, never scanned for reads
2. **ViewEngine = Materialized View** - Per-AI cached data derived from events
3. **Queries = Read from ViewEngine** - O(1) or O(k) access, never O(n) scans

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Master Event Log                         │
│  (Append-only, shared, ~100ns writes via outbox)            │
└─────────────────────────────────────────────────────────────┘
                              │
                    sync() with cursor
                              ↓
┌─────────────────────────────────────────────────────────────┐
│                  Per-AI ViewEngine                           │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ Ephemeral Caches (In-Memory Ring Buffers)               ││
│  │  - recent_dms: VecDeque<CachedDM>         [max 100]     ││
│  │  - recent_broadcasts: HashMap<channel, VecDeque> [100]  ││
│  │  - active_dialogues: HashMap<id, DialogueState>         ││
│  │  - pending_tasks: HashMap<id, TaskState>                ││
│  │  - file_actions: VecDeque<FileAction>     [max 100]     ││
│  └─────────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────────┐│
│  │ Persistent State (Simple File)                          ││
│  │  - cursor: u64 → {ai_id}.cursor file (8 bytes)          ││
│  │  - trust_scores: rebuilt from events on startup         ││
│  └─────────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────────┐│
│  │ Statistics (Derived Counters)                           ││
│  │  - unread_dms, active_dialogues, pending_votes, etc.    ││
│  └─────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
                              │
                       Query Methods
                              ↓
┌─────────────────────────────────────────────────────────────┐
│  get_recent_dms() → read from recent_dms cache             │
│  get_recent_broadcasts() → read from recent_broadcasts     │
│  get_dialogue_messages() → read from active_dialogues      │
│  get_tasks() → read from pending_tasks                     │
│  NO EVENT LOG SCANNING                                      │
└─────────────────────────────────────────────────────────────┘
```

## Data Structures

### Cached Message Types

```rust
/// Cached DM for quick access
#[derive(Clone, Debug)]
pub struct CachedDM {
    pub id: u64,           // Event sequence number
    pub from_ai: String,
    pub content: String,
    pub timestamp: u64,    // Microseconds since epoch
    pub read: bool,        // Marked as read?
}

/// Cached broadcast message
#[derive(Clone, Debug)]
pub struct CachedBroadcast {
    pub id: u64,
    pub from_ai: String,
    pub channel: String,
    pub content: String,
    pub timestamp: u64,
}

/// Dialogue state with message history
#[derive(Clone, Debug)]
pub struct DialogueState {
    pub id: u64,
    pub initiator: String,
    pub responder: String,
    pub topic: String,
    pub status: String,         // "active", "ended", "merged"
    pub current_turn: String,   // Who should respond next
    pub messages: VecDeque<DialogueMessage>,  // Recent messages (max 100)
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug)]
pub struct DialogueMessage {
    pub sequence: u64,
    pub from_ai: String,
    pub content: String,
    pub timestamp: u64,
}

/// Task state
#[derive(Clone, Debug)]
pub struct TaskState {
    pub id: u64,
    pub description: String,
    pub priority: i32,
    pub status: String,         // "pending", "claimed", "in_progress", "completed"
    pub assignee: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// File action for stigmergy
#[derive(Clone, Debug)]
pub struct CachedFileAction {
    pub id: u64,
    pub ai_id: String,
    pub path: String,
    pub action: String,  // "read", "write", "exec"
    pub timestamp: u64,
}
```

### Extended ViewEngine (NO BTree - Pure Event-Driven)

```rust
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::fs;
use std::io::{Read, Write};

/// Cache size limits
const MAX_CACHED_DMS: usize = 100;
const MAX_CACHED_BROADCASTS_PER_CHANNEL: usize = 100;
const MAX_DIALOGUE_MESSAGES: usize = 100;
const MAX_CACHED_FILE_ACTIONS: usize = 100;

pub struct ViewEngine {
    ai_id: String,
    view_dir: PathBuf,  // Directory for view files

    // Persistent state - simple file, NOT BTree
    cursor: u64,        // Persisted to {ai_id}.cursor (8 bytes)

    // Statistics (counters, ephemeral)
    stats: ViewStats,

    // Trust aggregation (ephemeral, rebuilt from events)
    ai_trust: HashMap<String, TrustScore>,

    // ============ Content Caches (All Ephemeral) ============

    /// Recent DMs received (ring buffer, newest at back)
    recent_dms: VecDeque<CachedDM>,

    /// Recent broadcasts per channel (ring buffer per channel)
    recent_broadcasts: HashMap<String, VecDeque<CachedBroadcast>>,

    /// Active dialogues with message history
    active_dialogues: HashMap<u64, DialogueState>,

    /// Pending/active tasks
    tasks: HashMap<u64, TaskState>,

    /// Recent file actions for stigmergy
    recent_file_actions: VecDeque<CachedFileAction>,
}

impl ViewEngine {
    /// Persist cursor to simple file (8 bytes)
    fn persist_cursor(&self) -> std::io::Result<()> {
        let cursor_path = self.view_dir.join(format!("{}.cursor", self.ai_id));
        let mut file = fs::File::create(&cursor_path)?;
        file.write_all(&self.cursor.to_le_bytes())?;
        file.sync_all()?;  // Ensure durability
        Ok(())
    }

    /// Load cursor from simple file
    fn load_cursor(&self) -> u64 {
        let cursor_path = self.view_dir.join(format!("{}.cursor", self.ai_id));
        let mut buf = [0u8; 8];
        fs::File::open(&cursor_path)
            .and_then(|mut f| f.read_exact(&mut buf))
            .map(|_| u64::from_le_bytes(buf))
            .unwrap_or(0)  // Start from 0 if no cursor file
    }
}
```

**Why no BTree?**
- Cursor is a single 8-byte value - BTree is overkill
- Simple file write is atomic on most filesystems (8 bytes < block size)
- No complex page allocation, shadow copies, or transaction overhead
- Faster, simpler, fewer dependencies

## Implementation

### 1. Extend apply_event()

```rust
pub fn apply_event(&mut self, event: &Event) -> ViewResult<()> {
    let header = &event.header;
    let source_ai = header.source_ai_str();

    match header.event_type {
        event_type::DIRECT_MESSAGE => {
            if let EventPayload::DirectMessage(payload) = &event.payload {
                // Update counter (existing)
                if payload.to_ai == self.ai_id && source_ai != self.ai_id {
                    self.stats.unread_dms += 1;
                }

                // NEW: Cache the message if it's TO me
                if payload.to_ai == self.ai_id {
                    self.recent_dms.push_back(CachedDM {
                        id: header.sequence,
                        from_ai: source_ai.to_string(),
                        content: payload.content.clone(),
                        timestamp: header.timestamp,
                        read: false,
                    });
                    // Maintain ring buffer size
                    while self.recent_dms.len() > MAX_CACHED_DMS {
                        self.recent_dms.pop_front();
                    }
                }
            }
        }

        event_type::BROADCAST => {
            if let EventPayload::Broadcast(payload) = &event.payload {
                // NEW: Cache broadcast
                let channel = payload.channel.clone();
                let broadcasts = self.recent_broadcasts
                    .entry(channel.clone())
                    .or_insert_with(VecDeque::new);

                broadcasts.push_back(CachedBroadcast {
                    id: header.sequence,
                    from_ai: source_ai.to_string(),
                    channel,
                    content: payload.content.clone(),
                    timestamp: header.timestamp,
                });

                while broadcasts.len() > MAX_CACHED_BROADCASTS_PER_CHANNEL {
                    broadcasts.pop_front();
                }
            }
        }

        event_type::DIALOGUE_START => {
            if let EventPayload::DialogueStart(payload) = &event.payload {
                // Update counter (existing)
                if source_ai == self.ai_id || payload.responder == self.ai_id {
                    self.stats.active_dialogues += 1;
                }

                // NEW: Create dialogue state
                if source_ai == self.ai_id || payload.responder == self.ai_id {
                    let mut messages = VecDeque::new();
                    messages.push_back(DialogueMessage {
                        sequence: header.sequence,
                        from_ai: source_ai.to_string(),
                        content: payload.topic.clone(),  // Topic as first message
                        timestamp: header.timestamp,
                    });

                    self.active_dialogues.insert(header.sequence, DialogueState {
                        id: header.sequence,
                        initiator: source_ai.to_string(),
                        responder: payload.responder.clone(),
                        topic: payload.topic.clone(),
                        status: "active".to_string(),
                        current_turn: payload.responder.clone(),
                        messages,
                        created_at: header.timestamp,
                        updated_at: header.timestamp,
                    });
                }
            }
        }

        event_type::DIALOGUE_RESPOND => {
            if let EventPayload::DialogueRespond(payload) = &event.payload {
                // NEW: Add message to dialogue
                if let Some(dialogue) = self.active_dialogues.get_mut(&payload.dialogue_id) {
                    dialogue.messages.push_back(DialogueMessage {
                        sequence: header.sequence,
                        from_ai: source_ai.to_string(),
                        content: payload.content.clone(),
                        timestamp: header.timestamp,
                    });

                    // Maintain message limit
                    while dialogue.messages.len() > MAX_DIALOGUE_MESSAGES {
                        dialogue.messages.pop_front();
                    }

                    // Update turn
                    dialogue.current_turn = if source_ai == dialogue.initiator {
                        dialogue.responder.clone()
                    } else {
                        dialogue.initiator.clone()
                    };
                    dialogue.updated_at = header.timestamp;
                }
            }
        }

        event_type::DIALOGUE_END => {
            if let EventPayload::DialogueEnd(payload) = &event.payload {
                // Update counter (existing)
                if self.stats.active_dialogues > 0 {
                    self.stats.active_dialogues -= 1;
                }

                // NEW: Mark dialogue as ended (keep in cache for reference)
                if let Some(dialogue) = self.active_dialogues.get_mut(&payload.dialogue_id) {
                    dialogue.status = payload.status.clone();
                    dialogue.updated_at = header.timestamp;
                }
            }
        }

        event_type::TASK_CREATE => {
            if let EventPayload::TaskCreate(payload) = &event.payload {
                // NEW: Add task to cache
                self.tasks.insert(header.sequence, TaskState {
                    id: header.sequence,
                    description: payload.description.clone(),
                    priority: payload.priority,
                    status: "pending".to_string(),
                    assignee: None,
                    created_at: header.timestamp,
                    updated_at: header.timestamp,
                });
            }
        }

        event_type::TASK_CLAIM => {
            if let EventPayload::TaskClaim(payload) = &event.payload {
                if let Some(task) = self.tasks.get_mut(&payload.task_id) {
                    task.status = "claimed".to_string();
                    task.assignee = Some(source_ai.to_string());
                    task.updated_at = header.timestamp;
                }

                // Update counter (existing)
                if source_ai == self.ai_id {
                    self.stats.my_tasks += 1;
                }
            }
        }

        event_type::TASK_COMPLETE => {
            if let EventPayload::TaskComplete(payload) = &event.payload {
                if let Some(task) = self.tasks.get_mut(&payload.task_id) {
                    task.status = "completed".to_string();
                    task.updated_at = header.timestamp;
                }

                // Update counter (existing)
                if source_ai == self.ai_id && self.stats.my_tasks > 0 {
                    self.stats.my_tasks -= 1;
                }
            }
        }

        event_type::FILE_ACTION => {
            if let EventPayload::FileAction(payload) = &event.payload {
                // NEW: Cache file action
                self.recent_file_actions.push_back(CachedFileAction {
                    id: header.sequence,
                    ai_id: source_ai.to_string(),
                    path: payload.path.clone(),
                    action: payload.action.clone(),
                    timestamp: header.timestamp,
                });

                while self.recent_file_actions.len() > MAX_CACHED_FILE_ACTIONS {
                    self.recent_file_actions.pop_front();
                }
            }
        }

        // ... existing handlers for other event types ...

        _ => {}
    }

    Ok(())
}
```

### 2. Add Query Methods to ViewEngine

```rust
impl ViewEngine {
    /// Get recent DMs (from cache, O(k))
    pub fn get_recent_dms(&self, limit: usize) -> Vec<&CachedDM> {
        self.recent_dms.iter().rev().take(limit).collect()
    }

    /// Get unread DMs (from cache, O(k))
    pub fn get_unread_dms(&self) -> Vec<&CachedDM> {
        self.recent_dms.iter().filter(|dm| !dm.read).collect()
    }

    /// Mark DM as read
    pub fn mark_dm_read(&mut self, dm_id: u64) {
        if let Some(dm) = self.recent_dms.iter_mut().find(|dm| dm.id == dm_id) {
            if !dm.read {
                dm.read = true;
                if self.stats.unread_dms > 0 {
                    self.stats.unread_dms -= 1;
                }
            }
        }
    }

    /// Get recent broadcasts (from cache, O(k))
    pub fn get_recent_broadcasts(&self, channel: &str, limit: usize) -> Vec<&CachedBroadcast> {
        self.recent_broadcasts
            .get(channel)
            .map(|bc| bc.iter().rev().take(limit).collect())
            .unwrap_or_default()
    }

    /// Get dialogue state (from cache, O(1))
    pub fn get_dialogue(&self, dialogue_id: u64) -> Option<&DialogueState> {
        self.active_dialogues.get(&dialogue_id)
    }

    /// Get dialogue messages (from cache, O(1))
    pub fn get_dialogue_messages(&self, dialogue_id: u64) -> Vec<&DialogueMessage> {
        self.active_dialogues
            .get(&dialogue_id)
            .map(|d| d.messages.iter().collect())
            .unwrap_or_default()
    }

    /// Get active dialogues for this AI
    pub fn get_my_dialogues(&self) -> Vec<&DialogueState> {
        self.active_dialogues.values()
            .filter(|d| d.status == "active" &&
                       (d.initiator == self.ai_id || d.responder == self.ai_id))
            .collect()
    }

    /// Get dialogues where it's my turn
    pub fn get_my_turn_dialogues(&self) -> Vec<&DialogueState> {
        self.active_dialogues.values()
            .filter(|d| d.status == "active" && d.current_turn == self.ai_id)
            .collect()
    }

    /// Get pending tasks (from cache)
    pub fn get_pending_tasks(&self) -> Vec<&TaskState> {
        self.tasks.values()
            .filter(|t| t.status == "pending")
            .collect()
    }

    /// Get task by ID (from cache, O(1))
    pub fn get_task(&self, task_id: u64) -> Option<&TaskState> {
        self.tasks.get(&task_id)
    }

    /// Get recent file actions (from cache, O(k))
    pub fn get_recent_file_actions(&self, limit: usize) -> Vec<&CachedFileAction> {
        self.recent_file_actions.iter().rev().take(limit).collect()
    }
}
```

### 3. Update v2_client.rs to Use ViewEngine

```rust
impl V2Client {
    /// Get recent DMs - NOW READS FROM VIEW, NOT EVENT LOG
    pub fn recent_dms(&mut self, limit: usize) -> V2Result<Vec<Message>> {
        self.sync()?;  // Ensures view is up to date

        Ok(self.view.get_recent_dms(limit)
            .into_iter()
            .map(|dm| Message {
                id: dm.id as i32,
                from_ai: dm.from_ai.clone(),
                to_ai: Some(self.ai_id.clone()),
                content: dm.content.clone(),
                message_type: MessageType::Direct,
                channel: String::new(),
                timestamp: timestamp_to_datetime(dm.timestamp),
            })
            .collect())
    }

    /// Get recent broadcasts - NOW READS FROM VIEW
    pub fn recent_broadcasts(&mut self, limit: usize, channel: Option<&str>) -> V2Result<Vec<Message>> {
        self.sync()?;

        let channel = channel.unwrap_or("general");
        Ok(self.view.get_recent_broadcasts(channel, limit)
            .into_iter()
            .map(|bc| Message {
                id: bc.id as i32,
                from_ai: bc.from_ai.clone(),
                to_ai: None,
                content: bc.content.clone(),
                message_type: MessageType::Broadcast,
                channel: bc.channel.clone(),
                timestamp: timestamp_to_datetime(bc.timestamp),
            })
            .collect())
    }

    /// Get dialogue messages - NOW READS FROM VIEW
    pub fn get_dialogue_messages(&mut self, dialogue_id: u64) -> V2Result<Vec<(u64, String, String, u64)>> {
        self.sync()?;

        Ok(self.view.get_dialogue_messages(dialogue_id)
            .into_iter()
            .map(|msg| (msg.sequence, msg.from_ai.clone(), msg.content.clone(), msg.timestamp))
            .collect())
    }

    // ... similar updates for other query methods ...
}
```

### 4. Startup Cache Rebuild (No BTree)

On ViewEngine open, caches are empty. Rebuild from recent events:

```rust
impl ViewEngine {
    pub fn open(ai_id: &str, data_dir: &Path) -> ViewResult<Self> {
        let view_dir = data_dir.join("views");
        fs::create_dir_all(&view_dir)?;

        let mut view = Self {
            ai_id: ai_id.to_string(),
            view_dir,
            cursor: 0,
            stats: ViewStats::default(),
            ai_trust: HashMap::new(),
            recent_dms: VecDeque::new(),
            recent_broadcasts: HashMap::new(),
            active_dialogues: HashMap::new(),
            tasks: HashMap::new(),
            recent_file_actions: VecDeque::new(),
        };

        // Load cursor from simple file (8 bytes)
        view.cursor = view.load_cursor();

        Ok(view)
    }

    /// Rebuild caches by replaying recent events
    /// Called after open() if caches need warming
    pub fn warm_cache(&mut self, event_log: &mut EventLogReader) -> ViewResult<()> {
        // Only replay last N events to populate caches
        const WARMUP_EVENTS: u64 = 10000;

        let start_seq = self.cursor.saturating_sub(WARMUP_EVENTS);
        if start_seq > 0 {
            event_log.seek_to_sequence(start_seq)?;
        }

        // Replay events to populate caches (but don't update cursor)
        while let Ok(Some(event)) = event_log.try_read() {
            if event.header.sequence > self.cursor {
                break;
            }
            self.apply_event(&event)?;
        }

        Ok(())
    }
}
```

## Performance Analysis

### Before (Current)
- `recent_dms(10)`: O(95,000) - scan entire event log
- `recent_broadcasts(10)`: O(95,000) - scan entire event log
- `get_dialogues()`: O(95,000) - scan entire event log
- Multiple queries compound: session-start = 5+ scans = O(475,000)

### After (With Caching)
- `recent_dms(10)`: O(10) - read from ring buffer
- `recent_broadcasts(10)`: O(10) - read from ring buffer
- `get_dialogues()`: O(active dialogues) - read from HashMap
- All queries: O(k) where k = items requested

### Memory Usage
- 100 DMs × ~1KB = ~100KB
- 100 broadcasts × 5 channels × ~1KB = ~500KB
- 100 dialogues × ~5KB (with messages) = ~500KB
- 100 tasks × ~500B = ~50KB
- 100 file actions × ~200B = ~20KB
- **Total: ~1.2MB per AI** (acceptable)

### Startup Cost
- Warm cache from last 10K events: ~50ms
- Acceptable tradeoff for O(1) queries during runtime

## Migration Path

1. **Phase 1**: Add cache data structures to ViewEngine (no behavior change)
2. **Phase 2**: Extend apply_event() to populate caches
3. **Phase 3**: Add query methods to ViewEngine
4. **Phase 4**: Update v2_client.rs to use ViewEngine queries
5. **Phase 5**: Remove old event log scanning code from v2_client.rs
6. **Phase 6**: Add cache warming on startup

Each phase can be deployed and tested independently. Backward compatible throughout.

## Testing Strategy

1. **Unit tests**: Verify cache population and eviction
2. **Integration tests**: Verify queries return correct data
3. **Performance tests**: Verify O(k) query time
4. **Stress tests**: Verify memory bounds with high event volume
5. **Multi-AI tests**: Verify per-AI isolation

## Conclusion

This design:
- Eliminates O(n) event log scans for all queries
- Uses industry-standard materialized view pattern
- Maintains event sourcing principles (log = truth, view = cache)
- Bounds memory usage with ring buffers
- Is backward compatible and incrementally deployable
- Respects the quality standards expected for AI infrastructure

**This is the proper fix, not a workaround.**
