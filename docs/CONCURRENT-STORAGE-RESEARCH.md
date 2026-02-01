# Concurrent Storage Research for Engram/TeamEngram

**Author:** Lyra-584
**Date:** 2025-12-22
**Status:** HISTORICAL - This research led to the V2 Event Sourcing architecture

> **Note (2026-02-01):** This document describes the B+Tree corruption problem that
> motivated the switch to Event Sourcing. The B+Tree approach was abandoned in favor
> of the current V2 architecture (outboxes → sequencer → eventlog → ViewEngine caches).
> See TEAMENGRAM-V2-ARCHITECTURE.md for current implementation.

---

## Executive Summary

The current TeamEngram architecture suffers from **multi-process write corruption** when multiple AI instances open the same shared store file simultaneously. Each process has its own:
- Memory-mapped view (`MmapMut`)
- Page allocation state (`ShadowAllocator`)
- Transaction tracking (`dirty_pages`, `current_txn`)

When 4 AIs write concurrently, they allocate overlapping pages and corrupt each other's B+Tree structures.

**This document researches proven solutions from industry and academia, then recommends an AI-specific hybrid architecture.**

---

## Part 1: Current Architecture Analysis

### What We Have (TeamEngram)

```
┌─────────────────────────────────────────────────────────────┐
│                    teamengram.engram                        │
│  ┌─────────┐  ┌──────────────────────────────────────────┐  │
│  │ MetaPage│  │              B+Tree Pages                │  │
│  │ (root,  │  │  (Leaf/Branch pages, copy-on-write)      │  │
│  │  txn_id)│  │                                          │  │
│  └─────────┘  └──────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
         ↑                    ↑                    ↑
    ┌────┴────┐          ┌────┴────┐          ┌────┴────┐
    │ AI #1   │          │ AI #2   │          │ AI #3   │
    │ mmap    │          │ mmap    │          │ mmap    │
    │ alloc   │          │ alloc   │          │ alloc   │
    └─────────┘          └─────────┘          └─────────┘

    PROBLEM: Each AI has independent view of file state.
    Page allocation conflicts → B+Tree corruption.
```

### What Works (Engram - Per-AI Notebook)

```
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│ sage.engram     │  │ lyra.engram     │  │ cascade.engram  │
│ (Sage's notes)  │  │ (Lyra's notes)  │  │ (Cascade notes) │
└─────────────────┘  └─────────────────┘  └─────────────────┘
         ↑                    ↑                    ↑
    ┌────┴────┐          ┌────┴────┐          ┌────┴────┐
    │ Sage    │          │ Lyra    │          │ Cascade │
    │ (owner) │          │ (owner) │          │ (owner) │
    └─────────┘          └─────────┘          └─────────┘

    WORKS: Each AI owns their file exclusively. No conflicts.
```

### Current shadow.rs Strengths

Our `ShadowAllocator` already implements:
1. **Copy-on-Write** - `write_page()` creates shadow copies, never modifies committed pages
2. **Two-Slot Root Switching** - `root_primary` / `root_shadow` with atomic `active_root` toggle
3. **Transaction IDs** - Monotonic `txn_id` for ordering
4. **Checksums** - Page-level integrity verification

These are the building blocks for MVCC. The missing piece: **cross-process coordination**.

---

## Part 2: Industry Solutions Research

### 2.1 LMDB (Lightning Memory-Mapped Database)

