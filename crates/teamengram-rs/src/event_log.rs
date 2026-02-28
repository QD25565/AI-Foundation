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
use std::sync::Arc;
use memmap2::MmapMut;
use crate::event::{Event, EventHeader, EventPayload};
use crate::crypto::{TeamEngramCrypto, FLAG_ENCRYPTED};

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

    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Decryption error: encrypted event but no decryption key configured")]
    NoDecryptionKey,
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
    /// Optional encryption context. When set, all new event payloads are
    /// encrypted with AES-256-GCM before writing. Already-encrypted events
    /// (e.g., during compaction passthrough) are written as-is.
    crypto: Option<Arc<TeamEngramCrypto>>,
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
            crypto: None,
        })
    }

    /// Set the encryption context for this writer.
    /// When set, all new event payloads will be encrypted with AES-256-GCM.
    pub fn set_crypto(&mut self, crypto: Arc<TeamEngramCrypto>) {
        self.crypto = Some(crypto);
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
        // Compress payload if it exceeds the threshold (typically ~3x savings on text)
        let (mut payload_bytes, compressed) = event.payload.to_bytes_compressed();
        let mut header = event.header.clone();

        // Set compressed flag if payload was compressed
        if compressed {
            header.flags |= crate::event::FLAG_COMPRESSED;
        }

        // Assign sequence number
        let sequence = self.header().head_sequence() + 1;
        header.sequence = sequence;

        // Encrypt payload if crypto context is configured.
        // Order: serialize → compress → encrypt (write path)
        // Reader reverses: decrypt → decompress → deserialize
        if let Some(crypto) = &self.crypto {
            payload_bytes = crypto
                .encrypt_payload(&payload_bytes, sequence, header.event_type)
                .map_err(|e| EventLogError::Encryption(e.to_string()))?;
            header.flags |= FLAG_ENCRYPTED;
        }

        header.payload_len = payload_bytes.len() as u16;

        // Recalculate checksum with final payload (compressed+encrypted)
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

    /// Append raw event bytes (for efficiency when forwarding from outbox).
    ///
    /// If encryption is enabled and the event is NOT already encrypted (checked
    /// via FLAG_ENCRYPTED in the header flags), the payload will be encrypted
    /// in-flight. Already-encrypted events (e.g., during compaction passthrough)
    /// are written as-is to avoid double-encryption.
    pub fn append_raw(&mut self, header_bytes: &[u8; 64], payload_bytes: &[u8], sequence: u64) -> EventLogResult<u64> {
        // Check if event is already encrypted (e.g., compaction passthrough)
        let existing_flags = u16::from_le_bytes([header_bytes[52], header_bytes[53]]);
        let already_encrypted = existing_flags & FLAG_ENCRYPTED != 0;

        // Encrypt payload if crypto is configured and event is not already encrypted
        let (final_header_bytes, final_payload);
        if let Some(crypto) = &self.crypto {
            if !already_encrypted {
                let event_type = u16::from_le_bytes([header_bytes[48], header_bytes[49]]);
                let encrypted = crypto
                    .encrypt_payload(payload_bytes, sequence, event_type)
                    .map_err(|e| EventLogError::Encryption(e.to_string()))?;

                // Rebuild header with FLAG_ENCRYPTED set + updated payload_len + checksum
                let mut header = EventHeader::from_bytes(header_bytes);
                header.flags |= FLAG_ENCRYPTED;
                header.payload_len = encrypted.len() as u16;
                header.checksum = header.calculate_checksum(&encrypted);
                final_header_bytes = header.to_bytes();
                final_payload = encrypted;
            } else {
                // Already encrypted — passthrough (compaction)
                final_header_bytes = *header_bytes;
                final_payload = payload_bytes.to_vec();
            }
        } else {
            // No encryption configured — passthrough
            final_header_bytes = *header_bytes;
            final_payload = payload_bytes.to_vec();
        }

        let total_size = 4 + 64 + final_payload.len();
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
        let len_bytes = ((64 + final_payload.len()) as u32).to_le_bytes();
        self.mmap[current_offset..current_offset + 4].copy_from_slice(&len_bytes);

        // Write header
        self.mmap[current_offset + 4..current_offset + 68].copy_from_slice(&final_header_bytes);

        // Write payload
        self.mmap[current_offset + 68..current_offset + 68 + final_payload.len()]
            .copy_from_slice(&final_payload);

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

    /// Open or create an event log at a specific file path.
    /// Used for compaction temp files where the standard path derivation doesn't apply.
    pub fn open_at_path(path: &Path, initial_size: usize) -> EventLogResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let needs_init = !path.exists()
            || std::fs::metadata(path).map(|m| m.len() == 0).unwrap_or(true);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;

        let file_size = if needs_init {
            file.set_len(initial_size as u64)?;
            initial_size
        } else {
            file.metadata()?.len() as usize
        };

        let mmap = unsafe { MmapMut::map_mut(&file)? };

        if needs_init {
            let header = unsafe { &mut *(mmap.as_ptr() as *mut EventLogHeader as *mut EventLogHeader) };
            header.magic = EVENT_LOG_MAGIC;
            header.version = EVENT_LOG_VERSION;
            header.flags = AtomicU32::new(0);
            header.head_sequence = AtomicU64::new(0);
            header.head_offset = AtomicU64::new(EVENT_LOG_HEADER_SIZE as u64);
            header.event_count = AtomicU64::new(0);
            header.created_at = crate::store::now_millis();
            header.last_write_at = AtomicU64::new(0);
            header.checkpoints = [Checkpoint::default(); NUM_CHECKPOINTS];
        } else {
            let header = unsafe { &*(mmap.as_ptr() as *const EventLogHeader) };
            if !header.is_valid() {
                return Err(EventLogError::InvalidMagic);
            }
        }

        Ok(Self {
            mmap,
            file,
            path: path.to_path_buf(),
            file_size,
            next_checkpoint_idx: 0,
            crypto: None,
        })
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
    /// Optional decryption context. When set, encrypted event payloads
    /// (FLAG_ENCRYPTED) are decrypted before deserialization.
    crypto: Option<Arc<TeamEngramCrypto>>,
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
            crypto: None,
        })
    }

    /// Set the decryption context for this reader.
    /// When set, encrypted event payloads (FLAG_ENCRYPTED) are decrypted
    /// transparently during `try_read()`.
    pub fn set_crypto(&mut self, crypto: Arc<TeamEngramCrypto>) {
        self.crypto = Some(crypto);
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

    /// Try to read the next event.
    ///
    /// If the event payload is encrypted (FLAG_ENCRYPTED), it is decrypted
    /// transparently before decompression and deserialization.
    /// Read order: decrypt → decompress → deserialize (reverse of write order).
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

        let payload_bytes = &raw[64..];

        // Decrypt if FLAG_ENCRYPTED is set (before decompression)
        let decrypted;
        let final_payload = if header.flags & FLAG_ENCRYPTED != 0 {
            if let Some(crypto) = &self.crypto {
                decrypted = crypto
                    .decrypt_payload(payload_bytes, header.sequence, header.event_type)
                    .map_err(|e| EventLogError::Encryption(
                        format!("Decryption failed for seq {}: {}", header.sequence, e)
                    ))?;
                &decrypted
            } else {
                return Err(EventLogError::NoDecryptionKey);
            }
        } else {
            payload_bytes
        };

        // Decompress + deserialize (from_bytes_with_flags handles FLAG_COMPRESSED)
        // Note: header.flags still has FLAG_ENCRYPTED set, but from_bytes_with_flags
        // only checks FLAG_COMPRESSED — the encrypted bit is safely ignored.
        match EventPayload::from_bytes_with_flags(final_payload, header.flags) {
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

    /// Open an existing event log at a specific file path.
    pub fn open_at_path(path: &Path) -> EventLogResult<Self> {
        let file = OpenOptions::new()
            .read(true)
            .open(path)?;

        let mmap = unsafe { memmap2::Mmap::map(&file)? };

        let header = unsafe { &*(mmap.as_ptr() as *const EventLogHeader) };
        if !header.is_valid() {
            return Err(EventLogError::InvalidMagic);
        }

        Ok(Self {
            mmap,
            path: path.to_path_buf(),
            position: EVENT_LOG_HEADER_SIZE,
            last_sequence: 0,
            crypto: None,
        })
    }
}

// ─── COMPACTION ─────────────────────────────────────────────────────────────

/// Retention policy for event log compaction.
/// Ephemeral events older than their retention period are removed.
/// Non-ephemeral events (DMs, dialogues, tasks, etc.) are always kept.
pub struct CompactionPolicy {
    /// Max age for PRESENCE_UPDATE events (hours). Default: 24.
    pub presence_hours: u64,
    /// Max age for DM_READ events (hours). Default: 168 (7 days).
    pub dm_read_hours: u64,
    /// Max age for ROOM_JOIN/LEAVE/MUTE events (hours). Default: 168.
    pub room_ephemeral_hours: u64,
    /// Max age for FILE_ACTION events (hours). Default: 168.
    pub file_action_hours: u64,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            presence_hours: 24,
            dm_read_hours: 168,
            room_ephemeral_hours: 168,
            file_action_hours: 168,
        }
    }
}

