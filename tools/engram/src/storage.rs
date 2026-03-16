//! Core Engram storage implementation
//!
//! Phase 2: Memory-mapped I/O for zero-copy reads + LRU cache + batch operations
//! Phase 3.1: Bloom filters for fast negative tag lookups
//! Phase 3.2: Index persistence for O(1) startup
//! Phase 4: Full integration - vectors, graph, vault, hybrid recall

use crate::{
    crypto::EngramCipher,
    bloom::BloomFilter,
    error::Result,
    fingerprint::FingerprintIndex256,
    graph::{GraphIndex, EdgeType},
    header::{EngramHeader, HEADER_SIZE, flags as header_flags},
    hnsw::HnswIndex,
    note::{Note, NoteEntry},
    recall::{RecallConfig, RecallResult, BM25Corpus, recency_score_at},
    vault::Vault,
    vector::VectorStore,
    EngramError, EngramStats, VerifyResult,
};
use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Magic bytes for index section
const INDEX_MAGIC: u64 = 0x454E47494E444558; // "ENGINDEX" in hex

/// Maximum number of pinned notes (enforced at Engram core level)
pub const MAX_PINNED: usize = 30;

/// LRU cache entry
struct CacheEntry {
    note: Note,
    last_access: u64,
}

/// LRU cache for decompressed notes
struct NoteCache {
    entries: HashMap<u64, CacheEntry>,
    max_size: usize,
    access_counter: AtomicU64,
    hit_count: u64,
}

impl NoteCache {
    fn new(max_size: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(max_size),
            max_size,
            access_counter: AtomicU64::new(0),
            hit_count: 0,
        }
    }

    fn get(&mut self, id: u64) -> Option<&Note> {
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.last_access = self.access_counter.fetch_add(1, Ordering::Relaxed);
            self.hit_count += 1;
            Some(&entry.note)
        } else {
            None
        }
    }

    fn insert(&mut self, id: u64, note: Note) {
        // Evict if at capacity
        if self.entries.len() >= self.max_size {
            self.evict_lru();
        }

        let access = self.access_counter.fetch_add(1, Ordering::Relaxed);
        self.entries.insert(id, CacheEntry {
            note,
            last_access: access,
        });
    }

    fn evict_lru(&mut self) {
        if let Some((&lru_id, _)) = self.entries
            .iter()
            .min_by_key(|(_, e)| e.last_access)
        {
            self.entries.remove(&lru_id);
        }
    }

    fn invalidate(&mut self, id: u64) {
        self.entries.remove(&id);
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.hit_count = 0;
    }

    #[allow(dead_code)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)]
    fn hits(&self) -> usize {
        self.hit_count as usize
    }
}

/// The Engram database
pub struct Engram {
    /// Path to the database file
    path: PathBuf,

    /// File handle (for writes)
    file: File,

    /// Memory-mapped view (for reads) - None if file is empty/new
    mmap: Option<Mmap>,

    /// Size of file covered by current mmap (for lazy remap optimization)
    mmap_valid_size: u64,

    /// Header (cached in memory)
    header: EngramHeader,

    /// Read-only mode
    read_only: bool,

    /// Note index: id -> file offset
    note_index: HashMap<u64, u64>,

    /// Pinned note IDs
    pinned: Vec<u64>,

    /// Next note ID to assign
    next_id: u64,

    /// Tag index: tag -> [note_ids]
    tag_index: HashMap<String, Vec<u64>>,

    /// Temporal index: sorted (timestamp, note_id) pairs
    temporal_index: Vec<(i64, u64)>,

    /// LRU cache for decompressed notes
    cache: NoteCache,

    /// Bloom filter for fast tag negative lookups
    tag_bloom: BloomFilter,

    // ═══════════════════════════════════════════════════════════════════
    // Phase 4: Vector, Graph, Vault integration
    // ═══════════════════════════════════════════════════════════════════

    /// Vector store for embeddings
    vector_store: VectorStore,

    /// HNSW index for O(log n) approximate nearest neighbor search
    hnsw_index: HnswIndex,

    /// Knowledge graph with PageRank
    graph: GraphIndex,

    /// Encrypted key-value vault
    vault: Vault,

    /// Recall configuration for hybrid search
    recall_config: RecallConfig,

    /// Encryption cipher for note content
    cipher: EngramCipher,

    /// Fingerprint index for sub-microsecond associative recall.
    /// Stored as sidecar file (.engram.fp) alongside the main .engram file.
    fingerprint_index: FingerprintIndex256,

    /// Cached BM25 corpus statistics (IDF + avgdl) — invalidated on note writes.
    /// Avoids rebuilding IDF from scratch on every recall call.
    bm25_cache: Option<BM25Corpus>,
    /// Note count when BM25 cache was built (cheap invalidation check)
    bm25_cache_note_count: u64,

    /// Statistics
    cache_hits: u64,
    cache_misses: u64,

    /// Last known file modification time (for multi-process sync)
    /// When another process modifies the file, we detect it and reload
    last_index_mtime: Option<std::time::SystemTime>,
}

/// Default cache size (number of notes)
const DEFAULT_CACHE_SIZE: usize = 1000;

