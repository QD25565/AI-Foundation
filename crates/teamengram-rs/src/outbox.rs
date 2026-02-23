//! Per-AI SPSC Outbox for Event Sourcing
//!
//! Each AI gets a private memory-mapped outbox file. Events are written
//! wait-free (~100ns) and consumed by the Sequencer thread.
//!
//! Architecture:
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                     Per-AI Outbox File                          │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  Header (64 bytes)                                              │
//! │  - magic: u64                                                   │
//! │  - version: u32                                                 │
//! │  - ai_id_hash: u32                                              │
//! │  - head: AtomicU64 (producer write position)                    │
//! │  - tail: AtomicU64 (consumer read position)                     │
//! │  - capacity: u64                                                │
//! │  - last_sequence: AtomicU64 (last assigned sequence number)     │
//! │  - flags: u32                                                   │
//! │  - _padding                                                     │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  Ring Buffer Data (capacity bytes)                              │
//! │  - Variable-length events with length prefix                    │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Wait-free guarantees:
//! - Producer (AI) never blocks - writes to head, advances atomically
//! - Consumer (Sequencer) never blocks - reads from tail, advances atomically
//! - No locks, no CAS loops, pure Release/Acquire ordering

use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use memmap2::MmapMut;
use crate::event::{Event, EventHeader, EventPayload};
use crate::wake::signal_sequencer;

/// Magic number for outbox files
pub const OUTBOX_MAGIC: u64 = 0x4F55_5442_4F58_5632; // "OUTBOXV2"

/// Outbox file format version
pub const OUTBOX_VERSION: u32 = 1;

/// Default outbox capacity (1MB - enough for ~10,000 small events)
pub const DEFAULT_OUTBOX_CAPACITY: usize = 1024 * 1024;

/// Minimum outbox capacity (64KB)
pub const MIN_OUTBOX_CAPACITY: usize = 64 * 1024;

/// Maximum outbox capacity (16MB)
pub const MAX_OUTBOX_CAPACITY: usize = 16 * 1024 * 1024;

/// Header size (64 bytes, cache-line aligned)
pub const OUTBOX_HEADER_SIZE: usize = 64;

/// Outbox flags
pub mod flags {
    /// Outbox is being compacted (Sequencer should wait)
    pub const COMPACTING: u32 = 1 << 0;
    /// Outbox has been closed (no more writes)
    pub const CLOSED: u32 = 1 << 1;
    /// Outbox is in error state
    pub const ERROR: u32 = 1 << 2;
}

/// Result type for outbox operations
pub type OutboxResult<T> = Result<T, OutboxError>;

/// Outbox errors
#[derive(Debug, thiserror::Error)]
pub enum OutboxError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Invalid magic number")]
    InvalidMagic,

    #[error("Version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: u32, got: u32 },

    #[error("AI ID mismatch: expected hash {expected}, got {got}")]
    AiIdMismatch { expected: u32, got: u32 },

    #[error("Outbox full: need {needed} bytes, have {available}")]
    Full { needed: usize, available: usize },

    #[error("Event too large: {size} bytes (max {max})")]
    EventTooLarge { size: usize, max: usize },

    #[error("Outbox closed")]
    Closed,

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),
}

/// Outbox header stored at the beginning of the memory-mapped file
#[repr(C, align(64))]
pub struct OutboxHeader {
    /// Magic number for validation
    pub magic: u64,
    /// File format version
    pub version: u32,
    /// Hash of AI ID (for quick validation)
    pub ai_id_hash: u32,
    /// Producer write position (owned by AI)
    pub head: AtomicU64,
    /// Consumer read position (owned by Sequencer)
    pub tail: AtomicU64,
    /// Capacity of the data buffer
    pub capacity: u64,
    /// Last sequence number assigned by Sequencer (for resumption)
    pub last_sequence: AtomicU64,
    /// Flags (compacting, closed, error)
    pub flags: AtomicU32,
    /// Reserved for future use
    pub _reserved: [u8; 4],
}