/// Statistics from a compaction run.
pub struct CompactionStats {
    pub events_kept: u64,
    pub events_removed: u64,
    /// Used bytes in original log (not file size).
    pub bytes_before: u64,
    /// Used bytes in compacted log (not file size).
    pub bytes_after: u64,
}

/// Get maximum retention hours for an event type.
/// Returns u64::MAX for non-ephemeral types (always kept).
fn retention_hours_for(evt_type: u16, policy: &CompactionPolicy) -> u64 {
    use crate::event::event_type as et;
    match evt_type {
        et::PRESENCE_UPDATE => policy.presence_hours,
        et::DM_READ => policy.dm_read_hours,
        et::ROOM_JOIN | et::ROOM_LEAVE | et::ROOM_MUTE => policy.room_ephemeral_hours,
        et::FILE_ACTION => policy.file_action_hours,
        // Deprecated — always remove
        et::LOCK_ACQUIRE | et::LOCK_RELEASE | et::PHEROMONE_DEPOSIT => 0,
        // Everything else: keep forever
        _ => u64::MAX,
    }
}

/// Find the minimum cursor across all AI view engines.
/// Returns u64::MAX if no cursors exist (safe default: all events kept).
fn find_min_cursor(base_dir: &Path) -> EventLogResult<u64> {
    let view_dir = base_dir.join("views");

    if !view_dir.exists() {
        return Ok(u64::MAX);
    }

    let mut min_cursor = u64::MAX;

    for entry in std::fs::read_dir(&view_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("cursor") {
            if let Ok(bytes) = std::fs::read(&path) {
                if bytes.len() == 8 {
                    let cursor = u64::from_le_bytes(bytes.try_into().unwrap());
                    if cursor < min_cursor {
                        min_cursor = cursor;
                    }
                }
            }
        }
    }

    Ok(min_cursor)
}

