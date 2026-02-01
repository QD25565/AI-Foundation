//! AI-Foundation Protocol (AFP)
//!
//! The backbone of global AI interconnect. Enables AIs anywhere in the world
//! to connect to teambooks, coordinate, and collaborate while maintaining
//! their personal memory (Notebook).
//!
//! # Architecture
//!
//! ```text
//! +-------------------------------------------------------------+
//! |                    APPLICATION LAYER                        |
//! |  Notebook (memory) | Teambook (coordination) | Vision | ... |
//! +-------------------------------------------------------------+
//! |                    PROTOCOL LAYER (AFP)                     |
//! |  Messages | Channels | Identity | Encryption | Routing      |
//! +-------------------------------------------------------------+
//! |                    SERIALIZATION                            |
//! |  CBOR (RFC 8949) - binary, self-describing, standardized    |
//! +-------------------------------------------------------------+
//! |                    TRANSPORT                                |
//! |  QUIC (primary) | WebSocket (fallback) | Unix socket (local)|
//! +-------------------------------------------------------------+
//! ```
//!
//! # Security Model
//!
//! - **TPM-bound keys**: Private keys never leave hardware
//! - **Hardware fingerprinting**: Prevents ban evasion
//! - **Tiered trust**: Anonymous → Verified → Trusted → Owner
//! - **Ed25519 signatures**: All messages cryptographically signed

pub mod identity;
pub mod fingerprint;
pub mod keys;
pub mod message;
pub mod transport;
pub mod server;
pub mod client;
pub mod error;

pub use identity::{AIIdentity, TrustLevel};
pub use fingerprint::HardwareFingerprint;
pub use keys::{KeyStorage, KeyPair};
pub use message::{AFPMessage, MessageType, Payload};
pub use error::{AFPError, Result};

/// Protocol version
pub const AFP_VERSION: u8 = 1;

/// Default QUIC port
pub const DEFAULT_QUIC_PORT: u16 = 31415;

/// Default WebSocket port
pub const DEFAULT_WS_PORT: u16 = 31416;

/// Maximum message size (1MB)
pub const MAX_MESSAGE_SIZE: usize = 1024 * 1024;