impl OutboxHeader {
    /// Check if the header is valid
    pub fn is_valid(&self) -> bool {
        self.magic == OUTBOX_MAGIC && self.version == OUTBOX_VERSION
    }

    /// Get available space for writing
    #[inline]
    pub fn available_write(&self) -> u64 {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        self.capacity - (head - tail)
    }

    /// Get available data for reading
    #[inline]
    pub fn available_read(&self) -> u64 {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);
        head - tail
    }

    /// Check if outbox is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.available_read() == 0
    }

    /// Check if a flag is set
    #[inline]
    pub fn has_flag(&self, flag: u32) -> bool {
        self.flags.load(Ordering::Acquire) & flag != 0
    }

    /// Set a flag
    #[inline]
    pub fn set_flag(&self, flag: u32) {
        self.flags.fetch_or(flag, Ordering::Release);
    }

    /// Clear a flag
    #[inline]
    pub fn clear_flag(&self, flag: u32) {
        self.flags.fetch_and(!flag, Ordering::Release);
    }
}

/// Hash function for AI IDs (FNV-1a)
pub fn hash_ai_id(ai_id: &str) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for byte in ai_id.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

/// Producer-side outbox (used by AI to write events)
pub struct OutboxProducer {
    mmap: MmapMut,
    path: PathBuf,
    ai_id: String,
    capacity: usize,
}

impl OutboxProducer {
    /// Open or create an outbox for the given AI
    pub fn open(ai_id: &str, base_dir: Option<&Path>) -> OutboxResult<Self> {
        Self::open_with_capacity(ai_id, base_dir, DEFAULT_OUTBOX_CAPACITY)
    }

    /// Open or create an outbox with specific capacity
    pub fn open_with_capacity(ai_id: &str, base_dir: Option<&Path>, capacity: usize) -> OutboxResult<Self> {
        let capacity = capacity.clamp(MIN_OUTBOX_CAPACITY, MAX_OUTBOX_CAPACITY);
        let path = outbox_path(ai_id, base_dir);

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file_size = OUTBOX_HEADER_SIZE + capacity;
        let needs_init = !path.exists() || std::fs::metadata(&path)?.len() == 0;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)?;

        if needs_init {
            file.set_len(file_size as u64)?;
        }

        let mut mmap = unsafe { MmapMut::map_mut(&file)? };

        if needs_init {
            // Initialize header
            let header = unsafe { &mut *(mmap.as_mut_ptr() as *mut OutboxHeader) };
            header.magic = OUTBOX_MAGIC;
            header.version = OUTBOX_VERSION;
            header.ai_id_hash = hash_ai_id(ai_id);
            header.head = AtomicU64::new(0);
            header.tail = AtomicU64::new(0);
            header.capacity = capacity as u64;
            header.last_sequence = AtomicU64::new(0);
            header.flags = AtomicU32::new(0);

            mmap.flush()?;
        } else {
            // Validate existing header
            let header = unsafe { &*(mmap.as_ptr() as *const OutboxHeader) };
            if !header.is_valid() {
                return Err(OutboxError::InvalidMagic);
            }
            if header.ai_id_hash != hash_ai_id(ai_id) {
                return Err(OutboxError::AiIdMismatch {
                    expected: hash_ai_id(ai_id),
                    got: header.ai_id_hash,
                });
            }
        }

