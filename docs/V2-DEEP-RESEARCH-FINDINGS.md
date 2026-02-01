# V2 Architecture Deep Research Findings

**Date:** Jan 31, 2026
**Researchers:** 4 Opus 4.5 Sub-Agents
**Problem:** O(n) scanning on every query with 95K+ events

---

## Executive Summary

Four Opus 4.5 agents performed independent deep research into:
1. **Current Codebase Analysis** - What's implemented vs. what's missing
2. **Event Sourcing Best Practices** - Kafka, EventStoreDB, Axon, CQRS patterns
3. **Time-Series & Log-Structured Storage** - LSM trees, Prometheus, InfluxDB, Kafka segments
4. **Index Structures & Algorithms** - Skip lists, bloom filters, sparse indexes, inverted indexes

**Consensus Finding:** The infrastructure EXISTS but is UNUSED. Checkpoints are written every 10,000 events but never used for queries. ViewEngine syncs correctly but only stores counts. All query methods create new readers starting at position 0.

---

## 1. Codebase Analysis (Agent 3)

### What EXISTS
- **Checkpoints**: Written every 10,000 events (event_log.rs lines 320-396)
- **seek_to_sequence()**: Method exists to jump to any sequence (event_log.rs lines 472-499)
- **ViewEngine.sync()**: Uses cursor + seek correctly for incremental updates
- **Outboxes**: Wait-free ring buffers work correctly
- **Sequencer**: Event-driven, no polling

### What's MISSING
- **ViewEngine only stores COUNTS**:
```rust
pub struct ViewStats {
    pub cursor: u64,           // ← Correct: tracks position
    pub unread_dms: u64,       // ← Problem: just a counter
    pub active_dialogues: u64, // ← Problem: just a counter
    // ... all counters, no content
}
```

- **Every query creates new reader at position 0**:
```rust
// v2_client.rs pattern in ALL query methods:
let mut temp_reader = EventLogReader::open(Some(&self.base_dir))?;  // STARTS AT 0!
loop {
    let event = temp_reader.try_read()?;  // SCANS EVERYTHING
    if matches_query(event) { ... }
}
```

### The Fix Gap
ViewEngine.apply_event() SEES every event but throws away content:
```rust
event_type::DIRECT_MESSAGE => {
    if payload.to_ai == self.ai_id {
        self.stats.unread_dms += 1;  // Just increment counter, discard message!
    }
}
```

---

## 2. Event Sourcing Best Practices (Agent 1)

### Industry Standard Pattern
```
Event Log → Subscription → Projection Handler → Read Model
                ↑
            Checkpoint Store
```

**Key Insight:** The event log is for WRITES. Read models (projections) are for QUERIES.

### Kafka Pattern
- Consumers track position via **offset** (simple integer)
- Resume from any offset - never scan from beginning
- Log compaction removes obsolete records while preserving offsets

### EventStoreDB Pattern
- **Catch-up Subscriptions**: Consumer tells server where to start
- **Persistent Subscriptions**: Server-side position tracking
- Indexes stored separately: stream hash → physical offset

### Axon Pattern (Snapshots)
- Snapshots store aggregate state at a version
- Reload: find snapshot → apply only events after snapshot
- O(k) where k = events since snapshot, not O(n) total

### CQRS Principle
```
WRITE: Commands → Event Log (optimized for sequential writes)
READ:  Queries → Projections (optimized for specific queries)
```
Projections are updated incrementally as events arrive.

---

## 3. Time-Series & Log-Structured Storage (Agent 2)

### LSM Tree Pattern
```
MemTable (RAM) → SSTable (L0) → Compaction → L1 → L2...
```
- Writes go to memory first (fast)
- Periodic flush to sorted immutable files
- Bloom filters prevent unnecessary disk reads

**Amplification Trade-offs:**
| Strategy | Write Amplification | Read Amplification | Space Amplification |
|----------|--------------------:|-------------------:|--------------------:|
| Size-Tiered | Low (~O(log N)) | High | High (O(T)) |
| Leveled | High (~11x) | Low | Low (<12%) |

