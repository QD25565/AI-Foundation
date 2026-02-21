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
}

impl Note {
    pub fn new(id: u64, content: String, tags: Vec<String>) -> Self {
        Self { id, timestamp: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            content, tags, pinned: false, pagerank: 0.0 }
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

#[derive(Debug, Clone)]
pub struct NoteEntry {
    pub id: u64, pub timestamp: i64, pub flags: u32, pub content_len: u32,
    pub tags_len: u16, pub _padding: [u8; 6], pub tags_data: Vec<u8>, pub content_data: Vec<u8>,
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
        Ok(Self { id, timestamp, flags, content_len: content_data.len() as u32, tags_len: tags_data.len() as u16, _padding: [0; 6], tags_data, content_data })
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
        Ok(Self { id, timestamp, flags: entry_flags, content_len: content_data.len() as u32, tags_len: tags_data.len() as u16, _padding: [0; 6], tags_data, content_data })
    }

    pub fn total_size(&self) -> usize { NOTE_HEADER_SIZE + self.tags_len as usize + self.content_len as usize }
    pub fn is_tombstone(&self) -> bool { self.flags & flags::TOMBSTONE != 0 }
    pub fn is_pinned(&self) -> bool { self.flags & flags::PINNED != 0 }
    pub fn is_encrypted(&self) -> bool { self.flags & flags::ENCRYPTED != 0 }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.total_size());
        bytes.extend_from_slice(&self.id.to_le_bytes()); bytes.extend_from_slice(&self.timestamp.to_le_bytes());
        bytes.extend_from_slice(&self.flags.to_le_bytes()); bytes.extend_from_slice(&self.content_len.to_le_bytes());
        bytes.extend_from_slice(&self.tags_len.to_le_bytes()); bytes.extend_from_slice(&self._padding);
        bytes.extend_from_slice(&self.tags_data); bytes.extend_from_slice(&self.content_data); bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < NOTE_HEADER_SIZE { return Err(EngramError::InvalidNoteEntry(0)); }
        let id = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let timestamp = i64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let flags = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let content_len = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        let tags_len = u16::from_le_bytes(bytes[24..26].try_into().unwrap());
        if bytes.len() < NOTE_HEADER_SIZE + tags_len as usize + content_len as usize { return Err(EngramError::InvalidNoteEntry(id)); }
        let tags_start = NOTE_HEADER_SIZE; let tags_end = tags_start + tags_len as usize;
        Ok(Self { id, timestamp, flags, content_len, tags_len, _padding: [0; 6],
            tags_data: bytes[tags_start..tags_end].to_vec(), content_data: bytes[tags_end..tags_end + content_len as usize].to_vec() })
    }

    pub fn to_note(&self) -> Result<Note> {
        if self.is_encrypted() { return Err(EngramError::DecryptionError("Note encrypted - use to_note_decrypted()".to_string())); }
        let content = if self.flags & flags::COMPRESSED != 0 {
            String::from_utf8_lossy(&zstd::decode_all(&self.content_data[..]).map_err(|e| EngramError::DecompressionFailed(e.to_string()))?).to_string()
        } else { String::from_utf8_lossy(&self.content_data).to_string() };
        let tags: Vec<String> = self.tags_data.split(|&b| b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).to_string()).collect();
        Ok(Note { id: self.id, timestamp: self.timestamp, content, tags, pinned: self.is_pinned(), pagerank: 0.0 })
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
        Ok(Note { id: self.id, timestamp: self.timestamp, content, tags, pinned: self.is_pinned(), pagerank: 0.0 })
    }

    pub fn tombstone(id: u64) -> Self {
        Self { id, timestamp: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0), flags: flags::TOMBSTONE,
            content_len: 0, tags_len: 0, _padding: [0; 6], tags_data: Vec::new(), content_data: Vec::new() }
    }
}