/// Compact the event log, removing expired ephemeral events.
///
/// **Stop-the-world**: the caller must ensure no writer is active during
/// compaction (the sequencer should be paused).
///
/// Algorithm:
/// 1. Find minimum cursor across all AI views
/// 2. Read all events from current log
/// 3. Write survivors to temp file (preserving original sequence numbers)
/// 4. Atomic swap: rename temp → main
///
/// Safety: events at or above min_cursor are always kept regardless of type
/// or age. Only ephemeral events below min_cursor AND older than their
/// retention period are removed.
pub fn compact_event_log(base_dir: &Path, policy: &CompactionPolicy) -> EventLogResult<CompactionStats> {
    let log_path = event_log_path(Some(base_dir));
    if !log_path.exists() {
        return Ok(CompactionStats {
            events_kept: 0,
            events_removed: 0,
            bytes_before: 0,
            bytes_after: 0,
        });
    }

    let min_cursor = find_min_cursor(base_dir)?;

    // No cursors = no AI has synced yet. Cannot safely compact anything
    // because a new AI could start and need the full event history.
    if min_cursor == u64::MAX {
        let reader = EventLogReader::open_at_path(&log_path)?;
        let used = reader.header().head_offset();
        return Ok(CompactionStats {
            events_kept: reader.event_count(),
            events_removed: 0,
            bytes_before: used,
            bytes_after: used,
        });
    }

    let now_micros = crate::store::now_millis() * 1000;

    // Open reader on current log
    let mut reader = EventLogReader::open_at_path(&log_path)?;
    let bytes_before = reader.header().head_offset();

    // Create temp file for compacted log
    let temp_path = log_path.with_extension("compact.tmp");
    let mut writer = EventLogWriter::open_at_path(&temp_path, DEFAULT_INITIAL_SIZE)?;

    let mut events_kept = 0u64;
    let mut events_removed = 0u64;

    loop {
        let raw = match reader.try_read_raw()? {
            Some(r) => r,
            None => break,
        };

        if raw.len() < 64 {
            continue; // skip corrupted
        }

        // Parse header fields from raw bytes (no full deserialization)
        let sequence = u64::from_le_bytes(raw[0..8].try_into().unwrap());
        let timestamp_micros = u64::from_le_bytes(raw[8..16].try_into().unwrap());
        let evt_type = u16::from_le_bytes(raw[48..50].try_into().unwrap());

        // Safety: NEVER remove events at or above min_cursor
        let should_keep = if sequence >= min_cursor {
            true
        } else {
            let max_age = retention_hours_for(evt_type, policy);
            if max_age == u64::MAX {
                true // non-ephemeral, always keep
            } else if max_age == 0 {
                false // deprecated, always remove
            } else {
                let age_hours = if now_micros > timestamp_micros {
                    (now_micros - timestamp_micros) / 3_600_000_000
                } else {
                    0 // future timestamp, keep
                };
                age_hours < max_age
            }
        };

        if should_keep {
            let header_bytes: [u8; 64] = raw[..64].try_into().unwrap();
            let payload_bytes = &raw[64..];
            writer.append_raw(&header_bytes, payload_bytes, sequence)?;
            events_kept += 1;
        } else {
            events_removed += 1;
        }
    }

    let bytes_after = writer.used_space() as u64;
    writer.sync()?;
    drop(writer);
    drop(reader);

    // Atomic swap with rollback on failure
    let backup_path = log_path.with_extension("compact.bak");
    std::fs::rename(&log_path, &backup_path)?;
    if let Err(e) = std::fs::rename(&temp_path, &log_path) {
        // Rollback: restore backup — log failure but still return the original error
        if let Err(rb_err) = std::fs::rename(&backup_path, &log_path) {
            eprintln!("[COMPACTION] CRITICAL: rollback rename also failed: {}. Backup at {:?}", rb_err, backup_path);
        }
        return Err(EventLogError::Io(e));
    }
    if let Err(e) = std::fs::remove_file(&backup_path) {
        eprintln!("[COMPACTION] Failed to remove backup file {:?}: {}", backup_path, e);
    }

    Ok(CompactionStats {
        events_kept,
        events_removed,
        bytes_before,
        bytes_after,
    })
}