impl Engram {
    /// Open or create an Engram database
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if path.exists() {
            Self::open_existing(&path, false)
        } else {
            Self::create_new(&path)
        }
    }

    /// Open an existing database read-only
    pub fn open_readonly(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_existing(path.as_ref(), true)
    }

    /// Create a new database
    fn create_new(path: &Path) -> Result<Self> {
        let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| "default".to_string());

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        let header = EngramHeader::new(&ai_id);
        header.write_to(&mut file)?;

        // Ensure file is at least header size
        file.set_len(HEADER_SIZE as u64)?;
        file.sync_all()?;

        Ok(Self {
            path: path.to_path_buf(),
            file,
            mmap: None, // No mmap for empty file
            mmap_valid_size: 0,
            header,
            read_only: false,
            note_index: HashMap::new(),
            pinned: Vec::new(),
            next_id: 1,
            tag_index: HashMap::new(),
            temporal_index: Vec::new(),
            cache: NoteCache::new(DEFAULT_CACHE_SIZE),
            tag_bloom: BloomFilter::fast(10_000), // Fast filter for tags
            // Phase 4: Vector, Graph, Vault
            vector_store: VectorStore::new(),
            hnsw_index: HnswIndex::new(),
            graph: GraphIndex::new(),
            vault: Vault::new(&ai_id)?,
            recall_config: RecallConfig::default(),
            cipher: EngramCipher::new(&ai_id)?,
            fingerprint_index: FingerprintIndex256::new(),
            bm25_cache: None,
            bm25_cache_note_count: 0,
            cache_hits: 0,
            cache_misses: 0,
            last_index_mtime: std::fs::metadata(path).ok().and_then(|m| m.modified().ok()),
        })
    }

    /// Open an existing database
    fn open_existing(path: &Path, read_only: bool) -> Result<Self> {
        // #15: Refuse to open symlinked .engram files (prevents symlink attacks)
        let file_meta = std::fs::symlink_metadata(path)?;
        if file_meta.file_type().is_symlink() {
            return Err(EngramError::IntegrityError(
                "refusing to open symlinked .engram file".into(),
            ));
        }

        let file = if read_only {
            OpenOptions::new().read(true).open(path)?
        } else {
            OpenOptions::new().read(true).write(true).open(path)?
        };

        // Create memory map
        let file_len = file.metadata()?.len();
        let mmap = if file_len > HEADER_SIZE as u64 {
            Some(unsafe { Mmap::map(&file)? })
        } else {
            None
        };
        let mmap_valid_size = if mmap.is_some() { file_len } else { 0 };

        // Read AI_ID from environment (will update from header after reading)
        let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| "default".to_string());

        let mut db = Self {
            path: path.to_path_buf(),
            file,
            mmap,
            mmap_valid_size,
            header: EngramHeader::new("temp"), // Will be replaced
            read_only,
            note_index: HashMap::new(),
            pinned: Vec::new(),
            next_id: 1,
            tag_index: HashMap::new(),
            temporal_index: Vec::new(),
            cache: NoteCache::new(DEFAULT_CACHE_SIZE),
            tag_bloom: BloomFilter::fast(10_000), // Will be populated during index load/rebuild
            // Phase 4: Vector, Graph, Vault
            vector_store: VectorStore::new(),
            hnsw_index: HnswIndex::new(),
            graph: GraphIndex::new(),
            vault: Vault::new(&ai_id)?,
            recall_config: RecallConfig::default(),
            cipher: EngramCipher::new(&ai_id)?,
            fingerprint_index: {
                // Try loading from sidecar file, fall back to empty
                // FingerprintIndex256::load auto-promotes V1/V2 entries to 256-bit
                let fp_path = FingerprintIndex256::sidecar_path(path);
                FingerprintIndex256::load(&fp_path).unwrap_or_default()
            },
            bm25_cache: None,
            bm25_cache_note_count: 0,
            cache_hits: 0,
            cache_misses: 0,
            last_index_mtime: None, // Will be set after loading indexes
        };

        // Read and verify header (use mmap if available)
        db.header = if let Some(ref mmap) = db.mmap {
            let header_bytes: &[u8; HEADER_SIZE] = mmap[..HEADER_SIZE]
                .try_into()
                .map_err(|_| EngramError::IntegrityError("Header slice conversion failed".into()))?;
            EngramHeader::from_bytes(header_bytes)?
        } else {
            EngramHeader::read_from(&mut db.file)?
        };

        // #16: Verify AI_ID ownership — prevent cross-AI file access
        if ai_id != "default" {
            use sha2::{Sha256, Digest};
            let mut hasher = Sha256::new();
            hasher.update(ai_id.as_bytes());
            let expected_hash: [u8; 32] = hasher.finalize().into();
            if db.header.ai_id_hash != expected_hash {
                return Err(EngramError::IntegrityError(
                    format!("AI_ID ownership mismatch: .engram file belongs to a different AI"),
                ));
            }
        }

        // Try to load persisted indexes (O(1) startup)
        // If not available or invalid, fall back to rebuilding from note log (O(n))
        if !db.load_persisted_indexes()? {
            // Build indexes by scanning the note log
            db.rebuild_indexes()?;
        }

        // Record current file mtime for multi-process sync detection
        db.last_index_mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());

        Ok(db)
    }

    /// Rebuild in-memory indexes by scanning the note log
    fn rebuild_indexes(&mut self) -> Result<()> {
        self.note_index.clear();
        self.pinned.clear();
        self.tag_index.clear();
        self.temporal_index.clear();
        self.tag_bloom.clear();
        self.next_id = 1;
        self.cache.clear();

        if self.header.note_log_size == 0 {
            return Ok(());
        }

        // Validate note log offset+size against actual file size
        let file_len = self.file.seek(SeekFrom::End(0))?;
        let log_end = self.header.note_log_offset.checked_add(self.header.note_log_size)
            .ok_or_else(|| EngramError::IntegrityError("note log offset+size overflow".into()))?;
        if log_end > file_len {
            return Err(EngramError::IntegrityError(format!(
                "note log extends past EOF: offset={} size={} file_len={}",
                self.header.note_log_offset, self.header.note_log_size, file_len
            )));
        }

        // Use mmap for fast scanning if available
        // Take mmap out temporarily to avoid borrow conflict
        let has_mmap = self.mmap.is_some();
        if has_mmap {
            let mmap = self.mmap.take().unwrap();
            self.rebuild_indexes_mmap(&mmap)?;
            self.mmap = Some(mmap);
        } else {
            self.rebuild_indexes_file()?;
        }

        // Sort temporal index
        self.temporal_index.sort_by_key(|(ts, _)| *ts);

        Ok(())
    }

    /// Rebuild indexes using memory-mapped file (fast path)
    fn rebuild_indexes_mmap(&mut self, mmap: &Mmap) -> Result<()> {
        let mut offset = self.header.note_log_offset as usize;
        let end_offset = offset + self.header.note_log_size as usize;

        while offset + 32 <= end_offset && offset + 32 <= mmap.len() {
            let header_bytes = &mmap[offset..offset + 32];

            let id = u64::from_le_bytes(header_bytes[0..8].try_into().unwrap());
            let timestamp = i64::from_le_bytes(header_bytes[8..16].try_into().unwrap());
            let flags = u32::from_le_bytes(header_bytes[16..20].try_into().unwrap());
            let content_len = u32::from_le_bytes(header_bytes[20..24].try_into().unwrap()) as usize;
            let tags_len = u16::from_le_bytes(header_bytes[24..26].try_into().unwrap()) as usize;

            let entry_size = 32 + tags_len + content_len;

            // Update next_id
            if id >= self.next_id {
                self.next_id = id + 1;
            }

            // Skip tombstones for indexing
            if flags & crate::note::flags::TOMBSTONE == 0 {
                self.note_index.insert(id, offset as u64);
                self.temporal_index.push((timestamp, id));

                if flags & crate::note::flags::PINNED != 0 {
                    self.pinned.push(id);
                }

                // Read tags for tag index and bloom filter
                if tags_len > 0 && offset + 32 + tags_len <= mmap.len() {
                    let tags_bytes = &mmap[offset + 32..offset + 32 + tags_len];

                    for tag in tags_bytes.split(|&b| b == 0).filter(|s| !s.is_empty()) {
                        let tag_str = String::from_utf8_lossy(tag).to_string();
                        self.tag_bloom.insert(&tag_str); // Add to bloom filter
                        self.tag_index
                            .entry(tag_str)
                            .or_insert_with(Vec::new)
                            .push(id);
                    }
                }
            }

            offset += entry_size;
        }

        Ok(())
    }

    /// Rebuild indexes using file I/O (fallback)
    fn rebuild_indexes_file(&mut self) -> Result<()> {
        let mut offset = self.header.note_log_offset;
        let end_offset = offset + self.header.note_log_size;

        while offset < end_offset {
            self.file.seek(SeekFrom::Start(offset))?;

            let mut header_buf = [0u8; 32];
            if self.file.read_exact(&mut header_buf).is_err() {
                break;
            }

            let id = u64::from_le_bytes(header_buf[0..8].try_into().unwrap());
            let timestamp = i64::from_le_bytes(header_buf[8..16].try_into().unwrap());
            let flags = u32::from_le_bytes(header_buf[16..20].try_into().unwrap());
            let content_len = u32::from_le_bytes(header_buf[20..24].try_into().unwrap());
            let tags_len = u16::from_le_bytes(header_buf[24..26].try_into().unwrap());

            let entry_size = 32 + tags_len as u64 + content_len as u64;

            if id >= self.next_id {
                self.next_id = id + 1;
            }

            if flags & crate::note::flags::TOMBSTONE == 0 {
                self.note_index.insert(id, offset);
                self.temporal_index.push((timestamp, id));

                if flags & crate::note::flags::PINNED != 0 {
                    self.pinned.push(id);
                }

                if tags_len > 0 {
                    let mut tags_buf = vec![0u8; tags_len as usize];
                    self.file.read_exact(&mut tags_buf)?;

                    for tag in tags_buf.split(|&b| b == 0).filter(|s| !s.is_empty()) {
                        let tag_str = String::from_utf8_lossy(tag).to_string();
                        self.tag_bloom.insert(&tag_str); // Add to bloom filter
                        self.tag_index
                            .entry(tag_str)
                            .or_insert_with(Vec::new)
                            .push(id);
                    }
                }
            }

            offset += entry_size;
        }

        Ok(())
    }

    /// Check if file was modified by another process and reload indexes if needed
    /// This enables multi-process sync - when CLI modifies the file, MCP sees changes
    fn refresh_if_modified(&mut self) -> Result<()> {
        let current_mtime = std::fs::metadata(&self.path)
            .ok()
            .and_then(|m| m.modified().ok());

        // If we have a known mtime and current mtime is different, reload
        if let (Some(known), Some(current)) = (self.last_index_mtime, current_mtime) {
            if current > known {
                // File was modified by another process - reload everything
                // Remap the file first to see new content
                let file_len = self.file.metadata()?.len();
                if file_len > HEADER_SIZE as u64 {
                    self.mmap = Some(unsafe { Mmap::map(&self.file)? });
                    self.mmap_valid_size = file_len;
                }

                // Re-read header
                if let Some(ref mmap) = self.mmap {
                    if mmap.len() >= HEADER_SIZE {
                        let header_bytes: &[u8; HEADER_SIZE] = mmap[..HEADER_SIZE]
                            .try_into()
                            .map_err(|_| EngramError::IntegrityError("Header slice conversion failed".into()))?;
                        self.header = EngramHeader::from_bytes(header_bytes)?;
                    }
                }

                // Reload indexes
                if !self.load_persisted_indexes()? {
                    self.rebuild_indexes()?;
                }

                // Update our known mtime
                self.last_index_mtime = current_mtime;
            }
        } else if current_mtime.is_some() && self.last_index_mtime.is_none() {
            // First time checking - just record the mtime
            self.last_index_mtime = current_mtime;
        }

        Ok(())
    }

    /// Remap the file after writes (or call sync() to batch multiple writes)
    fn remap(&mut self) -> Result<()> {
        let file_len = self.file.metadata()?.len();
        if file_len > HEADER_SIZE as u64 {
            self.mmap = Some(unsafe { Mmap::map(&self.file)? });
            self.mmap_valid_size = file_len;
        }
        Ok(())
    }

    /// Check if a note offset is within the current mmap
    fn is_offset_in_mmap(&self, offset: u64) -> bool {
        offset < self.mmap_valid_size
    }

    // ═══════════════════════════════════════════════════════════════════
    // WRITE OPERATIONS
    // ═══════════════════════════════════════════════════════════════════

    /// Store a new memory
    pub fn remember(&mut self, content: &str, tags: &[&str]) -> Result<u64> {
        if self.read_only {
            return Err(EngramError::ReadOnly);
        }

        // Refresh from disk if another process modified the file
        // This prevents overwriting their changes when we persist
        self.refresh_if_modified()?;

        let id = self.next_id;
        self.next_id += 1;

        let entry = NoteEntry::new_encrypted(id, content, tags, true, &self.cipher)?;
        self.write_note_entry(&entry)?;

        // Update indexes
        let offset = self.header.note_log_offset + self.header.note_log_size - entry.total_size() as u64;
        self.note_index.insert(id, offset);
        self.temporal_index.push((entry.timestamp, id));

        for tag in tags {
            self.tag_bloom.insert(&tag.to_string()); // Add to bloom filter
            self.tag_index
                .entry(tag.to_string())
                .or_insert_with(Vec::new)
                .push(id);
        }

        // Update fingerprint index for associative recall
        self.fingerprint_index.upsert(id, content, tags);

        // Update header
        self.header.note_count += 1;
        self.header.active_notes += 1;
        self.header.touch();
        self.header.write_to(&mut self.file)?;

        // Note: We don't remap here - lazy remap for performance
        // New notes will be read via file I/O until sync() is called
        // This makes writes ~5x faster

        // Persist indexes to ensure embeddings/graph survive restart
        self.persist_indexes()?;

        // Invalidate BM25 corpus cache — note set changed
        self.bm25_cache = None;

        Ok(id)
    }

    /// Store a memory with prospective trigger keywords.
    ///
    /// Trigger keywords get 3x bloom weight in the fingerprint, causing this note
    /// to surface automatically when the AI's working context matches the triggers.
    /// Example: `remember_with_triggers("Check PQC status", &["security"], &["federation", "pqc"])`
    /// will surface when federation-related work begins.
    pub fn remember_with_triggers(&mut self, content: &str, tags: &[&str], triggers: &[&str]) -> Result<u64> {
        if self.read_only {
            return Err(EngramError::ReadOnly);
        }

        self.refresh_if_modified()?;

        let id = self.next_id;
        self.next_id += 1;

        let entry = NoteEntry::new_encrypted(id, content, tags, true, &self.cipher)?;
        self.write_note_entry(&entry)?;

        let offset = self.header.note_log_offset + self.header.note_log_size - entry.total_size() as u64;
        self.note_index.insert(id, offset);
        self.temporal_index.push((entry.timestamp, id));

        for tag in tags {
            self.tag_bloom.insert(&tag.to_string());
            self.tag_index
                .entry(tag.to_string())
                .or_insert_with(Vec::new)
                .push(id);
        }

        // Fingerprint with trigger boost
        self.fingerprint_index.upsert_with_triggers(id, content, tags, triggers);

        self.header.note_count += 1;
        self.header.active_notes += 1;
        self.header.touch();
        self.header.write_to(&mut self.file)?;

        self.persist_indexes()?;
        self.bm25_cache = None;

        Ok(id)
    }

    /// Store an ephemeral (working) memory that expires after ttl_hours.
    /// Expired notes are filtered from recall results automatically.
    /// Use for session-scoped context, in-progress task notes, or any memory
    /// that should self-destruct rather than accumulate indefinitely.
    pub fn remember_working(&mut self, content: &str, tags: &[&str], ttl_hours: u16) -> Result<u64> {
        if self.read_only {
            return Err(EngramError::ReadOnly);
        }

        self.refresh_if_modified()?;

        let id = self.next_id;
        self.next_id += 1;

        let mut entry = NoteEntry::new_encrypted(id, content, tags, true, &self.cipher)?;
        entry.ttl_hours = ttl_hours;
        self.write_note_entry(&entry)?;

        let offset = self.header.note_log_offset + self.header.note_log_size - entry.total_size() as u64;
        self.note_index.insert(id, offset);
        self.temporal_index.push((entry.timestamp, id));

        for tag in tags {
            self.tag_bloom.insert(&tag.to_string());
            self.tag_index
                .entry(tag.to_string())
                .or_insert_with(Vec::new)
                .push(id);
        }

        // Update fingerprint index for associative recall
        self.fingerprint_index.upsert(id, content, tags);

        self.header.note_count += 1;
        self.header.active_notes += 1;
        self.header.touch();
        self.header.write_to(&mut self.file)?;

        self.persist_indexes()?;

        // Invalidate BM25 corpus cache — note set changed
        self.bm25_cache = None;

        Ok(id)
    }

    /// Store multiple memories in a single batch (much faster for bulk inserts)
    pub fn remember_batch(&mut self, items: &[(&str, &[&str])]) -> Result<Vec<u64>> {
        if self.read_only {
            return Err(EngramError::ReadOnly);
        }

        if items.is_empty() {
            return Ok(Vec::new());
        }

        let mut ids = Vec::with_capacity(items.len());
        let mut entries = Vec::with_capacity(items.len());

        // Create all entries first
        for (content, tags) in items {
            let id = self.next_id;
            self.next_id += 1;
            ids.push(id);

            let entry = NoteEntry::new_encrypted(id, content, tags, true, &self.cipher)?;
            entries.push((id, entry, tags.to_vec()));
        }

        // Invalidate persisted indexes BEFORE writing - new data will overwrite them
        self.invalidate_persisted_indexes()?;

        // Seek to write position once
        let write_offset = self.header.note_log_offset + self.header.note_log_size;
        self.file.seek(SeekFrom::Start(write_offset))?;

        // Write all entries
        let mut total_bytes = 0u64;
        for (id, entry, tags) in &entries {
            let bytes = entry.to_bytes();
            self.file.write_all(&bytes)?;

            // Update indexes
            let offset = write_offset + total_bytes;
            self.note_index.insert(*id, offset);
            self.temporal_index.push((entry.timestamp, *id));

            for tag in tags {
                self.tag_bloom.insert(&tag.to_string()); // Add to bloom filter
                self.tag_index
                    .entry(tag.to_string())
                    .or_insert_with(Vec::new)
                    .push(*id);
            }

            total_bytes += bytes.len() as u64;
        }

        // Update fingerprint index for all batch items
        for (i, (content, tags)) in items.iter().enumerate() {
            self.fingerprint_index.upsert(ids[i], content, tags);
        }

        // Update header once
        self.header.note_log_size += total_bytes;
        self.header.note_count += entries.len() as u64;
        self.header.active_notes += entries.len() as u64;
        self.header.touch();
        self.header.write_to(&mut self.file)?;

        // Note: We don't remap here - lazy remap for performance
        // Call sync() after batch writes to remap if needed

        // Persist indexes to ensure embeddings/graph survive restart
        self.persist_indexes()?;

        // Invalidate BM25 corpus cache — note set changed
        self.bm25_cache = None;

        Ok(ids)
    }

    /// Write a note entry to the file
    fn write_note_entry(&mut self, entry: &NoteEntry) -> Result<()> {
        // Invalidate persisted indexes BEFORE writing - new data will overwrite them
        // The indexes are stored at note_log_offset + note_log_size, which is exactly
        // where we're about to write. Without this, the persisted index data gets
        // corrupted and load_persisted_indexes() fails on next open.
        self.invalidate_persisted_indexes()?;

        let bytes = entry.to_bytes();

        // Seek to end of note log
        let write_offset = self.header.note_log_offset + self.header.note_log_size;
        self.file.seek(SeekFrom::Start(write_offset))?;
        self.file.write_all(&bytes)?;

        // Update header
        self.header.note_log_size += bytes.len() as u64;

        Ok(())
    }

    /// Mark a note as deleted (tombstone)
    pub fn forget(&mut self, id: u64) -> Result<()> {
        if self.read_only {
            return Err(EngramError::ReadOnly);
        }

        // Refresh from disk if another process modified the file
        self.refresh_if_modified()?;

        if !self.note_index.contains_key(&id) {
            return Err(EngramError::NoteNotFound(id));
        }

        // Write tombstone entry
        let tombstone = NoteEntry::tombstone(id);
        self.write_note_entry(&tombstone)?;

        // Update indexes
        self.note_index.remove(&id);
        self.pinned.retain(|&pid| pid != id);
        self.temporal_index.retain(|(_, nid)| *nid != id);

        for ids in self.tag_index.values_mut() {
            ids.retain(|&nid| nid != id);
        }

        // Invalidate cache
        self.cache.invalidate(id);

        // Tombstone in fingerprint index (belt-and-suspenders for sidecar consumers)
        self.fingerprint_index.mark_tombstoned(id);
        // Remove from fingerprint index
        self.fingerprint_index.remove(id);

        // Update header
        self.header.active_notes -= 1;
        self.header.touch();
        self.header.write_to(&mut self.file)?;

        // Note: We don't remap here - lazy remap for performance

        // Persist indexes to ensure embeddings/graph survive restart
        self.persist_indexes()?;

        // Invalidate BM25 corpus cache — note set changed
        self.bm25_cache = None;

        Ok(())
    }

    /// Update a note's content and/or tags (preserves ID)
    ///
    /// If content is None, keeps existing content.
    /// If tags is None, keeps existing tags.
    pub fn update(&mut self, id: u64, content: Option<&str>, tags: Option<&[&str]>) -> Result<()> {
        if self.read_only {
            return Err(EngramError::ReadOnly);
        }

        // Refresh from disk if another process modified the file
        self.refresh_if_modified()?;

        // Get existing note
        let existing = self.get(id)?
            .ok_or(EngramError::NoteNotFound(id))?;

        // Determine final content and tags
        let final_content = content.unwrap_or(&existing.content);
        let existing_tags: Vec<&str> = existing.tags.iter().map(|s| s.as_str()).collect();
        let final_tags: &[&str] = tags.unwrap_or(&existing_tags);

        // Remove old tags from index
        for ids in self.tag_index.values_mut() {
            ids.retain(|&nid| nid != id);
        }

        // Write new entry with same ID, stamping updated_at so recency reflects edits
        let mut entry = NoteEntry::new_encrypted(id, final_content, final_tags, true, &self.cipher)?;
        entry.updated_at = chrono::Utc::now().timestamp() as u32;
        self.write_note_entry(&entry)?;

        // Update note_index to point to new offset
        let offset = self.header.note_log_offset + self.header.note_log_size - entry.total_size() as u64;
        self.note_index.insert(id, offset);

        // Add new tags to index
        for tag in final_tags {
            self.tag_bloom.insert(&tag.to_string());
            self.tag_index
                .entry(tag.to_string())
                .or_insert_with(Vec::new)
                .push(id);
        }

        // Invalidate cache
        self.cache.invalidate(id);

        // Update fingerprint index with new content/tags
        self.fingerprint_index.upsert(id, final_content, final_tags);

        // Update header timestamp
        self.header.touch();
        self.header.write_to(&mut self.file)?;

        // Persist indexes
        self.persist_indexes()?;

        // Invalidate BM25 corpus cache — note content changed
        self.bm25_cache = None;

        Ok(())
    }

    /// Pin a note
    pub fn pin(&mut self, id: u64) -> Result<()> {
        if self.read_only {
            return Err(EngramError::ReadOnly);
        }

        // Refresh from disk if another process modified the file
        self.refresh_if_modified()?;

        if !self.note_index.contains_key(&id) {
            return Err(EngramError::NoteNotFound(id));
        }

        // Enforce pinned limit at core level
        if !self.pinned.contains(&id) && self.pinned.len() >= MAX_PINNED {
            return Err(EngramError::PinnedLimitReached(MAX_PINNED));
        }

        if !self.pinned.contains(&id) {
            self.pinned.push(id);
            self.cache.invalidate(id); // Invalidate to refresh pinned status
            self.persist_indexes()?; // Persist pins to .engram file
        }

        Ok(())
    }

    /// Unpin a note
    pub fn unpin(&mut self, id: u64) -> Result<()> {
        if self.read_only {
            return Err(EngramError::ReadOnly);
        }

        // Refresh from disk if another process modified the file
        self.refresh_if_modified()?;

        let had_pin = self.pinned.contains(&id);
        self.pinned.retain(|&pid| pid != id);
        self.cache.invalidate(id);
        if had_pin {
            self.persist_indexes()?; // Persist pins to .engram file
        }
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════════
    // READ OPERATIONS (Memory-Mapped + Cached)
    // ═══════════════════════════════════════════════════════════════════

    /// Get a note by ID (uses cache + mmap with file I/O fallback)
    pub fn get(&mut self, id: u64) -> Result<Option<Note>> {
        // Check cache first
        if let Some(note) = self.cache.get(id) {
            self.cache_hits += 1;
            return Ok(Some(note.clone()));
        }

        self.cache_misses += 1;

        let offset = match self.note_index.get(&id) {
            Some(&off) => off,
            None => return Ok(None),
        };

        // Read using mmap if offset is within mapped range, else use file I/O
        // This handles lazy remap - newly written notes not yet in mmap
        let note = if self.is_offset_in_mmap(offset) {
            if let Some(ref mmap) = self.mmap {
                self.read_note_mmap(mmap, offset)?
            } else {
                self.read_note_file(offset)?
            }
        } else {
            // Note written after last mmap - use file I/O
            self.read_note_file(offset)?
        };

        // Cache the result
        if let Some(ref n) = note {
            self.cache.insert(id, n.clone());
        }

        Ok(note)
    }

    /// Read a note using memory-mapped I/O (zero-copy until decompression)
    fn read_note_mmap(&self, mmap: &Mmap, offset: u64) -> Result<Option<Note>> {
        let offset = offset as usize;

        if offset + 32 > mmap.len() {
            return Ok(None);
        }

        // Read header directly from mapped memory
        let header_bytes = &mmap[offset..offset + 32];

        let content_len = u32::from_le_bytes(header_bytes[20..24].try_into().unwrap()) as usize;
        let tags_len = u16::from_le_bytes(header_bytes[24..26].try_into().unwrap()) as usize;

        let entry_size = 32 + tags_len + content_len;
        if offset + entry_size > mmap.len() {
            return Ok(None);
        }

        // Read full entry from mapped memory (still zero-copy)
        let entry_bytes = &mmap[offset..offset + entry_size];

        let entry = NoteEntry::from_bytes(entry_bytes)?;

        if entry.is_tombstone() {
            return Ok(None);
        }

        match entry.to_note_decrypted(&self.cipher) {
            Ok(mut note) => {
                note.pinned = self.pinned.contains(&note.id);
                Ok(Some(note))
            }
            Err(_) => Ok(None), // Skip notes that can't be decrypted (key mismatch)
        }
    }

    /// Read a note using file I/O (fallback)
    fn read_note_file(&mut self, offset: u64) -> Result<Option<Note>> {
        self.file.seek(SeekFrom::Start(offset))?;

        let mut header_buf = [0u8; 32];
        self.file.read_exact(&mut header_buf)?;

        let content_len = u32::from_le_bytes(header_buf[20..24].try_into().unwrap());
        let tags_len = u16::from_le_bytes(header_buf[24..26].try_into().unwrap());

        let entry_size = 32 + tags_len as usize + content_len as usize;
        let mut full_buf = vec![0u8; entry_size];
        full_buf[..32].copy_from_slice(&header_buf);

        self.file.seek(SeekFrom::Start(offset + 32))?;
        self.file.read_exact(&mut full_buf[32..])?;

        let entry = NoteEntry::from_bytes(&full_buf)?;

        if entry.is_tombstone() {
            return Ok(None);
        }

        match entry.to_note_decrypted(&self.cipher) {
            Ok(mut note) => {
                note.pinned = self.pinned.contains(&note.id);
                Ok(Some(note))
            }
            Err(_) => Ok(None), // Skip notes that can't be decrypted (key mismatch)
        }
    }

    /// Get multiple notes by ID (batched, uses cache + mmap)
    pub fn get_batch(&mut self, ids: &[u64]) -> Result<Vec<Option<Note>>> {
        let mut results = Vec::with_capacity(ids.len());

        for &id in ids {
            results.push(self.get(id)?);
        }

        Ok(results)
    }

    /// Get most recent notes
    pub fn recent(&mut self, limit: usize) -> Result<Vec<Note>> {
        // Check for external modifications (multi-process sync)
        self.refresh_if_modified()?;

        // Collect IDs first to avoid borrow issues
        let ids: Vec<u64> = self.temporal_index
            .iter()
            .rev()
            .take(limit * 2)
            .map(|(_, id)| *id)
            .collect();

        let mut notes = Vec::new();
        for id in ids {
            if let Some(note) = self.get(id)? {
                notes.push(note);
                if notes.len() >= limit {
                    break;
                }
            }
        }

        Ok(notes)
    }

    /// Get notes by tag
    ///
    /// Uses Bloom filter for fast negative lookups - if a tag definitely
    /// doesn't exist, returns immediately without scanning the index.
    pub fn by_tag(&mut self, tag: &str) -> Result<Vec<Note>> {
        // Fast path: Bloom filter says tag definitely doesn't exist
        if !self.tag_bloom.might_contain(&tag.to_string()) {
            return Ok(Vec::new());
        }

        // Bloom filter says "might exist" - check the actual index
        let ids = match self.tag_index.get(tag) {
            Some(ids) => ids.clone(),
            None => return Ok(Vec::new()), // False positive from bloom filter
        };

        let mut notes = Vec::new();
        for id in ids {
            if let Some(note) = self.get(id)? {
                notes.push(note);
            }
        }

        Ok(notes)
    }

    /// Get all tags with note counts, sorted by count descending
    pub fn all_tags(&self) -> Vec<(String, usize)> {
        let mut tags: Vec<(String, usize)> = self.tag_index
            .iter()
            .map(|(tag, ids)| (tag.clone(), ids.len()))
            .collect();
        tags.sort_by(|a, b| b.1.cmp(&a.1));
        tags
    }

    /// Get notes in a time range
    pub fn temporal_range(&mut self, start: i64, end: i64) -> Result<Vec<Note>> {
        // Collect IDs first to avoid borrow issues
        let ids: Vec<u64> = self.temporal_index
            .iter()
            .filter(|(ts, _)| *ts >= start && *ts <= end)
            .map(|(_, id)| *id)
            .collect();

        let mut notes = Vec::new();
        for id in ids {
            if let Some(note) = self.get(id)? {
                notes.push(note);
            }
        }

        Ok(notes)
    }

    /// Get pinned notes
    pub fn pinned(&mut self) -> Result<Vec<Note>> {
        // Check for external modifications (multi-process sync)
        self.refresh_if_modified()?;

        let ids = self.pinned.clone();
        let mut notes = Vec::new();

        for id in ids {
            if let Some(note) = self.get(id)? {
                notes.push(note);
            }
        }

        Ok(notes)
    }

    /// List all notes
    pub fn list(&mut self, limit: usize) -> Result<Vec<Note>> {
        self.recent(limit)
    }

    // ═══════════════════════════════════════════════════════════════════
    // MAINTENANCE
    // ═══════════════════════════════════════════════════════════════════

    /// Database statistics
    pub fn stats(&mut self) -> EngramStats {
        // Check for external modifications (multi-process sync)
        let _ = self.refresh_if_modified(); // Ignore errors for stats

        let file_size = self.file.metadata().map(|m| m.len()).unwrap_or(0);

        EngramStats {
            note_count: self.header.note_count,
            active_notes: self.header.active_notes,
            tombstone_count: self.header.note_count - self.header.active_notes,
            pinned_count: self.pinned.len() as u64,
            vector_count: self.vector_store.len() as u64,
            edge_count: self.graph.edge_count() as u64,
            tag_count: self.tag_index.len() as u64,
            vault_entries: self.vault.len() as u64,
            file_size,
            created_at: self.header.created_at,
            modified_at: self.header.modified_at,
        }
    }

    /// Cache statistics
    pub fn cache_stats(&self) -> (u64, u64, f64) {
        let total = self.cache_hits + self.cache_misses;
        let hit_rate = if total > 0 {
            self.cache_hits as f64 / total as f64
        } else {
            0.0
        };
        (self.cache_hits, self.cache_misses, hit_rate)
    }

    /// Clear the read cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
        self.cache_hits = 0;
        self.cache_misses = 0;
    }

    /// Verify database integrity
    pub fn verify(&mut self) -> Result<VerifyResult> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Verify header
        if let Err(e) = self.header.verify() {
            errors.push(format!("Header verification failed: {}", e));
        }

        // Verify note count matches index
        if self.note_index.len() as u64 != self.header.active_notes {
            warnings.push(format!(
                "Active notes mismatch: header says {}, index has {}",
                self.header.active_notes,
                self.note_index.len()
            ));
        }

        // Verify all indexed notes are readable
        let mut unreadable = 0;
        let ids: Vec<u64> = self.note_index.keys().copied().collect();
        for id in ids {
            if self.get(id)?.is_none() {
                unreadable += 1;
            }
        }
        if unreadable > 0 {
            warnings.push(format!("{} indexed notes are unreadable", unreadable));
        }

        // Report cache stats
        let (hits, misses, rate) = self.cache_stats();
        if hits + misses > 0 {
            warnings.push(format!(
                "Cache: {} hits, {} misses, {:.1}% hit rate",
                hits, misses, rate * 100.0
            ));
        }

        Ok(VerifyResult {
            is_valid: errors.is_empty(),
            errors,
            warnings,
        })
    }

    /// Force sync to disk and update memory map
    /// Call after batch writes to make all data available via mmap
    pub fn sync(&mut self) -> Result<()> {
        self.file.sync_all()?;
        self.remap()
    }

    /// Get the file path
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Check if memory-mapped
    pub fn is_mapped(&self) -> bool {
        self.mmap.is_some()
    }

    /// Rebuild fingerprint index from all existing notes using IDF-weighted SimHash.
    ///
    /// Call this after upgrading to fingerprint support, or when the .engram.fp
    /// sidecar is missing/corrupt. Two-pass approach:
    ///   1. Collect all notes and tokenize → build IDF table from corpus
    ///   2. Compute IDF-weighted fingerprints (rare tokens contribute more)
    ///
    /// IDF weighting improves retrieval precision by ~10-15% vs uniform weighting
    /// at zero scan-time cost (fingerprints are the same 128-bit format).
    ///
    /// Returns the number of notes fingerprinted.
    pub fn backfill_fingerprints(&mut self) -> Result<usize> {
        use crate::fingerprint::{
            tokenize_and_stem, create_stemmer, IdfTable, Fingerprint256, FingerprintEntry256,
        };

        let ids: Vec<u64> = self.note_index.keys().copied().collect();
        let stemmer = create_stemmer();

        // Phase 1: Collect all notes and build tokenized corpus for IDF
        let mut notes_data: Vec<(u64, String, Vec<String>)> = Vec::with_capacity(ids.len());
        let mut corpus: Vec<Vec<String>> = Vec::with_capacity(ids.len());

        let mut skipped = 0usize;
        for id in &ids {
            match self.get(*id) {
                Ok(Some(note)) => {
                    let mut tokens = tokenize_and_stem(&note.content, &stemmer);
                    // Include tag tokens in corpus (same as from_text_with_idf bloom path)
                    for tag in &note.tags {
                        let lower = tag.to_lowercase();
                        let stemmed = stemmer.stem(&lower).into_owned();
                        if !stemmed.is_empty() {
                            tokens.push(stemmed);
                        }
                    }
                    corpus.push(tokens);
                    notes_data.push((*id, note.content, note.tags));
                }
                Ok(None) => {} // tombstone or missing
                Err(_) => {
                    // Decryption error (wrong key) or corrupt entry — skip gracefully.
                    // This allows backfill to work on mixed-key notebooks where some
                    // notes were encrypted with a different AI_ID cipher.
                    skipped += 1;
                }
            }
        }
        if skipped > 0 {
            eprintln!("engram: backfill skipped {} notes (decrypt/read errors)", skipped);
        }

        // Phase 2: Build IDF table from full corpus
        let idf = IdfTable::from_corpus(&corpus);

        // Phase 3: Compute IDF-weighted 256-bit fingerprints
        self.fingerprint_index = FingerprintIndex256::with_capacity(notes_data.len());

        for (id, content, tags) in &notes_data {
            let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
            let fp = Fingerprint256::from_text_with_idf(content, &tag_refs, &idf);
            self.fingerprint_index.upsert_entry(FingerprintEntry256 {
                fingerprint: fp,
                note_id: *id,
                flags: 0,
            });
        }

        // Mark pinned notes before saving sidecar
        self.fingerprint_index.mark_pinned(&self.pinned);

        // Save V3 sidecar (48-byte entries with 128-bit SimHash + 128-bit Bloom)
        let fp_path = FingerprintIndex256::sidecar_path(&self.path);
        self.fingerprint_index.save(&fp_path)
            .map_err(|e| EngramError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("fingerprint save failed: {}", e),
            )))?;

        Ok(notes_data.len())
    }

    /// Get the current fingerprint index entry count.
    pub fn fingerprint_count(&self) -> usize {
        self.fingerprint_index.len()
    }

    /// Get a read-only reference to the fingerprint index entries.
    pub fn fingerprint_entries(&self) -> &[crate::fingerprint::FingerprintEntry256] {
        self.fingerprint_index.entries()
    }

    /// Scan fingerprint index for top-K matches against a query fingerprint.
    ///
    /// Used for calibration and debugging. Returns results sorted by score descending.
    pub fn fingerprint_scan_top_k(
        &self,
        context: &crate::fingerprint::Fingerprint256,
        k: usize,
        max_hd: Option<u32>,
    ) -> Vec<crate::fingerprint::ScanResult> {
        self.fingerprint_index.scan_top_k(context, k, max_hd)
    }

    /// Scan fingerprint index for best single match.
    pub fn fingerprint_scan_best(
        &self,
        context: &crate::fingerprint::Fingerprint256,
        max_hd: Option<u32>,
    ) -> Option<crate::fingerprint::ScanResult> {
        self.fingerprint_index.scan_best(context, max_hd)
    }

    // ═══════════════════════════════════════════════════════════════════
    // VECTOR OPERATIONS (Phase 4)
    // ═══════════════════════════════════════════════════════════════════

    /// Add an embedding vector for a note
    pub fn add_embedding(&mut self, id: u64, embedding: &[f32]) -> Result<()> {
        if self.read_only {
            return Err(EngramError::ReadOnly);
        }

        // Add to vector store
        self.vector_store.add(id, embedding)?;

        // Add to HNSW index for fast ANN search
        self.hnsw_index.add(id, embedding);

        // Persist to ensure embeddings survive restart
        self.persist_indexes()?;

        Ok(())
    }

    /// Search for similar notes using vector similarity
    /// Returns (note_id, similarity_score) pairs sorted by similarity
    pub fn search_similar(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        let raw = if self.hnsw_index.is_empty() {
            // Fall back to brute-force if no HNSW index
            self.vector_store.nearest(query, k)
        } else {
            // Use HNSW for O(log n) approximate nearest neighbor search
            // Request extra candidates to account for filtering
            self.hnsw_index.search(query, k + 10, (k + 10).max(50))
        };

        // Filter out deleted notes (HNSW/VectorStore may contain stale entries)
        raw.into_iter()
            .filter(|(id, _)| self.note_index.contains_key(id))
            .take(k)
            .collect()
    }

    /// Get embedding vector for a note
    pub fn get_embedding(&self, id: u64) -> Option<&[f32]> {
        self.vector_store.get(id)
    }

    /// Check if a note has an embedding
    /// Check if a note has a non-zero embedding
    /// Zero-vectors are not valid embeddings (sparse storage uses zeros for missing IDs)
    pub fn has_embedding(&self, id: u64) -> bool {
        self.vector_store.has(id)
    }

    // ═══════════════════════════════════════════════════════════════════
    // VAULT OPERATIONS (Phase 4 - Encrypted Key-Value Store)
    // ═══════════════════════════════════════════════════════════════════

    /// Store a secret value in the encrypted vault
    ///
    /// Note: Auto-persists indexes to ensure vault durability across sessions.
    pub fn vault_set(&mut self, key: &str, value: &[u8]) -> Result<()> {
        if self.read_only {
            return Err(EngramError::ReadOnly);
        }
        // Refresh from disk if another process modified the file
        self.refresh_if_modified()?;
        self.vault.set(key, value)?;
        // Auto-persist to ensure vault durability
        self.persist_indexes()
    }

    /// Store a string value in the encrypted vault
    pub fn vault_set_string(&mut self, key: &str, value: &str) -> Result<()> {
        self.vault_set(key, value.as_bytes())
    }

    /// Get a secret value from the vault
    pub fn vault_get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.vault.get(key)
    }

    /// Get a string value from the vault
    pub fn vault_get_string(&self, key: &str) -> Result<Option<String>> {
        match self.vault.get(key)? {
            Some(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).to_string())),
            None => Ok(None),
        }
    }

    /// Delete a key from the vault
    ///
    /// Note: Auto-persists indexes to ensure vault durability across sessions.
    pub fn vault_delete(&mut self, key: &str) -> bool {
        if self.read_only {
            return false;
        }
        let deleted = self.vault.delete(key);
        if deleted {
            // Auto-persist to ensure vault durability
            let _ = self.persist_indexes();
        }
        deleted
    }

    /// List all keys in the vault
    pub fn vault_keys(&self) -> Vec<String> {
        self.vault.keys()
    }

    /// Check if a key exists in the vault
    pub fn vault_contains(&self, key: &str) -> bool {
        self.vault.contains(key)
    }

    // ═══════════════════════════════════════════════════════════════════
    // GRAPH OPERATIONS (Phase 4 - Knowledge Graph with PageRank)
    // ═══════════════════════════════════════════════════════════════════

    /// Add an edge between two notes
    pub fn add_edge(&mut self, from: u64, to: u64, weight: f32, edge_type: EdgeType) {
        if !self.read_only {
            self.graph.add_edge(from, to, weight, edge_type);
            // Persist to ensure graph edges survive restart
            let _ = self.persist_indexes();
        }
    }

    /// Add a semantic edge (high cosine similarity)
    pub fn add_semantic_edge(&mut self, from: u64, to: u64, similarity: f32) {
        self.add_edge(from, to, similarity, EdgeType::Semantic);
    }

    /// Add a temporal edge (created close in time)
    pub fn add_temporal_edge(&mut self, from: u64, to: u64) {
        self.add_edge(from, to, 1.0, EdgeType::Temporal);
    }

    /// Remove an edge between two notes
    pub fn remove_edge(&mut self, from: u64, to: u64) -> bool {
        if !self.read_only {
            let removed = self.graph.remove_edge(from, to);
            if removed {
                let _ = self.persist_indexes();
            }
            removed
        } else {
            false
        }
    }

    /// Invalidate an edge between two notes, removing it from graph scoring
    ///
    /// Use this when a newer note supersedes or contradicts the relationship
    /// represented by this edge. Differs from `remove_edge` semantically:
    /// this is "this connection is no longer valid", not "this was a mistake".
    ///
    /// Triggers PageRank recompute so the change is immediately reflected
    /// in recall scoring without requiring a restart.
    ///
    /// Returns true if an edge was found and invalidated.
    pub fn invalidate_edge(&mut self, from: u64, to: u64) -> bool {
        if !self.read_only {
            let removed = self.graph.invalidate_edge(from, to);
            if removed {
                self.graph.compute_pagerank(20, 0.85);
                let _ = self.persist_indexes();
            }
            removed
        } else {
            false
        }
    }

    /// Get related notes via graph edges
    pub fn get_related(&self, id: u64) -> Vec<(u64, f32, EdgeType)> {
        self.graph
            .neighbors(id)
            .iter()
            .map(|edge| (edge.target, edge.weight, edge.edge_type))
            .collect()
    }

    /// Get PageRank score for a note
    pub fn get_pagerank(&self, id: u64) -> f32 {
        self.graph.get_pagerank(id)
    }

    /// Compute PageRank scores for all notes
    pub fn compute_pagerank(&mut self) {
        self.graph.compute_pagerank(20, 0.85);
    }

    /// Remove a note from the graph
    pub fn remove_from_graph(&mut self, id: u64) {
        if !self.read_only {
            self.graph.remove_node(id);
        }
    }

    /// Dump graph info for debugging
    /// Returns (edge_count, node_count, sample_node_ids)
    pub fn dump_graph_info(&self) -> (usize, usize, Vec<u64>) {
        let edge_count = self.graph.edge_count();
        let node_count = self.graph.node_count();
        let sample_nodes = self.graph.sample_nodes(20);
        (edge_count, node_count, sample_nodes)
    }

    // ═══════════════════════════════════════════════════════════════════
    // HYBRID RECALL (Phase 4 - Vector + Keyword + Graph + Recency)
    // ═══════════════════════════════════════════════════════════════════

    /// Set recall configuration
    pub fn set_recall_config(&mut self, config: RecallConfig) {
        self.recall_config = config;
    }

    /// Hybrid recall - combines vector similarity, keyword matching, graph scores, and recency
    ///
    /// This is the main search function that intelligently combines all signals:
    /// - Vector similarity (semantic meaning)
    /// - Keyword matching (BM25 with cached corpus IDF)
    /// - Graph scores (PageRank importance)
    /// - Recency (temporal relevance)
    pub fn recall(&mut self, query: &str, query_embedding: Option<&[f32]>, limit: usize) -> Result<Vec<RecallResult>> {
        let mut candidates: std::collections::HashMap<u64, RecallResult> = std::collections::HashMap::new();

        // Pre-compute timestamp once for all recency scoring (avoids per-note syscall)
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        // 1. Vector search (if embedding provided)
        if let Some(embedding) = query_embedding {
            let similar = self.search_similar(embedding, limit * 3);
            for (id, sim) in similar {
                if let Some(note) = self.get(id)? {
                    candidates.insert(id, RecallResult {
                        note,
                        vector_score: sim,
                        keyword_score: 0.0,
                        graph_score: 0.0,
                        recency_score: 0.0,
                        final_score: 0.0,
                    });
                }
            }
        }

        // 2. Keyword search (BM25 with cached corpus IDF)
        // Load all notes for BM25 scoring. The corpus (IDF + avgdl) is cached across
        // recalls and only rebuilt when notes are added/deleted/updated.
        let all_ids: Vec<u64> = self.temporal_index
            .iter()
            .rev()
            .map(|(_, id)| *id)
            .collect();

        let mut notes_for_bm25: Vec<(u64, Note)> = Vec::with_capacity(all_ids.len());
        for id in all_ids {
            if let Some(note) = self.get(id)? {
                notes_for_bm25.push((id, note));
            }
        }

        // Build or reuse cached BM25 corpus statistics (IDF + avgdl).
        // Rebuilding only on note count change is a cheap heuristic that avoids
        // O(n * words) IDF computation on every recall. The cache is also explicitly
        // invalidated by remember/forget/update methods for correctness.
        let current_note_count = notes_for_bm25.len() as u64;
        if self.bm25_cache.is_none() || self.bm25_cache_note_count != current_note_count {
            let content_refs: Vec<&str> = notes_for_bm25.iter().map(|(_, n)| n.content.as_str()).collect();
            self.bm25_cache = Some(BM25Corpus::new(&content_refs));
            self.bm25_cache_note_count = current_note_count;
        }

        // Score each note against the query using corpus-aware BM25
        if let Some(ref corpus) = self.bm25_cache {
            for (id, note) in notes_for_bm25 {
                let bm25 = corpus.score(&note.content, query, 1.2, 0.75);
                if bm25 > 0.0 {
                    candidates.entry(id).or_insert_with(|| RecallResult {
                        note: note.clone(),
                        vector_score: 0.0,
                        keyword_score: 0.0,
                        graph_score: 0.0,
                        recency_score: 0.0,
                        final_score: 0.0,
                    }).keyword_score = bm25;
                }
            }
        }

        // Filter expired working-memory notes before scoring
        candidates.retain(|_, result| !result.note.is_expired());

        // 3. Add graph scores and recency for all candidates
        for (id, result) in candidates.iter_mut() {
            result.graph_score = self.graph.get_pagerank(*id);
            // Use effective_timestamp_nanos() so edits (updated_at) boost recency,
            // not just the original creation time. Uses pre-computed now_nanos to
            // avoid calling chrono::Utc::now() per candidate.
            result.recency_score = recency_score_at(
                result.note.effective_timestamp_nanos(),
                self.recall_config.recency_half_life_hours,
                now_nanos,
            );
        }

        // 4. Normalize scores within each category
        let mut vector_scores: Vec<(u64, f32)> = candidates.iter().map(|(id, r)| (*id, r.vector_score)).collect();
        let mut keyword_scores: Vec<(u64, f32)> = candidates.iter().map(|(id, r)| (*id, r.keyword_score)).collect();
        let mut graph_scores: Vec<(u64, f32)> = candidates.iter().map(|(id, r)| (*id, r.graph_score)).collect();

        // Vector: quality-gated at 0.3 cosine sim. If best match is below 0.3,
        // the signal is noise — attenuate instead of inflating to 1.0.
        crate::recall::normalize_scores_gated(&mut vector_scores, 0.3);
        crate::recall::normalize_scores(&mut keyword_scores);
        crate::recall::normalize_scores(&mut graph_scores);

        // Update with normalized scores
        for (id, score) in vector_scores {
            if let Some(r) = candidates.get_mut(&id) {
                r.vector_score = score;
            }
        }
        for (id, score) in keyword_scores {
            if let Some(r) = candidates.get_mut(&id) {
                r.keyword_score = score;
            }
        }
        for (id, score) in graph_scores {
            if let Some(r) = candidates.get_mut(&id) {
                r.graph_score = score;
            }
        }

        // 5. Compute weighted final score
        for result in candidates.values_mut() {
            result.final_score =
                self.recall_config.vector_weight * result.vector_score +
                self.recall_config.keyword_weight * result.keyword_score +
                self.recall_config.graph_weight * result.graph_score +
                self.recall_config.recency_weight * result.recency_score;
        }

        // 6. Sort by final score and return top results
        let mut results: Vec<RecallResult> = candidates.into_values().collect();
        results.sort_by(|a, b| b.final_score.partial_cmp(&a.final_score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        Ok(results)
    }

    /// Simple keyword-only recall (no embedding required)
    pub fn recall_by_keyword(&mut self, query: &str, limit: usize) -> Result<Vec<RecallResult>> {
        self.recall(query, None, limit)
    }

    // ═══════════════════════════════════════════════════════════════════
    // AUTO-LINKING (Phase 4 - Automatic Edge Creation)
    // ═══════════════════════════════════════════════════════════════════

    /// Automatically create semantic edges for a note based on embedding similarity
    /// Returns the number of edges created
    pub fn auto_link_semantic(&mut self, id: u64, threshold: f32, max_links: usize) -> Result<usize> {
        let embedding = match self.vector_store.get(id) {
            Some(e) => e.to_vec(),
            None => return Ok(0),
        };

        let similar = self.search_similar(&embedding, max_links + 1);
        let mut count = 0;

        for (other_id, similarity) in similar {
            if other_id != id && similarity >= threshold {
                self.add_semantic_edge(id, other_id, similarity);
                count += 1;
                if count >= max_links {
                    break;
                }
            }
        }

        Ok(count)
    }

    /// Automatically create temporal edges for notes within a time window
    /// Returns the number of edges created
    pub fn auto_link_temporal(&mut self, id: u64, window_minutes: i64) -> Result<usize> {
        let note = match self.get(id)? {
            Some(n) => n,
            None => return Ok(0),
        };

        let window_nanos = window_minutes * 60 * 1_000_000_000;
        let start = note.timestamp - window_nanos;
        let end = note.timestamp + window_nanos;

        // Find notes within the time window
        let nearby_ids: Vec<u64> = self.temporal_index
            .iter()
            .filter(|(ts, nid)| *ts >= start && *ts <= end && *nid != id)
            .map(|(_, nid)| *nid)
            .collect();

        for other_id in &nearby_ids {
            self.add_temporal_edge(id, *other_id);
        }

        Ok(nearby_ids.len())
    }

    /// Full auto-linking: semantic + temporal + recompute PageRank
    pub fn auto_link(&mut self, id: u64) -> Result<(usize, usize)> {
        let semantic = self.auto_link_semantic(id, 0.65, 5)?;
        let temporal = self.auto_link_temporal(id, 30)?; // 30-minute window

        // Recompute PageRank after adding edges
        if semantic > 0 || temporal > 0 {
            self.compute_pagerank();
        }

        Ok((semantic, temporal))
    }

    // ═══════════════════════════════════════════════════════════════════
    // INDEX PERSISTENCE (Phase 3.2 - O(1) Startup)
    // ═══════════════════════════════════════════════════════════════════

    /// Persist all in-memory indexes to disk for O(1) startup
    ///
    /// This writes the note_index, tag_index, temporal_index, pinned list,
    /// bloom filter, and next_id to the file, allowing subsequent opens to
    /// skip the O(n) log scan.
    pub fn persist_indexes(&mut self) -> Result<()> {
        if self.read_only {
            return Err(EngramError::ReadOnly);
        }

        // Serialize all indexes
        let index_data = self.serialize_indexes();

        // Calculate checksum
        let checksum = crc32fast::hash(&index_data);

        // Build final section: magic + checksum + data
        let mut section = Vec::with_capacity(12 + index_data.len());
        section.extend_from_slice(&INDEX_MAGIC.to_le_bytes());
        section.extend_from_slice(&checksum.to_le_bytes());
        section.extend_from_slice(&index_data);

        // Write to end of file (after note log)
        let write_offset = self.header.note_log_offset + self.header.note_log_size;
        self.file.seek(SeekFrom::Start(write_offset))?;
        self.file.write_all(&section)?;

        // Update header with index location
        // We use tag_index_offset/size to store the combined index section
        self.header.tag_index_offset = write_offset;
        self.header.tag_index_size = section.len() as u64;
        self.header.flags |= header_flags::HAS_PERSISTED_INDEX;
        self.header.touch();
        self.header.write_to(&mut self.file)?;

        self.file.sync_all()?;
        self.remap()?;

        // Mark pinned notes in fingerprint index before saving sidecar
        self.fingerprint_index.mark_pinned(&self.pinned);

        // Save fingerprint index to sidecar file (supplementary — don't fail main persist)
        let fp_path = FingerprintIndex256::sidecar_path(&self.path);
        if let Err(e) = self.fingerprint_index.save(&fp_path) {
            eprintln!("Warning: failed to save fingerprint index: {}", e);
        }

        // Update our known mtime after writing (for multi-process sync)
        self.last_index_mtime = std::fs::metadata(&self.path).ok().and_then(|m| m.modified().ok());

        Ok(())
    }

    /// Check if indexes are persisted
    pub fn has_persisted_indexes(&self) -> bool {
        self.header.flags & header_flags::HAS_PERSISTED_INDEX != 0
    }

    /// Serialize all indexes to bytes
    fn serialize_indexes(&self) -> Vec<u8> {
        let mut data = Vec::new();

        // next_id (8 bytes)
        data.extend_from_slice(&self.next_id.to_le_bytes());

        // note_index: count + entries
        data.extend_from_slice(&(self.note_index.len() as u64).to_le_bytes());
        for (&id, &offset) in &self.note_index {
            data.extend_from_slice(&id.to_le_bytes());
            data.extend_from_slice(&offset.to_le_bytes());
        }

        // temporal_index: count + entries
        data.extend_from_slice(&(self.temporal_index.len() as u64).to_le_bytes());
        for &(timestamp, id) in &self.temporal_index {
            data.extend_from_slice(&timestamp.to_le_bytes());
            data.extend_from_slice(&id.to_le_bytes());
        }

        // pinned: count + entries
        data.extend_from_slice(&(self.pinned.len() as u64).to_le_bytes());
        for &id in &self.pinned {
            data.extend_from_slice(&id.to_le_bytes());
        }

        // tag_index: count + entries (tag_len, tag_bytes, id_count, ids...)
        data.extend_from_slice(&(self.tag_index.len() as u64).to_le_bytes());
        for (tag, ids) in &self.tag_index {
            let tag_bytes = tag.as_bytes();
            data.extend_from_slice(&(tag_bytes.len() as u32).to_le_bytes());
            data.extend_from_slice(tag_bytes);
            data.extend_from_slice(&(ids.len() as u64).to_le_bytes());
            for &id in ids {
                data.extend_from_slice(&id.to_le_bytes());
            }
        }

        // bloom filter
        let bloom_bytes = self.tag_bloom.to_bytes();
        data.extend_from_slice(&(bloom_bytes.len() as u64).to_le_bytes());
        data.extend_from_slice(&bloom_bytes);

        // vault
        let vault_bytes = self.vault.serialize();
        data.extend_from_slice(&(vault_bytes.len() as u64).to_le_bytes());
        data.extend_from_slice(&vault_bytes);

        // vector_store
        let vector_bytes = self.vector_store.serialize();
        data.extend_from_slice(&(vector_bytes.len() as u64).to_le_bytes());
        data.extend_from_slice(&vector_bytes);

        // graph
        let graph_bytes = self.graph.serialize();
        data.extend_from_slice(&(graph_bytes.len() as u64).to_le_bytes());
        data.extend_from_slice(&graph_bytes);

        // hnsw index (graph structure only — vectors already in vector_store above)
        let hnsw_bytes = self.hnsw_index.serialize();
        data.extend_from_slice(&(hnsw_bytes.len() as u64).to_le_bytes());
        data.extend_from_slice(&hnsw_bytes);

        data
    }

    /// Deserialize indexes from bytes
    fn deserialize_indexes(&mut self, data: &[u8]) -> Result<()> {
        let mut cursor = 0;

        // Safety limits for untrusted data — prevents OOM from corrupted/malicious files.
        // These are generous upper bounds; real data is orders of magnitude smaller.
        const MAX_NOTE_COUNT: usize = 10_000_000;    // 10M notes
        const MAX_TEMPORAL_COUNT: usize = 10_000_000; // 10M entries
        const MAX_PINNED_COUNT: usize = 100_000;      // 100K pinned
        const MAX_TAG_COUNT: usize = 1_000_000;       // 1M tags
        const MAX_TAG_LEN: usize = 1_024;             // 1KB per tag string
        const MAX_IDS_PER_TAG: usize = 10_000_000;    // 10M note IDs per tag
        const MAX_SECTION_LEN: usize = 512 * 1024 * 1024; // 512MB per section

        // Helper to read bytes
        let read_u64 = |cursor: &mut usize| -> Result<u64> {
            if *cursor + 8 > data.len() {
                return Err(EngramError::IntegrityError("Index data truncated".into()));
            }
            let val = u64::from_le_bytes(data[*cursor..*cursor + 8].try_into().unwrap());
            *cursor += 8;
            Ok(val)
        };

        let read_i64 = |cursor: &mut usize| -> Result<i64> {
            if *cursor + 8 > data.len() {
                return Err(EngramError::IntegrityError("Index data truncated".into()));
            }
            let val = i64::from_le_bytes(data[*cursor..*cursor + 8].try_into().unwrap());
            *cursor += 8;
            Ok(val)
        };

        let read_u32 = |cursor: &mut usize| -> Result<u32> {
            if *cursor + 4 > data.len() {
                return Err(EngramError::IntegrityError("Index data truncated".into()));
            }
            let val = u32::from_le_bytes(data[*cursor..*cursor + 4].try_into().unwrap());
            *cursor += 4;
            Ok(val)
        };

        // next_id
        self.next_id = read_u64(&mut cursor)?;

        // note_index
        let note_count = read_u64(&mut cursor)? as usize;
        if note_count > MAX_NOTE_COUNT {
            return Err(EngramError::IntegrityError(
                format!("Note count {} exceeds maximum {}", note_count, MAX_NOTE_COUNT)
            ));
        }
        self.note_index = HashMap::with_capacity(note_count);
        for _ in 0..note_count {
            let id = read_u64(&mut cursor)?;
            let offset = read_u64(&mut cursor)?;
            self.note_index.insert(id, offset);
        }

        // temporal_index
        let temporal_count = read_u64(&mut cursor)? as usize;
        if temporal_count > MAX_TEMPORAL_COUNT {
            return Err(EngramError::IntegrityError(
                format!("Temporal count {} exceeds maximum {}", temporal_count, MAX_TEMPORAL_COUNT)
            ));
        }
        self.temporal_index = Vec::with_capacity(temporal_count);
        for _ in 0..temporal_count {
            let timestamp = read_i64(&mut cursor)?;
            let id = read_u64(&mut cursor)?;
            self.temporal_index.push((timestamp, id));
        }

        // pinned
        let pinned_count = read_u64(&mut cursor)? as usize;
        if pinned_count > MAX_PINNED_COUNT {
            return Err(EngramError::IntegrityError(
                format!("Pinned count {} exceeds maximum {}", pinned_count, MAX_PINNED_COUNT)
            ));
        }
        self.pinned = Vec::with_capacity(pinned_count);
        for _ in 0..pinned_count {
            self.pinned.push(read_u64(&mut cursor)?);
        }

        // tag_index
        let tag_count = read_u64(&mut cursor)? as usize;
        if tag_count > MAX_TAG_COUNT {
            return Err(EngramError::IntegrityError(
                format!("Tag count {} exceeds maximum {}", tag_count, MAX_TAG_COUNT)
            ));
        }
        self.tag_index = HashMap::with_capacity(tag_count);
        for _ in 0..tag_count {
            let tag_len = read_u32(&mut cursor)? as usize;
            if tag_len > MAX_TAG_LEN {
                return Err(EngramError::IntegrityError(
                    format!("Tag length {} exceeds maximum {}", tag_len, MAX_TAG_LEN)
                ));
            }
            if cursor + tag_len > data.len() {
                return Err(EngramError::IntegrityError("Tag data truncated".into()));
            }
            let tag = String::from_utf8_lossy(&data[cursor..cursor + tag_len]).to_string();
            cursor += tag_len;

            let id_count = read_u64(&mut cursor)? as usize;
            if id_count > MAX_IDS_PER_TAG {
                return Err(EngramError::IntegrityError(
                    format!("ID count {} for tag exceeds maximum {}", id_count, MAX_IDS_PER_TAG)
                ));
            }
            let mut ids = Vec::with_capacity(id_count);
            for _ in 0..id_count {
                ids.push(read_u64(&mut cursor)?);
            }
            self.tag_index.insert(tag, ids);
        }

        // bloom filter
        let bloom_len = read_u64(&mut cursor)? as usize;
        if bloom_len > MAX_SECTION_LEN {
            return Err(EngramError::IntegrityError(
                format!("Bloom filter length {} exceeds maximum {}", bloom_len, MAX_SECTION_LEN)
            ));
        }
        if cursor + bloom_len > data.len() {
            return Err(EngramError::IntegrityError("Bloom filter data truncated".into()));
        }
        if let Some(bloom) = BloomFilter::from_bytes(&data[cursor..cursor + bloom_len]) {
            self.tag_bloom = bloom;
        }
        cursor += bloom_len;

        // vault (optional - may not exist in older files)
        if cursor + 8 <= data.len() {
            let vault_len = read_u64(&mut cursor)? as usize;
            if vault_len > MAX_SECTION_LEN {
                return Err(EngramError::IntegrityError(
                    format!("Vault length {} exceeds maximum {}", vault_len, MAX_SECTION_LEN)
                ));
            }
            if cursor + vault_len <= data.len() {
                self.vault.deserialize(&data[cursor..cursor + vault_len])?;
                cursor += vault_len;
            }
        }

        // vector_store (optional)
        if cursor + 8 <= data.len() {
            let vector_len = read_u64(&mut cursor)? as usize;
            if vector_len > MAX_SECTION_LEN {
                return Err(EngramError::IntegrityError(
                    format!("Vector store length {} exceeds maximum {}", vector_len, MAX_SECTION_LEN)
                ));
            }
            if cursor + vector_len <= data.len() {
                self.vector_store.deserialize(&data[cursor..cursor + vector_len])?;
                cursor += vector_len;
            }
        }

        // graph (optional)
        if cursor + 8 <= data.len() {
            let graph_len = read_u64(&mut cursor)? as usize;
            if graph_len > MAX_SECTION_LEN {
                return Err(EngramError::IntegrityError(
                    format!("Graph length {} exceeds maximum {}", graph_len, MAX_SECTION_LEN)
                ));
            }
            if cursor + graph_len <= data.len() {
                let _ = self.graph.deserialize(&data[cursor..cursor + graph_len]);
                cursor += graph_len;
            }
        }

        // hnsw index (optional — new in persistence v2)
        if cursor + 8 <= data.len() {
            let hnsw_len = read_u64(&mut cursor)? as usize;
            if hnsw_len > MAX_SECTION_LEN {
                return Err(EngramError::IntegrityError(
                    format!("HNSW index length {} exceeds maximum {}", hnsw_len, MAX_SECTION_LEN)
                ));
            }
            if cursor + hnsw_len <= data.len() {
                if let Some(hnsw) = crate::hnsw::HnswIndex::deserialize(&data[cursor..cursor + hnsw_len]) {
                    self.hnsw_index = hnsw;
                    // Vectors repopulated in load_persisted_indexes() after vector_store is loaded
                }
                // cursor += hnsw_len;
            }
        }

        Ok(())
    }

    /// Load persisted indexes from file (O(1) startup)
    fn load_persisted_indexes(&mut self) -> Result<bool> {
        // Check if indexes are persisted
        if self.header.flags & header_flags::HAS_PERSISTED_INDEX == 0 {
            return Ok(false);
        }

        if self.header.tag_index_size == 0 {
            return Ok(false);
        }

        // Validate section offset+size against actual file size to prevent
        // seeking past EOF or allocating based on corrupt header values
        let file_len = self.file.seek(SeekFrom::End(0))?;

        let tag_offset = self.header.tag_index_offset;
        let tag_size = self.header.tag_index_size;
        let tag_end = tag_offset.checked_add(tag_size).ok_or_else(|| {
            EngramError::IntegrityError("tag index offset+size overflow".into())
        })?;
        if tag_end > file_len {
            return Err(EngramError::IntegrityError(format!(
                "tag index section extends past EOF: offset={} size={} file_len={}",
                tag_offset, tag_size, file_len
            )));
        }

        // Cap section size to prevent OOM from corrupt header (512MB max)
        const MAX_INDEX_SECTION: u64 = 512 * 1024 * 1024;
        if tag_size > MAX_INDEX_SECTION {
            return Err(EngramError::IntegrityError(format!(
                "tag index section too large: {} bytes (max {})",
                tag_size, MAX_INDEX_SECTION
            )));
        }

        // Read the index section
        self.file.seek(SeekFrom::Start(tag_offset))?;

        let section_size = tag_size as usize;
        if section_size < 12 {
            return Ok(false);
        }

        let mut section = vec![0u8; section_size];
        self.file.read_exact(&mut section)?;

        // Verify magic
        let magic = u64::from_le_bytes(section[0..8].try_into().unwrap());
        if magic != INDEX_MAGIC {
            return Ok(false);
        }

        // Verify checksum
        let stored_checksum = u32::from_le_bytes(section[8..12].try_into().unwrap());
        let computed_checksum = crc32fast::hash(&section[12..]);
        if stored_checksum != computed_checksum {
            return Ok(false);
        }

        // Deserialize indexes
        self.deserialize_indexes(&section[12..])?;

        // HNSW graph structure: if deserialized from index section, validate node IDs
        // against note_index (notes may have been deleted between save and load),
        // then repopulate vectors. If invalid, fall back to full rebuild.
        if !self.hnsw_index.is_empty() {
            let valid_ids: std::collections::HashSet<u64> = self.note_index.keys().copied().collect();
            let dangling = self.hnsw_index.validate_node_ids(&valid_ids);

            if dangling.is_empty() {
                // All nodes valid — just repopulate vectors for distance calcs
                let vectors = self.vector_store.all_vectors();
                self.hnsw_index.repopulate_vectors(&vectors);
            } else {
                // Dangling refs found (deleted notes) — rebuild from scratch
                self.hnsw_index = crate::hnsw::HnswIndex::new();
                for &id in self.note_index.keys() {
                    if let Some(embedding) = self.vector_store.get(id) {
                        self.hnsw_index.add(id, embedding);
                    }
                }
            }
        } else if self.vector_store.count() > 0 {
            // No persisted HNSW (older file format) — rebuild from embeddings
            for &id in self.note_index.keys() {
                if let Some(embedding) = self.vector_store.get(id) {
                    self.hnsw_index.add(id, embedding);
                }
            }
        }

        // Sanity check: if header reports notes but index has far fewer,
        // the persisted index is stale — fall back to rebuild.
        // Previous check only caught empty indexes; this also catches partial staleness
        // (e.g. index saved when only a few notes were accessible due to key mismatch).
        let expected = self.header.active_notes as usize;
        let actual = self.note_index.len();
        if expected > 0 && actual < expected / 2 {
            eprintln!(
                "engram: persisted index stale (header={} active, index={}), rebuilding",
                expected, actual
            );
            self.note_index.clear();
            self.pinned.clear();
            self.tag_index.clear();
            self.temporal_index.clear();
            self.tag_bloom.clear();
            self.next_id = 1;
            self.cache.clear();
            return Ok(false);
        }

        Ok(true)
    }

    /// Invalidate persisted indexes (call after writes if indexes were persisted)
    ///
    /// This is an optimization - after writes, the persisted indexes are stale.
    /// Either call persist_indexes() again, or clear the flag so next open
    /// will rebuild from the log.
    fn invalidate_persisted_indexes(&mut self) -> Result<()> {
        if self.header.flags & header_flags::HAS_PERSISTED_INDEX != 0 {
            self.header.flags &= !header_flags::HAS_PERSISTED_INDEX;
            self.header.tag_index_size = 0;
            self.header.touch();
            self.header.write_to(&mut self.file)?;
        }
        Ok(())
    }
}


// Ensure indexes are persisted when Engram is dropped
impl Drop for Engram {
    fn drop(&mut self) {
        // Best-effort persist on close - ignore errors since we're dropping
        if !self.read_only {
            let _ = self.persist_indexes();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_create_and_open() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        // Create new database
        {
            let mut db = Engram::open(&path).unwrap();
            assert_eq!(db.stats().active_notes, 0);
            assert!(!db.is_mapped()); // No notes yet
        }

        // Reopen existing
        {
            let mut db = Engram::open(&path).unwrap();
            assert_eq!(db.stats().active_notes, 0);
        }
    }

    #[test]
    fn test_remember_and_get() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        let id = db.remember("Test note content", &["tag1", "tag2"]).unwrap();
        assert_eq!(id, 1);
        // Note: with lazy remap, mmap isn't updated until sync() is called
        // But get() still works via file I/O fallback

        let note = db.get(id).unwrap().unwrap();
        assert_eq!(note.content, "Test note content");
        assert_eq!(note.tags, vec!["tag1", "tag2"]);

        // After sync(), mmap should be updated
        db.sync().unwrap();
        assert!(db.is_mapped());
    }

    #[test]
    fn test_remember_batch() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        let items: Vec<(&str, &[&str])> = vec![
            ("Note 1", &["a", "b"][..]),
            ("Note 2", &["b", "c"][..]),
            ("Note 3", &["c", "d"][..]),
        ];

        let ids = db.remember_batch(&items).unwrap();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids, vec![1, 2, 3]);

        // Verify all notes
        for (i, id) in ids.iter().enumerate() {
            let note = db.get(*id).unwrap().unwrap();
            assert_eq!(note.content, format!("Note {}", i + 1));
        }
    }

    #[test]
    fn test_cache() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        let id = db.remember("Cached note", &[]).unwrap();

        // First read - cache miss
        let _ = db.get(id).unwrap();
        let (hits, misses, _) = db.cache_stats();
        assert_eq!(hits, 0);
        assert_eq!(misses, 1);

        // Second read - cache hit
        let _ = db.get(id).unwrap();
        let (hits, misses, _) = db.cache_stats();
        assert_eq!(hits, 1);
        assert_eq!(misses, 1);
    }

    #[test]
    fn test_forget() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        let id = db.remember("To be deleted", &[]).unwrap();
        assert!(db.get(id).unwrap().is_some());

        db.forget(id).unwrap();
        assert!(db.get(id).unwrap().is_none());
    }

    #[test]
    fn test_persistence() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        // Create and write
        let id = {
            let mut db = Engram::open(&path).unwrap();
            db.remember("Persistent content", &["persist"]).unwrap()
        };

        // Reopen and verify
        {
            let mut db = Engram::open(&path).unwrap();
            assert!(db.is_mapped());
            let note = db.get(id).unwrap().unwrap();
            assert_eq!(note.content, "Persistent content");
            assert_eq!(note.tags, vec!["persist"]);
        }
    }

    #[test]
    fn test_by_tag() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        db.remember("Note 1", &["rust", "coding"]).unwrap();
        db.remember("Note 2", &["rust", "database"]).unwrap();
        db.remember("Note 3", &["python"]).unwrap();

        let rust_notes = db.by_tag("rust").unwrap();
        assert_eq!(rust_notes.len(), 2);

        let python_notes = db.by_tag("python").unwrap();
        assert_eq!(python_notes.len(), 1);
    }

    #[test]
    fn test_recent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        for i in 1..=10 {
            db.remember(&format!("Note {}", i), &[]).unwrap();
        }

        let recent = db.recent(5).unwrap();
        assert_eq!(recent.len(), 5);
        // Most recent first
        assert!(recent[0].content.contains("10"));
    }

    #[test]
    fn test_large_batch() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        // Create 1000 notes in batch
        let items: Vec<(String, Vec<&str>)> = (0..1000)
            .map(|i| (format!("Note {}", i), vec!["batch"]))
            .collect();

        let items_ref: Vec<(&str, &[&str])> = items
            .iter()
            .map(|(s, t)| (s.as_str(), t.as_slice()))
            .collect();

        let ids = db.remember_batch(&items_ref).unwrap();
        assert_eq!(ids.len(), 1000);

        // Verify stats
        assert_eq!(db.stats().active_notes, 1000);

        // Call sync() to update mmap
        db.sync().unwrap();
        assert!(db.is_mapped());
    }

    #[test]
    fn test_index_persistence() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        // Create database with some notes
        {
            let mut db = Engram::open(&path).unwrap();
            db.remember("Note 1", &["alpha", "beta"]).unwrap();
            db.remember("Note 2", &["beta", "gamma"]).unwrap();
            db.remember("Note 3", &["gamma"]).unwrap();
            db.pin(1).unwrap();

            // remember() + pin() auto-persist on every write — indexes are already persisted
            assert!(db.has_persisted_indexes());

            // Calling persist_indexes() explicitly is idempotent
            db.persist_indexes().unwrap();
            assert!(db.has_persisted_indexes());
        }

        // Reopen - should load persisted indexes (O(1) startup)
        {
            let mut db = Engram::open(&path).unwrap();
            assert!(db.has_persisted_indexes());

            // Verify all data is correct
            assert_eq!(db.stats().active_notes, 3);

            let note1 = db.get(1).unwrap().unwrap();
            assert_eq!(note1.content, "Note 1");
            assert_eq!(note1.tags, vec!["alpha", "beta"]);

            let alpha_notes = db.by_tag("alpha").unwrap();
            assert_eq!(alpha_notes.len(), 1);

            let beta_notes = db.by_tag("beta").unwrap();
            assert_eq!(beta_notes.len(), 2);

            // Pinned should be restored
            let pinned = db.pinned().unwrap();
            assert_eq!(pinned.len(), 1);
            assert_eq!(pinned[0].id, 1);
        }
    }

    #[test]
    fn test_index_persistence_with_bloom() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        // Create database with tags
        {
            let mut db = Engram::open(&path).unwrap();
            for i in 0..100 {
                let tags: Vec<&str> = vec!["common"];
                db.remember(&format!("Note {}", i), &tags).unwrap();
            }
            db.persist_indexes().unwrap();
        }

        // Reopen and test bloom filter
        {
            let mut db = Engram::open(&path).unwrap();

            // Existing tag - bloom filter should say "might exist"
            let common_notes = db.by_tag("common").unwrap();
            assert_eq!(common_notes.len(), 100);

            // Non-existent tag - bloom filter should say "definitely not"
            let nonexistent = db.by_tag("nonexistent_xyz_12345").unwrap();
            assert_eq!(nonexistent.len(), 0);
        }
    }

    #[test]
    fn test_index_persistence_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        // Create with many tags and notes
        let mut original_ids: Vec<u64> = Vec::new();
        {
            let mut db = Engram::open(&path).unwrap();
            for i in 0..50 {
                let tag = format!("tag_{}", i % 10);
                let id = db.remember(&format!("Content {}", i), &[&tag]).unwrap();
                original_ids.push(id);
            }
            db.persist_indexes().unwrap();
        }

        // Reopen and verify everything matches
        {
            let mut db = Engram::open(&path).unwrap();

            for &id in &original_ids {
                let note = db.get(id).unwrap();
                assert!(note.is_some(), "Note {} should exist", id);
            }

            // Check tag index
            for i in 0..10 {
                let tag = format!("tag_{}", i);
                let notes = db.by_tag(&tag).unwrap();
                assert_eq!(notes.len(), 5, "Tag {} should have 5 notes", tag);
            }
        }
    }

    #[test]
    fn test_write_after_persisted_indexes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        // Create and persist
        {
            let mut db = Engram::open(&path).unwrap();
            db.remember("Initial note", &["init"]).unwrap();
            db.persist_indexes().unwrap();
        }

        // Reopen and write more
        {
            let mut db = Engram::open(&path).unwrap();
            assert!(db.has_persisted_indexes());

            // Write new note (in-memory indexes update, persisted indexes become stale)
            let id = db.remember("New note", &["new"]).unwrap();

            // Should still work correctly
            let note = db.get(id).unwrap().unwrap();
            assert_eq!(note.content, "New note");

            let new_notes = db.by_tag("new").unwrap();
            assert_eq!(new_notes.len(), 1);
        }

        // Reopen without re-persisting - should fall back to rebuild
        {
            let mut db = Engram::open(&path).unwrap();

            // Both notes should be accessible
            assert_eq!(db.stats().active_notes, 2);

            let init_notes = db.by_tag("init").unwrap();
            assert_eq!(init_notes.len(), 1);

            let new_notes = db.by_tag("new").unwrap();
            assert_eq!(new_notes.len(), 1);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // PHASE 4 TESTS - Vector, Vault, Graph, Recall
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn test_vector_operations() {
        use crate::vector::DIMS;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        // Create a note
        let id = db.remember("Test note for vector", &["vector"]).unwrap();

        // Create a 512-dim embedding
        let mut embedding = vec![0.0f32; DIMS];
        embedding[0] = 1.0;
        embedding[1] = 0.5;

        // Add embedding
        db.add_embedding(id, &embedding).unwrap();

        // Verify we can get it back
        assert!(db.has_embedding(id));
        let retrieved = db.get_embedding(id).unwrap();
        assert_eq!(retrieved[0], 1.0);
        assert_eq!(retrieved[1], 0.5);

        // Search for similar
        let results = db.search_similar(&embedding, 5);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, id);
    }

    #[test]
    fn test_vector_search_multiple() {
        use crate::vector::DIMS;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        // Create notes with different embeddings (different directions)
        for i in 1..=10 {
            let id = db.remember(&format!("Note {}", i), &[]).unwrap();

            let mut embedding = vec![0.0f32; DIMS];
            let angle = i as f32 * 0.3;
            embedding[0] = angle.cos();
            embedding[1] = angle.sin();

            db.add_embedding(id, &embedding).unwrap();
        }

        // Search for a specific direction
        let mut query = vec![0.0f32; DIMS];
        let target_angle = 1.5f32; // Close to note 5 (angle = 1.5)
        query[0] = target_angle.cos();
        query[1] = target_angle.sin();

        let results = db.search_similar(&query, 3);
        assert_eq!(results.len(), 3);
        // Note 5 should be closest
        assert_eq!(results[0].0, 5);
    }

    #[test]
    fn test_hnsw_persistence_roundtrip() {
        use crate::vector::DIMS;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let results_before;

        // Phase 1: Create notes with embeddings, search, close
        {
            let mut db = Engram::open(&path).unwrap();

            for i in 1..=20 {
                let id = db.remember(&format!("Vector note {}", i), &["vector"]).unwrap();
                let mut embedding = vec![0.0f32; DIMS];
                let angle = i as f32 * 0.2;
                embedding[0] = angle.cos();
                embedding[1] = angle.sin();
                db.add_embedding(id, &embedding).unwrap();
            }

            // Search before close
            let mut query = vec![0.0f32; DIMS];
            let target = 2.0f32; // Close to note 10 (angle = 2.0)
            query[0] = target.cos();
            query[1] = target.sin();
            results_before = db.search_similar(&query, 5);
            assert!(!results_before.is_empty());

            db.persist_indexes().unwrap();
        } // db dropped, file closed

        // Phase 2: Reopen and verify same search results
        {
            let db = Engram::open(&path).unwrap();

            let mut query = vec![0.0f32; DIMS];
            let target = 2.0f32;
            query[0] = target.cos();
            query[1] = target.sin();
            let results_after = db.search_similar(&query, 5);

            assert_eq!(results_before.len(), results_after.len(),
                "Same number of results after reopen");
            for (before, after) in results_before.iter().zip(results_after.iter()) {
                assert_eq!(before.0, after.0,
                    "Same note IDs in same order after reopen");
            }
        }
    }

    #[test]
    fn test_hnsw_persistence_dangling_refs() {
        use crate::vector::DIMS;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        // Phase 1: Create notes with embeddings
        {
            let mut db = Engram::open(&path).unwrap();

            for i in 1..=10 {
                let id = db.remember(&format!("Note {}", i), &["test"]).unwrap();
                let mut embedding = vec![0.0f32; DIMS];
                let angle = i as f32 * 0.3;
                embedding[0] = angle.cos();
                embedding[1] = angle.sin();
                db.add_embedding(id, &embedding).unwrap();
            }

            db.persist_indexes().unwrap();
        }

        // Phase 2: Delete some notes (creates dangling refs in persisted HNSW)
        {
            let mut db = Engram::open(&path).unwrap();
            db.forget(3).unwrap();
            db.forget(7).unwrap();
            db.persist_indexes().unwrap();
        }

        // Phase 3: Reopen — HNSW should detect dangling refs and rebuild
        {
            let mut db = Engram::open(&path).unwrap();

            // Should still work — dangling refs handled gracefully
            let mut query = vec![0.0f32; DIMS];
            query[0] = 1.0;
            let results = db.search_similar(&query, 5);

            // Should not return deleted notes
            for (id, _) in &results {
                assert!(*id != 3 && *id != 7,
                    "Deleted note {} should not appear in results", id);
            }

            // Remaining notes should still be searchable
            assert_eq!(db.stats().active_notes, 8);
        }
    }

    #[test]
    fn test_vault_operations() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        // Set a secret
        db.vault_set("API_KEY", b"sk-secret-123").unwrap();
        db.vault_set_string("DB_PASSWORD", "hunter2").unwrap();

        // Get secrets
        let api_key = db.vault_get("API_KEY").unwrap().unwrap();
        assert_eq!(api_key, b"sk-secret-123");

        let db_pass = db.vault_get_string("DB_PASSWORD").unwrap().unwrap();
        assert_eq!(db_pass, "hunter2");

        // Check existence
        assert!(db.vault_contains("API_KEY"));
        assert!(!db.vault_contains("NONEXISTENT"));

        // List keys
        let keys = db.vault_keys();
        assert_eq!(keys.len(), 2);

        // Delete a key
        assert!(db.vault_delete("API_KEY"));
        assert!(!db.vault_contains("API_KEY"));
    }

    #[test]
    fn test_graph_operations() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        // Create notes
        let id1 = db.remember("Note 1", &[]).unwrap();
        let id2 = db.remember("Note 2", &[]).unwrap();
        let id3 = db.remember("Note 3", &[]).unwrap();

        // Add edges
        db.add_semantic_edge(id1, id2, 0.9);
        db.add_temporal_edge(id2, id3);
        db.add_edge(id3, id1, 0.7, EdgeType::Manual);

        // Get related
        let related = db.get_related(id1);
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].0, id2);
        assert_eq!(related[0].1, 0.9);

        // Compute PageRank
        db.compute_pagerank();

        // All nodes should have some PageRank
        assert!(db.get_pagerank(id1) > 0.0);
        assert!(db.get_pagerank(id2) > 0.0);
        assert!(db.get_pagerank(id3) > 0.0);

        // Verify edge count in stats
        assert_eq!(db.stats().edge_count, 3);
    }

    #[test]
    fn test_recall_by_keyword() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        // Create notes with specific keywords
        db.remember("PostgreSQL connection pooling improved performance", &["database"]).unwrap();
        db.remember("Redis caching for session storage", &["database"]).unwrap();
        db.remember("Machine learning model training", &["ml"]).unwrap();

        // Search for database-related notes
        let results = db.recall_by_keyword("PostgreSQL", 10).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].note.content.contains("PostgreSQL"));
        assert!(results[0].keyword_score > 0.0);

        // Search for Redis
        let results = db.recall_by_keyword("Redis caching", 10).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].note.content.contains("Redis"));
    }

    #[test]
    fn test_hybrid_recall() {
        use crate::vector::DIMS;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        // Create notes with embeddings
        let id1 = db.remember("Database optimization techniques", &["db"]).unwrap();
        let id2 = db.remember("Machine learning algorithms", &["ml"]).unwrap();
        let id3 = db.remember("Database indexing strategies", &["db"]).unwrap();

        // Add embeddings (similar for database notes)
        let mut db_embedding = vec![0.0f32; DIMS];
        db_embedding[0] = 1.0;
        db_embedding[1] = 0.1;

        let mut ml_embedding = vec![0.0f32; DIMS];
        ml_embedding[0] = 0.1;
        ml_embedding[1] = 1.0;

        db.add_embedding(id1, &db_embedding).unwrap();
        db.add_embedding(id2, &ml_embedding).unwrap();
        db.add_embedding(id3, &db_embedding).unwrap();

        // Hybrid recall with embedding
        let results = db.recall("Database", Some(&db_embedding), 10).unwrap();
        assert!(results.len() >= 2);

        // First result should be database-related (high vector + keyword scores)
        assert!(results[0].note.content.contains("Database"));
        assert!(results[0].final_score > 0.0);
    }

    #[test]
    fn test_auto_link_temporal() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        // Create notes in quick succession (within temporal window)
        let _id1 = db.remember("First note", &[]).unwrap();
        let id2 = db.remember("Second note", &[]).unwrap();
        let _id3 = db.remember("Third note", &[]).unwrap();

        // Auto-link temporal edges
        let count = db.auto_link_temporal(id2, 30).unwrap();

        // Should link to notes within 30-minute window (all of them in this case)
        assert!(count >= 2);

        // Verify edges were created
        let related = db.get_related(id2);
        assert!(!related.is_empty());
    }

    #[test]
    fn test_auto_link_semantic() {
        use crate::vector::DIMS;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        // Create notes with embeddings
        let id1 = db.remember("Database topic A", &[]).unwrap();
        let id2 = db.remember("Database topic B", &[]).unwrap();
        let id3 = db.remember("Unrelated topic", &[]).unwrap();

        // Similar embeddings for database notes
        let mut db_embedding = vec![0.5f32; DIMS];
        db_embedding[0] = 1.0;

        let mut db_embedding2 = vec![0.5f32; DIMS];
        db_embedding2[0] = 0.99;

        let mut other_embedding = vec![0.1f32; DIMS];
        other_embedding[100] = 1.0;

        db.add_embedding(id1, &db_embedding).unwrap();
        db.add_embedding(id2, &db_embedding2).unwrap();
        db.add_embedding(id3, &other_embedding).unwrap();

        // Auto-link semantic edges (threshold 0.65)
        let count = db.auto_link_semantic(id1, 0.65, 5).unwrap();

        // Should link to similar notes
        assert!(count >= 1);

        // Verify edge was created
        let related = db.get_related(id1);
        assert!(!related.is_empty());
    }

    #[test]
    fn test_full_auto_link() {
        use crate::vector::DIMS;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        // Create notes
        let id1 = db.remember("Note one", &[]).unwrap();
        let id2 = db.remember("Note two", &[]).unwrap();

        // Add embeddings
        let embedding = vec![0.5f32; DIMS];
        db.add_embedding(id1, &embedding).unwrap();
        db.add_embedding(id2, &embedding).unwrap();

        // Full auto-link
        let (semantic, temporal) = db.auto_link(id1).unwrap();

        // Should create both types of edges
        assert!(semantic + temporal > 0);

        // PageRank should be computed
        assert!(db.get_pagerank(id1) > 0.0 || db.get_pagerank(id2) > 0.0);
    }

    #[test]
    fn test_recall_config() {
        use crate::recall::RecallConfig;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        // Set custom recall config
        let config = RecallConfig {
            vector_weight: 0.5,
            keyword_weight: 0.2,
            graph_weight: 0.2,
            recency_weight: 0.1,
            recency_half_life_hours: 12.0,
        };

        db.set_recall_config(config);

        // Create a note and test recall
        db.remember("Test note", &[]).unwrap();

        let results = db.recall_by_keyword("Test", 10).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_full_persistence_with_vault_vector_graph() {
        use crate::vector::DIMS;
        use crate::graph::EdgeType;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        // Create database with all features
        {
            let mut db = Engram::open(&path).unwrap();

            // Add notes
            let id1 = db.remember("First note", &["tag1"]).unwrap();
            let id2 = db.remember("Second note", &["tag2"]).unwrap();

            // Add vault entries
            db.vault_set_string("secret_key", "secret_value").unwrap();
            db.vault_set_string("another_key", "another_value").unwrap();

            // Add embeddings
            let mut embedding1 = vec![0.1f32; DIMS];
            embedding1[0] = 1.0;
            let mut embedding2 = vec![0.2f32; DIMS];
            embedding2[1] = 1.0;
            db.add_embedding(id1, &embedding1).unwrap();
            db.add_embedding(id2, &embedding2).unwrap();

            // Add graph edges
            db.add_edge(id1, id2, 0.8, EdgeType::Semantic);
            db.add_edge(id2, id1, 0.5, EdgeType::Temporal);
            db.compute_pagerank();

            // Persist everything
            db.persist_indexes().unwrap();

            // Verify before close
            assert!(db.has_persisted_indexes());
            assert_eq!(db.vault_keys().len(), 2);
            assert!(db.has_embedding(id1));
            assert!(db.has_embedding(id2));
            assert_eq!(db.graph.edge_count(), 2);
        }

        // Reopen and verify everything persisted
        {
            let mut db = Engram::open(&path).unwrap();

            // Verify notes
            let note1 = db.get(1).unwrap().unwrap();
            assert_eq!(note1.content, "First note");

            // Verify vault persisted
            let keys = db.vault_keys();
            assert_eq!(keys.len(), 2, "Vault should have 2 keys after reopen");
            let secret = db.vault_get_string("secret_key").unwrap().unwrap();
            assert_eq!(secret, "secret_value");

            // Verify vectors persisted
            assert!(db.has_embedding(1), "Vector 1 should persist");
            assert!(db.has_embedding(2), "Vector 2 should persist");

            // Verify graph persisted
            let related = db.get_related(1);
            assert!(!related.is_empty(), "Graph edges should persist");

            // Verify pagerank persisted
            let pr = db.get_pagerank(1);
            assert!(pr > 0.0, "PageRank should persist");
        }
    }

    #[test]
    fn test_backfill_fingerprints() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        // Create notes — fingerprints are auto-upserted by remember()
        db.remember("PostgreSQL connection pooling fixed the timeout", &["postgres", "fix"]).unwrap();
        db.remember("Redis caching improved latency by 10x", &["redis", "performance"]).unwrap();
        db.remember("Machine learning model training on GPU cluster", &["ml", "gpu"]).unwrap();
        db.remember("Kubernetes pod autoscaling configuration", &["k8s", "devops"]).unwrap();
        db.remember("TypeScript strict mode caught three bugs", &["typescript", "bugs"]).unwrap();

        assert_eq!(db.fingerprint_count(), 5);

        // Simulate a pre-upgrade database: clear the in-memory index
        // (as if opened from a .engram without a .fp sidecar)
        db.fingerprint_index = crate::fingerprint::FingerprintIndex256::new();
        assert_eq!(db.fingerprint_count(), 0);

        // Backfill should rebuild everything
        let count = db.backfill_fingerprints().unwrap();
        assert_eq!(count, 5);
        assert_eq!(db.fingerprint_count(), 5);

        // Verify sidecar file was created
        let fp_path = crate::fingerprint::FingerprintIndex256::sidecar_path(&path);
        assert!(fp_path.exists(), "Sidecar .engram.fp file should exist");

        // Verify fingerprints are functional — scan should find a match
        // 128-bit SimHash: HD range 0-128, use adaptive_max_hd_128 for small corpus
        let context = crate::fingerprint::Fingerprint256::from_text("PostgreSQL connection pooling timeout fix", &[]);
        let result = db.fingerprint_index.scan_best(&context, Some(50));
        assert!(result.is_some(), "Scan should find a match after backfill");
        let sr = result.unwrap();
        assert!(sr.score > 0, "Match should have positive score");
        // The PostgreSQL note (id=1) should be the best match
        assert_eq!(sr.note_id, 1, "PostgreSQL note should be best match for PostgreSQL query");
    }

    #[test]
    fn test_fingerprint_persistence_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        // Phase 1: Create notes, persist, close
        {
            let mut db = Engram::open(&path).unwrap();

            db.remember("Rust async runtime configuration", &["rust", "async"]).unwrap();
            db.remember("Python data pipeline with pandas", &["python", "data"]).unwrap();
            db.remember("Go concurrency with goroutines and channels", &["go", "concurrency"]).unwrap();

            assert_eq!(db.fingerprint_count(), 3);
            db.persist_indexes().unwrap();
        }

        // Phase 2: Reopen — sidecar should be loaded automatically
        {
            let db = Engram::open(&path).unwrap();
            assert_eq!(db.fingerprint_count(), 3, "Fingerprints should survive reopen");

            // Verify scan works on reopened index
            let context = crate::fingerprint::Fingerprint256::from_text("Rust async tokio runtime", &[]);
            let result = db.fingerprint_index.scan_best(&context, Some(50));
            assert!(result.is_some(), "Scan should work after reopen");
            let sr = result.unwrap();
            assert_eq!(sr.note_id, 1, "Rust note should match Rust query after reopen");
        }
    }

    #[test]
    fn test_fingerprint_lifecycle_forget_update() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut db = Engram::open(&path).unwrap();

        let id1 = db.remember("Original content about databases", &["db"]).unwrap();
        let id2 = db.remember("Content about networking protocols", &["network"]).unwrap();
        assert_eq!(db.fingerprint_count(), 2);

        // Forget should remove from fingerprint index
        db.forget(id1).unwrap();
        assert_eq!(db.fingerprint_count(), 1);

        // Scan should no longer find the forgotten note
        let context = crate::fingerprint::Fingerprint256::from_text("databases", &["db"]);
        let result = db.fingerprint_index.scan_best(&context, Some(50));
        if let Some(sr) = result {
            assert_ne!(sr.note_id, id1, "Forgotten note should not appear in scan");
        }

        // Update should re-compute fingerprint
        db.update(id2, Some("Updated content about machine learning"), None).unwrap();
        assert_eq!(db.fingerprint_count(), 1);

        // Scan for the updated content should match
        let context = crate::fingerprint::Fingerprint256::from_text("machine learning", &[]);
        let result = db.fingerprint_index.scan_best(&context, Some(50));
        assert!(result.is_some(), "Updated note should be findable");
        let sr = result.unwrap();
        assert_eq!(sr.note_id, id2, "Updated note should match ML query");
    }
}
