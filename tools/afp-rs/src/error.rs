//! Error types for AFP

use thiserror::Error;

/// AFP Error types
#[derive(Error, Debug)]
pub enum AFPError {
    // Identity errors
    #[error("Invalid AI ID format: {0}")]
    InvalidAIID(String),

    #[error("Invalid public key: {0}")]
    InvalidPublicKey(String),

    #[error("Signature verification failed")]
    SignatureVerificationFailed,

    #[error("Identity not found: {0}")]
    IdentityNotFound(String),

    // Key storage errors
    #[error("Key storage not available: {0}")]
    KeyStorageUnavailable(String),

    #[error("Key not found")]
    KeyNotFound,

    #[error("Key generation failed: {0}")]
    KeyGenerationFailed(String),

    #[error("Signing failed: {0}")]
    SigningFailed(String),

    // Transport errors
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Send failed: {0}")]
    SendFailed(String),

    #[error("Receive failed: {0}")]
    ReceiveFailed(String),

    #[error("Handshake failed: {0}")]
    HandshakeFailed(String),

    #[error("TLS error: {0}")]
    TlsError(String),

    // Message errors
    #[error("Serialization failed: {0}")]
    SerializationFailed(String),

    #[error("Deserialization failed: {0}")]
    DeserializationFailed(String),

    #[error("Message too large: {size} > {max}")]
    MessageTooLarge { size: usize, max: usize },

    #[error("Invalid message version: {0}")]
    InvalidMessageVersion(u8),

    #[error("Invalid message type")]
    InvalidMessageType,

    // Authentication errors
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("Insufficient trust level: required {required:?}, have {have:?}")]
    InsufficientTrustLevel {
        required: crate::TrustLevel,
        have: crate::TrustLevel,
    },

    #[error("Banned: {reason}")]
    Banned { reason: String },

    // Hardware fingerprint errors
    #[error("Fingerprint generation failed: {0}")]
    FingerprintFailed(String),

    #[error("Fingerprint mismatch")]
    FingerprintMismatch,

    // Server errors
    #[error("Server not running")]
    ServerNotRunning,

    #[error("Server already running")]
    ServerAlreadyRunning,

    #[error("Bind failed: {0}")]
    BindFailed(String),

    // Generic errors
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Result type for AFP operations
pub type Result<T> = std::result::Result<T, AFPError>;
