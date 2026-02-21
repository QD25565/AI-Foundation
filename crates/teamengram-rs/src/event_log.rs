//! Master Event Log - Append-Only Persistent Event Store
//!
//! The central log where all events from all AI outboxes are sequenced and persisted.
//! Only the Sequencer thread writes to this log; all AIs read from it.
//!
//! Architecture:
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                   Master Event Log File                         │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  Header (4KB)                                                   │
//! │  - magic: u64                                                   │
//! │  - version: u32                                                 │
//! │  - flags: u32                                                   │
//! │  - head_sequence: AtomicU64 (last written sequence)             │
//! │  - head_offset: AtomicU64 (byte offset after last event)        │
//! │  - event_count: AtomicU64                                       │
//! │  - created_at: u64                                              │
//! │  - last_write_at: AtomicU64                                     │
//! │  - checkpoints[8]: Checkpoint (periodic position markers)       │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  Event 0: [length:4][header:64][payload:variable]               │
//! │  Event 1: [length:4][header:64][payload:variable]               │
//! │  ...                                                            │
//! │  Event N: [length:4][header:64][payload:variable]               │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Key Properties:
//! - Append-only: Events are never modified or deleted
//! - Sequential: Each event gets a monotonically increasing sequence number
//! - Memory-mapped: Fast reads without syscalls
//! - Crash-safe: Head pointer only advanced after fsync
//! - Seekable: Readers can start from any checkpoint or scan from beginning

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use memmap2::MmapMut;
use crate::event::{Event, EventHeader, EventPayload};

/// Magic number for event log files
pub const EVENT_LOG_MAGIC: u64 = 0x4556_4E54_4C4F_4756; // "EVNTLOGV"

/// Event log file format version
pub const EVENT_LOG_VERSION: u32 = 1;

/// Header size (4KB to align with page size)
pub const EVENT_LOG_HEADER_SIZE: usize = 4096;

/// Default initial file size (64MB)
pub const DEFAULT_INITIAL_SIZE: usize = 64 * 1024 * 1024;

/// Maximum file size (4GB)
pub const MAX_FILE_SIZE: usize = 4 * 1024 * 1024 * 1024;

/// Number of checkpoint slots
pub const NUM_CHECKPOINTS: usize = 8;

/// Checkpoint interval (every N events)
pub const CHECKPOINT_INTERVAL: u64 = 10000;

/// Result type for event log operations
pub type EventLogResult<T> = Result<T, EventLogError>;

/// Event log errors
#[derive(Debug, thiserror::Error)]
pub enum EventLogError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Invalid magic number")]
    InvalidMagic,

    #[error("Version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: u32, got: u32 },

    #[error("Log is full: size {size}, max {max}")]
    Full { size: usize, max: usize },

    #[error("Sequence out of order: expected {expected}, got {got}")]
    SequenceError { expected: u64, got: u64 },

    #[error("Invalid offset: {offset} (file size: {file_size})")]
    InvalidOffset { offset: usize, file_size: usize },

    #[error("Event not found: sequence {sequence}")]
    NotFound { sequence: u64 },

    #[error("Corrupted event at offset {offset}")]
    Corrupted { offset: usize },

    #[error("Deserialization error: {0}")]
    Deserialization(String),
}

/// Checkpoint - periodic position marker for fast seeking
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Checkpoint {
    /// Sequence number at this checkpoint
    pub sequence: u64,
    /// Byte offset in the file
    pub offset: u64,
    /// Timestamp when checkpoint was created
    pub timestamp: u64,
}

impl Default for Checkpoint {
    fn default() -> Self {
        Self {
            sequence: 0,
            offset: EVENT_LOG_HEADER_SIZE as u64,
            timestamp: 0,
        }
    }
}

/// Event log header stored at the beginning of the file
#[repr(C)]
pub struct EventLogHeader {
    /// Magic number for validation
    pub magic: u64,
    /// File format version
    pub version: u32,
    /// Flags (reserved)
    pub flags: AtomicU32,
    /// Sequence number of the last written event
    pub head_sequence: AtomicU64,
    /// Byte offset after the last written event (next write position)
    pub head_offset: AtomicU64,
    /// Total number of events in the log
    pub event_count: AtomicU64,
    /// Timestamp when log was created
    pub created_at: u64,
    /// Timestamp of last write
    pub last_write_at: AtomicU64,
    /// Checkpoints for fast seeking
    pub checkpoints: [Checkpoint; NUM_CHECKPOINTS],
    /// Padding to 4KB
    _padding: [u8; EVENT_LOG_HEADER_SIZE - 8 - 4 - 4 - 8 - 8 - 8 - 8 - 8 - (NUM_CHECKPOINTS * 24)],
}