**Source:** [How LMDB Works](https://xgwang.me/posts/how-lmdb-works/) | [LMDB Tech](http://www.lmdb.tech/doc/)

**Architecture:**
- **Single Writer, Multiple Readers** (SWMR)
- Writers acquire mutex, serialize all writes
- Readers see consistent MVCC snapshots without locks
- Copy-on-write B+Tree with shadow paging (like ours!)
- NO WAL needed - shadow paging provides crash safety

**Key Insight:**
> "Writes are fully serialized; only one write transaction may be active at a time.
> The database structure is multi-versioned so readers run with no locks."

**Why This Works:**
- Writers don't block readers (readers see old snapshot)
- Readers don't block writers (copy-on-write)
- Only writers block other writers (acceptable for most workloads)

**Limitation:**
- Long-running read transactions block page reclamation
- Single writer can be bottleneck for write-heavy workloads

---

### 2.2 redb (Rust Embedded Database)

**Source:** [redb Design Doc](https://github.com/cberner/redb/blob/master/docs/design.md)

**Architecture:**
- Pure Rust, copy-on-write B-tree
- MVCC with "two commit slots" and atomic "god byte" flip
- Epoch-based page reclamation
- **No WAL** - checksums + atomic metadata updates

**Key Innovation:**
> "Read transactions make a private copy of the root of the b-tree, and are registered
> in the database so that no pages that root references are freed."

**MVCC Implementation:**
1. Reader calls `begin_read()` → gets private snapshot of root
2. Writer modifies tree, creates new pages (CoW)
3. Writer commits → flips "god byte" to new root
4. Old pages freed only after all readers holding them complete

**Crash Recovery Without WAL:**
- Checksums on all pages with monotonic transaction IDs
- On crash: verify checksums, use valid commit slot
- If primary slot invalid, fall back to secondary

---

### 2.3 SQLite WAL Mode

**Source:** [SQLite WAL](https://sqlite.org/wal.html) | [WAL Format](https://sqlite.org/walformat.html)

**Architecture:**
- Three files: main DB (X), WAL (X-wal), shared memory index (X-shm)
- Writes go to WAL sequentially (fast)
- Readers check WAL first, then main DB
- Checkpoint transfers WAL to main DB

**Concurrency Model:**
> "WAL provides more concurrency as readers do not block writers and a writer
> does not block readers. Reading and writing can proceed concurrently."

**Limitation:**
- Requires shared memory → doesn't work over network filesystems
- WAL can grow unbounded without checkpointing

---

### 2.4 Append-Only Logs

**Source:** [Atomic Shared Log Writes](https://nblumhardt.com/2016/08/atomic-shared-log-file-writes/) | [Appending from Multiple Processes](https://nullprogram.com/blog/2016/08/03/)

**Architecture:**
- Multiple processes append to same file
- `O_APPEND` on Unix, `FILE_APPEND_DATA` on Windows
- Writes up to sector size (~4KB) are atomic

**Key Insight:**
> "POSIX O_APPEND semantics provide atomic writes even when appending to a log file
> from multiple threads or processes."

**Use Case:**
- High-volume append workloads (logs, events, messages)
- Naturally suits broadcasts and DMs
- Index rebuilt on read or maintained separately

**Limitations:**
- Reads require scanning (without index)
- Need compaction to reclaim space
- Complex state (dialogues, votes) need more structure

---

### 2.5 Lock-Free B+Trees (Academic)

**Source:** [Lehman & Yao B-link Tree](https://www.csd.uoc.gr/~hy460/pdf/p650-lehman.pdf) | [Lock-Free B-Tree Survey](https://15721.courses.cs.cmu.edu/spring2016/papers/a16-graefe.pdf)

**B-link Tree (1981):**
- Modified B+Tree with "link pointers" between siblings
- Operations lock at most 3 nodes at a time
- **Searches are completely lock-free**
- Splits are safe: new node linked before parent updated

**Lock-Free Techniques (Modern):**
- Compare-And-Swap (CAS) for atomic updates
- Hazard pointers for safe memory reclamation
- Epoch-based garbage collection

**Practical Considerations:**
- Very complex to implement correctly
- File-based CAS requires special syscalls or memory-mapped atomics
- Best suited for in-memory databases

---

## Part 3: Recommended Architecture for AI-Foundation

Based on the research, I recommend a **Hybrid Architecture** that matches data access patterns:

### 3.1 Data Classification

| Data Type | Access Pattern | Writers | Solution |
|-----------|---------------|---------|----------|
| Notebook (per-AI) | Read/Write by owner only | 1 | Per-AI file (current) |
| Broadcasts | Append-heavy, read-all | N concurrent | Append-only log |
| Direct Messages | Append-heavy, read-filtered | N concurrent | Append-only log |
| Dialogues | State machine, moderate writes | N (but serialize per dialogue) | SWMR B+Tree |
| Votes | State machine, rare writes | N | SWMR B+Tree |
| Rooms | State + membership | N | SWMR B+Tree |
| Locks | Acquire/release, time-bound | N | SWMR B+Tree |
| Presence | Frequent updates | N | Append-only + compact |

### 3.2 Proposed Architecture

```
┌────────────────────────────────────────────────────────────────────┐
│                        TeamEngram Storage                          │
├────────────────────────────────────────────────────────────────────┤
│                                                                    │
│  ┌─────────────────────────────────┐  ┌─────────────────────────┐  │
│  │     Append-Only Message Log     │  │   SWMR State B+Tree     │  │
│  │  ┌───────────────────────────┐  │  │  ┌───────────────────┐  │  │
│  │  │ broadcasts.log            │  │  │  │ state.teamengram  │  │  │
│  │  │ - Sequential writes       │  │  │  │ - Dialogues       │  │  │
│  │  │ - O_APPEND atomic         │  │  │  │ - Votes           │  │  │
│  │  │ - Periodic compaction     │  │  │  │ - Rooms           │  │  │
│  │  └───────────────────────────┘  │  │  │ - Locks           │  │  │
│  │  ┌───────────────────────────┐  │  │  │ - File claims     │  │  │
│  │  │ dms.log                   │  │  │  └───────────────────┘  │  │
│  │  │ - Per-recipient indexed   │  │  │                         │  │
│  │  │ - O_APPEND atomic         │  │  │  Writer Lock:           │  │
│  │  └───────────────────────────┘  │  │  - LockFileEx (Windows) │  │
│  │  ┌───────────────────────────┐  │  │  - flock (Unix)         │  │
│  │  │ presence.log              │  │  │                         │  │
│  │  │ - Heartbeat records       │  │  │  Readers:               │  │
│  │  │ - Compact on startup      │  │  │  - MVCC snapshots       │  │
│  │  └───────────────────────────┘  │  │  - No locks needed      │  │
│  └─────────────────────────────────┘  └─────────────────────────┘  │
│                                                                    │
└────────────────────────────────────────────────────────────────────┘
         ↑           ↑           ↑           ↑
    ┌────┴────┐ ┌────┴────┐ ┌────┴────┐ ┌────┴────┐
    │ Sage    │ │ Cascade │ │ Lyra    │ │Resonance│
    │ append  │ │ append  │ │ append  │ │ append  │
    │ to logs │ │ to logs │ │ to logs │ │ to logs │
    │         │ │         │ │         │ │         │
    │ acquire │ │ acquire │ │ acquire │ │ acquire │
    │ lock    │ │ lock    │ │ lock    │ │ lock    │
    │ for     │ │ for     │ │ for     │ │ for     │
    │ state   │ │ state   │ │ state   │ │ state   │
    └─────────┘ └─────────┘ └─────────┘ └─────────┘
```

---

## Part 4: Implementation Plan

### Phase 1: Single-Writer Lock for B+Tree (Immediate Fix)

Add file-level locking to `ShadowAllocator`:

```rust
// shadow.rs additions

#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    LockFileEx, UnlockFileEx,
    LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY
};

#[cfg(unix)]
use libc::{flock, LOCK_EX, LOCK_UN, LOCK_NB};

pub struct ShadowAllocator {
    // ... existing fields ...
    write_lock_held: bool,
}

impl ShadowAllocator {
    /// Acquire exclusive write lock (blocking)
    pub fn acquire_write_lock(&mut self) -> Result<()> {
        if self.write_lock_held {
            return Ok(()); // Already have it
        }

        #[cfg(windows)]
        {
            let handle = self.file.as_raw_handle();
            let mut overlapped = std::mem::zeroed();
            let result = unsafe {
                LockFileEx(
                    handle as _,
                    LOCKFILE_EXCLUSIVE_LOCK,
                    0,
                    1, 0,  // Lock first byte
                    &mut overlapped
                )
            };
            if result == 0 {
                bail!("Failed to acquire write lock");
            }
        }

        #[cfg(unix)]
        {
            let fd = self.file.as_raw_fd();
            if unsafe { flock(fd, LOCK_EX) } != 0 {
                bail!("Failed to acquire write lock");
            }
        }

        self.write_lock_held = true;
        Ok(())
    }

    /// Release write lock
    pub fn release_write_lock(&mut self) -> Result<()> {
        if !self.write_lock_held {
            return Ok(());
        }

        #[cfg(windows)]
        {
            let handle = self.file.as_raw_handle();
            let mut overlapped = std::mem::zeroed();
            unsafe {
                UnlockFileEx(handle as _, 0, 1, 0, &mut overlapped);
            }
        }

        #[cfg(unix)]
        {
            let fd = self.file.as_raw_fd();
            unsafe { flock(fd, LOCK_UN); }
        }

        self.write_lock_held = false;
        Ok(())
    }

    /// Write with automatic lock acquisition
    pub fn write_with_lock<F, R>(&mut self, f: F) -> Result<R>
    where
        F: FnOnce(&mut Self) -> Result<R>
    {
        self.acquire_write_lock()?;
        let result = f(self);
        // Flush and release
        self.mmap.flush()?;
        self.release_write_lock()?;
        result
    }
}
```

**Effort:** 1-2 days
**Impact:** Eliminates B+Tree corruption for shared state

---

### Phase 2: MVCC Reader Snapshots

Add snapshot isolation for readers:

```rust
// mvcc.rs (new file)

pub struct ReadSnapshot {
    /// Root page at snapshot time
    root: PageId,
    /// Transaction ID at snapshot time
    txn_id: u64,
    /// Registration in active readers list
    reader_slot: usize,
}

impl ShadowAllocator {
    /// Begin a read-only snapshot
    pub fn begin_read(&self) -> ReadSnapshot {
        let meta = self.meta_page();
        ReadSnapshot {
            root: meta.active_root_page(),
            txn_id: meta.txn_id,
            reader_slot: self.register_reader(),
        }
    }

    /// Register reader to prevent page reclamation
    fn register_reader(&self) -> usize {
        // Use shared memory segment for cross-process reader tracking
        // Similar to SQLite's WAL-index (.shm file)
        todo!()
    }
}
```

**Effort:** 3-5 days
**Impact:** Readers never block writers, writers never block readers

---

### Phase 3: Append-Only Message Logs

Separate high-volume append workloads:

```rust
// append_log.rs (new file)

use std::fs::{File, OpenOptions};
use std::io::{Write, BufReader, BufRead};

pub struct AppendLog {
    file: File,
    path: PathBuf,
}

impl AppendLog {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)  // O_APPEND - atomic appends
            .read(true)
            .open(path.as_ref())?;

        Ok(Self {
            file,
            path: path.as_ref().to_path_buf(),
        })
    }

    /// Append a record (atomic up to ~4KB)
    pub fn append(&mut self, record: &[u8]) -> Result<u64> {
        // Format: [len:u32][data][crc:u32][newline]
        let mut buf = Vec::with_capacity(record.len() + 12);
        buf.extend_from_slice(&(record.len() as u32).to_le_bytes());
        buf.extend_from_slice(record);
        buf.extend_from_slice(&crc32fast::hash(record).to_le_bytes());
        buf.push(b'\n');

        // Atomic append
        self.file.write_all(&buf)?;
        self.file.sync_data()?;  // Optional: durability vs performance

        Ok(self.file.metadata()?.len())
    }

    /// Iterate all records
    pub fn iter(&self) -> Result<impl Iterator<Item = Result<Vec<u8>>>> {
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        Ok(AppendLogIterator { reader })
    }
}
```

**Effort:** 2-3 days
**Impact:** True concurrent writes for broadcasts/DMs

---

### Phase 4: Indexing for Append Logs

Add fast lookups without full scans:

```rust
// log_index.rs (new file)

/// In-memory index rebuilt on startup, updated on append
pub struct LogIndex {
    /// Offset of each record by ID
    by_id: HashMap<u64, u64>,
    /// Records by recipient (for DMs)
    by_recipient: HashMap<String, Vec<u64>>,
    /// Records by timestamp range
    by_time: BTreeMap<u64, Vec<u64>>,
}

impl LogIndex {
    /// Rebuild index by scanning log
    pub fn rebuild(log: &AppendLog) -> Result<Self> {
        let mut index = Self::default();
        for (offset, record) in log.iter_with_offsets()? {
            index.insert(offset, &record);
        }
        Ok(index)
    }

    /// Optional: persist index to file for fast startup
    pub fn persist(&self, path: &Path) -> Result<()>;
    pub fn load(path: &Path) -> Result<Self>;
}
```

**Effort:** 2-3 days
**Impact:** Fast DM/broadcast queries without scanning

---

### Phase 5: Compaction

Reclaim space from append logs:

```rust
// compaction.rs

impl AppendLog {
    /// Compact log, removing old/deleted records
    pub fn compact(&mut self, keep: impl Fn(&Record) -> bool) -> Result<()> {
        let temp_path = self.path.with_extension("compact");
        let mut temp = AppendLog::open(&temp_path)?;

        for record in self.iter()? {
            if keep(&record?) {
                temp.append(&record)?;
            }
        }

        // Atomic swap
        std::fs::rename(&temp_path, &self.path)?;
        self.file = OpenOptions::new().append(true).read(true).open(&self.path)?;
        Ok(())
    }
}
```

**Effort:** 1-2 days
**Impact:** Bounded storage growth

---

## Part 5: Alternative Considered - Lock-Free B+Tree

For completeness, here's what a lock-free implementation would require:

### B-link Tree Modifications

1. **Link Pointers:** Each node has a "right link" to its sibling
2. **Safe Splits:**
   - Allocate new node
   - Copy half the keys
   - Set new node's right link
   - Atomically update original node's right link (CAS)
   - Update parent (can retry if parent also split)

3. **Safe Searches:**
   - If key > all keys in node AND right link exists → follow link
   - Never blocked by concurrent splits

### Why NOT Recommended for TeamEngram

1. **Complexity:** Hundreds of lines of careful code, subtle bugs
2. **File-based CAS:** Memory-mapped atomics work, but Windows/Linux differ
3. **Overkill:** Our write volume doesn't justify the complexity
4. **Single-writer is fast enough:** With CoW, writes are O(log n) page copies

---

## Part 6: Encryption and Vector Search

### Encryption (Already Implemented)

Current `crypto.rs` uses XChaCha20-Poly1305. This is correct and doesn't affect concurrency:
- Each note encrypted with unique nonce
- Decryption happens after read, encryption before write
- Key derivation from AI_ID + device secret

### Vector Search (Engram, not TeamEngram)

HNSW index in `hnsw_index.rs` is per-AI. Concurrency not an issue.

For SHARED vector search (if ever needed):
- Read-only queries: No coordination needed (HNSW is read-safe)
- Index updates: Serialize with write lock
- Or: Each AI maintains local index, periodic sync

---

## Part 7: Summary and Recommendations

### Immediate Action (Fix Corruption)

Implement **Phase 1: Single-Writer Lock** in `shadow.rs`:
- Add `LockFileEx`/`flock` around write transactions
- ~50 lines of code, 1-2 days work
- Eliminates all B+Tree corruption

### Short-Term (Better Concurrency)

Implement **Phase 2 + 3**:
- MVCC reader snapshots (readers never block)
- Append-only logs for broadcasts/DMs (true concurrent appends)

### Long-Term (Full Architecture)

Complete **Phases 4-5**:
- Indexing for fast queries
- Compaction for bounded storage

### NOT Recommended

- **Single shared daemon for ALL data:** Cascade's suggestion - bottleneck, single point of failure
- **PostgreSQL:** Regression, why we built Engram/TeamEngram
- **Per-AI isolation with no sharing:** Loses team collaboration
- **Lock-free B+Tree:** Overkill for our write volume

---

## References

1. [LMDB Design](http://www.lmdb.tech/doc/) - Single-writer MVCC
2. [redb Design Doc](https://github.com/cberner/redb/blob/master/docs/design.md) - Pure Rust CoW B-tree
3. [SQLite WAL](https://sqlite.org/wal.html) - WAL-mode concurrency
4. [SQLite Locking](https://sqlite.org/lockingv3.html) - File locking strategies
5. [Lehman & Yao B-link Tree](https://www.csd.uoc.gr/~hy460/pdf/p650-lehman.pdf) - Lock-free foundations
6. [Graefe B-Tree Survey](https://15721.courses.cs.cmu.edu/spring2016/papers/a16-graefe.pdf) - Comprehensive locking techniques
7. [libmdbx](https://github.com/erthink/libmdbx) - LMDB improvements
8. [Atomic Appends](https://nblumhardt.com/2016/08/atomic-shared-log-file-writes/) - O_APPEND semantics
9. [LockFileEx](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex) - Windows file locking

---

*This document represents thorough research into concurrent storage solutions, adapted specifically for AI-Foundation's requirements. The hybrid architecture balances simplicity, correctness, and performance for our team coordination use case.*
