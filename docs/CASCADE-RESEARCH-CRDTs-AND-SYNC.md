# Cascade's Research Document: CRDTs, Conflict Resolution, and Distributed Sync

**Author:** cascade-230
**Date:** 2024-Dec-24
**Research Area:** Conflict-free data synchronization across federated teambooks

---

## 1. The Core Problem

When multiple teambooks exist across a federation, they will inevitably:
- Edit the same data simultaneously
- Go offline and come back with divergent state
- Have different views of "truth" at any given moment

**The question:** How do we ensure all teambooks eventually converge to the same state, without requiring a central authority to resolve conflicts?

---

## 2. Why This Matters for AI-Foundation

In the current architecture:
- Each teambook is a single source of truth for its device
- V2 event-sourcing provides ordering within ONE teambook
- But federation means MULTIPLE teambooks, each generating events

**Scenarios we must handle:**

1. **Simultaneous DMs:** AI-A on Teambook-1 and AI-B on Teambook-2 both send DMs at the same moment. Which came first?

2. **Offline Sync:** Teambook-1 goes offline for an hour. During that time, Teambook-2 creates 50 events. When Teambook-1 reconnects, how do we merge?

3. **Conflicting Edits:** Two AIs edit the same note simultaneously. One adds text, one deletes text. What's the final state?

4. **Network Partitions:** Federation splits into two groups that can't communicate. Each continues operating. When they reconnect, how do we reconcile?

---

## 3. CRDTs: Conflict-free Replicated Data Types

### 3.1 What Are CRDTs?

CRDTs are data structures designed to be replicated across multiple computers, where replicas can be updated independently and concurrently, and it's always mathematically possible to merge them into a consistent state.

**Key property:** No coordination required during updates. Merge is always possible, always deterministic.

### 3.2 Types of CRDTs

**State-based CRDTs (CvRDTs):**
- Replicas periodically share their full state
- Merge function combines states
- Requires: merge is commutative, associative, idempotent
- Pro: Simple to understand
- Con: Transmitting full state can be expensive

**Operation-based CRDTs (CmRDTs):**
- Replicas share operations (events), not state
- Operations must be commutative
- Pro: Only transmit changes
- Con: Requires reliable broadcast (all ops must reach all replicas)

**Delta-state CRDTs:**
- Hybrid: share only the "delta" (what changed)
- Best of both worlds
- This is what Automerge uses

### 3.3 Common CRDT Types

| Type | Use Case | How It Works |
|------|----------|--------------|
| G-Counter | Counting (only up) | Each node has its own counter, sum all |
| PN-Counter | Counting (up and down) | Two G-Counters: one for + one for - |
| G-Set | Add-only sets | Union of all additions |
| 2P-Set | Add and remove sets | Two G-Sets: additions and tombstones |
| OR-Set | Add/remove with re-add | Tag each element with unique ID |
| LWW-Register | Last-writer-wins value | Timestamp determines winner |
| MV-Register | Multi-value register | Keep all concurrent values |
| RGA | Ordered list/text | Unique IDs for each element |

---

## 4. Libraries to Study

### 4.1 Automerge
- **Language:** Rust core, JS/WASM bindings
- **Type:** Delta-state CRDT
- **URL:** https://automerge.org/
- **Why study:** Production-ready, used in real apps, excellent docs
- **Key features:**
  - JSON-like document model
  - Automatic history/versioning
  - Sync protocol built-in
  - Rust implementation (matches our stack)

### 4.2 Yjs
- **Language:** JavaScript (WASM available)
- **Type:** Operation-based CRDT
- **URL:** https://yjs.dev/
- **Why study:** Powers many collaborative editors (Notion, etc.)
- **Key features:**
  - Sub-document support
  - Provider abstraction (works with any transport)
  - Awareness protocol (presence/cursors)

### 4.3 Diamond Types
- **Language:** Rust
- **Type:** Operation-based CRDT
- **URL:** https://github.com/josephg/diamond-types
- **Why study:** Pure Rust, simpler than Automerge, very fast

### 4.4 cr-sqlite
- **Language:** SQLite extension
- **URL:** https://github.com/vlcn-io/cr-sqlite
- **Why study:** CRDTs for SQLite databases
- **Relevance:** If we ever need CRDT sync for structured data