        Ok(Self {
            mmap,
            path,
            ai_id: ai_id.to_string(),
            capacity,
        })
    }

    /// Get the outbox header
    fn header(&self) -> &OutboxHeader {
        unsafe { &*(self.mmap.as_ptr() as *const OutboxHeader) }
    }

    /// Get mutable data buffer
    fn data_mut(&mut self) -> &mut [u8] {
        let ptr = unsafe { self.mmap.as_mut_ptr().add(OUTBOX_HEADER_SIZE) };
        unsafe { std::slice::from_raw_parts_mut(ptr, self.capacity) }
    }

    /// Write an event to the outbox (wait-free for producer)
    ///
    /// Returns the local position in the outbox (NOT the global sequence number).
    /// The Sequencer assigns global sequence numbers when processing.
    ///
    /// # Multi-Process Safety
    /// Uses atomic fetch_add to RESERVE space BEFORE writing. This ensures that
    /// even if multiple processes mmap the same outbox file, each writer gets
    /// exclusive access to their reserved region. Classic lock-free ring buffer pattern.
    pub fn write_event(&mut self, event: &Event) -> OutboxResult<u64> {
        if self.header().has_flag(flags::CLOSED) {
            return Err(OutboxError::Closed);
        }

        // Serialize event
        let payload_bytes = event.payload.to_bytes();
        let header_bytes = event.header.to_bytes();

        // Total size: 4 (length prefix) + 64 (header) + payload
        let total_size = 4 + header_bytes.len() + payload_bytes.len();
        let available = self.header().available_write() as usize;

        if total_size > available {
            return Err(OutboxError::Full {
                needed: total_size,
                available,
            });
        }

        // Maximum event size is 64KB (sanity limit)
        if total_size > 65536 {
            return Err(OutboxError::EventTooLarge {
                size: total_size,
                max: 65536,
            });
        }

        // CRITICAL: Reserve space atomically BEFORE writing data.
        // This prevents race conditions when multiple processes mmap the same outbox.
        // Old bug: load head → write data → fetch_add could cause two writers to
        // write at the same position, leaving gaps with uninitialized data.
        // Fix: fetch_add FIRST to get exclusive region, THEN write.
        let header = unsafe { &*(self.mmap.as_ptr() as *const OutboxHeader) };
        let reserved_pos = header.head.fetch_add(total_size as u64, Ordering::AcqRel) as usize;

        let capacity = self.capacity;
        let data = self.data_mut();

        // Write length prefix (4 bytes, little-endian) at RESERVED position
        let len_bytes = ((header_bytes.len() + payload_bytes.len()) as u32).to_le_bytes();
        for (i, &b) in len_bytes.iter().enumerate() {
            data[(reserved_pos + i) % capacity] = b;
        }

        // Write header
        for (i, &b) in header_bytes.iter().enumerate() {
            data[(reserved_pos + 4 + i) % capacity] = b;
        }

        // Write payload
        for (i, &b) in payload_bytes.iter().enumerate() {
            data[(reserved_pos + 4 + header_bytes.len() + i) % capacity] = b;
        }

        // Memory barrier to ensure all writes are visible before signaling
        std::sync::atomic::fence(Ordering::Release);

        // Flush to disk so Sequencer can see the write
        let _ = self.mmap.flush();

        // Signal sequencer daemon that new events are available (instant wake, no polling!)
        signal_sequencer();

        Ok(reserved_pos as u64)
    }

    /// Write a raw event from header and payload bytes (for efficiency)
    ///
    /// # Multi-Process Safety
    /// Uses atomic fetch_add to RESERVE space BEFORE writing. See write_event() docs.
    pub fn write_raw(&mut self, header_bytes: &[u8; 64], payload_bytes: &[u8]) -> OutboxResult<u64> {
        if self.header().has_flag(flags::CLOSED) {
            return Err(OutboxError::Closed);
        }

        let total_size = 4 + 64 + payload_bytes.len();
        let available = self.header().available_write() as usize;

        if total_size > available {
            return Err(OutboxError::Full {
                needed: total_size,
                available,
            });
        }

        // CRITICAL: Reserve space atomically BEFORE writing data.
        // See write_event() for detailed explanation of the multi-process race condition fix.
        let header = unsafe { &*(self.mmap.as_ptr() as *const OutboxHeader) };
        let reserved_pos = header.head.fetch_add(total_size as u64, Ordering::AcqRel) as usize;

        let capacity = self.capacity;
        let data = self.data_mut();

        // Write length prefix at RESERVED position
        let len_bytes = ((64 + payload_bytes.len()) as u32).to_le_bytes();
        for (i, &b) in len_bytes.iter().enumerate() {
            data[(reserved_pos + i) % capacity] = b;
        }

        // Write header
        for (i, &b) in header_bytes.iter().enumerate() {
            data[(reserved_pos + 4 + i) % capacity] = b;
        }

        // Write payload
        for (i, &b) in payload_bytes.iter().enumerate() {
            data[(reserved_pos + 4 + 64 + i) % capacity] = b;
        }

        // Memory barrier to ensure all writes are visible before signaling
        std::sync::atomic::fence(Ordering::Release);

        // Flush to disk so Sequencer can see the write
        let _ = self.mmap.flush();

        // Signal sequencer daemon that new events are available (instant wake, no polling!)
        signal_sequencer();

        Ok(reserved_pos as u64)
    }

    /// Check available space
    #[inline]
    pub fn available_space(&self) -> usize {
        self.header().available_write() as usize
    }

    /// Check if outbox is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.header().is_empty()
    }

    /// Get the AI ID for this outbox
    pub fn ai_id(&self) -> &str {
        &self.ai_id
    }

    /// Get the file path
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Flush changes to disk
    pub fn flush(&self) -> OutboxResult<()> {
        self.mmap.flush()?;
        Ok(())
    }

    /// Close the outbox (no more writes allowed)
    pub fn close(&self) {
        self.header().set_flag(flags::CLOSED);
        let _ = self.mmap.flush();
    }
}

