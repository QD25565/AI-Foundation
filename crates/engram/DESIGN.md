# Engram: Purpose-Built AI Memory Database

## What is Engram?

**Engram** (noun): In neuroscience, the physical trace of a memory in the brain - the actual neural substrate where a memory lives.

Engram is a single-file, memory-mapped database designed from scratch for AI memory. Not a general-purpose database with vectors bolted on. Every line of code exists to serve one purpose: **storing and retrieving AI memories as fast as physically possible**.

## Why Not SQLite?

SQLite is excellent. We used it. But for AI memory, we're fighting it:

| What We Need | SQLite Reality |
|--------------|----------------|
| Native 512d vectors | BLOBs with marshaling overhead |
| Graph traversal | JOIN queries, no native edges |
| Temporal queries | Index on timestamp column |
| Hybrid search | Multiple queries + merge in app |
| Zero-copy reads | Row parsing overhead |
| AI-native operations | Generic SQL |

Engram does what SQLite can't: treat vectors, graphs, and time as **first-class citizens**.

## Design Principles

1. **Memory-mapped everything** - Zero-copy reads, instant cold start
2. **Append-only writes** - Simple, fast, crash-safe
3. **Fixed schema** - Notes, vectors, edges, tags, vault. That's it.
4. **Single writer** - No MVCC complexity
5. **AI-native** - Operations map directly to AI memory patterns

## File Format

```
┌─────────────────────────────────────────────────────────────────┐
│                      engram.db (single file)                     │
├──────────────────────────────────────────────────────────────────┤
│ Offset 0x0000: HEADER (4KB, page-aligned)                        │
├──────────────────────────────────────────────────────────────────┤
│ Offset 0x1000: NOTE LOG (variable, append-only)                  │
├──────────────────────────────────────────────────────────────────┤
│ Offset varies: VECTOR STORE (contiguous float32 array)           │
├──────────────────────────────────────────────────────────────────┤
│ Offset varies: HNSW INDEX (graph structure for ANN)              │
├──────────────────────────────────────────────────────────────────┤
│ Offset varies: GRAPH INDEX (note relationships)                  │
├──────────────────────────────────────────────────────────────────┤
│ Offset varies: TEMPORAL INDEX (sorted timestamp array)           │
├──────────────────────────────────────────────────────────────────┤
│ Offset varies: TAG INDEX (tag → note_ids mapping)                │
├──────────────────────────────────────────────────────────────────┤
│ Offset varies: VAULT (encrypted key-value store)                 │
└──────────────────────────────────────────────────────────────────┘
```

### Header (4KB)

```rust
#[repr(C)]
struct EngramHeader {
    magic: [u8; 8],           // "ENGRAM01"
    version: u32,             // Format version
    flags: u32,               // Feature flags

    ai_id_hash: [u8; 32],     // SHA-256 of AI_ID (isolation)
    created_at: i64,          // Unix timestamp
    modified_at: i64,         // Unix timestamp

    note_count: u64,          // Total notes (including tombstones)
    active_notes: u64,        // Notes minus tombstones
    vector_dimensions: u32,   // 512 for EmbeddingGemma

    // Section offsets (absolute file positions)
    note_log_offset: u64,
    note_log_size: u64,
    vector_store_offset: u64,
    vector_store_size: u64,
    hnsw_index_offset: u64,
    hnsw_index_size: u64,
    graph_index_offset: u64,
    graph_index_size: u64,
    temporal_index_offset: u64,
    temporal_index_size: u64,
    tag_index_offset: u64,
    tag_index_size: u64,
    vault_offset: u64,
    vault_size: u64,

    // Checksums
    header_checksum: u32,     // CRC32 of header

    _reserved: [u8; 3896],    // Pad to 4KB
}
```

### Note Log Entry

```rust
#[repr(C)]
struct NoteEntry {
    // Fixed header (32 bytes)
    id: u64,                  // Monotonic note ID
    timestamp: i64,           // Created timestamp (nanos since epoch)
    flags: u32,               // TOMBSTONE, PINNED, HAS_VECTOR, etc.
    content_len: u32,         // Compressed content length
    tags_len: u16,            // Tags blob length
    _padding: [u8; 6],        // Align to 8 bytes

    // Variable data follows:
    // - tags: [u8; tags_len] - null-separated tag strings
    // - content: [u8; content_len] - zstd compressed UTF-8
}
```

### Vector Store

Contiguous array of float32 vectors. Vector for note ID `n` is at offset `n * 512 * 4`.

