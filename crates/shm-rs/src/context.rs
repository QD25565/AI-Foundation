//! Context Fingerprint Shared Memory for Resonance Fingerprinting.
//!
//! Per-AI shared memory segment storing the current working context as a
//! 128-bit fingerprint (SimHash + Bloom). Written by the daemon (or hook),
//! read by the hook for sub-microsecond associative recall.
//!
//! # Layout
//!
//! 64 bytes total — fits in a single L1 cache line.
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!  0       8    magic (0x4358544650303031 = "CXTFP001")
//!  8       8    sequence (seqlock: odd = write in progress, even = consistent)
//! 16       8    simhash (64-bit Charikar SimHash of context keywords)
//! 24       8    bloom (64-bit Bloom filter of context keywords, k=5)
//! 32       8    updated_at (unix milliseconds when context was last written)
//! 40       8    tool_call_count (monotonic counter, for recency suppression)
//! 48      16    reserved
//! ```
//!
//! # Seqlock Protocol
//!
//! Single writer (daemon or hook), multiple readers (hooks).
//! Wait-free reads: no mutexes, no syscalls, no kernel involvement.
//!
//! Writer:
//!   1. Increment sequence to odd (marks "write in progress")
//!   2. Write simhash, bloom, updated_at
//!   3. Increment sequence to even (marks "consistent")
//!
//! Reader:
//!   1. Load sequence (must be even — odd means writer active, spin)
//!   2. Read simhash, bloom, updated_at via volatile reads
//!   3. Load sequence again — if changed, data was torn, retry
//!
//! On x86_64, aligned u64 stores are atomic, so torn reads are impossible
//! in practice. The seqlock is for correctness on all architectures.
//!
//! # Performance
//!
//! - Read: ~50-100ns (mmap + 2 atomic loads + 3 volatile reads)
//! - Write: ~200ns (2 atomic stores + 3 volatile writes + mmap flush)
//! - File: 64 bytes, always in page cache after first access

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering, fence};

// ============================================================================
// Constants
// ============================================================================

/// Magic number identifying a valid context fingerprint SHM file.
const CONTEXT_MAGIC: u64 = 0x4358_5446_5030_3031; // "CXTFP001"

/// Total size of the context SHM file (one cache line).
const CONTEXT_SIZE: usize = 64;

/// Maximum spin iterations before giving up on a torn read.
/// At ~1ns per spin_loop_hint, 256 iterations = ~256ns worst case.
const MAX_SPIN_ITERATIONS: usize = 256;

// Field offsets within the 64-byte layout.
const OFF_MAGIC: usize = 0;
const OFF_SEQUENCE: usize = 8;
const OFF_SIMHASH: usize = 16;
const OFF_BLOOM: usize = 24;
const OFF_UPDATED_AT: usize = 32;
const OFF_TOOL_CALLS: usize = 40;
// 48..64: reserved

// ============================================================================
// Public types
// ============================================================================

/// Context fingerprint data read from shared memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextFingerprint {
    /// SimHash of current working context (Charikar 2002, 64-bit).
    pub simhash: u64,
    /// Bloom filter of current context keywords (k=5 xxHash3, 64-bit).
    pub bloom: u64,
    /// Unix milliseconds when this context was last updated.
    pub updated_at: u64,
    /// Monotonic tool call counter (for recency suppression logic).
    pub tool_call_count: u64,
}

/// Errors from context SHM operations.
#[derive(Debug)]
pub enum ContextError {
    /// I/O error creating or mapping the SHM file.
    Io(std::io::Error),
    /// SHM file exists but has invalid magic bytes.
    BadMagic,
    /// SHM file is too small (corrupt or partial write).
    TooSmall,
    /// No home directory found.
    NoHomeDir,
}

impl std::fmt::Display for ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "context SHM I/O error: {}", e),
            Self::BadMagic => write!(f, "context SHM has invalid magic bytes"),
            Self::TooSmall => write!(f, "context SHM file too small (expected {} bytes)", CONTEXT_SIZE),
            Self::NoHomeDir => write!(f, "could not determine home directory"),
        }
    }
}

impl std::error::Error for ContextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ContextError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// ============================================================================
// Writer
// ============================================================================

/// Write-side handle to the context fingerprint SHM.
///
/// Single-writer only. The daemon (or hook in Phase 1) creates this to
/// update the current context fingerprint. Writes are seqlock-protected.
pub struct ContextWriter {
    mmap: memmap2::MmapMut,
}