### Prometheus Pattern
- 2-hour blocks, merged into larger time windows (6h → 18h → 54h)
- Block contains: chunks + index + metadata + tombstones
- Time-based retention: delete entire blocks older than X

### Kafka Segment Pattern
```
partition-0/
├── 00000000.log        # Records 0-999
├── 00000000.index      # Sparse: offset → byte position (every 4KB)
├── 00000000.timeindex  # Sparse: timestamp → offset
├── 00001000.log        # Records 1000-1999
└── ...
```
- **Sparse index**: One entry every 4KB, not every record
- **Lookup**: Binary search index → seek to offset → linear scan
- **Retention**: Delete entire segments older than threshold

### Recommended Architecture
```
┌─────────────────────────────────────────────────────────────────────┐
│                        EVENT LOG SYSTEM                              │
├─────────────────────────────────────────────────────────────────────┤
│  WRITE PATH:                                                         │
│  Event → Append to Active Segment → If full: Close + Create new     │
│                                                                      │
│  READ PATH:                                                          │
│  Query → Find relevant segments → Binary search sparse index         │
│        → Seek to offset → Linear scan to results                     │
│                                                                      │
│  STORAGE:                                                            │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐               │
│  │ Segment  │ │ Segment  │ │ Segment  │ │ Active   │               │
│  │ T-6 hrs  │ │ T-4 hrs  │ │ T-2 hrs  │ │ Segment  │               │
│  │ (closed) │ │ (closed) │ │ (closed) │ │ (open)   │               │
│  ├──────────┤ ├──────────┤ ├──────────┤ ├──────────┤               │
│  │ .log     │ │ .log     │ │ .log     │ │ .log     │               │
│  │ .idx     │ │ .idx     │ │ .idx     │ │ (in-mem) │               │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘               │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 4. Index Structures & Algorithms (Agent 4)

### Skip Lists (O(log n) search)
- Probabilistic data structure with hierarchical linked lists
- Each level "skips" more elements
- Simpler than balanced trees, excellent for in-memory indexing
- Good for sequence-number navigation

### Bloom Filters (O(1) "might contain")
- Probabilistic: "definitely not in set" or "possibly in set"
- 10 bits/element = ~1% false positive rate
- 100K events × 10 bits = 125 KB per bloom filter
- Use for: "Skip this segment - definitely no DMs for AI X"

### Sparse Indexes (Kafka pattern)
- Index every Nth entry (N=100 or every 4KB)
- Binary search → seek → linear scan
- 1M events with N=100 = 10,000 index entries = 80 KB

### Inverted Indexes (for entity queries)
```rust
dm_index: HashMap<AIId, Vec<SequenceNumber>>

// "Last 10 DMs to AI X" becomes:
dm_index["AI_X"].iter().rev().take(10)  // O(1) lookup + O(k) iteration
```

### Recommended Three-Layer Index
```
┌─────────────────┬──────────────────┬────────────────────────┐
│  Sparse Index   │  Inverted Index  │  Bloom Filters         │
│  (sequence→off) │  (entity→seqs)   │  (segment membership)  │
├─────────────────┴──────────────────┴────────────────────────┤
│                    APPEND-ONLY LOG                           │
└──────────────────────────────────────────────────────────────┘
```

---

## 5. Concrete Recommendations

### Option A: Minimum Viable Fix (Fastest)

**Enhance ViewEngine to cache content** (not just counts):

```rust
pub struct EnhancedViewEngine {
    cursor: u64,
    // Content caches (ring buffers, bounded)
    recent_dms: VecDeque<(u64, String, String, String)>,     // seq, from, to, content
    recent_broadcasts: VecDeque<(u64, String, String, String)>, // seq, from, channel, content
    // State caches
    dialogues: HashMap<u64, DialogueState>,
    tasks: HashMap<u64, TaskState>,
}