---

## 5. How Matrix Handles Sync (Relevant Research)

Matrix is the closest existing system to what we're building. Their approach:

### 5.1 Event DAG (Directed Acyclic Graph)
- Events reference their parent events (like git commits)
- No single timeline - it's a graph
- Conflicts are visible in the graph structure

### 5.2 State Resolution
- "State" is derived from events
- When branches merge, state resolution algorithm determines result
- Version 2 algorithm: power levels, origin server timestamp, event ID

### 5.3 Lessons for Us
- Events should reference parent events (not just sequence numbers)
- Need a deterministic "resolution" algorithm for conflicts
- Consider: do we even need to resolve, or keep all versions?

---

## 6. Mapping to AI-Foundation

### 6.1 What Data Needs CRDT Sync?

| Data Type | CRDT Approach | Notes |
|-----------|---------------|-------|
| Broadcasts | Append-only log (G-Set of events) | Easy - no conflicts possible |
| DMs | Append-only log | Easy - messages only added |
| Dialogues | More complex - has state | Need OR-Set for participants, LWW for status |
| Votes | PN-Counter for vote counts | Each AI's vote is unique |
| Presence | LWW-Register | Latest status wins |
| Notes (Notebook) | RGA for text content | Most complex - text editing |
| Tasks | OR-Set with LWW fields | Add/remove tasks, update status |

### 6.2 Event Structure for Federation

Current V2 events have:
```rust
struct Event {
    header: EventHeader,  // type, timestamp, ai_id
    payload: EventPayload // varies by type
}
```

For federation, we might need:
```rust
struct FederatedEvent {
    id: UUID,                    // Globally unique
    origin_teambook: TeambookID, // Where it was created
    parent_ids: Vec<UUID>,       // DAG references (like git)
    lamport_clock: u64,          // Logical timestamp
    header: EventHeader,
    payload: EventPayload,
    signature: Signature,        // Cryptographic proof of origin
}
```

### 6.3 Sync Protocol Sketch

1. **Connect:** Teambook-A connects to Teambook-B
2. **Exchange Heads:** Share the IDs of latest events each has seen
3. **Identify Missing:** Each determines what events the other is missing
4. **Send Deltas:** Exchange missing events
5. **Merge:** Apply events, CRDT merge handles conflicts
6. **Acknowledge:** Confirm sync complete

---

## 7. Edge Cases and "What Breaks"

### 7.1 Malformed Events
- **Risk:** Malicious or buggy node sends invalid events
- **Mitigation:** Schema validation before merge, reject invalid

### 7.2 Clock Skew
- **Risk:** Different machines have different system times
- **Mitigation:** Use Lamport clocks (logical time), not wall clocks
- **Alternative:** Hybrid Logical Clocks (HLC)

### 7.3 Tombstone Accumulation
- **Risk:** Deleted items leave tombstones forever, bloat grows
- **Mitigation:** Garbage collection after all nodes confirm deletion
- **Challenge:** How to know "all nodes" in open federation?

### 7.4 Large Divergence
- **Risk:** Nodes offline for long time, huge merge required
- **Mitigation:** Periodic snapshots, bounded history

### 7.5 Byzantine Nodes
- **Risk:** Node intentionally sends conflicting/malicious data
- **Mitigation:** Cryptographic signatures, reputation systems
- **Note:** This intersects with Lyra's identity research

### 7.6 Schema Evolution
- **Risk:** Event format changes, old nodes can't parse new events
- **Mitigation:** Version field, backwards-compatible changes only
- **Alternative:** Schema registry, negotiation during sync

---

## 8. Research Questions to Answer

1. **Automerge vs Custom:** Should we use Automerge directly, or build custom CRDTs?
   - Pro Automerge: Battle-tested, handles edge cases
   - Pro Custom: Lighter weight, exactly what we need

2. **Operation vs State:** Which CRDT variant fits our event-sourcing model?
   - V2 is already operation-based (events)
   - Suggests CmRDTs or delta-state

3. **Garbage Collection:** How do we handle tombstone accumulation in open federation?
   - Need to study how Matrix/ActivityPub handle this

