//! Note structure and serialization

use crate::{crypto::EngramCipher, error::Result, EngramError};
use serde::{Deserialize, Serialize};

pub const NOTE_HEADER_SIZE: usize = 32;

pub mod flags {
    pub const TOMBSTONE: u32 = 1 << 0;
    pub const PINNED: u32 = 1 << 1;
    pub const HAS_VECTOR: u32 = 1 << 2;
    pub const COMPRESSED: u32 = 1 << 3;
    pub const ENCRYPTED: u32 = 1 << 4;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub timestamp: i64,
    pub content: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub pagerank: f32,
    /// Seconds since Unix epoch when this note was last edited (0 = never updated, use timestamp)
    pub updated_at: u32,
    /// If > 0, note expires this many hours after creation (working/ephemeral memory)
    pub ttl_hours: u16,
}

impl Note {
    pub fn new(id: u64, content: String, tags: Vec<String>) -> Self {
        Self { id, timestamp: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            content, tags, pinned: false, pagerank: 0.0, updated_at: 0, ttl_hours: 0 }
    }

    /// Returns the most recent timestamp in nanoseconds.
    /// Uses updated_at (converted from seconds) if set, otherwise falls back to timestamp.
    pub fn effective_timestamp_nanos(&self) -> i64 {
        if self.updated_at > 0 {
            (self.updated_at as i64) * 1_000_000_000
        } else {
            self.timestamp
        }
    }

    /// Returns true if this note has expired based on its TTL.
    pub fn is_expired(&self) -> bool {
        if self.ttl_hours == 0 {
            return false;
        }
        let now_secs = chrono::Utc::now().timestamp();
        let created_secs = self.timestamp / 1_000_000_000;
        now_secs > created_secs + (self.ttl_hours as i64 * 3600)
    }

    pub fn age_string(&self) -> String {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let diff_secs = (now - self.timestamp) / 1_000_000_000;
        if diff_secs < 0 { "just now".to_string() }
        else if diff_secs < 60 { format!("{}sec ago", diff_secs) }
        else if diff_secs < 3600 { format!("{}min ago", diff_secs / 60) }
        else if diff_secs < 86400 { format!("{}hrs ago", diff_secs / 3600) }
        else if diff_secs < 2592000 { format!("{}days ago", diff_secs / 86400) }
        else if diff_secs < 31536000 { format!("{}months ago", diff_secs / 2592000) }
        else { format!("{}years ago", diff_secs / 31536000) }
    }
}

/// Binary layout (32 bytes fixed header, backward compatible):
/// [0..8]   id: u64
/// [8..16]  timestamp: i64  (nanoseconds since epoch, creation time)
/// [16..20] flags: u32
/// [20..24] content_len: u32
/// [24..26] tags_len: u16
/// [26..30] updated_at: u32 (seconds since epoch, 0 = never updated; was _padding[0..4])
/// [30..32] ttl_hours: u16  (0 = no expiry; was _padding[4..6])
#[derive(Debug, Clone)]
pub struct NoteEntry {
    pub id: u64, pub timestamp: i64, pub flags: u32, pub content_len: u32,
    pub tags_len: u16,
    /// Seconds since epoch when last updated (0 = use timestamp). Stored in bytes[26..30].
    pub updated_at: u32,
    /// Hours until expiry from creation (0 = permanent). Stored in bytes[30..32].
    pub ttl_hours: u16,
    pub tags_data: Vec<u8>, pub content_data: Vec<u8>,
}

impl NoteEntry {
    pub fn new(id: u64, content: &str, tags: &[&str], compress: bool) -> Result<Self> {
        let timestamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let tags_data: Vec<u8> = tags.iter().flat_map(|t| t.as_bytes().iter().chain(std::iter::once(&0u8))).copied().collect();
        let (content_data, flags) = if compress && content.len() > 100 {
            let compressed = zstd::encode_all(content.as_bytes(), 3).map_err(|e| EngramError::CompressionFailed(e.to_string()))?;
            if compressed.len() < content.len() { (compressed, flags::COMPRESSED | flags::HAS_VECTOR) }
            else { (content.as_bytes().to_vec(), flags::HAS_VECTOR) }
        } else { (content.as_bytes().to_vec(), flags::HAS_VECTOR) };
        Ok(Self { id, timestamp, flags, content_len: content_data.len() as u32,
            tags_len: tags_data.len() as u16, updated_at: 0, ttl_hours: 0, tags_data, content_data })
    }