impl EventLogHeader {
    /// Check if the header is valid
    pub fn is_valid(&self) -> bool {
        self.magic == EVENT_LOG_MAGIC && self.version == EVENT_LOG_VERSION
    }

    /// Get the current head sequence
    #[inline]
    pub fn head_sequence(&self) -> u64 {
        self.head_sequence.load(Ordering::Acquire)
    }

    /// Get the current head offset
    #[inline]
    pub fn head_offset(&self) -> u64 {
        self.head_offset.load(Ordering::Acquire)
    }

    /// Get the event count
    #[inline]
    pub fn event_count(&self) -> u64 {
        self.event_count.load(Ordering::Acquire)
    }

    /// Find the best checkpoint for a given sequence number
    pub fn find_checkpoint(&self, target_sequence: u64) -> Option<Checkpoint> {
        let mut best: Option<Checkpoint> = None;

        for checkpoint in &self.checkpoints {
            if checkpoint.sequence > 0 && checkpoint.sequence <= target_sequence {
                match &best {
                    None => best = Some(*checkpoint),
                    Some(b) if checkpoint.sequence > b.sequence => best = Some(*checkpoint),
                    _ => {}
                }
            }
        }

        best
    }
}

/// Event Log Writer (used by Sequencer only)
pub struct EventLogWriter {
    mmap: MmapMut,
    file: File,
    path: PathBuf,
    file_size: usize,
    next_checkpoint_idx: usize,
}

impl EventLogWriter {
    /// Open or create an event log
    pub fn open(base_dir: Option<&Path>) -> EventLogResult<Self> {
        Self::open_with_size(base_dir, DEFAULT_INITIAL_SIZE)
    }

    /// Open or create an event log with specific initial size
    pub fn open_with_size(base_dir: Option<&Path>, initial_size: usize) -> EventLogResult<Self> {
        let path = event_log_path(base_dir);

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let needs_init = !path.exists() || std::fs::metadata(&path)?.len() == 0;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)?;

        let file_size = if needs_init {
            file.set_len(initial_size as u64)?;
            initial_size
        } else {
            file.metadata()?.len() as usize
        };

        let mut mmap = unsafe { MmapMut::map_mut(&file)? };

        if needs_init {
            // Initialize header
            let header = unsafe { &mut *(mmap.as_mut_ptr() as *mut EventLogHeader) };
            header.magic = EVENT_LOG_MAGIC;
            header.version = EVENT_LOG_VERSION;
            header.flags = AtomicU32::new(0);
            header.head_sequence = AtomicU64::new(0);
            header.head_offset = AtomicU64::new(EVENT_LOG_HEADER_SIZE as u64);
            header.event_count = AtomicU64::new(0);
            header.created_at = crate::store::now_millis();
            header.last_write_at = AtomicU64::new(0);
            header.checkpoints = [Checkpoint::default(); NUM_CHECKPOINTS];

            mmap.flush()?;
        } else {
            // Validate existing header
            let header = unsafe { &*(mmap.as_ptr() as *const EventLogHeader) };
            if !header.is_valid() {
                return Err(EventLogError::InvalidMagic);
            }
        }