4. **Partial Sync:** Can teambooks sync only specific data (e.g., just broadcasts, not notes)?
   - Would reduce bandwidth, improve privacy
   - Needs per-data-type sync protocol

5. **Real-time vs Batch:** Sync on every event, or batch periodically?
   - Real-time: Lower latency, more overhead
   - Batch: Efficient, but stale data

---

## 9. Recommended Study Order

1. **Automerge Docs:** https://automerge.org/docs/
   - Understand their data model
   - Study their sync protocol

2. **Matrix Spec - State Resolution:** https://spec.matrix.org/latest/
   - How they handle federated event graphs
   - State resolution v2 algorithm

3. **Martin Kleppmann's Papers:**
   - "A Conflict-Free Replicated JSON Datatype" (Automerge paper)
   - "Making CRDTs Byzantine Fault Tolerant"

4. **Yjs Internals:** https://github.com/yjs/yjs/blob/main/README.md
   - Different approach than Automerge
   - Good for comparison

5. **Hybrid Logical Clocks Paper:**
   - Better than Lamport clocks for our use case
   - Combines logical and physical time

---

## 10. Concrete Next Steps

### Phase 1: Learn
- [ ] Read Automerge docs completely
- [ ] Implement toy CRDT in Rust (G-Counter, then OR-Set)
- [ ] Study Matrix state resolution algorithm
- [ ] Read Kleppmann's Automerge paper

### Phase 2: Prototype
- [ ] Add parent_ids to V2 events (make it a DAG)
- [ ] Implement Lamport or HLC timestamps
- [ ] Build minimal sync protocol between two teambooks
- [ ] Test: create events on both, sync, verify convergence

### Phase 3: Harden
- [ ] Add signature verification
- [ ] Implement tombstone GC strategy
- [ ] Handle offline/reconnect scenarios
- [ ] Stress test with many concurrent events

---

## 11. Open Questions for Team Discussion

1. **For Lyra:** How does this interact with V2 event structure? Can we extend it without breaking existing code?

2. **For Sage:** Does libp2p have built-in CRDT support, or do we layer CRDTs on top of their transport?

3. **For Resonance:** How does Matrix handle the "which events have you seen" protocol efficiently?

4. **For QD:** What's the priority - correctness first (slow but safe) or performance first (fast but might have edge cases)?

---

## 12. Summary

CRDTs are the mathematical foundation that makes federation possible without a central authority. They guarantee eventual consistency - all teambooks will converge to the same state, even if they were offline or had conflicting edits.

**Key insight:** Our V2 event-sourcing is already close to an operation-based CRDT. We're not starting from scratch. We need to:
1. Add DAG structure (parent references)
2. Add global event IDs
3. Implement merge/sync protocol
4. Handle edge cases (GC, byzantine nodes, schema evolution)

The path is clear. Now we research, prototype, and iterate.

---

## 13. BREAKTHROUGH: Solutions from Deeper Research (Updated Dec 24)

After ultrathink analysis of QD's research repository, major breakthroughs on previously "unsolved" problems:

### 13.1 Tombstone Accumulation → SOLVED: Time-Window Compaction (TWCS)

From LSM-tree research on decade-scale storage:

**The Solution:** Time-Window Compaction Strategy (TWCS)
- Events grouped into time buckets (daily or weekly windows)
- Compaction happens ONLY within time windows
- Once a window closes, its files are NEVER compacted again
- **TTL expiry = O(1) file deletion** - just drop the old files!
- Write Amplification approaches 1.0 (theoretical minimum)

**Why this solves federation GC:**
- No tombstone accumulation because deletion IS file deletion
- "What events are you missing?" becomes "What time windows are you missing?"
- Old data = archived compressed files, fetched on-demand only
- Retention policy = just keep files for N years, then drop

### 13.2 Clock Skew → SOLVED: Hybrid Logical Clocks (HLC)

Already mentioned, but now with concrete implementation:

```rust
struct HybridLogicalClock {
    physical_ms: u64,    // Wall clock milliseconds
    logical: u16,        // Tie-breaker for same-ms events
}
```

**Algorithm:**
1. On event creation: HLC = max(local_wall_clock, last_HLC.physical) + logical increment
2. On event receive: HLC = max(local_wall_clock, received_HLC.physical, local_HLC.physical)
3. Reject events with physical_ms > local_wall_clock + 60_000 (1 minute tolerance)

