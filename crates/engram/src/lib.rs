//! # Engram
//!
//! Purpose-built AI memory database. The physical substrate for AI memory.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use engram::Engram;
//!
//! fn main() -> engram::Result<()> {
//!     let mut db = Engram::open("memory.engram")?;
//!
//!     // Store a memory
//!     let id = db.remember("PostgreSQL connection pooling fixed the timeout", &["postgres", "fix"])?;
//!
//!     // Get recent memories
//!     let notes = db.list(10)?;
//!
//!     // Get by tag
//!     let postgres_notes = db.by_tag("postgres")?;
//!
//!     Ok(())
//! }
//! ```

pub mod bloom;
pub mod cognitive;
pub mod crypto;
pub mod embedding;
pub mod error;
pub mod header;
pub mod note;
pub mod storage;
pub mod vector;
pub mod graph;
pub mod temporal;
pub mod tags;
pub mod vault;
pub mod hnsw;
pub mod recall;

pub use error::{EngramError, Result};
pub use note::Note;
pub use storage::Engram;

/// Engram file format version
pub const FORMAT_VERSION: u32 = 1;

/// Magic bytes identifying an Engram file
pub const MAGIC: &[u8; 8] = b"ENGRAM01";

/// Default vector dimensions (EmbeddingGemma)
pub const DEFAULT_DIMENSIONS: u32 = 512;

/// Page size for alignment
pub const PAGE_SIZE: usize = 4096;

/// Statistics about an Engram database
#[derive(Debug, Clone)]
pub struct EngramStats {
    pub note_count: u64,
    pub active_notes: u64,
    pub tombstone_count: u64,
    pub pinned_count: u64,
    pub vector_count: u64,
    pub edge_count: u64,
    pub tag_count: u64,
    pub vault_entries: u64,
    pub file_size: u64,
    pub created_at: i64,
    pub modified_at: i64,
}

/// Result of a compaction operation
#[derive(Debug, Clone)]
pub struct CompactStats {
    pub notes_removed: u64,
    pub bytes_reclaimed: u64,
    pub duration_ms: u64,
}

/// Result of integrity verification
#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_magic_bytes() {
        assert_eq!(MAGIC.len(), 8);
        assert_eq!(&MAGIC[..], b"ENGRAM01");
    }
}