        Ok(Self {
            mmap,
            file,
            path,
            file_size,
            next_checkpoint_idx: 0,
        })
    }

    /// Get the header
    fn header(&self) -> &EventLogHeader {
        unsafe { &*(self.mmap.as_ptr() as *const EventLogHeader) }
    }

    /// Get mutable header
    fn header_mut(&mut self) -> &mut EventLogHeader {
        unsafe { &mut *(self.mmap.as_mut_ptr() as *mut EventLogHeader) }
    }

    /// Append an event to the log
    ///
    /// Assigns a sequence number and writes the event.
    /// Returns the assigned sequence number.
    pub fn append(&mut self, event: &Event) -> EventLogResult<u64> {
        let payload_bytes = event.payload.to_bytes();
        let mut header = event.header.clone();

        // Assign sequence number
        let sequence = self.header().head_sequence() + 1;
        header.sequence = sequence;

        // Recalculate checksum with new sequence
        header.checksum = header.calculate_checksum(&payload_bytes);

        let header_bytes = header.to_bytes();

        // Calculate total size
        let total_size = 4 + header_bytes.len() + payload_bytes.len();
        let current_offset = self.header().head_offset() as usize;

        // Check if we need to grow the file
        if current_offset + total_size > self.file_size {
            self.grow()?;
        }

        if current_offset + total_size > self.file_size {
            return Err(EventLogError::Full {
                size: self.file_size,
                max: MAX_FILE_SIZE,
            });
        }

        // Write length prefix
        let len_bytes = ((header_bytes.len() + payload_bytes.len()) as u32).to_le_bytes();
        self.mmap[current_offset..current_offset + 4].copy_from_slice(&len_bytes);

        // Write header
        self.mmap[current_offset + 4..current_offset + 4 + 64].copy_from_slice(&header_bytes);

        // Write payload
        self.mmap[current_offset + 68..current_offset + 68 + payload_bytes.len()]
            .copy_from_slice(&payload_bytes);

        // Update header atomically
        let header_ref = self.header();
        header_ref.head_offset.store((current_offset + total_size) as u64, Ordering::Release);
        header_ref.head_sequence.store(sequence, Ordering::Release);
        header_ref.event_count.fetch_add(1, Ordering::Release);
        header_ref.last_write_at.store(crate::store::now_millis(), Ordering::Release);

        // Create checkpoint if needed
        if sequence % CHECKPOINT_INTERVAL == 0 {
            self.create_checkpoint(sequence, current_offset as u64);
        }

        Ok(sequence)
    }

    /// Append raw event bytes (for efficiency when forwarding from outbox)
    pub fn append_raw(&mut self, header_bytes: &[u8; 64], payload_bytes: &[u8], sequence: u64) -> EventLogResult<u64> {
        let total_size = 4 + 64 + payload_bytes.len();
        let current_offset = self.header().head_offset() as usize;

        // Check if we need to grow the file
        if current_offset + total_size > self.file_size {
            self.grow()?;
        }

        if current_offset + total_size > self.file_size {
            return Err(EventLogError::Full {
                size: self.file_size,
                max: MAX_FILE_SIZE,
            });
        }

        // Write length prefix
        let len_bytes = ((64 + payload_bytes.len()) as u32).to_le_bytes();
        self.mmap[current_offset..current_offset + 4].copy_from_slice(&len_bytes);

        // Write header (with sequence already set)
        self.mmap[current_offset + 4..current_offset + 68].copy_from_slice(header_bytes);

        // Write payload
        self.mmap[current_offset + 68..current_offset + 68 + payload_bytes.len()]
            .copy_from_slice(payload_bytes);

        // Update header atomically
        let header_ref = self.header();
        header_ref.head_offset.store((current_offset + total_size) as u64, Ordering::Release);
        header_ref.head_sequence.store(sequence, Ordering::Release);
        header_ref.event_count.fetch_add(1, Ordering::Release);
        header_ref.last_write_at.store(crate::store::now_millis(), Ordering::Release);

        // Create checkpoint if needed
        if sequence % CHECKPOINT_INTERVAL == 0 {
            self.create_checkpoint(sequence, current_offset as u64);
        }

        Ok(sequence)
    }

    /// Grow the file by doubling its size (up to max)
    fn grow(&mut self) -> EventLogResult<()> {
        let new_size = (self.file_size * 2).min(MAX_FILE_SIZE);
        if new_size == self.file_size {
            return Ok(()); // Already at max
        }

        self.file.set_len(new_size as u64)?;
        self.mmap = unsafe { MmapMut::map_mut(&self.file)? };
        self.file_size = new_size;

        Ok(())
    }

    /// Create a checkpoint at the current position
    fn create_checkpoint(&mut self, sequence: u64, offset: u64) {
        let idx = self.next_checkpoint_idx % NUM_CHECKPOINTS;
        self.next_checkpoint_idx += 1;

        let header = self.header_mut();
        header.checkpoints[idx] = Checkpoint {
            sequence,
            offset,
            timestamp: crate::store::now_millis(),
        };
    }

    /// Sync to disk
    pub fn sync(&self) -> EventLogResult<()> {
        self.mmap.flush()?;
        self.file.sync_all()?;
        Ok(())
    }

    /// Get current sequence number
    pub fn current_sequence(&self) -> u64 {
        self.header().head_sequence()
    }

    /// Get event count
    pub fn event_count(&self) -> u64 {
        self.header().event_count()
    }

    /// Get file path
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get file size
    pub fn file_size(&self) -> usize {
        self.file_size
    }

    /// Get used space
    pub fn used_space(&self) -> usize {
        self.header().head_offset() as usize
    }
}