```rust
// Direct memory access - no parsing
fn get_vector(&self, note_id: u64) -> &[f32; 512] {
    let offset = note_id as usize * 512;
    unsafe { &*(self.vector_mmap[offset..].as_ptr() as *const [f32; 512]) }
}
```

### HNSW Index

Hierarchical Navigable Small World graph for approximate nearest neighbor search.

```rust
struct HnswIndex {
    entry_point: u64,         // Top-level entry node
    max_level: u32,           // Maximum layer
    ef_construction: u32,     // Build-time beam width

    // Per-level adjacency lists
    levels: Vec<Vec<Vec<u64>>>,  // levels[layer][node] = neighbors
}
```

### Graph Index (Note Relationships)

```rust
struct GraphIndex {
    // Forward edges: note_id → [(target_id, weight, edge_type)]
    forward: HashMap<u64, Vec<(u64, f32, EdgeType)>>,

    // Reverse edges for PageRank computation
    reverse: HashMap<u64, Vec<u64>>,

    // Precomputed PageRank scores
    pagerank: Vec<f32>,
}

enum EdgeType {
    Semantic,    // High cosine similarity
    Temporal,    // Created within time window
    Manual,      // Explicitly linked
    Tag,         // Shared tag
}
```

### Temporal Index

Sorted array enabling efficient range queries.

```rust
struct TemporalIndex {
    // Sorted by timestamp
    entries: Vec<(i64, u64)>,  // (timestamp, note_id)
}

fn range(&self, start: i64, end: i64) -> impl Iterator<Item = u64> {
    let start_idx = self.entries.binary_search_by_key(&start, |e| e.0);
    let end_idx = self.entries.binary_search_by_key(&end, |e| e.0);
    self.entries[start_idx..end_idx].iter().map(|e| e.1)
}
```

### Tag Index

```rust
struct TagIndex {
    // Tag string → list of note IDs
    tags: HashMap<String, Vec<u64>>,

    // Bloom filter for fast "does this tag exist?" checks
    bloom: BloomFilter,
}
```

### Vault (Encrypted Key-Value)

```rust
struct VaultEntry {
    key_hash: [u8; 32],       // SHA-256 of key (key itself not stored)
    nonce: [u8; 24],          // XChaCha20-Poly1305 nonce
    ciphertext_len: u32,
    ciphertext: Vec<u8>,      // Encrypted value
}
```

## Core API

```rust
pub struct Engram {
    // ... internal state
}

impl Engram {
    /// Open or create an Engram database
    pub fn open(path: impl AsRef<Path>) -> Result<Self>;

    /// Open read-only (for concurrent readers)
    pub fn open_readonly(path: impl AsRef<Path>) -> Result<Self>;

    // ═══════════════════════════════════════════════════════════
    // WRITE OPERATIONS
    // ═══════════════════════════════════════════════════════════

    /// Store a new memory
    /// Returns note ID. Auto-generates embedding if embedder configured.
    pub fn remember(&mut self, content: &str, tags: &[&str]) -> Result<u64>;

    /// Store with pre-computed embedding
    pub fn remember_with_embedding(
        &mut self,
        content: &str,
        tags: &[&str],
        embedding: &[f32; 512]
    ) -> Result<u64>;

    /// Mark a note as deleted (tombstone)
    pub fn forget(&mut self, id: u64) -> Result<()>;

    /// Create a relationship between notes
    pub fn link(&mut self, from: u64, to: u64, weight: f32) -> Result<()>;

    /// Pin/unpin a note
    pub fn pin(&mut self, id: u64) -> Result<()>;
    pub fn unpin(&mut self, id: u64) -> Result<()>;

    // ═══════════════════════════════════════════════════════════
    // READ OPERATIONS
    // ═══════════════════════════════════════════════════════════

    /// Hybrid search: vector similarity + keyword + graph + recency
    pub fn recall(&self, query: &str, limit: usize) -> Result<Vec<Note>>;

    /// Pure vector similarity search
    pub fn vector_search(&self, embedding: &[f32; 512], limit: usize) -> Vec<(u64, f32)>;

    /// Get note by ID
    pub fn get(&self, id: u64) -> Option<Note>;

    /// Get multiple notes by ID (batch)
    pub fn get_many(&self, ids: &[u64]) -> Vec<Note>;

    /// Get most recent notes
    pub fn recent(&self, limit: usize) -> Vec<Note>;

    /// Get notes by tag
    pub fn by_tag(&self, tag: &str) -> Vec<Note>;

    /// Get notes in time range
    pub fn temporal_range(&self, start: i64, end: i64) -> Vec<Note>;

    /// Get graph neighbors
    pub fn neighbors(&self, id: u64) -> Vec<(u64, f32)>;

    /// Get pinned notes
    pub fn pinned(&self) -> Vec<Note>;

    // ═══════════════════════════════════════════════════════════
    // VAULT (ENCRYPTED KEY-VALUE)
    // ═══════════════════════════════════════════════════════════

    pub fn vault_set(&mut self, key: &str, value: &[u8]) -> Result<()>;
    pub fn vault_get(&self, key: &str) -> Option<Vec<u8>>;
    pub fn vault_delete(&mut self, key: &str) -> Result<()>;
    pub fn vault_keys(&self) -> Vec<String>;

    // ═══════════════════════════════════════════════════════════
    // MAINTENANCE
    // ═══════════════════════════════════════════════════════════

    /// Remove tombstones, rebuild indexes, reclaim space
    pub fn compact(&mut self) -> Result<CompactStats>;

    /// Recompute PageRank scores
    pub fn recompute_pagerank(&mut self) -> Result<()>;

    /// Rebuild HNSW index (after many insertions)
    pub fn rebuild_hnsw(&mut self) -> Result<()>;

    /// Database statistics
    pub fn stats(&self) -> EngramStats;

    /// Verify integrity
    pub fn verify(&self) -> Result<VerifyResult>;

    /// Force sync to disk
    pub fn sync(&self) -> Result<()>;
}
```