/// Consumer-side outbox reader (used by Sequencer to drain events)
pub struct OutboxConsumer {
    mmap: MmapMut,
    #[allow(dead_code)]
    path: PathBuf,
    ai_id: String,
    capacity: usize,
}

impl OutboxConsumer {
    /// Open an existing outbox for reading
    pub fn open(ai_id: &str, base_dir: Option<&Path>) -> OutboxResult<Self> {
        let path = outbox_path(ai_id, base_dir);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)?;

        let mmap = unsafe { MmapMut::map_mut(&file)? };

        // Validate header
        let header = unsafe { &*(mmap.as_ptr() as *const OutboxHeader) };
        if !header.is_valid() {
            return Err(OutboxError::InvalidMagic);
        }
        if header.ai_id_hash != hash_ai_id(ai_id) {
            return Err(OutboxError::AiIdMismatch {
                expected: hash_ai_id(ai_id),
                got: header.ai_id_hash,
            });
        }

        let capacity = header.capacity as usize;

        Ok(Self {
            mmap,
            path,
            ai_id: ai_id.to_string(),
            capacity,
        })
    }

    /// Get the outbox header
    fn header(&self) -> &OutboxHeader {
        unsafe { &*(self.mmap.as_ptr() as *const OutboxHeader) }
    }

    /// Get data buffer
    fn data(&self) -> &[u8] {
        let ptr = unsafe { self.mmap.as_ptr().add(OUTBOX_HEADER_SIZE) };
        unsafe { std::slice::from_raw_parts(ptr, self.capacity) }
    }

    /// Try to read the next event (wait-free for consumer)
    ///
    /// Returns None if no events available.
    /// The returned bytes are: [header: 64 bytes][payload: variable]
    pub fn try_read_raw(&self) -> Option<Vec<u8>> {
        self.try_read_raw_with_position().map(|(data, _)| data)
    }

    /// Try to read the next event WITH the tail position for CAS-based commit.
    ///
    /// Returns (event_data, tail_position) or None if no events available.
    /// The tail_position MUST be passed to commit_read_cas() for linearizable commit.
    /// This is the preferred method for multi-process safety.
    pub fn try_read_raw_with_position(&self) -> Option<(Vec<u8>, u64)> {
        let available = self.header().available_read() as usize;
        if available < 4 {
            return None;
        }

        let tail = self.header().tail.load(Ordering::Acquire) as usize;
        let capacity = self.capacity;
        let data = self.data();

        // Read length prefix
        let len_bytes = [
            data[tail % capacity],
            data[(tail + 1) % capacity],
            data[(tail + 2) % capacity],
            data[(tail + 3) % capacity],
        ];
        let len = u32::from_le_bytes(len_bytes) as usize;

        // Sanity check: length must be reasonable (header 64 bytes + payload, max 64KB)
        if len == 0 || len > 65536 {
            // Corrupted data - tail is pointing at garbage
            return None;
        }

        if available < len + 4 {
            return None; // Event not fully written yet
        }

        // Read event data
        let mut event_data = vec![0u8; len];
        for i in 0..len {
            event_data[i] = data[(tail + 4 + i) % capacity];
        }

        Some((event_data, tail as u64))
    }

    /// Commit the read using CAS (compare-and-swap) for linearizable, multi-process-safe commit.
    ///
    /// This is the PREFERRED method. Uses compare_exchange to ensure only one consumer
    /// advances the tail for each event. If the tail has already been advanced by another
    /// process, this returns false (no change made).
    ///
    /// Arguments:
    /// - expected_tail: The tail position returned by try_read_raw_with_position()
    /// - event_size: The size of the event data (not including 4-byte length prefix)
    ///
    /// Returns:
    /// - true: Successfully advanced tail (this process "won" the commit)
    /// - false: Tail was already advanced by another process (skip this event)
    pub fn commit_read_cas(&self, expected_tail: u64, event_size: usize) -> bool {
        let total_size = 4 + event_size; // length prefix + event
        let new_tail = expected_tail + total_size as u64;

        self.header().tail.compare_exchange(
            expected_tail,
            new_tail,
            Ordering::Release,
            Ordering::Relaxed
        ).is_ok()
    }

    /// Commit the read (advance tail after processing)
    ///
    /// DEPRECATED: This method uses fetch_add which is NOT linearizable.
    /// If two processes call this simultaneously, both will advance the tail,
    /// causing corruption. Use commit_read_cas() instead for multi-process safety.
    ///
    /// Only use this if you are CERTAIN only one consumer exists.
    #[deprecated(note = "Use commit_read_cas() for multi-process safety")]
    pub fn commit_read(&self, event_size: usize) {
        let total_size = 4 + event_size; // length prefix + event
        self.header().tail.fetch_add(total_size as u64, Ordering::Release);
    }

    /// Try to read and parse the next event
    pub fn try_read(&self) -> Option<OutboxResult<Event>> {
        let raw = self.try_read_raw()?;

        if raw.len() < 64 {
            return Some(Err(OutboxError::Deserialization(
                "Event too small for header".to_string()
            )));
        }

        // Parse header
        let header_bytes: [u8; 64] = raw[..64].try_into().unwrap();
        let header = EventHeader::from_bytes(&header_bytes);

        // Parse payload
        let payload_bytes = &raw[64..];
        match EventPayload::from_bytes(payload_bytes) {
            Some(payload) => Some(Ok(Event { header, payload })),
            None => Some(Err(OutboxError::Deserialization(
                "Failed to deserialize event payload".to_string()
            ))),
        }
    }

    /// Check if there are pending events
    #[inline]
    pub fn has_pending(&self) -> bool {
        self.header().available_read() > 0
    }

    /// Get number of pending bytes
    #[inline]
    pub fn pending_bytes(&self) -> usize {
        self.header().available_read() as usize
    }

    /// Get the AI ID
    pub fn ai_id(&self) -> &str {
        &self.ai_id
    }

    /// Get the last sequence number assigned to events from this outbox
    pub fn last_sequence(&self) -> u64 {
        self.header().last_sequence.load(Ordering::Acquire)
    }

    /// Update the last sequence number (called by Sequencer after assigning)
    pub fn set_last_sequence(&self, seq: u64) {
        self.header().last_sequence.store(seq, Ordering::Release);
    }

    /// Check if the outbox is closed
    pub fn is_closed(&self) -> bool {
        self.header().has_flag(flags::CLOSED)
    }

    /// Reset tail to head, clearing all pending events.
    ///
    /// USE WITH CAUTION: This discards all pending events in the outbox.
    /// Only use this to repair a corrupted outbox where the tail pointer
    /// is misaligned/invalid.
    ///
    /// Returns the number of bytes that were pending (now discarded).
    pub fn reset_tail_to_head(&self) -> u64 {
        let head = self.header().head.load(Ordering::Acquire);
        let old_tail = self.header().tail.swap(head, Ordering::Release);
        head.saturating_sub(old_tail)
    }

    /// Check if the tail appears to be corrupted.
    ///
    /// Returns Some(reason) if corruption detected, None if appears valid.
    pub fn check_corruption(&self) -> Option<String> {
        let available = self.header().available_read() as usize;
        if available < 4 {
            return None; // Empty outbox is valid
        }

        let tail = self.header().tail.load(Ordering::Acquire) as usize;
        let capacity = self.capacity;
        let data = self.data();

        // Read length prefix at current tail
        let len_bytes = [
            data[tail % capacity],
            data[(tail + 1) % capacity],
            data[(tail + 2) % capacity],
            data[(tail + 3) % capacity],
        ];
        let len = u32::from_le_bytes(len_bytes) as usize;

        // Check for obvious corruption
        if len == 0 {
            return Some(format!("Length at tail is 0 (tail={})", tail));
        }
        if len > 65536 {
            return Some(format!(
                "Length at tail is invalid: {} bytes (max 65536, tail={}, raw bytes={:02x}{:02x}{:02x}{:02x})",
                len, tail, len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]
            ));
        }
        if available < len + 4 {
            return Some(format!(
                "Length {} + 4 exceeds available {} (tail={})",
                len, available, tail
            ));
        }

        None
    }

    /// Peek all pending events without committing (for read-your-own-writes)
    ///
    /// Returns all events from tail to head. Does NOT advance tail.
    /// Use this to see events that haven't been merged yet by the sequencer.
    pub fn peek_all_pending(&self) -> Vec<OutboxResult<Event>> {
        let mut events = Vec::new();
        let mut pos = self.header().tail.load(Ordering::Relaxed) as usize;
        let head = self.header().head.load(Ordering::Acquire) as usize;
        let capacity = self.capacity;
        let data = self.data();

        // DEBUG: uncomment to trace
        // eprintln!("peek_all_pending: pos={}, head={}, capacity={}, data.len={}", pos, head, capacity, data.len());

        while pos < head {
            let available = head - pos;
            if available < 4 {
                break;
            }

            // Read length prefix
            let len_bytes = [
                data[pos % capacity],
                data[(pos + 1) % capacity],
                data[(pos + 2) % capacity],
                data[(pos + 3) % capacity],
            ];
            let len = u32::from_le_bytes(len_bytes) as usize;

            // Sanity checks: len should be reasonable (64 bytes header + payload, max 64KB)
            if len == 0 || len > 65536 || available < len + 4 {
                // Corrupted data or incomplete write - stop reading
                break;
            }

            // Read event data
            let mut event_data = vec![0u8; len];
            for i in 0..len {
                event_data[i] = data[(pos + 4 + i) % capacity];
            }

            // Parse event
            if event_data.len() >= 64 {
                let header_bytes: [u8; 64] = event_data[..64].try_into().unwrap();
                let header = EventHeader::from_bytes(&header_bytes);
                let payload_bytes = &event_data[64..];
                match EventPayload::from_bytes(payload_bytes) {
                    Some(payload) => events.push(Ok(Event { header, payload })),
                    None => events.push(Err(OutboxError::Deserialization(
                        "Failed to deserialize event payload".to_string()
                    ))),
                }
            }

            pos += 4 + len;
        }

        events
    }
}