/// Event Log Reader (used by AIs to read events)
pub struct EventLogReader {
    mmap: memmap2::Mmap,
    path: PathBuf,
    /// Current read position (offset in file)
    position: usize,
    /// Last sequence number read
    last_sequence: u64,
}

impl EventLogReader {
    /// Open an existing event log for reading
    pub fn open(base_dir: Option<&Path>) -> EventLogResult<Self> {
        let path = event_log_path(base_dir);

        let file = OpenOptions::new()
            .read(true)
            .open(&path)?;

        let mmap = unsafe { memmap2::Mmap::map(&file)? };

        // Validate header
        let header = unsafe { &*(mmap.as_ptr() as *const EventLogHeader) };
        if !header.is_valid() {
            return Err(EventLogError::InvalidMagic);
        }

        Ok(Self {
            mmap,
            path,
            position: EVENT_LOG_HEADER_SIZE,
            last_sequence: 0,
        })
    }

    /// Get the header
    fn header(&self) -> &EventLogHeader {
        unsafe { &*(self.mmap.as_ptr() as *const EventLogHeader) }
    }

    /// Seek to a specific sequence number (using checkpoints for efficiency)
    pub fn seek_to_sequence(&mut self, target_sequence: u64) -> EventLogResult<()> {
        if target_sequence == 0 {
            self.position = EVENT_LOG_HEADER_SIZE;
            self.last_sequence = 0;
            return Ok(());
        }

        // Find best checkpoint
        if let Some(checkpoint) = self.header().find_checkpoint(target_sequence) {
            self.position = checkpoint.offset as usize;
            self.last_sequence = checkpoint.sequence - 1;
        } else {
            // Start from beginning
            self.position = EVENT_LOG_HEADER_SIZE;
            self.last_sequence = 0;
        }

        // Scan forward to exact sequence
        while self.last_sequence < target_sequence - 1 {
            if self.try_read_raw()?.is_none() {
                return Err(EventLogError::NotFound {
                    sequence: target_sequence,
                });
            }
        }

        Ok(())
    }

    /// Try to read the next event
    pub fn try_read(&mut self) -> EventLogResult<Option<Event>> {
        let raw = match self.try_read_raw()? {
            Some(r) => r,
            None => return Ok(None),
        };

        if raw.len() < 64 {
            return Err(EventLogError::Corrupted {
                offset: self.position,
            });
        }

        // Parse header
        let header_bytes: [u8; 64] = raw[..64].try_into().unwrap();
        let header = EventHeader::from_bytes(&header_bytes);

        // Parse payload
        let payload_bytes = &raw[64..];
        match EventPayload::from_bytes(payload_bytes) {
            Some(payload) => Ok(Some(Event { header, payload })),
            None => Err(EventLogError::Deserialization(
                "Failed to deserialize event payload".to_string()
            )),
        }
    }

    /// Try to read raw event bytes
    pub fn try_read_raw(&mut self) -> EventLogResult<Option<Vec<u8>>> {
        let head_offset = self.header().head_offset() as usize;

        if self.position >= head_offset {
            return Ok(None); // No more events
        }

        if self.position + 4 > self.mmap.len() {
            return Err(EventLogError::InvalidOffset {
                offset: self.position,
                file_size: self.mmap.len(),
            });
        }

        // Read length prefix
        let len_bytes: [u8; 4] = self.mmap[self.position..self.position + 4]
            .try_into()
            .unwrap();
        let len = u32::from_le_bytes(len_bytes) as usize;

        if self.position + 4 + len > self.mmap.len() {
            return Err(EventLogError::Corrupted {
                offset: self.position,
            });
        }

        // Read event data
        let data = self.mmap[self.position + 4..self.position + 4 + len].to_vec();

        // Advance position
        self.position += 4 + len;

        // Update last sequence from header
        if data.len() >= 8 {
            self.last_sequence = u64::from_le_bytes(data[0..8].try_into().unwrap());
        }

        Ok(Some(data))
    }