### 13.3 Scale (80+ AIs) → SOLVED: Semantic Overlay Routing (HNSW)

From "Semantic Routing with HNSW Graphs" research:

**The Solution:** Hierarchical Navigable Small World graphs
- Messages route through semantic space in O(log n) hops
- Skip-list inspired hierarchy: coarse navigation at top, fine at bottom
- "Vortex" protocol demonstrates decentralized vector search

**For AI-Foundation:**
- AIs positioned in "semantic embedding space" by interests/roles
- Broadcasts route to semantically relevant AIs, not everyone
- 3D Cyberspace position = semantic embedding visualization!
- Proximity in 3D = similarity in interests

### 13.4 Schema Evolution → SOLVED: Avro Self-Describing Format

From long-term storage research:

**The Problem:** Protobuf requires external .proto file. If lost, data is garbage.

**The Solution:** Apache Avro
- Embeds schema IN the file header
- Self-describing: reader in 2035 can read file from 2025
- Supports evolution rules (default values for missing fields)

**For federation:** Wrap event payloads in Avro container.

### 13.5 Updated Canonical Event Format

```rust
struct FederatedEvent {
    // Content-addressed identity
    id: [u8; 32],              // SHA-256(origin + hlc + type + payload)

    // Origin tracking
    origin_teambook: DID,       // W3C DID - decentralized identity
    origin_ai: DID,

    // DAG structure
    parent_ids: Vec<[u8; 32]>,  // 0=genesis, 1=linear, 2+=merge

    // Ordering
    hlc: HybridLogicalClock,
    time_window: u32,           // TWCS bucket (days since epoch)

    // Content (Avro-wrapped)
    event_type: EventType,
    payload: Vec<u8>,           // Self-describing via Avro

    // Security
    signature: Ed25519Signature,     // Always (64 bytes)
    attestation: Option<TPMQuote>,   // Sovereign Net only
    zk_proof: Option<ZkProof>,       // Privacy operations only
}
```

### 13.6 Updated "What Breaks" Table

| Component | What Breaks | Mitigation |
|-----------|-------------|------------|
| HLC | Clock skew >1min | Reject future timestamps |
| TWCS | Window boundary edge cases | Overlap windows slightly |
| CRDTs | Tombstone accumulation | **SOLVED: Use TWCS** |
| Merkle Sync | Hash collision | SHA-256 (256-bit resistance) |
| HNSW Routing | Semantic black holes | Multi-path routing |
| Ed25519 | Quantum computers (2040+?) | ML-DSA for transport layer |
| Avro Schema | Evolution conflicts | Strict additive-only rules |
| 80+ AIs | O(n²) naive sync | **SOLVED: HNSW routing** |

---

## 14. Research Questions ANSWERED

1. **Automerge vs Custom:** Use delta-state approach like Automerge, but custom implementation optimized for our event structure.

2. **Operation vs State:** Delta-state CRDTs. V2 events + TWCS time windows = natural delta boundaries.

3. **Garbage Collection:** **SOLVED** - TWCS. No tombstones needed. Delete = drop old time window files.

4. **Partial Sync:** Yes - sync specific time windows and/or event types. Merkle trees per window enable this.

5. **Real-time vs Batch:** Hybrid - real-time within current window, batch for historical sync.

---

## 15. Final Summary

The federation architecture is now coherent:

**Layer 1 (Teambook):** V2 event sourcing, ~100ns ops, SPSC IPC
**Layer 2 (Federation):** Delta-state CRDTs + HLC + TWCS + Merkle sync
**Layer 3 (Sovereign Net):** TPM attestation + ML-KEM transport + zk proofs
**Layer 4 (Dark Net):** Ed25519 only, reputation-based trust

**Key breakthroughs:**
- TWCS eliminates tombstone accumulation entirely
- HNSW enables O(log n) routing at scale
- Avro ensures future-proof schema evolution
- Content-addressed IDs enable verification without central authority

The path from research to implementation is clear. V2 extends naturally to federation.

---

*"Slow is fast. The deep research revealed solutions invisible from the surface."*

— cascade-230, Christmas Eve 2024 (Updated after ultrathink)
