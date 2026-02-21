//! Engram file header - first 4KB of the file

use crate::{error::Result, EngramError, MAGIC, FORMAT_VERSION, PAGE_SIZE, DEFAULT_DIMENSIONS};
use std::io::{Read, Write, Seek, SeekFrom};

/// Header size is exactly one page (4KB)
pub const HEADER_SIZE: usize = PAGE_SIZE;

/// Fixed portion of the header (everything except reserved padding)
const HEADER_FIXED_SIZE: usize = 200;

/// Engram file header
#[derive(Debug, Clone)]
#[repr(C)]
pub struct EngramHeader {
    // Identification (16 bytes)
    pub magic: [u8; 8],
    pub version: u32,
    pub flags: u32,

    // Ownership (48 bytes)
    pub ai_id_hash: [u8; 32],
    pub created_at: i64,
    pub modified_at: i64,

    // Counts (24 bytes)
    pub note_count: u64,
    pub active_notes: u64,
    pub vector_dimensions: u32,
    pub _padding1: u32,

    // Section offsets (112 bytes = 14 * 8)
    pub note_log_offset: u64,
    pub note_log_size: u64,
    pub vector_store_offset: u64,
    pub vector_store_size: u64,
    pub hnsw_index_offset: u64,
    pub hnsw_index_size: u64,
    pub graph_index_offset: u64,
    pub graph_index_size: u64,
    pub temporal_index_offset: u64,
    pub temporal_index_size: u64,
    pub tag_index_offset: u64,
    pub tag_index_size: u64,
    pub vault_offset: u64,
    pub vault_size: u64,

    // Integrity (4 bytes)
    pub header_checksum: u32,
}

impl EngramHeader {
    /// Create a new header for a fresh database
    pub fn new(ai_id: &str) -> Self {
        use sha2::{Sha256, Digest};

        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let mut hasher = Sha256::new();
        hasher.update(ai_id.as_bytes());
        let ai_id_hash: [u8; 32] = hasher.finalize().into();

        let mut header = Self {
            magic: *MAGIC,
            version: FORMAT_VERSION,
            flags: 0,
            ai_id_hash,
            created_at: now,
            modified_at: now,
            note_count: 0,
            active_notes: 0,
            vector_dimensions: DEFAULT_DIMENSIONS,
            _padding1: 0,
            // Initially all sections start after header
            note_log_offset: PAGE_SIZE as u64,
            note_log_size: 0,
            vector_store_offset: PAGE_SIZE as u64,
            vector_store_size: 0,
            hnsw_index_offset: PAGE_SIZE as u64,
            hnsw_index_size: 0,
            graph_index_offset: PAGE_SIZE as u64,
            graph_index_size: 0,
            temporal_index_offset: PAGE_SIZE as u64,
            temporal_index_size: 0,
            tag_index_offset: PAGE_SIZE as u64,
            tag_index_size: 0,
            vault_offset: PAGE_SIZE as u64,
            vault_size: 0,
            header_checksum: 0,
        };

        header.header_checksum = header.compute_checksum();
        header
    }

    /// Compute CRC32 checksum of header (excluding checksum field)
    fn compute_checksum(&self) -> u32 {
        let bytes = self.to_bytes();
        // Checksum everything except the last 4 bytes (the checksum itself)
        // and the padding
        crc32fast::hash(&bytes[..HEADER_FIXED_SIZE - 4])
    }

    /// Verify header integrity
    pub fn verify(&self) -> Result<()> {
        // Check magic
        if self.magic != *MAGIC {
            return Err(EngramError::InvalidMagic);
        }

        // Check version
        if self.version > FORMAT_VERSION {
            return Err(EngramError::UnsupportedVersion(self.version));
        }

        // Check checksum
        let expected = self.compute_checksum();
        if self.header_checksum != expected {
            return Err(EngramError::HeaderCorrupted);
        }

        Ok(())
    }

    /// Serialize header to bytes
    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut bytes = [0u8; HEADER_SIZE];
        let mut cursor = std::io::Cursor::new(&mut bytes[..]);

        // Write fields in order
        cursor.write_all(&self.magic).unwrap();
        cursor.write_all(&self.version.to_le_bytes()).unwrap();
        cursor.write_all(&self.flags.to_le_bytes()).unwrap();
        cursor.write_all(&self.ai_id_hash).unwrap();
        cursor.write_all(&self.created_at.to_le_bytes()).unwrap();
        cursor.write_all(&self.modified_at.to_le_bytes()).unwrap();
        cursor.write_all(&self.note_count.to_le_bytes()).unwrap();
        cursor.write_all(&self.active_notes.to_le_bytes()).unwrap();
        cursor.write_all(&self.vector_dimensions.to_le_bytes()).unwrap();
        cursor.write_all(&self._padding1.to_le_bytes()).unwrap();
        cursor.write_all(&self.note_log_offset.to_le_bytes()).unwrap();
        cursor.write_all(&self.note_log_size.to_le_bytes()).unwrap();
        cursor.write_all(&self.vector_store_offset.to_le_bytes()).unwrap();
        cursor.write_all(&self.vector_store_size.to_le_bytes()).unwrap();
        cursor.write_all(&self.hnsw_index_offset.to_le_bytes()).unwrap();
        cursor.write_all(&self.hnsw_index_size.to_le_bytes()).unwrap();
        cursor.write_all(&self.graph_index_offset.to_le_bytes()).unwrap();
        cursor.write_all(&self.graph_index_size.to_le_bytes()).unwrap();
        cursor.write_all(&self.temporal_index_offset.to_le_bytes()).unwrap();
        cursor.write_all(&self.temporal_index_size.to_le_bytes()).unwrap();
        cursor.write_all(&self.tag_index_offset.to_le_bytes()).unwrap();
        cursor.write_all(&self.tag_index_size.to_le_bytes()).unwrap();
        cursor.write_all(&self.vault_offset.to_le_bytes()).unwrap();
        cursor.write_all(&self.vault_size.to_le_bytes()).unwrap();
        cursor.write_all(&self.header_checksum.to_le_bytes()).unwrap();

