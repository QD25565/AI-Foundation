//! VisionEngram (.veng) - Visual memory storage for AI agents
//!
//! Provides persistent storage for visual memories that can be linked to
//! text notes in the Engram notebook. Optimized for AI interpretation with
//! embedded thumbnails.
//!
//! File Format:
//! - Header (32 bytes): Magic "VENG", version, entry count, index offset
//! - Entries: Variable-length records with thumbnail data
//! - Index: Fixed-size records for O(1) lookup by ID
//!
//! Design Philosophy:
//! - Thumbnails inline for fast access (~5-15KB each)
//! - Original images stored externally in images/ directory
//! - Link to Engram notes via note_id (0 = standalone visual)
//! - AI-optimized: 512x512 max, edge enhancement, text-preserving

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

// File format constants
pub const VENG_MAGIC: &[u8; 4] = b"VENG";
pub const VENG_VERSION: u16 = 1;
pub const HEADER_SIZE: usize = 32;
pub const INDEX_ENTRY_SIZE: usize = 24; // id:8 + offset:8 + length:4 + flags:4

/// Thumbnail format for storage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ThumbnailFormat {
    WebP = 0,
    Jpeg = 1,
    Png = 2,
}

impl Default for ThumbnailFormat {
    fn default() -> Self {
        ThumbnailFormat::WebP // Best compression for AI vision
    }
}

impl From<u8> for ThumbnailFormat {
    fn from(v: u8) -> Self {
        match v {
            0 => ThumbnailFormat::WebP,
            1 => ThumbnailFormat::Jpeg,
            2 => ThumbnailFormat::Png,
            _ => ThumbnailFormat::WebP,
        }
    }
}

/// Flags for VisionEntry
pub mod flags {
    pub const HAS_ORIGINAL: u8 = 1 << 0;    // Has full-res original stored
    pub const IS_SCREENSHOT: u8 = 1 << 1;   // Captured via screenshot
    pub const IS_UPLOAD: u8 = 1 << 2;       // User-uploaded image
    pub const HAS_CONTEXT: u8 = 1 << 3;     // Has text context/caption
    pub const DELETED: u8 = 1 << 4;         // Soft deleted (tombstone)
    pub const AI_OPTIMIZED: u8 = 1 << 5;    // Thumbnail was AI-optimized
}

/// Entry in the VisionEngram store
#[derive(Debug, Clone)]
pub struct VisionEntry {
    pub id: u64,
    pub note_id: u64,           // Link to Engram note (0 = standalone)
    pub timestamp: i64,
    pub original_width: u32,
    pub original_height: u32,
    pub thumbnail_width: u16,
    pub thumbnail_height: u16,
    pub thumbnail_format: ThumbnailFormat,
    pub flags: u8,
    pub thumbnail_data: Vec<u8>,
    pub original_path: Option<String>, // Relative path like "images/1234.webp"
    pub context: Option<String>,       // Optional caption/description
}

impl VisionEntry {
    /// Create a new VisionEntry from an image
    pub fn new(
        id: u64,
        note_id: u64,
        thumbnail_data: Vec<u8>,
        thumbnail_width: u16,
        thumbnail_height: u16,
        original_width: u32,
        original_height: u32,
    ) -> Self {
        Self {
            id,
            note_id,
            timestamp: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            original_width,
            original_height,
            thumbnail_width,
            thumbnail_height,
            thumbnail_format: ThumbnailFormat::default(),
            flags: flags::AI_OPTIMIZED,
            thumbnail_data,
            original_path: None,
            context: None,
        }
    }

    /// Set the original image path
    pub fn with_original(mut self, path: impl Into<String>) -> Self {
        self.original_path = Some(path.into());
        self.flags |= flags::HAS_ORIGINAL;
        self
    }