/// Get the outbox file path for an AI
pub fn outbox_path(ai_id: &str, base_dir: Option<&Path>) -> PathBuf {
    let base = base_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".ai-foundation")
        });

    base.join("shared").join("outbox").join(format!("{}.outbox", ai_id))
}

/// List all outbox files in the directory
pub fn list_outboxes(base_dir: Option<&Path>) -> io::Result<Vec<String>> {
    let base = base_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".ai-foundation")
        });

    let outbox_dir = base.join("shared").join("outbox");

    if !outbox_dir.exists() {
        return Ok(Vec::new());
    }

    let mut ai_ids = Vec::new();
    for entry in std::fs::read_dir(outbox_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "outbox").unwrap_or(false) {
            if let Some(stem) = path.file_stem() {
                ai_ids.push(stem.to_string_lossy().to_string());
            }
        }
    }

    Ok(ai_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::event::{event_type, BroadcastPayload};

    #[test]
    fn test_outbox_create_and_open() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Create outbox
        let producer = OutboxProducer::open("test-ai", Some(base)).unwrap();
        assert_eq!(producer.ai_id(), "test-ai");
        assert!(producer.is_empty());
        drop(producer);

        // Reopen
        let producer = OutboxProducer::open("test-ai", Some(base)).unwrap();
        assert_eq!(producer.ai_id(), "test-ai");
    }

    #[test]
    fn test_outbox_write_read() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Create and write
        let mut producer = OutboxProducer::open("test-ai", Some(base)).unwrap();

        let event = Event::broadcast("test-ai", "general", "Hello, World!");
        producer.write_event(&event).unwrap();

        assert!(!producer.is_empty());
        drop(producer);

        // Read from consumer
        let consumer = OutboxConsumer::open("test-ai", Some(base)).unwrap();
        assert!(consumer.has_pending());

        let read_event = consumer.try_read().unwrap().unwrap();
        assert_eq!(read_event.header.event_type, event_type::BROADCAST);

        if let EventPayload::Broadcast(payload) = read_event.payload {
            assert_eq!(payload.channel, "general");
            assert_eq!(payload.content, "Hello, World!");
        } else {
            panic!("Expected broadcast payload");
        }
    }

    #[test]
    fn test_outbox_multiple_events() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        let mut producer = OutboxProducer::open("test-ai", Some(base)).unwrap();

        // Write multiple events
        for i in 0..10 {
            let event = Event::broadcast("test-ai", "general", &format!("Message {}", i));
            producer.write_event(&event).unwrap();
        }

        drop(producer);

        // Read all
        let consumer = OutboxConsumer::open("test-ai", Some(base)).unwrap();

        for i in 0..10 {
            let raw = consumer.try_read_raw().unwrap();
            let event = consumer.try_read().unwrap().unwrap();

            if let EventPayload::Broadcast(payload) = event.payload {
                assert_eq!(payload.channel, "general");
                assert_eq!(payload.content, format!("Message {}", i));
            }

            consumer.commit_read(raw.len());
        }

        assert!(!consumer.has_pending());
    }

    #[test]
    fn test_outbox_hash_consistency() {
        // Same AI ID should always produce same hash
        let hash1 = hash_ai_id("ai-1");
        let hash2 = hash_ai_id("ai-1");
        assert_eq!(hash1, hash2);

        // Different AI IDs should produce different hashes
        let hash3 = hash_ai_id("ai-2");
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_outbox_full() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Create tiny outbox
        let mut producer = OutboxProducer::open_with_capacity(
            "test-ai",
            Some(base),
            MIN_OUTBOX_CAPACITY
        ).unwrap();

        // Fill it up with large events
        let big_content = "X".repeat(10000);
        let mut count = 0;
        loop {
            let event = Event::broadcast("test-ai", "general", &big_content);
            match producer.write_event(&event) {
                Ok(_) => count += 1,
                Err(OutboxError::Full { .. }) => break,
                Err(e) => panic!("Unexpected error: {:?}", e),
            }
            if count > 100 {
                panic!("Should have filled up by now");
            }
        }

        assert!(count > 0, "Should have written at least one event");
    }

    #[test]
    fn test_list_outboxes() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Create multiple outboxes
        let _p1 = OutboxProducer::open("ai-1", Some(base)).unwrap();
        let _p2 = OutboxProducer::open("ai-2", Some(base)).unwrap();
        let _p3 = OutboxProducer::open("ai-3", Some(base)).unwrap();

        let mut ai_ids = list_outboxes(Some(base)).unwrap();
        ai_ids.sort();

        assert_eq!(ai_ids, vec!["ai-3", "ai-1", "ai-2"]);
    }
}