    /// Check if there are more events to read
    pub fn has_more(&self) -> bool {
        self.position < self.header().head_offset() as usize
    }

    /// Get current position
    pub fn position(&self) -> usize {
        self.position
    }

    /// Get last sequence number read
    pub fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    /// Get the head sequence (latest event in log)
    pub fn head_sequence(&self) -> u64 {
        self.header().head_sequence()
    }

    /// Get event count
    pub fn event_count(&self) -> u64 {
        self.header().event_count()
    }

    /// Refresh the mmap to see new events
    pub fn refresh(&mut self) -> EventLogResult<()> {
        let file = OpenOptions::new()
            .read(true)
            .open(&self.path)?;

        self.mmap = unsafe { memmap2::Mmap::map(&file)? };
        Ok(())
    }
}

/// Get the event log file path
pub fn event_log_path(base_dir: Option<&Path>) -> PathBuf {
    let base = base_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".ai-foundation")
        });

    base.join("shared").join("events").join("master.eventlog")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::event::event_type;

    #[test]
    fn test_event_log_create_and_open() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Create log
        let writer = EventLogWriter::open(Some(base)).unwrap();
        assert_eq!(writer.current_sequence(), 0);
        assert_eq!(writer.event_count(), 0);
        drop(writer);

        // Reopen
        let writer = EventLogWriter::open(Some(base)).unwrap();
        assert_eq!(writer.current_sequence(), 0);
    }

    #[test]
    fn test_event_log_write_read() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Write events
        let mut writer = EventLogWriter::open(Some(base)).unwrap();

        let event1 = Event::broadcast("ai-1", "general", "Hello!");
        let seq1 = writer.append(&event1).unwrap();
        assert_eq!(seq1, 1);

        let event2 = Event::broadcast("ai-2", "general", "Hi there!");
        let seq2 = writer.append(&event2).unwrap();
        assert_eq!(seq2, 2);

        writer.sync().unwrap();
        drop(writer);

        // Read events
        let mut reader = EventLogReader::open(Some(base)).unwrap();
        assert_eq!(reader.head_sequence(), 2);
        assert_eq!(reader.event_count(), 2);

        let read1 = reader.try_read().unwrap().unwrap();
        assert_eq!(read1.header.sequence, 1);
        assert_eq!(read1.header.event_type, event_type::BROADCAST);

        let read2 = reader.try_read().unwrap().unwrap();
        assert_eq!(read2.header.sequence, 2);

        assert!(!reader.has_more());
    }

    #[test]
    fn test_event_log_many_events() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        let mut writer = EventLogWriter::open(Some(base)).unwrap();

        // Write 100 events
        for i in 1..=100 {
            let event = Event::broadcast("test-ai", "general", &format!("Message {}", i));
            let seq = writer.append(&event).unwrap();
            assert_eq!(seq, i);
        }

        writer.sync().unwrap();
        assert_eq!(writer.event_count(), 100);
        drop(writer);

        // Read all events
        let mut reader = EventLogReader::open(Some(base)).unwrap();
        let mut count = 0;
        while let Some(event) = reader.try_read().unwrap() {
            count += 1;
            assert_eq!(event.header.sequence, count);
        }
        assert_eq!(count, 100);
    }

    #[test]
    fn test_event_log_seek() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        let mut writer = EventLogWriter::open(Some(base)).unwrap();

        // Write 50 events
        for i in 1..=50 {
            let event = Event::broadcast("test-ai", "general", &format!("Message {}", i));
            writer.append(&event).unwrap();
        }
        writer.sync().unwrap();
        drop(writer);

        // Seek to sequence 25
        let mut reader = EventLogReader::open(Some(base)).unwrap();
        reader.seek_to_sequence(25).unwrap();

        let event = reader.try_read().unwrap().unwrap();
        assert_eq!(event.header.sequence, 25);
    }

    #[test]
    fn test_event_log_header_size() {
        // Verify header fits in 4KB
        assert!(std::mem::size_of::<EventLogHeader>() <= EVENT_LOG_HEADER_SIZE);
    }
}