    /// Set context/caption
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self.flags |= flags::HAS_CONTEXT;
        self
    }

    /// Mark as screenshot
    pub fn as_screenshot(mut self) -> Self {
        self.flags |= flags::IS_SCREENSHOT;
        self
    }

    /// Serialize entry to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let original_path_bytes = self.original_path.as_ref()
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_default();
        let context_bytes = self.context.as_ref()
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_default();

        // Calculate total size
        // Fixed header: 8+8+8+4+4+2+2+1+1+4+2+2 = 46 bytes
        let header_len = 46;
        let total_len = header_len
            + self.thumbnail_data.len()
            + original_path_bytes.len()
            + context_bytes.len();

        let mut bytes = Vec::with_capacity(total_len);

        // Write fixed header
        bytes.extend_from_slice(&self.id.to_le_bytes());
        bytes.extend_from_slice(&self.note_id.to_le_bytes());
        bytes.extend_from_slice(&self.timestamp.to_le_bytes());
        bytes.extend_from_slice(&self.original_width.to_le_bytes());
        bytes.extend_from_slice(&self.original_height.to_le_bytes());
        bytes.extend_from_slice(&self.thumbnail_width.to_le_bytes());
        bytes.extend_from_slice(&self.thumbnail_height.to_le_bytes());
        bytes.push(self.thumbnail_format as u8);
        bytes.push(self.flags);
        bytes.extend_from_slice(&(self.thumbnail_data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(original_path_bytes.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(context_bytes.len() as u16).to_le_bytes());

        // Write variable data
        bytes.extend_from_slice(&self.thumbnail_data);
        bytes.extend_from_slice(&original_path_bytes);
        bytes.extend_from_slice(&context_bytes);

        bytes
    }

    /// Deserialize entry from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 46 {
            bail!("VisionEntry data too short: {} bytes", bytes.len());
        }

        let id = u64::from_le_bytes(bytes[0..8].try_into()?);
        let note_id = u64::from_le_bytes(bytes[8..16].try_into()?);
        let timestamp = i64::from_le_bytes(bytes[16..24].try_into()?);
        let original_width = u32::from_le_bytes(bytes[24..28].try_into()?);
        let original_height = u32::from_le_bytes(bytes[28..32].try_into()?);
        let thumbnail_width = u16::from_le_bytes(bytes[32..34].try_into()?);
        let thumbnail_height = u16::from_le_bytes(bytes[34..36].try_into()?);
        let thumbnail_format = ThumbnailFormat::from(bytes[36]);
        let entry_flags = bytes[37];
        let thumbnail_len = u32::from_le_bytes(bytes[38..42].try_into()?) as usize;
        let path_len = u16::from_le_bytes(bytes[42..44].try_into()?) as usize;
        let context_len = u16::from_le_bytes(bytes[44..46].try_into()?) as usize;

        let expected_len = 46 + thumbnail_len + path_len + context_len;
        if bytes.len() < expected_len {
            bail!("VisionEntry data truncated: expected {} bytes, got {}", expected_len, bytes.len());
        }

        let thumb_start = 46;
        let thumb_end = thumb_start + thumbnail_len;
        let path_end = thumb_end + path_len;
        let context_end = path_end + context_len;

        let thumbnail_data = bytes[thumb_start..thumb_end].to_vec();

        let original_path = if path_len > 0 {
            Some(String::from_utf8_lossy(&bytes[thumb_end..path_end]).to_string())
        } else {
            None
        };

        let context = if context_len > 0 {
            Some(String::from_utf8_lossy(&bytes[path_end..context_end]).to_string())
        } else {
            None
        };

        Ok(Self {
            id,
            note_id,
            timestamp,
            original_width,
            original_height,
            thumbnail_width,
            thumbnail_height,
            thumbnail_format,
            flags: entry_flags,
            thumbnail_data,
            original_path,
            context,
        })
    }

    /// Get human-readable age string
    pub fn age_string(&self) -> String {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let diff_secs = (now - self.timestamp) / 1_000_000_000;
        if diff_secs < 0 { "just now".to_string() }
        else if diff_secs < 60 { format!("{}sec ago", diff_secs) }
        else if diff_secs < 3600 { format!("{}min ago", diff_secs / 60) }
        else if diff_secs < 86400 { format!("{}hrs ago", diff_secs / 3600) }
        else if diff_secs < 2592000 { format!("{}days ago", diff_secs / 86400) }
        else { format!("{}months ago", diff_secs / 2592000) }
    }

    /// Check if this entry is deleted (tombstone)
    pub fn is_deleted(&self) -> bool {
        self.flags & flags::DELETED != 0
    }
}

/// Index entry for fast O(1) lookup
#[derive(Debug, Clone, Copy)]
struct IndexEntry {
    id: u64,
    offset: u64,
    length: u32,
    flags: u32,
}

impl IndexEntry {
    fn to_bytes(&self) -> [u8; INDEX_ENTRY_SIZE] {
        let mut bytes = [0u8; INDEX_ENTRY_SIZE];
        bytes[0..8].copy_from_slice(&self.id.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.offset.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.length.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.flags.to_le_bytes());
        bytes
    }

    fn from_bytes(bytes: &[u8; INDEX_ENTRY_SIZE]) -> Self {
        Self {
            id: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            offset: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            length: u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            flags: u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
        }
    }
}

/// VisionEngram storage engine
pub struct VisionEngram {
    path: PathBuf,
    images_dir: PathBuf,
    index: HashMap<u64, IndexEntry>,
    next_id: u64,
    entry_count: u64,
    read_only: bool,
}

impl VisionEngram {
    /// Open or create a VisionEngram store
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let images_dir = path.parent()
            .unwrap_or(Path::new("."))
            .join("images");