    pub fn new_encrypted(id: u64, content: &str, tags: &[&str], compress: bool, cipher: &EngramCipher) -> Result<Self> {
        let timestamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let tags_plain: Vec<u8> = tags.iter().flat_map(|t| t.as_bytes().iter().chain(std::iter::once(&0u8))).copied().collect();
        let (content_to_encrypt, mut entry_flags) = if compress && content.len() > 100 {
            let compressed = zstd::encode_all(content.as_bytes(), 3).map_err(|e| EngramError::CompressionFailed(e.to_string()))?;
            if compressed.len() < content.len() { (compressed, flags::COMPRESSED | flags::HAS_VECTOR) }
            else { (content.as_bytes().to_vec(), flags::HAS_VECTOR) }
        } else { (content.as_bytes().to_vec(), flags::HAS_VECTOR) };
        let content_data = cipher.encrypt(&content_to_encrypt)?;
        let tags_data = if tags_plain.is_empty() { Vec::new() } else { cipher.encrypt(&tags_plain)? };
        entry_flags |= flags::ENCRYPTED;
        Ok(Self { id, timestamp, flags: entry_flags, content_len: content_data.len() as u32,
            tags_len: tags_data.len() as u16, updated_at: 0, ttl_hours: 0, tags_data, content_data })
    }

    pub fn total_size(&self) -> usize { NOTE_HEADER_SIZE + self.tags_len as usize + self.content_len as usize }
    pub fn is_tombstone(&self) -> bool { self.flags & flags::TOMBSTONE != 0 }
    pub fn is_pinned(&self) -> bool { self.flags & flags::PINNED != 0 }
    pub fn is_encrypted(&self) -> bool { self.flags & flags::ENCRYPTED != 0 }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.total_size());
        bytes.extend_from_slice(&self.id.to_le_bytes());
        bytes.extend_from_slice(&self.timestamp.to_le_bytes());
        bytes.extend_from_slice(&self.flags.to_le_bytes());
        bytes.extend_from_slice(&self.content_len.to_le_bytes());
        bytes.extend_from_slice(&self.tags_len.to_le_bytes());
        bytes.extend_from_slice(&self.updated_at.to_le_bytes()); // bytes[26..30]
        bytes.extend_from_slice(&self.ttl_hours.to_le_bytes());  // bytes[30..32]
        bytes.extend_from_slice(&self.tags_data);
        bytes.extend_from_slice(&self.content_data);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < NOTE_HEADER_SIZE { return Err(EngramError::InvalidNoteEntry(0)); }
        let id = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let timestamp = i64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let flags = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let content_len = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        let tags_len = u16::from_le_bytes(bytes[24..26].try_into().unwrap());
        // bytes[26..30] = updated_at (was _padding[0..4]; old files read 0 → "never updated")
        let updated_at = u32::from_le_bytes(bytes[26..30].try_into().unwrap());
        // bytes[30..32] = ttl_hours (was _padding[4..6]; old files read 0 → "no expiry")
        let ttl_hours = u16::from_le_bytes(bytes[30..32].try_into().unwrap());
        if bytes.len() < NOTE_HEADER_SIZE + tags_len as usize + content_len as usize {
            return Err(EngramError::InvalidNoteEntry(id));
        }
        let tags_start = NOTE_HEADER_SIZE;
        let tags_end = tags_start + tags_len as usize;
        Ok(Self { id, timestamp, flags, content_len, tags_len, updated_at, ttl_hours,
            tags_data: bytes[tags_start..tags_end].to_vec(),
            content_data: bytes[tags_end..tags_end + content_len as usize].to_vec() })
    }

    pub fn to_note(&self) -> Result<Note> {
        if self.is_encrypted() { return Err(EngramError::DecryptionError("Note encrypted - use to_note_decrypted()".to_string())); }
        let content = if self.flags & flags::COMPRESSED != 0 {
            String::from_utf8_lossy(&zstd::decode_all(&self.content_data[..]).map_err(|e| EngramError::DecompressionFailed(e.to_string()))?).to_string()
        } else { String::from_utf8_lossy(&self.content_data).to_string() };
        let tags: Vec<String> = self.tags_data.split(|&b| b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).to_string()).collect();
        Ok(Note { id: self.id, timestamp: self.timestamp, content, tags, pinned: self.is_pinned(),
            pagerank: 0.0, updated_at: self.updated_at, ttl_hours: self.ttl_hours })
    }

    pub fn to_note_decrypted(&self, cipher: &EngramCipher) -> Result<Note> {
        if !self.is_encrypted() { return self.to_note(); }
        let decrypted = cipher.decrypt(&self.content_data)?;
        let content = if self.flags & flags::COMPRESSED != 0 {
            String::from_utf8_lossy(&zstd::decode_all(&decrypted[..]).map_err(|e| EngramError::DecompressionFailed(e.to_string()))?).to_string()
        } else { String::from_utf8_lossy(&decrypted).to_string() };
        let tags: Vec<String> = if self.tags_data.is_empty() { Vec::new() } else {
            cipher.decrypt(&self.tags_data)?.split(|&b| b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).to_string()).collect()
        };
        Ok(Note { id: self.id, timestamp: self.timestamp, content, tags, pinned: self.is_pinned(),
            pagerank: 0.0, updated_at: self.updated_at, ttl_hours: self.ttl_hours })
    }

    pub fn tombstone(id: u64) -> Self {
        Self { id, timestamp: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            flags: flags::TOMBSTONE, content_len: 0, tags_len: 0,
            updated_at: 0, ttl_hours: 0, tags_data: Vec::new(), content_data: Vec::new() }
    }
}