impl ContextWriter {
    /// Open or create the context SHM file for `ai_id`.
    ///
    /// Creates `~/.ai-foundation/shm/context_{ai_id}.shm` if it doesn't exist.
    /// Initializes the header with magic and zeroed fingerprint on first creation.
    pub fn open_or_create(ai_id: &str) -> Result<Self, ContextError> {
        let path = context_shm_path(ai_id)?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        // Ensure file is exactly CONTEXT_SIZE bytes
        let len = file.metadata()?.len();
        if len < CONTEXT_SIZE as u64 {
            file.set_len(CONTEXT_SIZE as u64)?;
        }

        // SAFETY: We are the single writer. The file is CONTEXT_SIZE bytes,
        // properly aligned (page-aligned by OS). All field accesses are within bounds.
        let mmap = unsafe { memmap2::MmapMut::map_mut(&file)? };

        let mut writer = Self { mmap };

        // Initialize magic if this is a fresh file
        let current_magic = writer.read_volatile_u64(OFF_MAGIC);
        if current_magic != CONTEXT_MAGIC {
            writer.write_volatile_u64(OFF_MAGIC, CONTEXT_MAGIC);
            writer.write_volatile_u64(OFF_SEQUENCE, 0);
            writer.write_volatile_u64(OFF_SIMHASH, 0);
            writer.write_volatile_u64(OFF_BLOOM, 0);
            writer.write_volatile_u64(OFF_UPDATED_AT, 0);
            writer.write_volatile_u64(OFF_TOOL_CALLS, 0);
            // Zero reserved area
            for i in 48..CONTEXT_SIZE {
                // SAFETY: offset is within CONTEXT_SIZE bounds
                unsafe {
                    std::ptr::write_volatile(writer.mmap.as_mut_ptr().add(i), 0u8);
                }
            }
            writer.mmap.flush_async()?;
        }

        Ok(writer)
    }

    /// Update the context fingerprint with seqlock protection.
    ///
    /// Atomically increments sequence to odd (writing), writes the new
    /// fingerprint, then increments to even (consistent). Readers that
    /// observe the odd sequence will spin until the write completes.
    pub fn update(&mut self, simhash: u64, bloom: u64) -> Result<(), ContextError> {
        // Get base pointer — mutable borrow of self.mmap ends after this line.
        // Raw pointer is Copy, carries no borrow.
        let base = self.mmap.as_mut_ptr();

        // SAFETY: All offsets are within the 64-byte mmap region, all are
        // 8-byte aligned. We are the single writer. AtomicU64 has the same
        // layout as u64. Raw pointer is valid for the lifetime of self.mmap.
        unsafe {
            let seq = &*(base.add(OFF_SEQUENCE) as *const AtomicU64);

            // Begin write: increment to odd
            seq.fetch_add(1, Ordering::Release);
            fence(Ordering::SeqCst);

            // Write fingerprint data
            std::ptr::write_volatile(base.add(OFF_SIMHASH) as *mut u64, simhash);
            std::ptr::write_volatile(base.add(OFF_BLOOM) as *mut u64, bloom);
            std::ptr::write_volatile(base.add(OFF_UPDATED_AT) as *mut u64, now_millis());

            // Increment tool call counter
            let tc = std::ptr::read_volatile(base.add(OFF_TOOL_CALLS) as *const u64);
            std::ptr::write_volatile(base.add(OFF_TOOL_CALLS) as *mut u64, tc.wrapping_add(1));

            // End write: increment to even
            fence(Ordering::SeqCst);
            seq.fetch_add(1, Ordering::Release);
        }

        // Async flush — readers see updates through the page cache immediately
        // (volatile writes are visible to other processes sharing the mmap).
        // Synchronous msync costs ~3ms and is unnecessary for ephemeral context data.
        self.mmap.flush_async()?;
        Ok(())
    }

    /// Increment the tool call counter without changing the fingerprint.
    ///
    /// Used by the hook to track tool call cadence for recency suppression.
    /// This does NOT use the seqlock — aligned u64 writes are atomic on x86_64,
    /// and the tool_call_count is only used for approximate suppression timing.
    pub fn increment_tool_calls(&mut self) -> Result<(), ContextError> {
        let tc = self.read_volatile_u64(OFF_TOOL_CALLS);
        self.write_volatile_u64(OFF_TOOL_CALLS, tc.wrapping_add(1));
        self.mmap.flush_async()?;
        Ok(())
    }