        // Create images directory if it doesn't exist
        if !images_dir.exists() {
            fs::create_dir_all(&images_dir)
                .context("Failed to create images directory")?;
        }

        let mut store = Self {
            path: path.clone(),
            images_dir,
            index: HashMap::new(),
            next_id: 1,
            entry_count: 0,
            read_only: false,
        };

        if path.exists() {
            store.load_index()?;
        } else {
            store.init_file()?;
        }

        Ok(store)
    }

    /// Open in read-only mode
    pub fn open_readonly(path: impl AsRef<Path>) -> Result<Self> {
        let mut store = Self::open(path)?;
        store.read_only = true;
        Ok(store)
    }

    /// Initialize empty .veng file
    fn init_file(&self) -> Result<()> {
        let mut file = File::create(&self.path)
            .context("Failed to create .veng file")?;

        let header = self.create_header(0, HEADER_SIZE as u64);
        file.write_all(&header)?;
        file.sync_all()?;

        Ok(())
    }

    /// Create file header
    fn create_header(&self, entry_count: u64, index_offset: u64) -> [u8; HEADER_SIZE] {
        let mut header = [0u8; HEADER_SIZE];
        header[0..4].copy_from_slice(VENG_MAGIC);
        header[4..6].copy_from_slice(&VENG_VERSION.to_le_bytes());
        header[6..14].copy_from_slice(&entry_count.to_le_bytes());
        header[14..22].copy_from_slice(&index_offset.to_le_bytes());
        // Bytes 22-32 reserved for future use
        header
    }

    /// Load index from file
    fn load_index(&mut self) -> Result<()> {
        let mut file = File::open(&self.path)
            .context("Failed to open .veng file")?;

        // Read header
        let mut header = [0u8; HEADER_SIZE];
        file.read_exact(&mut header)?;

        // Verify magic
        if &header[0..4] != VENG_MAGIC {
            bail!("Invalid .veng file: wrong magic bytes");
        }

        let version = u16::from_le_bytes(header[4..6].try_into()?);
        if version > VENG_VERSION {
            bail!("Unsupported .veng version: {}", version);
        }

        self.entry_count = u64::from_le_bytes(header[6..14].try_into()?);
        let index_offset = u64::from_le_bytes(header[14..22].try_into()?);

        // Read index entries
        if self.entry_count > 0 && index_offset >= HEADER_SIZE as u64 {
            file.seek(SeekFrom::Start(index_offset))?;

            for _ in 0..self.entry_count {
                let mut entry_bytes = [0u8; INDEX_ENTRY_SIZE];
                file.read_exact(&mut entry_bytes)?;
                let entry = IndexEntry::from_bytes(&entry_bytes);
                if entry.id >= self.next_id {
                    self.next_id = entry.id + 1;
                }
                self.index.insert(entry.id, entry);
            }
        }

        Ok(())
    }

    /// Store a new visual memory
    pub fn store(&mut self, mut entry: VisionEntry) -> Result<u64> {
        if self.read_only {
            bail!("VisionEngram is read-only");
        }

        // Assign ID
        entry.id = self.next_id;
        self.next_id += 1;

        // Serialize entry
        let entry_bytes = entry.to_bytes();

        // Open file for append
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
            .context("Failed to open .veng file for writing")?;

        // Read current index offset
        let mut header = [0u8; HEADER_SIZE];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header)?;
        let old_index_offset = u64::from_le_bytes(header[14..22].try_into()?);

        // Calculate new entry offset (at old index location, or after header)
        let entry_offset = if old_index_offset >= HEADER_SIZE as u64 {
            old_index_offset
        } else {
            HEADER_SIZE as u64
        };

        // Write entry at calculated offset
        file.seek(SeekFrom::Start(entry_offset))?;
        file.write_all(&entry_bytes)?;

        // Create index entry
        let idx_entry = IndexEntry {
            id: entry.id,
            offset: entry_offset,
            length: entry_bytes.len() as u32,
            flags: entry.flags as u32,
        };
        self.index.insert(entry.id, idx_entry);
        self.entry_count += 1;

        // Write new index after all entries
        let new_index_offset = entry_offset + entry_bytes.len() as u64;
        for (_, idx) in &self.index {
            file.write_all(&idx.to_bytes())?;
        }

        // Update header
        file.seek(SeekFrom::Start(0))?;
        let new_header = self.create_header(self.entry_count, new_index_offset);
        file.write_all(&new_header)?;

        file.sync_all()?;

        Ok(entry.id)
    }

    /// Get a visual memory by ID
    pub fn get(&self, id: u64) -> Result<Option<VisionEntry>> {
        let Some(idx_entry) = self.index.get(&id) else {
            return Ok(None);
        };

        let mut file = File::open(&self.path)
            .context("Failed to open .veng file")?;

        file.seek(SeekFrom::Start(idx_entry.offset))?;

        let mut buffer = vec![0u8; idx_entry.length as usize];
        file.read_exact(&mut buffer)?;

        let entry = VisionEntry::from_bytes(&buffer)?;

        if entry.is_deleted() {
            return Ok(None);
        }

        Ok(Some(entry))
    }

    /// Get all visual memories for a note
    pub fn get_by_note(&self, note_id: u64) -> Result<Vec<VisionEntry>> {
        let mut entries = Vec::new();

        for (_, idx_entry) in &self.index {
            if idx_entry.flags & (flags::DELETED as u32) != 0 {
                continue;
            }

            if let Some(entry) = self.get(idx_entry.id)? {
                if entry.note_id == note_id {
                    entries.push(entry);
                }
            }
        }

        // Sort by timestamp
        entries.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

        Ok(entries)
    }

    /// List recent visual memories
    pub fn list_recent(&self, limit: usize) -> Result<Vec<VisionEntry>> {
        let mut entries: Vec<_> = self.index.iter()
            .filter(|(_, idx)| idx.flags & (flags::DELETED as u32) == 0)
            .collect();

        // Sort by ID descending (most recent first)
        entries.sort_by(|a, b| b.0.cmp(a.0));

        let mut result = Vec::new();
        for (id, _) in entries.into_iter().take(limit) {
            if let Some(entry) = self.get(*id)? {
                result.push(entry);
            }
        }

        Ok(result)
    }

    /// Delete a visual memory (soft delete)
    pub fn delete(&mut self, id: u64) -> Result<()> {
        if self.read_only {
            bail!("VisionEngram is read-only");
        }

        let Some(idx_entry) = self.index.get_mut(&id) else {
            return Ok(());
        };

        idx_entry.flags |= flags::DELETED as u32;
        self.persist_index()?;

        Ok(())
    }

    /// Save original image to images directory
    pub fn save_original(&self, id: u64, image_data: &[u8], format: &str) -> Result<String> {
        let filename = format!("{}.{}", id, format);
        let path = self.images_dir.join(&filename);

        fs::write(&path, image_data)
            .context("Failed to save original image")?;

        Ok(format!("images/{}", filename))
    }

    /// Get path to original image
    pub fn get_original_path(&self, relative_path: &str) -> PathBuf {
        self.path.parent()
            .unwrap_or(Path::new("."))
            .join(relative_path)
    }

    /// Get statistics
    pub fn stats(&self) -> VisionEngramStats {
        let active_count = self.index.iter()
            .filter(|(_, idx)| idx.flags & (flags::DELETED as u32) == 0)
            .count();

        VisionEngramStats {
            total_entries: self.entry_count,
            active_entries: active_count as u64,
            next_id: self.next_id,
        }
    }

    /// Persist index to file
    fn persist_index(&self) -> Result<()> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)?;

        // Read header to get index offset
        let mut header = [0u8; HEADER_SIZE];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header)?;
        let index_offset = u64::from_le_bytes(header[14..22].try_into()?);

        // Rewrite index
        file.seek(SeekFrom::Start(index_offset))?;
        for (_, idx) in &self.index {
            file.write_all(&idx.to_bytes())?;
        }

        file.sync_all()?;
        Ok(())
    }
}

