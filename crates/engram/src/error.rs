//! Error types for Engram

use thiserror::Error;

/// Result type alias for Engram operations
pub type Result<T> = std::result::Result<T, EngramError>;

/// Errors that can occur in Engram operations
#[derive(Error, Debug)]
pub enum EngramError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid magic bytes - not an Engram file")]
    InvalidMagic,

    #[error("Unsupported format version: {0}")]
    UnsupportedVersion(u32),

    #[error("Header checksum mismatch")]
    HeaderCorrupted,

    #[error("Note {0} not found")]
    NoteNotFound(u64),

    #[error("Note {0} is tombstoned")]
    NoteTombstoned(u64),

    #[error("Invalid note entry at offset {0}")]
    InvalidNoteEntry(u64),

    #[error("Decompression failed: {0}")]
    DecompressionFailed(String),

    #[error("Compression failed: {0}")]
    CompressionFailed(String),

    #[error("Vector dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: u32, got: u32 },

    #[error("Index out of bounds: {0}")]
    IndexOutOfBounds(u64),

    #[error("Vault key not found: {0}")]
    VaultKeyNotFound(String),

    #[error("Encryption error: {0}")]
    EncryptionError(String),

    #[error("Decryption error: {0}")]
    DecryptionError(String),

    #[error("Database is read-only")]
    ReadOnly,

    #[error("Database is locked by another process")]
    Locked,

    #[error("Memory map failed: {0}")]
    MmapFailed(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("File too large: {0} bytes exceeds maximum")]
    FileTooLarge(u64),

    #[error("Integrity check failed: {0}")]
    IntegrityError(String),

    #[error("Embedding error: {0}")]
    EmbeddingError(String),

    #[error("Pinned limit reached ({0}). Unpin a note first.")]
    PinnedLimitReached(usize),
}