impl EnhancedViewEngine {
    fn apply_event(&mut self, event: &Event) {
        match event.header.event_type {
            DIRECT_MESSAGE => {
                // Keep last 100 DMs in ring buffer
                if self.recent_dms.len() >= 100 {
                    self.recent_dms.pop_front();
                }
                self.recent_dms.push_back((seq, from, to, content));
            }
            // ... similar for other types
        }
    }

    fn get_recent_dms(&self, limit: usize) -> Vec<DM> {
        // O(k) - just iterate ring buffer
        self.recent_dms.iter().rev().take(limit).collect()
    }
}
```

**Effort:** ~4-8 hours
**Impact:** Queries become O(1) or O(k) instead of O(n)

### Option B: Full CQRS with Projections

Separate read models maintained incrementally:

```rust
// In-memory projection for DM inbox
pub struct DMInboxProjection {
    inboxes: HashMap<String, VecDeque<DMRecord>>,  // ai_id → recent DMs
    checkpoint: u64,
}

impl Projection for DMInboxProjection {
    fn apply(&mut self, event: &Event) {
        if let EventPayload::DirectMessage(dm) = &event.payload {
            let inbox = self.inboxes.entry(dm.to_ai.clone()).or_default();
            if inbox.len() >= 100 { inbox.pop_front(); }
            inbox.push_back(DMRecord::from(event));
        }
    }
}
```

**Effort:** ~8-16 hours
**Impact:** Full O(1) queries, proper separation of concerns

### Option C: Segmented Log with Indexes (Production-Grade)

Like Kafka/Prometheus:
- Segment by time (2-hour windows)
- Sparse index per segment
- Optional bloom filters for entity filtering

**Effort:** ~2-4 weeks
**Impact:** Scales to millions of events, proper retention

---

## 6. Priority Order

1. **Immediate** (Day 1): Fix the query methods to use seek instead of position 0
   - Use existing checkpoints: `seek_to_sequence(head - 20000)` before scanning
   - 5x improvement immediately

2. **Short Term** (Week 1): Enhance ViewEngine to cache content
   - Add ring buffers for recent DMs, broadcasts
   - Add HashMaps for dialogue/task state
   - Rebuild on startup from last 10K events

3. **Medium Term** (Month 1): Segment the log
   - Close segments every 2 hours
   - Add sparse indexes
   - Delete/archive segments older than N days

4. **Long Term**: Full CQRS if needed
   - Only if queries become more complex
   - If multiple consumers need different views

---

## 7. Academic Sources

### Foundational Papers
- **LSM-Tree (1996)**: O'Neil et al. "The Log-Structured Merge-Tree" Acta Informatica
- **Skip Lists (1990)**: Pugh, W. "Skip lists: a probabilistic alternative to balanced trees" CACM
- **Bloom Filters (1970)**: Bloom, B.H. "Space/time trade-offs in hash coding with allowable errors" CACM
- **DBSP (2024)**: "Incremental Computation on Streams and Its Applications to Databases" ACM SIGMOD

### Industry Resources
- [The Log: What every software engineer should know](https://engineering.linkedin.com/distributed-systems/log-what-every-software-engineer-should-know-about-real-time-datas-unifying) - Jay Kreps
- [Kafka Design](https://kafka.apache.org/documentation/#design)
- [EventStoreDB Indexing](https://developers.eventstore.com/server/v5/indexes)
- [Prometheus TSDB](https://prometheus.io/docs/prometheus/latest/storage/)
- [RocksDB Compaction](https://github.com/facebook/rocksdb/wiki/Compaction)

---

## 8. Summary

**The problem is clear:** Every query scans from position 0 because ViewEngine only stores counts.

**The infrastructure exists:** Checkpoints, seek_to_sequence(), cursor tracking - all implemented but unused for queries.

**The fix is straightforward:**
1. Use existing seek in query methods (immediate 5x gain)
2. Cache content in ViewEngine (proper O(1) queries)

**This is not novel:** Kafka, EventStoreDB, Prometheus, InfluxDB all solved this same problem with:
- Sparse indexes for offset navigation
- In-memory caches for recent data
- Time-based segmentation for retention

The V2 architecture is sound. The implementation just needs to use the infrastructure it already has.