## Performance Targets

Based on what's physically possible with modern hardware:

| Operation | Target | Notes |
|-----------|--------|-------|
| `remember()` | < 1ms | Append + index update |
| `recall()` (hybrid) | < 5ms | HNSW + BM25 + rerank |
| `vector_search()` | < 1ms | Pure HNSW |
| `get()` | < 100μs | Direct mmap read |
| `recent(10)` | < 50μs | Temporal index |
| `by_tag()` | < 200μs | Hash lookup + reads |
| Cold start | < 10ms | mmap, no parsing |
| Memory usage | ~2x file size | mmap + working set |

## Comparison: Engram vs SQLite

| Aspect | SQLite | Engram |
|--------|--------|--------|
| Vector storage | BLOB (marshal/unmarshal) | Native float32 array |
| Vector search | External HNSW + JOIN | Built-in HNSW |
| Graph edges | Separate table + JOINs | Native adjacency lists |
| Cold start | Parse schema, load indexes | mmap and go |
| Concurrent readers | WAL mode overhead | Direct mmap |
| Schema | Flexible (overhead) | Fixed (optimized) |
| AI-native ops | SQL translation | Direct methods |

## Implementation Phases

### Phase 1: Foundation
- File format, header, note log
- Basic read/write without indexes
- **Benchmark: raw write/read throughput**

### Phase 2: Memory Mapping
- mmap for note log
- Zero-copy reads
- **Benchmark: cold start time, read latency**

### Phase 3: Vector Store + HNSW
- Contiguous vector array
- HNSW index implementation
- **Benchmark: recall@10, queries/second**

### Phase 4: Graph Index
- Adjacency lists
- PageRank computation
- **Benchmark: neighbor queries, PageRank time**

### Phase 5: Temporal + Tag Indexes
- Sorted temporal array
- Tag hash map + bloom filter
- **Benchmark: range queries, tag lookups**

### Phase 6: Vault
- XChaCha20-Poly1305 encryption
- Key derivation from AI_ID
- **Benchmark: encrypt/decrypt throughput**

### Phase 7: Hybrid Recall
- Combine vector + keyword + graph + recency
- RRF or learned fusion
- **Benchmark: end-to-end recall quality + latency**

### Phase 8: CLI + Migration
- engram-cli with all operations
- SQLite → Engram migration tool
- **Test: migrate existing notebooks**

## Future Possibilities

- **Distributed mode**: Shard by note ID range
- **Replication**: Append-only log makes this natural
- **Compression**: Dictionary compression across similar notes
- **Learned index**: Replace HNSW with learned ANN
- **GPU acceleration**: Vector ops on GPU

## Non-Goals

- General-purpose SQL
- Multi-writer transactions
- Flexible schema
- Backward compatibility with SQLite
- Being everything to everyone

Engram does one thing: **AI memory**. And it does it better than anything else.

---

*"The engram is the physical trace of memory. We're building the substrate."*