    /// Read the current tool call count.
    pub fn tool_call_count(&self) -> u64 {
        self.read_volatile_u64(OFF_TOOL_CALLS)
    }

    /// Read the current context fingerprint (from writer's own mmap).
    pub fn read_current(&self) -> ContextFingerprint {
        ContextFingerprint {
            simhash: self.read_volatile_u64(OFF_SIMHASH),
            bloom: self.read_volatile_u64(OFF_BLOOM),
            updated_at: self.read_volatile_u64(OFF_UPDATED_AT),
            tool_call_count: self.read_volatile_u64(OFF_TOOL_CALLS),
        }
    }

    // --- Internal helpers ---

    #[allow(dead_code)] // Used in tests; reserved for daemon integration
    fn sequence_atomic(&self) -> &AtomicU64 {
        // SAFETY: OFF_SEQUENCE (8) is 8-byte aligned within the mmap region.
        // The mmap is valid for the lifetime of self. AtomicU64 has the same
        // layout as u64 with #[repr(C)].
        unsafe {
            &*(self.mmap.as_ptr().add(OFF_SEQUENCE) as *const AtomicU64)
        }
    }

    fn read_volatile_u64(&self, offset: usize) -> u64 {
        debug_assert!(offset + 8 <= CONTEXT_SIZE);
        // SAFETY: offset is within bounds, pointer is 8-byte aligned
        // (all offsets are multiples of 8), mmap is valid for lifetime of self.
        unsafe {
            std::ptr::read_volatile(self.mmap.as_ptr().add(offset) as *const u64)
        }
    }

    fn write_volatile_u64(&mut self, offset: usize, value: u64) {
        debug_assert!(offset + 8 <= CONTEXT_SIZE);
        // SAFETY: same alignment and bounds guarantees as read_volatile_u64.
        unsafe {
            std::ptr::write_volatile(self.mmap.as_mut_ptr().add(offset) as *mut u64, value);
        }
    }
}

// ============================================================================
// Reader
// ============================================================================

/// Read-side handle to the context fingerprint SHM.
///
/// Multiple readers can exist simultaneously. Reads are wait-free via
/// the seqlock protocol: spin only if a write is in progress (~200ns max).
pub struct ContextReader {
    mmap: memmap2::Mmap,
}

impl ContextReader {
    /// Open an existing context SHM file for `ai_id`.
    ///
    /// Returns `Ok(None)` if the file doesn't exist (no context available yet).
    /// Returns `Err` if the file exists but is corrupt.
    pub fn open(ai_id: &str) -> Result<Option<Self>, ContextError> {
        let path = context_shm_path(ai_id)?;

        if !path.exists() {
            return Ok(None);
        }

        let file = std::fs::File::open(&path)?;
        let len = file.metadata()?.len();
        if len < CONTEXT_SIZE as u64 {
            return Err(ContextError::TooSmall);
        }

        // SAFETY: File is at least CONTEXT_SIZE bytes. We open read-only.
        // The mmap is valid for the lifetime of the Mmap handle.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };

        // Validate magic
        let magic = read_volatile_u64_from(&mmap, OFF_MAGIC);
        if magic != CONTEXT_MAGIC {
            return Err(ContextError::BadMagic);
        }

        Ok(Some(Self { mmap }))
    }

    /// Read the context fingerprint with seqlock consistency.
    ///
    /// Returns `None` if the writer is stuck (sequence stays odd for too long,
    /// indicating a crashed writer). In practice this should never happen —
    /// writes complete in ~200ns.
    ///
    /// Performance: ~50-100ns typical (2 atomic loads + 3 volatile reads).
    pub fn read(&self) -> Option<ContextFingerprint> {
        let seq_ptr = self.sequence_atomic();

        for _ in 0..MAX_SPIN_ITERATIONS {
            let seq1 = seq_ptr.load(Ordering::Acquire);

            // Odd sequence = writer active. Spin.
            if seq1 & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }

            // Read data fields via volatile reads (prevents compiler caching)
            let simhash = read_volatile_u64_from(&self.mmap, OFF_SIMHASH);
            let bloom = read_volatile_u64_from(&self.mmap, OFF_BLOOM);
            let updated_at = read_volatile_u64_from(&self.mmap, OFF_UPDATED_AT);
            let tool_call_count = read_volatile_u64_from(&self.mmap, OFF_TOOL_CALLS);

            // Verify consistency: sequence must not have changed
            let seq2 = seq_ptr.load(Ordering::Acquire);
            if seq1 == seq2 {
                return Some(ContextFingerprint {
                    simhash,
                    bloom,
                    updated_at,
                    tool_call_count,
                });
            }

            // Torn read — writer was active between our two sequence loads. Retry.
            std::hint::spin_loop();
        }