        bytes
    }

    /// Deserialize header from bytes
    pub fn from_bytes(bytes: &[u8; HEADER_SIZE]) -> Result<Self> {
        let mut cursor = std::io::Cursor::new(bytes);

        let mut magic = [0u8; 8];
        cursor.read_exact(&mut magic)?;

        let mut buf4 = [0u8; 4];
        let mut buf8 = [0u8; 8];
        let mut buf32 = [0u8; 32];

        cursor.read_exact(&mut buf4)?;
        let version = u32::from_le_bytes(buf4);

        cursor.read_exact(&mut buf4)?;
        let flags = u32::from_le_bytes(buf4);

        cursor.read_exact(&mut buf32)?;
        let ai_id_hash = buf32;

        cursor.read_exact(&mut buf8)?;
        let created_at = i64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let modified_at = i64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let note_count = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let active_notes = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf4)?;
        let vector_dimensions = u32::from_le_bytes(buf4);

        cursor.read_exact(&mut buf4)?;
        let _padding1 = u32::from_le_bytes(buf4);

        cursor.read_exact(&mut buf8)?;
        let note_log_offset = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let note_log_size = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let vector_store_offset = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let vector_store_size = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let hnsw_index_offset = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let hnsw_index_size = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let graph_index_offset = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let graph_index_size = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let temporal_index_offset = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let temporal_index_size = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let tag_index_offset = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let tag_index_size = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let vault_offset = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf8)?;
        let vault_size = u64::from_le_bytes(buf8);

        cursor.read_exact(&mut buf4)?;
        let header_checksum = u32::from_le_bytes(buf4);

        let header = Self {
            magic,
            version,
            flags,
            ai_id_hash,
            created_at,
            modified_at,
            note_count,
            active_notes,
            vector_dimensions,
            _padding1,
            note_log_offset,
            note_log_size,
            vector_store_offset,
            vector_store_size,
            hnsw_index_offset,
            hnsw_index_size,
            graph_index_offset,
            graph_index_size,
            temporal_index_offset,
            temporal_index_size,
            tag_index_offset,
            tag_index_size,
            vault_offset,
            vault_size,
            header_checksum,
        };

        header.verify()?;
        Ok(header)
    }

    /// Update modified timestamp and recompute checksum
    pub fn touch(&mut self) {
        self.modified_at = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        self.header_checksum = self.compute_checksum();
    }

    /// Read header from file
    pub fn read_from<R: Read + Seek>(reader: &mut R) -> Result<Self> {
        reader.seek(SeekFrom::Start(0))?;
        let mut bytes = [0u8; HEADER_SIZE];
        reader.read_exact(&mut bytes)?;
        Self::from_bytes(&bytes)
    }

    /// Write header to file
    pub fn write_to<W: Write + Seek>(&self, writer: &mut W) -> Result<()> {
        writer.seek(SeekFrom::Start(0))?;
        writer.write_all(&self.to_bytes())?;
        Ok(())
    }
}

/// Header flags
pub mod flags {
    pub const COMPRESSED: u32 = 1 << 0;       // Notes are zstd compressed
    pub const ENCRYPTED_VAULT: u32 = 1 << 1;  // Vault is encrypted
    pub const HAS_HNSW: u32 = 1 << 2;          // HNSW index present
    pub const HAS_GRAPH: u32 = 1 << 3;         // Graph index present
    pub const HAS_PERSISTED_INDEX: u32 = 1 << 4; // Index sections are persisted (O(1) startup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_roundtrip() {
        let header = EngramHeader::new("test-ai-123");
        let bytes = header.to_bytes();
        let restored = EngramHeader::from_bytes(&bytes).unwrap();

        assert_eq!(header.magic, restored.magic);
        assert_eq!(header.version, restored.version);
        assert_eq!(header.ai_id_hash, restored.ai_id_hash);
        assert_eq!(header.note_count, restored.note_count);
    }

    #[test]
    fn test_header_size() {
        assert_eq!(HEADER_SIZE, 4096);
    }

    #[test]
    fn test_invalid_magic() {
        let mut bytes = [0u8; HEADER_SIZE];
        bytes[0..8].copy_from_slice(b"NOTENGRA");

        let result = EngramHeader::from_bytes(&bytes);
        assert!(matches!(result, Err(EngramError::InvalidMagic)));
    }
}