/// Statistics about the VisionEngram store
#[derive(Debug, Clone)]
pub struct VisionEngramStats {
    pub total_entries: u64,
    pub active_entries: u64,
    pub next_id: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_create_and_store() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.veng");

        let mut store = VisionEngram::open(&path).unwrap();

        let entry = VisionEntry::new(
            0, // Will be assigned
            123, // note_id
            vec![1, 2, 3, 4], // thumbnail
            64, 64, // thumbnail dims
            1920, 1080, // original dims
        )
        .with_context("Test screenshot");

        let id = store.store(entry).unwrap();
        assert_eq!(id, 1);

        let retrieved = store.get(1).unwrap().unwrap();
        assert_eq!(retrieved.note_id, 123);
        assert_eq!(retrieved.context.as_deref(), Some("Test screenshot"));
    }

    #[test]
    fn test_entry_serialization() {
        let entry = VisionEntry::new(
            42,
            100,
            vec![0xFF; 100],
            256, 256,
            1920, 1080,
        )
        .with_original("images/42.webp")
        .with_context("A test image");

        let bytes = entry.to_bytes();
        let restored = VisionEntry::from_bytes(&bytes).unwrap();

        assert_eq!(restored.id, 42);
        assert_eq!(restored.note_id, 100);
        assert_eq!(restored.thumbnail_data.len(), 100);
        assert_eq!(restored.original_path.as_deref(), Some("images/42.webp"));
        assert_eq!(restored.context.as_deref(), Some("A test image"));
    }
}