/// Get the event log file path
pub fn event_log_path(base_dir: Option<&Path>) -> PathBuf {
    let base = base_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| crate::store::ai_foundation_base_dir());

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

        let event1 = Event::broadcast("beta-002", "general", "Hello!");
        let seq1 = writer.append(&event1).unwrap();
        assert_eq!(seq1, 1);

        let event2 = Event::broadcast("alpha-001", "general", "Hi there!");
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

    #[test]
    fn test_append_raw_preserves_sequence() {
        let tmp = TempDir::new().unwrap();

        // Write events to source log
        let src_path = tmp.path().join("source.eventlog");
        let mut writer = EventLogWriter::open_at_path(&src_path, 1024 * 1024).unwrap();

        let e1 = Event::broadcast("ai-1", "general", "Hello");
        let e2 = Event::broadcast("ai-2", "general", "World");
        let seq1 = writer.append(&e1).unwrap();
        let seq2 = writer.append(&e2).unwrap();
        writer.sync().unwrap();
        drop(writer);

        // Read raw and write to dest log
        let mut reader = EventLogReader::open_at_path(&src_path).unwrap();
        let dest_path = tmp.path().join("dest.eventlog");
        let mut dest = EventLogWriter::open_at_path(&dest_path, 1024 * 1024).unwrap();

        let raw1 = reader.try_read_raw().unwrap().unwrap();
        let raw2 = reader.try_read_raw().unwrap().unwrap();
        let h1: [u8; 64] = raw1[..64].try_into().unwrap();
        let h2: [u8; 64] = raw2[..64].try_into().unwrap();
        let wrote1 = dest.append_raw(&h1, &raw1[64..], seq1).unwrap();
        let wrote2 = dest.append_raw(&h2, &raw2[64..], seq2).unwrap();
        dest.sync().unwrap();

        // Sequences must be preserved
        assert_eq!(wrote1, seq1);
        assert_eq!(wrote2, seq2);
        assert_eq!(dest.event_count(), 2);
        drop(dest);

        // Verify we can read them back with correct sequences
        let mut check = EventLogReader::open_at_path(&dest_path).unwrap();
        let r1 = check.try_read().unwrap().unwrap();
        let r2 = check.try_read().unwrap().unwrap();
        assert_eq!(r1.header.sequence, seq1);
        assert_eq!(r2.header.sequence, seq2);
        assert_eq!(r1.header.event_type, event_type::BROADCAST);
    }

    #[test]
    fn test_compact_removes_old_presence() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Write a mix of events: 5 presence + 5 broadcasts
        let mut writer = EventLogWriter::open(Some(base)).unwrap();
        for i in 0..10 {
            let event = if i % 2 == 0 {
                Event::presence_update("ai-1", "online", Some("working"))
            } else {
                Event::broadcast("ai-1", "general", &format!("msg {}", i))
            };
            writer.append(&event).unwrap();
        }
        writer.sync().unwrap();
        assert_eq!(writer.event_count(), 10);
        drop(writer);

        // Create a cursor file showing all events processed
        let view_dir = base.join("views");
        std::fs::create_dir_all(&view_dir).unwrap();
        std::fs::write(view_dir.join("ai-1.cursor"), &10u64.to_le_bytes()).unwrap();

        // Compact with 0-hour presence retention (remove all old presence)
        let policy = CompactionPolicy {
            presence_hours: 0,
            ..Default::default()
        };
        let stats = compact_event_log(base, &policy).unwrap();

        // 5 presence events removed, 5 broadcasts kept
        assert_eq!(stats.events_removed, 5);
        assert_eq!(stats.events_kept, 5);
        assert!(stats.bytes_after < stats.bytes_before);

        // Verify compacted log has only broadcasts
        let mut reader = EventLogReader::open(Some(base)).unwrap();
        let mut count = 0;
        while let Some(event) = reader.try_read().unwrap() {
            assert_eq!(event.header.event_type, event_type::BROADCAST);
            count += 1;
        }
        assert_eq!(count, 5);
    }

    #[test]
    fn test_compact_keeps_events_above_min_cursor() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Write 10 presence events
        let mut writer = EventLogWriter::open(Some(base)).unwrap();
        for _ in 0..10 {
            let event = Event::presence_update("ai-1", "online", None);
            writer.append(&event).unwrap();
        }
        writer.sync().unwrap();
        drop(writer);

        // Cursor at sequence 5 — events 6-10 must be kept even if ephemeral
        let view_dir = base.join("views");
        std::fs::create_dir_all(&view_dir).unwrap();
        std::fs::write(view_dir.join("ai-1.cursor"), &5u64.to_le_bytes()).unwrap();

        let policy = CompactionPolicy {
            presence_hours: 0,
            ..Default::default()
        };
        let stats = compact_event_log(base, &policy).unwrap();

        // Events 1-4 removed (below cursor, ephemeral, expired)
        // Event 5 kept (== min_cursor)
        // Events 6-10 kept (above min_cursor)
        assert_eq!(stats.events_removed, 4);
        assert_eq!(stats.events_kept, 6);
    }

    #[test]
    fn test_compact_no_cursors_keeps_everything() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Write 5 presence events
        let mut writer = EventLogWriter::open(Some(base)).unwrap();
        for _ in 0..5 {
            writer.append(&Event::presence_update("ai-1", "idle", None)).unwrap();
        }
        writer.sync().unwrap();
        drop(writer);

        // No cursor files — min_cursor = u64::MAX, everything kept
        let policy = CompactionPolicy { presence_hours: 0, ..Default::default() };
        let stats = compact_event_log(base, &policy).unwrap();

        assert_eq!(stats.events_removed, 0);
        assert_eq!(stats.events_kept, 5);
    }

    #[test]
    fn test_compact_deprecated_always_removed() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        let mut writer = EventLogWriter::open(Some(base)).unwrap();

        // Write a broadcast (kept) and a DM (kept)
        writer.append(&Event::broadcast("ai-1", "general", "keep me")).unwrap();
        writer.append(&Event::direct_message("ai-1", "ai-2", "keep me too")).unwrap();

        // Write a lock acquire event (deprecated — construct manually)
        let lock_event = Event::new("ai-1", EventPayload::LockAcquire(
            crate::event::LockAcquirePayload {
                resource: "/tmp/test".to_string(),
                duration_seconds: 60,
                reason: "test".to_string(),
            }
        ));
        writer.append(&lock_event).unwrap();

        writer.sync().unwrap();
        drop(writer);

        // Cursor past all events
        let view_dir = base.join("views");
        std::fs::create_dir_all(&view_dir).unwrap();
        std::fs::write(view_dir.join("ai-1.cursor"), &10u64.to_le_bytes()).unwrap();

        let stats = compact_event_log(base, &CompactionPolicy::default()).unwrap();

        // Lock event removed (deprecated), broadcast + DM kept
        assert_eq!(stats.events_removed, 1);
        assert_eq!(stats.events_kept, 2);
    }

    // ── Encryption at rest tests ─────────────────────────────────────────

    fn test_crypto() -> Arc<TeamEngramCrypto> {
        let mut key = [0u8; 32];
        for (i, b) in key.iter_mut().enumerate() {
            *b = i as u8;
        }
        Arc::new(TeamEngramCrypto::new(&key))
    }

    #[test]
    fn test_encrypted_write_read_round_trip() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let crypto = test_crypto();

        // Write encrypted events
        let mut writer = EventLogWriter::open(Some(base)).unwrap();
        writer.set_crypto(crypto.clone());

        let event1 = Event::broadcast("ai-1", "general", "Encrypted hello!");
        let event2 = Event::direct_message("ai-1", "ai-2", "Secret message");
        let seq1 = writer.append(&event1).unwrap();
        let seq2 = writer.append(&event2).unwrap();
        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);
        writer.sync().unwrap();
        drop(writer);

        // Read with crypto — should decrypt transparently
        let mut reader = EventLogReader::open(Some(base)).unwrap();
        reader.set_crypto(crypto);

        let read1 = reader.try_read().unwrap().unwrap();
        assert_eq!(read1.header.sequence, 1);
        assert_eq!(read1.header.event_type, event_type::BROADCAST);
        assert!(read1.header.flags & FLAG_ENCRYPTED != 0);

        let read2 = reader.try_read().unwrap().unwrap();
        assert_eq!(read2.header.sequence, 2);
        assert_eq!(read2.header.event_type, event_type::DIRECT_MESSAGE);
    }

    #[test]
    fn test_encrypted_read_without_key_fails() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let crypto = test_crypto();

        // Write encrypted
        let mut writer = EventLogWriter::open(Some(base)).unwrap();
        writer.set_crypto(crypto);
        writer.append(&Event::broadcast("ai-1", "general", "secret")).unwrap();
        writer.sync().unwrap();
        drop(writer);

        // Read WITHOUT crypto — should fail with NoDecryptionKey
        let mut reader = EventLogReader::open(Some(base)).unwrap();
        let result = reader.try_read();
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("no decryption key") || err_msg.contains("Decryption"));
    }

    #[test]
    fn test_encrypted_read_wrong_key_fails() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let crypto = test_crypto();

        // Write encrypted
        let mut writer = EventLogWriter::open(Some(base)).unwrap();
        writer.set_crypto(crypto);
        writer.append(&Event::broadcast("ai-1", "general", "secret")).unwrap();
        writer.sync().unwrap();
        drop(writer);

        // Read with wrong key — GCM auth should fail
        let mut wrong_key = [0u8; 32];
        wrong_key[0] = 0xFF;
        let wrong_crypto = Arc::new(TeamEngramCrypto::new(&wrong_key));
        let mut reader = EventLogReader::open(Some(base)).unwrap();
        reader.set_crypto(wrong_crypto);
        let result = reader.try_read();
        assert!(result.is_err());
    }

    #[test]
    fn test_unencrypted_readable_without_key() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        // Write WITHOUT encryption
        let mut writer = EventLogWriter::open(Some(base)).unwrap();
        writer.append(&Event::broadcast("ai-1", "general", "plaintext")).unwrap();
        writer.sync().unwrap();
        drop(writer);

        // Read without crypto — should work fine (backward compat)
        let mut reader = EventLogReader::open(Some(base)).unwrap();
        let event = reader.try_read().unwrap().unwrap();
        assert_eq!(event.header.sequence, 1);
        assert_eq!(event.header.flags & FLAG_ENCRYPTED, 0);
    }

    #[test]
    fn test_mixed_encrypted_unencrypted_events() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let crypto = test_crypto();

        // Write 2 unencrypted events
        let mut writer = EventLogWriter::open(Some(base)).unwrap();
        writer.append(&Event::broadcast("ai-1", "general", "plain 1")).unwrap();
        writer.append(&Event::broadcast("ai-1", "general", "plain 2")).unwrap();

        // Enable encryption mid-stream, write 2 more
        writer.set_crypto(crypto.clone());
        writer.append(&Event::broadcast("ai-1", "general", "encrypted 3")).unwrap();
        writer.append(&Event::broadcast("ai-1", "general", "encrypted 4")).unwrap();
        writer.sync().unwrap();
        drop(writer);

        // Read all 4 with crypto — should handle both transparently
        let mut reader = EventLogReader::open(Some(base)).unwrap();
        reader.set_crypto(crypto);

        let e1 = reader.try_read().unwrap().unwrap();
        assert_eq!(e1.header.flags & FLAG_ENCRYPTED, 0); // plain

        let e2 = reader.try_read().unwrap().unwrap();
        assert_eq!(e2.header.flags & FLAG_ENCRYPTED, 0); // plain

        let e3 = reader.try_read().unwrap().unwrap();
        assert!(e3.header.flags & FLAG_ENCRYPTED != 0); // encrypted

        let e4 = reader.try_read().unwrap().unwrap();
        assert!(e4.header.flags & FLAG_ENCRYPTED != 0); // encrypted

        assert!(!reader.has_more());
    }

    #[test]
    fn test_append_raw_encrypts_fresh_payload() {
        let tmp = TempDir::new().unwrap();
        let crypto = test_crypto();

        // Write unencrypted to source
        let src_path = tmp.path().join("src.eventlog");
        let mut src_writer = EventLogWriter::open_at_path(&src_path, 1024 * 1024).unwrap();
        let event = Event::broadcast("ai-1", "general", "hello raw");
        src_writer.append(&event).unwrap();
        src_writer.sync().unwrap();
        drop(src_writer);

        // Read raw, then write to encrypted dest via append_raw
        let mut reader = EventLogReader::open_at_path(&src_path).unwrap();
        let raw = reader.try_read_raw().unwrap().unwrap();
        let h: [u8; 64] = raw[..64].try_into().unwrap();
        let payload = &raw[64..];

        // Verify source is NOT encrypted
        let src_flags = u16::from_le_bytes([h[52], h[53]]);
        assert_eq!(src_flags & FLAG_ENCRYPTED, 0);

        let dest_path = tmp.path().join("dest.eventlog");
        let mut dest_writer = EventLogWriter::open_at_path(&dest_path, 1024 * 1024).unwrap();
        dest_writer.set_crypto(crypto.clone());
        dest_writer.append_raw(&h, payload, 1).unwrap();
        dest_writer.sync().unwrap();
        drop(dest_writer);

        // Read dest with crypto — should decrypt successfully
        let mut dest_reader = EventLogReader::open_at_path(&dest_path).unwrap();
        dest_reader.set_crypto(crypto);
        let decrypted = dest_reader.try_read().unwrap().unwrap();
        assert_eq!(decrypted.header.event_type, event_type::BROADCAST);
        assert!(decrypted.header.flags & FLAG_ENCRYPTED != 0);
    }

    #[test]
    fn test_append_raw_passthrough_already_encrypted() {
        let tmp = TempDir::new().unwrap();
        let crypto = test_crypto();

        // Write encrypted to source
        let src_path = tmp.path().join("src.eventlog");
        let mut src_writer = EventLogWriter::open_at_path(&src_path, 1024 * 1024).unwrap();
        src_writer.set_crypto(crypto.clone());
        src_writer.append(&Event::broadcast("ai-1", "general", "already encrypted")).unwrap();
        src_writer.sync().unwrap();
        drop(src_writer);

        // Read raw (encrypted bytes)
        let mut reader = EventLogReader::open_at_path(&src_path).unwrap();
        let raw = reader.try_read_raw().unwrap().unwrap();
        let h: [u8; 64] = raw[..64].try_into().unwrap();
        let payload = &raw[64..];

        // Verify source IS encrypted
        let src_flags = u16::from_le_bytes([h[52], h[53]]);
        assert!(src_flags & FLAG_ENCRYPTED != 0);

        // Write to dest (also with crypto) — should NOT double-encrypt
        let dest_path = tmp.path().join("dest.eventlog");
        let mut dest_writer = EventLogWriter::open_at_path(&dest_path, 1024 * 1024).unwrap();
        dest_writer.set_crypto(crypto.clone());
        dest_writer.append_raw(&h, payload, 1).unwrap();
        dest_writer.sync().unwrap();
        drop(dest_writer);

        // Read with crypto — if double-encrypted, this would fail
        let mut dest_reader = EventLogReader::open_at_path(&dest_path).unwrap();
        dest_reader.set_crypto(crypto);
        let event = dest_reader.try_read().unwrap().unwrap();
        assert_eq!(event.header.event_type, event_type::BROADCAST);
    }

    #[test]
    fn test_encrypted_many_events() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let crypto = test_crypto();

        let mut writer = EventLogWriter::open(Some(base)).unwrap();
        writer.set_crypto(crypto.clone());

        for i in 1..=50 {
            let event = Event::broadcast("test-ai", "general", &format!("Encrypted msg {}", i));
            let seq = writer.append(&event).unwrap();
            assert_eq!(seq, i);
        }
        writer.sync().unwrap();
        drop(writer);

        let mut reader = EventLogReader::open(Some(base)).unwrap();
        reader.set_crypto(crypto);
        let mut count = 0u64;
        while let Some(event) = reader.try_read().unwrap() {
            count += 1;
            assert_eq!(event.header.sequence, count);
            assert!(event.header.flags & FLAG_ENCRYPTED != 0);
        }
        assert_eq!(count, 50);
    }
}