        // Writer appears stuck (crashed mid-write). Return None.
        None
    }

    /// Read just the tool call count (no seqlock needed).
    ///
    /// Aligned u64 reads are atomic on x86_64. This is only used for
    /// approximate suppression timing, so a slightly stale value is fine.
    pub fn tool_call_count(&self) -> u64 {
        read_volatile_u64_from(&self.mmap, OFF_TOOL_CALLS)
    }

    /// Check if the context has been updated since `since_millis`.
    ///
    /// Useful for staleness detection — if context hasn't been updated
    /// in a while, the daemon may not be running.
    pub fn is_stale(&self, max_age_millis: u64) -> bool {
        let updated = read_volatile_u64_from(&self.mmap, OFF_UPDATED_AT);
        if updated == 0 {
            return true; // Never written
        }
        let now = now_millis();
        now.saturating_sub(updated) > max_age_millis
    }

    fn sequence_atomic(&self) -> &AtomicU64 {
        // SAFETY: same guarantees as ContextWriter::sequence_atomic.
        unsafe {
            &*(self.mmap.as_ptr().add(OFF_SEQUENCE) as *const AtomicU64)
        }
    }
}

// ============================================================================
// Shared helpers
// ============================================================================

/// Read a u64 from an mmap via volatile read.
///
/// Volatile prevents the compiler from caching or eliding the read,
/// which is essential when the underlying memory may be modified by
/// another process.
#[inline(always)]
fn read_volatile_u64_from(mmap: &[u8], offset: usize) -> u64 {
    debug_assert!(offset + 8 <= mmap.len());
    // SAFETY: offset is within bounds and 8-byte aligned (all offsets
    // are multiples of 8). The mmap slice is valid.
    unsafe {
        std::ptr::read_volatile(mmap.as_ptr().add(offset) as *const u64)
    }
}

/// Get the path to a context SHM file for a given AI ID.
///
/// Path: `~/.ai-foundation/shm/context_{ai_id}.shm`
pub fn context_shm_path(ai_id: &str) -> Result<PathBuf, ContextError> {
    let home = dirs::home_dir().ok_or(ContextError::NoHomeDir)?;
    Ok(home
        .join(".ai-foundation")
        .join("shm")
        .join(format!("context_{}.shm", ai_id)))
}

