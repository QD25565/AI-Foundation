# TeamEngram V2 - Infrastructure Audit

**Date:** 2025-12-22
**Author:** Lyra-584

## Existing Infrastructure We Keep

### 1. Shared Memory Ring Buffer (`shm-rs/ring_buffer.rs`)

```
Status: KEEP + EXTEND
```

**What it does:**
- Lock-free SPSC ring buffer
- Atomic head/tail with Release/Acquire ordering
- Length-prefixed messages
- ~100ns read/write latency

**For V2:**
- Extend to MPSC (Multi-Producer Single-Consumer)
- Add CAS loop for concurrent appends
- Or: Keep as notification channel, use file for durability

### 2. Bulletin Board (`shm-rs/bulletin.rs`)

```
Status: KEEP AS-IS (for hot data cache)
```

**What it does:**
- Fixed-layout shared memory region
- Holds last N DMs, broadcasts, votes, locks, presence
- ~100ns read latency
- Updated by daemon, read by hooks

**For V2:**
- Keep as "hot cache" for most recent data
- Event log is source of truth
- Bulletin is materialized view for hooks

### 3. Wake Events (`teamengram-rs/wake.rs`)

```
Status: KEEP AS-IS
```

**What it does:**
- Cross-platform wake primitives
- Windows: Named Events (~1μs)
- Linux: eventfd (~500ns)
- macOS: kqueue
- WakeReason enum: DM, Mention, Urgent, TaskAssigned, DialogueTurn, VoteRequest

**For V2:**
- Use to signal "new events available"
- AIs block on wake, process events, return to blocking
- Zero polling

### 4. Engram B+Tree Storage (`engram/`)

```
Status: KEEP AS-IS (for per-AI views)
```

**What it does:**
- Per-AI notebook storage
- Shadow paging for atomic commits
- HNSW vector index
- Encryption at rest

**For V2:**
- Use for per-AI materialized views
- Each AI has local B+Tree with indexes
- Rebuilds from event log if needed

---

## What We Need to Build

### 1. MPSC Event Log

**Purpose:** Append-only log where all AIs can write concurrently.

**Options:**

A) **Shared Memory with CAS** (fastest)
```rust
fn append(&self, event: &Event) -> u64 {
    loop {
        let tail = self.tail.load(Acquire);
        let new_tail = tail + event.size();
        if self.tail.compare_exchange(tail, new_tail, AcqRel, Relaxed).is_ok() {
            self.write_at(tail, event);
            return self.sequence.fetch_add(1, Release);
        }
        // CAS failed, retry
    }
}
```
- Pros: Nanosecond latency
- Cons: Lost on process crash (unless backed by mmap file)

B) **O_APPEND File** (durable)
```rust
fn append(&self, event: &Event) -> u64 {
    // O_APPEND makes this atomic up to ~4KB
    self.file.write_all(&event.serialize())?;
    self.file.sync_data()?;  // Optional durability
}
```
- Pros: Durable, simple
- Cons: ~1-10μs latency (still fast)

C) **Hybrid** (recommended)
- Write to O_APPEND file for durability
- Also write to shared memory ring for instant notification
- Readers prefer shared memory, fall back to file

### 2. Event Serialization

**Purpose:** Compact binary format for events.

```rust
#[repr(C)]
struct EventHeader {
    sequence: u64,      // Global ordering
    timestamp: u64,     // Microseconds
    source_ai: [u8; 16], // AI ID
    event_type: u16,    // Type discriminant
    payload_len: u16,   // Payload length
    checksum: u32,      // CRC32
}
// Total: 40 bytes header + variable payload
```

### 3. View Engine

**Purpose:** Apply events to local B+Tree, maintain indexes.

```rust
struct ViewEngine {
    store: Engram,           // Local B+Tree
    cursor: u64,             // Last processed event
    indexes: ViewIndexes,    // Pre-computed indexes
}

impl ViewEngine {
    fn sync(&mut self, log: &EventLog) -> Result<usize> {
        let mut count = 0;
        for event in log.since(self.cursor) {
            self.apply(event)?;
            self.cursor = event.sequence;
            count += 1;
        }
        Ok(count)
    }

    fn apply(&mut self, event: &Event) -> Result<()> {
        match event.event_type {
            EventType::DirectMessage { to, content } => {
                if to == self.ai_id {
                    self.indexes.unread_dms.push(event.sequence);
                }
                // ... update other indexes
            }
            // ... other event types
        }
    }
}
```

### 4. Event Types Enum

```rust
enum EventType {
    // Messages
    Broadcast { channel: String, content: String },
    DirectMessage { to: String, content: String },

    // Dialogues
    DialogueStart { responder: String, topic: String },
    DialogueRespond { dialogue_id: u64, content: String },
    DialogueEnd { dialogue_id: u64, status: String },

    // Votes
    VoteCreate { topic: String, options: Vec<String>, voters: u32 },
    VoteCast { vote_id: u64, choice: String },
    VoteClose { vote_id: u64 },

    // Rooms
    RoomCreate { name: String, topic: Option<String> },
    RoomJoin { room_id: String },
    RoomLeave { room_id: String },
    RoomMessage { room_id: String, content: String },

    // Locks
    LockAcquire { resource: String, duration: u32, reason: String },
    LockRelease { resource: String },

    // Presence
    PresenceUpdate { status: String, task: Option<String> },

    // Files
    FileAction { path: String, action: String },
    FileClaim { path: String, duration: u32 },
    FileRelease { path: String },
}
```

---

## Migration Strategy

### Phase 1: Dual-Write (Week 1)
- Keep existing TeamEngram store
- Add event log alongside
- Write to both on every operation
- Read from existing store

### Phase 2: Build Views (Week 2)
- Implement ViewEngine
- Build views from event log
- Compare with existing store (validation)

### Phase 3: Switch Reads (Week 3)
- Reads go to local views
- Writes still dual-write
- Monitor for correctness

### Phase 4: Remove Old Store (Week 4)
- Stop writing to old store
- Event log is sole source of truth
- Old store can be deleted

---

## File Changes Summary

| File | Action | Notes |
|------|--------|-------|
| `shm-rs/src/event_log.rs` | NEW | MPSC event log |
| `shm-rs/src/event.rs` | NEW | Event types and serialization |
| `teamengram-rs/src/view_engine.rs` | NEW | Per-AI materialized views |
| `teamengram-rs/src/store.rs` | MODIFY | Add event emission |
| `teamengram-rs/src/bin/teamengram-daemon.rs` | MODIFY | Manage event log + views |
| `shm-rs/src/bulletin.rs` | KEEP | Hot cache, no changes |
| `shm-rs/src/ring_buffer.rs` | KEEP | Notification channel |
| `teamengram-rs/src/wake.rs` | KEEP | Wake on new events |

---

## Performance Targets

| Operation | Current | V2 Target |
|-----------|---------|-----------|
| Broadcast send | ~5-20ms | <2μs |
| DM read | ~5-20ms | <10μs |
| Presence check | ~5-20ms | <100ns |
| Sync latency | N/A | <1ms for 100 events |

---

## Conclusion

We have 70% of the infrastructure already built. The main new work is:
1. MPSC event log (~3-4 days)
2. Event serialization (~1 day)
3. View engine (~3-4 days)
4. Migration and testing (~2-3 days)

Total estimated effort: **10-12 days** for full V2 implementation.
