//! Federation Protocol for AI-Foundation
//!
//! Decentralized mesh networking for Teambooks where:
//! - Each Teambook is a node in the federation
//! - Data stays local, sharing is opt-in
//! - Multiple transport types: QUIC, mDNS, Bluetooth, Passkeys
//! - No single point of failure

pub mod node;
pub mod endpoint;
pub mod sharing;
pub mod connection;
pub mod messages;
pub mod cache;
pub mod discovery;
pub mod adapter;
pub mod identity;
pub mod hlc;
pub mod sync;
pub mod registry;
pub mod manifest;
pub mod consent;
pub mod pairing;
pub mod gateway;
pub mod inbox;
pub mod stun;
pub mod transport;
pub mod session;
pub mod replication;

// Re-exports
pub use node::{FederationNode, NodeCapabilities};
pub use endpoint::Endpoint;
pub use sharing::{SharingPreferences, ParticipationRequirements, DataCategory, DmPolicy, NegotiatedSharing};
pub use connection::{FederationConnection, ConnectionState};
pub use messages::{FederationMessage, FederationPayload, SignedEvent, content_hash};
pub use cache::{SharedCache, CacheEntry};
pub use identity::TeambookIdentity;
pub use hlc::{HybridClock, HlcTimestamp};
pub use sync::{
    EventPushRequest, EventPushResponse,
    EventPullRequest, EventPullResponse,
    SyncError, SyncRejectReason, PresencePushRequest,
};
pub use registry::{AiRegistry, FederatedAiEntry, AiResolution};
pub use manifest::{
    PermissionManifest, ExposureConfig, ChannelPermission,
    ConnectionMode, InboundActions, BroadcastVisibility, DialogueVisibility, ChannelAccess,
};
pub use consent::AiConsentRecord;
pub use pairing::{ConnectCodeState, ConnectInvite};
pub use gateway::{
    FederationGateway, PeerEntry, PeerRegistryConfig,
    OutboundEventType,
};
pub use inbox::{
    FederationInboxEvent, InboxWriter, InboxState,
    process_push_request, process_presence_request,
};
pub use transport::{
    QuicTransport, FEDERATION_ALPN, MAX_MESSAGE_SIZE,
    identity_to_iroh_key, send_message, recv_message, send_message_finish,
};
pub use session::{PeerSession, PROTOCOL_VERSION};
pub use replication::{ReplicationCursor, CursorStore, ReplicationOrchestrator, PeerSyncStatus};
pub use adapter::{
    ToDeepNetNodeId, FromDeepNetNodeId,
    DeepNetTransportType, DeepNetNodeAddress, DeepNetBandwidthTier,
    ToDeepNetAddress, TrustCapabilities,
    hex_to_bytes_32, bytes_32_to_hex,
    estimate_bandwidth, transport_priority, sort_endpoints_by_priority,
};

use ed25519_dalek::{SigningKey, VerifyingKey, Signature, Signer, Verifier};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Trust levels for federation nodes (aligned with AFP)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TrustLevel {
    /// Unknown node, heavily rate-limited
    Anonymous = 0,
    /// Hardware fingerprint verified
    Verified = 1,
    /// Vouched by trusted node or TPM-bound
    Trusted = 2,
    /// Owner of the local Teambook
    Owner = 3,
}

impl Default for TrustLevel {
    fn default() -> Self {
        TrustLevel::Anonymous
    }
}

/// Transport type for connections
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportType {
    /// QUIC over public internet
    QuicInternet,
    /// QUIC over LAN
    QuicLan,
    /// mDNS discovered + direct connection
    Mdns,
    /// Bluetooth Low Energy
    BluetoothLe,
    /// Classic Bluetooth
    BluetoothClassic,
    /// Passkey-initiated connection
    Passkey,
    /// Relayed through another node
    Relay,
}

/// Federation protocol errors
#[derive(Error, Debug)]
pub enum FederationError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("Trust level insufficient: required {required:?}, got {actual:?}")]
    InsufficientTrust {
        required: TrustLevel,
        actual: TrustLevel,
    },

    #[error("Sharing requirements not met: {0}")]
    SharingRequirementsNotMet(String),

    #[error("Node not found: {0}")]
    NodeNotFound(String),

    #[error("Transport error: {0}")]
    TransportError(String),

    #[error("Discovery error: {0}")]
    DiscoveryError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Signature verification failed")]
    SignatureVerificationFailed,

    #[error("Cache error: {0}")]
    CacheError(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, FederationError>;

/// Generate a node ID from a public key
pub fn node_id_from_pubkey(pubkey: &VerifyingKey) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(pubkey.as_bytes());
    let hash = hasher.finalize();
    hex::encode(&hash[..16]) // First 16 bytes = 32 hex chars
}

/// Sign data with a signing key
pub fn sign_data(key: &SigningKey, data: &[u8]) -> Signature {
    key.sign(data)
}

/// Verify a signature
pub fn verify_signature(pubkey: &VerifyingKey, data: &[u8], signature: &Signature) -> bool {
    pubkey.verify(data, signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    #[test]
    fn test_node_id_generation() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        let node_id = node_id_from_pubkey(&verifying_key);
        assert_eq!(node_id.len(), 32); // 16 bytes = 32 hex chars

        // Same key should produce same ID
        let node_id2 = node_id_from_pubkey(&verifying_key);
        assert_eq!(node_id, node_id2);
    }

    #[test]
    fn test_sign_and_verify() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        let data = b"Hello, Federation!";
        let signature = sign_data(&signing_key, data);

        assert!(verify_signature(&verifying_key, data, &signature));
        assert!(!verify_signature(&verifying_key, b"Wrong data", &signature));
    }

    #[test]
    fn test_trust_level_ordering() {
        assert!(TrustLevel::Owner > TrustLevel::Trusted);
        assert!(TrustLevel::Trusted > TrustLevel::Verified);
        assert!(TrustLevel::Verified > TrustLevel::Anonymous);
    }
}