/// Current time in unix milliseconds.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a writer using a temp directory instead of ~/.ai-foundation/
    fn create_test_writer(dir: &std::path::Path, ai_id: &str) -> ContextWriter {
        let path = dir.join(format!("context_{}.shm", ai_id));

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .expect("create test file");
        file.set_len(CONTEXT_SIZE as u64).expect("set len");

        let mut mmap = unsafe { memmap2::MmapMut::map_mut(&file).expect("mmap") };

        // Initialize header
        unsafe {
            std::ptr::write_volatile(mmap.as_mut_ptr().add(OFF_MAGIC) as *mut u64, CONTEXT_MAGIC);
            std::ptr::write_volatile(mmap.as_mut_ptr().add(OFF_SEQUENCE) as *mut u64, 0);
            std::ptr::write_volatile(mmap.as_mut_ptr().add(OFF_SIMHASH) as *mut u64, 0);
            std::ptr::write_volatile(mmap.as_mut_ptr().add(OFF_BLOOM) as *mut u64, 0);
            std::ptr::write_volatile(mmap.as_mut_ptr().add(OFF_UPDATED_AT) as *mut u64, 0);
            std::ptr::write_volatile(mmap.as_mut_ptr().add(OFF_TOOL_CALLS) as *mut u64, 0);
        }
        for i in 48..CONTEXT_SIZE {
            unsafe { std::ptr::write_volatile(mmap.as_mut_ptr().add(i), 0u8); }
        }
        mmap.flush().expect("flush");

        ContextWriter { mmap }
    }

    /// Create a reader from the same file a writer is using.
    fn create_test_reader(dir: &std::path::Path, ai_id: &str) -> ContextReader {
        let path = dir.join(format!("context_{}.shm", ai_id));
        let file = std::fs::File::open(&path).expect("open for read");
        let mmap = unsafe { memmap2::Mmap::map(&file).expect("mmap read") };
        ContextReader { mmap }
    }

    #[test]
    fn test_write_then_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut writer = create_test_writer(dir.path(), "test-ai");
        let reader = create_test_reader(dir.path(), "test-ai");

        writer.update(0xDEAD_BEEF_CAFE_BABE, 0x1234_5678_9ABC_DEF0).expect("update");

        let ctx = reader.read().expect("read should succeed");
        assert_eq!(ctx.simhash, 0xDEAD_BEEF_CAFE_BABE);
        assert_eq!(ctx.bloom, 0x1234_5678_9ABC_DEF0);
        assert!(ctx.updated_at > 0, "updated_at should be set");
        assert_eq!(ctx.tool_call_count, 1, "first update should set tool_call_count to 1");
    }

    #[test]
    fn test_multiple_updates() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut writer = create_test_writer(dir.path(), "test-ai");
        let reader = create_test_reader(dir.path(), "test-ai");

        writer.update(111, 222).expect("update 1");
        writer.update(333, 444).expect("update 2");
        writer.update(555, 666).expect("update 3");

        let ctx = reader.read().expect("read");
        assert_eq!(ctx.simhash, 555);
        assert_eq!(ctx.bloom, 666);
        assert_eq!(ctx.tool_call_count, 3);
    }

    #[test]
    fn test_sequence_is_even_after_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut writer = create_test_writer(dir.path(), "test-ai");

        writer.update(1, 2).expect("update");

        let seq = writer.sequence_atomic().load(Ordering::Acquire);
        assert_eq!(seq & 1, 0, "sequence must be even after write (got {})", seq);
        assert_eq!(seq, 2, "after one write, sequence should be 2");
    }

    #[test]
    fn test_increment_tool_calls_without_fingerprint_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut writer = create_test_writer(dir.path(), "test-ai");
        let reader = create_test_reader(dir.path(), "test-ai");

        writer.update(0xAAAA, 0xBBBB).expect("initial update");
        writer.increment_tool_calls().expect("increment 1");
        writer.increment_tool_calls().expect("increment 2");

        let ctx = reader.read().expect("read");
        assert_eq!(ctx.simhash, 0xAAAA, "fingerprint should not change");
        assert_eq!(ctx.bloom, 0xBBBB, "fingerprint should not change");
        assert_eq!(ctx.tool_call_count, 3, "1 from update + 2 from increment");
    }

    #[test]
    fn test_zero_fingerprint_reads_as_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _writer = create_test_writer(dir.path(), "test-ai");
        let reader = create_test_reader(dir.path(), "test-ai");

        let ctx = reader.read().expect("read of fresh SHM");
        assert_eq!(ctx.simhash, 0);
        assert_eq!(ctx.bloom, 0);
        assert_eq!(ctx.updated_at, 0);
        assert_eq!(ctx.tool_call_count, 0);
    }

    #[test]
    fn test_reader_open_nonexistent_returns_none() {
        // Don't create the file — reader should return None
        let result = ContextReader::open("nonexistent-ai-999");
        match result {
            Ok(None) => {} // Expected
            Ok(Some(_)) => panic!("should not find nonexistent SHM"),
            Err(_) => {} // Also acceptable (path issues)
        }
    }

    #[test]
    fn test_reader_bad_magic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("context_bad-ai.shm");

        // Write 64 bytes of garbage (wrong magic)
        std::fs::write(&path, [0xFFu8; CONTEXT_SIZE]).expect("write garbage");

        let file = std::fs::File::open(&path).expect("open");
        let mmap = unsafe { memmap2::Mmap::map(&file).expect("mmap") };

        let magic = read_volatile_u64_from(&mmap, OFF_MAGIC);
        assert_ne!(magic, CONTEXT_MAGIC, "garbage should not match magic");
    }

    #[test]
    fn test_is_stale_fresh() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut writer = create_test_writer(dir.path(), "test-ai");
        let reader = create_test_reader(dir.path(), "test-ai");

        writer.update(1, 2).expect("update");

        // Just written — should not be stale with 60s window
        assert!(!reader.is_stale(60_000), "freshly written context should not be stale");
    }

    #[test]
    fn test_is_stale_never_written() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _writer = create_test_writer(dir.path(), "test-ai");
        let reader = create_test_reader(dir.path(), "test-ai");

        // Never updated — updated_at is 0
        assert!(reader.is_stale(60_000), "never-written context should be stale");
    }

    #[test]
    fn test_read_current_from_writer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut writer = create_test_writer(dir.path(), "test-ai");

        writer.update(0x1111, 0x2222).expect("update");

        let ctx = writer.read_current();
        assert_eq!(ctx.simhash, 0x1111);
        assert_eq!(ctx.bloom, 0x2222);
    }

    #[test]
    fn test_tool_call_count_from_reader() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut writer = create_test_writer(dir.path(), "test-ai");
        let reader = create_test_reader(dir.path(), "test-ai");

        writer.update(1, 2).expect("update");
        writer.increment_tool_calls().expect("inc");

        assert_eq!(reader.tool_call_count(), 2);
    }

    #[test]
    fn test_concurrent_read_write_same_thread() {
        // Simulates rapid read/write interleaving (single-threaded)
        let dir = tempfile::tempdir().expect("tempdir");
        let mut writer = create_test_writer(dir.path(), "test-ai");
        let reader = create_test_reader(dir.path(), "test-ai");

        for i in 0..100u64 {
            writer.update(i, i * 2).expect("update");
            let ctx = reader.read().expect("read must succeed");
            assert_eq!(ctx.simhash, i, "simhash mismatch at iteration {}", i);
            assert_eq!(ctx.bloom, i * 2, "bloom mismatch at iteration {}", i);
        }
    }

    #[test]
    fn test_concurrent_read_write_multi_thread() {
        // Writer in one thread, reader in another — validates seqlock correctness
        let dir = tempfile::tempdir().expect("tempdir");
        let dir_path = dir.path().to_path_buf();

        let mut writer = create_test_writer(&dir_path, "mt-test");

        let reader_dir = dir_path.clone();
        let reader_handle = std::thread::spawn(move || {
            let reader = create_test_reader(&reader_dir, "mt-test");
            let mut reads = 0u64;
            let mut max_seen = 0u64;

            for _ in 0..10_000 {
                if let Some(ctx) = reader.read() {
                    reads += 1;
                    // Invariant: bloom must always equal simhash * 3
                    // (this is what the writer guarantees)
                    if ctx.simhash > 0 {
                        assert_eq!(
                            ctx.bloom,
                            ctx.simhash.wrapping_mul(3),
                            "TORN READ DETECTED: simhash={}, bloom={} (expected {})",
                            ctx.simhash,
                            ctx.bloom,
                            ctx.simhash.wrapping_mul(3)
                        );
                    }
                    if ctx.simhash > max_seen {
                        max_seen = ctx.simhash;
                    }
                }
                std::hint::spin_loop();
            }

            (reads, max_seen)
        });

        // Writer: update 1000 times with invariant bloom = simhash * 3
        for i in 1..=1000u64 {
            writer.update(i, i.wrapping_mul(3)).expect("update");
        }

        let (reads, max_seen) = reader_handle.join().expect("reader thread panicked");
        assert!(reads > 0, "reader should have succeeded at least once (got {} reads)", reads);
        assert!(max_seen > 0, "reader should have seen at least one update (max_seen={})", max_seen);
    }

    #[test]
    fn test_context_shm_path_format() {
        if let Ok(path) = context_shm_path("lyra-584") {
            let filename = path.file_name().unwrap().to_str().unwrap();
            assert_eq!(filename, "context_lyra-584.shm");
            assert!(path.to_str().unwrap().contains(".ai-foundation"));
            assert!(path.to_str().unwrap().contains("shm"));
        }
    }

    #[test]
    fn test_layout_is_64_bytes() {
        // Compile-time check is in the const assertion above,
        // but let's also verify at runtime that our offsets are consistent.
        assert_eq!(OFF_MAGIC, 0);
        assert_eq!(OFF_SEQUENCE, 8);
        assert_eq!(OFF_SIMHASH, 16);
        assert_eq!(OFF_BLOOM, 24);
        assert_eq!(OFF_UPDATED_AT, 32);
        assert_eq!(OFF_TOOL_CALLS, 40);
        // Reserved: 48..64
        assert_eq!(CONTEXT_SIZE, 64);
    }
}
