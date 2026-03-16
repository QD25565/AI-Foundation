//! Federation Protocol for AI-Foundation Deep Net
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

// ---------------------------------------------------------------------------
// Post-Quantum Cryptography — Algorithm Agility Layer (Phase 1)
// ---------------------------------------------------------------------------
//
// These types make all federation signing/verification algorithm-agnostic.
// Phase 1: Ed25519 is the only active scheme. The agility layer is plumbing
// for Phase 2 (hybrid Ed25519 + ML-DSA-65) and Phase 3 (ML-DSA-65 only).
//
// See: docs/PQC-TRANSITION-ARCHITECTURE.md

/// Identifies which signature algorithm produced a signature.
///
/// Serialized as `u8` in wire format for compactness.
/// New schemes are added as federation adopts post-quantum cryptography.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SignatureScheme {
    /// Classical Ed25519 — 64-byte signatures, 32-byte public keys.
    Ed25519 = 0,

    /// Post-quantum ML-DSA-65 (FIPS 204) — 3,309-byte signatures, 1,952-byte public keys.
    /// Phase 2: not yet implemented.
    MlDsa65 = 1,

    /// Hybrid: Ed25519 + ML-DSA-65. Both must verify for the signature to be valid.
    /// Signature layout: `Ed25519(64 bytes) || ML-DSA-65(3,309 bytes)` = 3,373 bytes.
    /// Phase 2: not yet implemented.
    HybridEd25519MlDsa65 = 2,
}

impl Default for SignatureScheme {
    fn default() -> Self {
        SignatureScheme::Ed25519
    }
}

impl std::fmt::Display for SignatureScheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignatureScheme::Ed25519 => write!(f, "Ed25519"),
            SignatureScheme::MlDsa65 => write!(f, "ML-DSA-65"),
            SignatureScheme::HybridEd25519MlDsa65 => write!(f, "Ed25519+ML-DSA-65"),
        }
    }
}

/// Algorithm-agile signature container.
///
/// Replaces raw `ed25519_dalek::Signature` in all federation wire format structs.
/// Carries the scheme identifier alongside the raw signature bytes, so receivers
/// know which algorithm to use for verification without out-of-band negotiation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationSignature {
    /// Which algorithm(s) produced this signature.
    pub scheme: SignatureScheme,

    /// Raw signature bytes. Length depends on scheme:
    /// - `Ed25519`: 64 bytes
    /// - `MlDsa65`: 3,309 bytes
    /// - `HybridEd25519MlDsa65`: 3,373 bytes (Ed25519 first, then ML-DSA-65)
    #[serde(with = "hex_serde")]
    pub bytes: Vec<u8>,
}

impl FederationSignature {
    /// Wrap an Ed25519 signature.
    pub fn ed25519(sig: Signature) -> Self {
        Self {
            scheme: SignatureScheme::Ed25519,
            bytes: sig.to_bytes().to_vec(),
        }
    }

    /// Create a placeholder (all zeros). Used during message construction
    /// before the real signature is computed.
    pub fn placeholder() -> Self {
        Self {
            scheme: SignatureScheme::Ed25519,
            bytes: vec![0u8; 64],
        }
    }

    /// Extract the Ed25519 signature bytes, if this is an Ed25519 or hybrid signature.
    /// Returns `None` if the scheme doesn't contain an Ed25519 component.
    pub fn ed25519_bytes(&self) -> Option<[u8; 64]> {
        match self.scheme {
            SignatureScheme::Ed25519 if self.bytes.len() == 64 => {
                let mut arr = [0u8; 64];
                arr.copy_from_slice(&self.bytes);
                Some(arr)
            }
            SignatureScheme::HybridEd25519MlDsa65 if self.bytes.len() >= 64 => {
                let mut arr = [0u8; 64];
                arr.copy_from_slice(&self.bytes[..64]);
                Some(arr)
            }
            _ => None,
        }
    }

    /// Convert to a legacy `ed25519_dalek::Signature` for code that still
    /// needs the raw type (e.g., iroh transport interop).
    /// Returns `None` if this signature doesn't contain Ed25519 bytes.
    pub fn to_ed25519_signature(&self) -> Option<Signature> {
        self.ed25519_bytes().map(|b| Signature::from_bytes(&b))
    }
}

/// Errors from federation signature verification.
///
/// Returns specific reasons for failure — fail loud per QD directive.
/// Each variant tells the caller exactly what went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FederationVerifyError {
    /// The signature scheme is not supported by this build.
    UnsupportedScheme(SignatureScheme),

    /// Public key bytes could not be parsed for the expected algorithm.
    InvalidPublicKey,

    /// Signature bytes are the wrong length for the declared scheme.
    InvalidSignatureLength {
        scheme: SignatureScheme,
        expected: usize,
        actual: usize,
    },

    /// Ed25519 signature verification failed (wrong key or tampered data).
    Ed25519VerificationFailed,

    /// ML-DSA-65 signature verification failed.
    MlDsa65VerificationFailed,

    /// Hybrid verification: the Ed25519 component failed.
    HybridEd25519Failed,

    /// Hybrid verification: the ML-DSA-65 component failed.
    HybridMlDsa65Failed,
}

impl std::fmt::Display for FederationVerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedScheme(s) => write!(f, "unsupported signature scheme: {s}"),
            Self::InvalidPublicKey => write!(f, "malformed public key bytes"),
            Self::InvalidSignatureLength { scheme, expected, actual } => {
                write!(f, "{scheme} signature: expected {expected} bytes, got {actual}")
            }
            Self::Ed25519VerificationFailed => write!(f, "Ed25519 signature verification failed"),
            Self::MlDsa65VerificationFailed => write!(f, "ML-DSA-65 signature verification failed"),
            Self::HybridEd25519Failed => write!(f, "hybrid signature: Ed25519 component failed"),
            Self::HybridMlDsa65Failed => write!(f, "hybrid signature: ML-DSA-65 component failed"),
        }
    }
}

impl std::error::Error for FederationVerifyError {}

/// Verify a `FederationSignature` against Ed25519 public key bytes.
///
/// Dispatches to the correct algorithm based on scheme. Returns `Ok(())`
/// if verification succeeds, or a specific `FederationVerifyError` on failure.
///
/// In Phase 1, only `Ed25519` is implemented. `MlDsa65` and `Hybrid` variants
/// return `UnsupportedScheme` until Phase 2 adds the ML-DSA dependency.
pub fn verify_federation_signature(
    signature: &FederationSignature,
    ed25519_pubkey_bytes: &[u8],
    data: &[u8],
) -> std::result::Result<(), FederationVerifyError> {
    match signature.scheme {
        SignatureScheme::Ed25519 => {
            // Validate signature length
            if signature.bytes.len() != 64 {
                return Err(FederationVerifyError::InvalidSignatureLength {
                    scheme: SignatureScheme::Ed25519,
                    expected: 64,
                    actual: signature.bytes.len(),
                });
            }

            // Parse public key
            let pubkey_arr: [u8; 32] = ed25519_pubkey_bytes
                .try_into()
                .map_err(|_| FederationVerifyError::InvalidPublicKey)?;
            let verifying_key = VerifyingKey::from_bytes(&pubkey_arr)
                .map_err(|_| FederationVerifyError::InvalidPublicKey)?;

            // Parse and verify signature
            let mut sig_arr = [0u8; 64];
            sig_arr.copy_from_slice(&signature.bytes);
            let sig = Signature::from_bytes(&sig_arr);
            verifying_key
                .verify(data, &sig)
                .map_err(|_| FederationVerifyError::Ed25519VerificationFailed)
        }

        SignatureScheme::MlDsa65 => {
            // Phase 2: ML-DSA-65 verification
            Err(FederationVerifyError::UnsupportedScheme(SignatureScheme::MlDsa65))
        }

        SignatureScheme::HybridEd25519MlDsa65 => {
            // Phase 2: Both must verify — fail if either fails
            Err(FederationVerifyError::UnsupportedScheme(SignatureScheme::HybridEd25519MlDsa65))
        }
    }
}

/// Trait abstracting over signature algorithms for federation identity.
///
/// `TeambookIdentity` implements this with Ed25519 today.
/// Phase 2 adds `HybridIdentity` implementing both Ed25519 + ML-DSA-65.
pub trait FederationSigner: Send + Sync {
    /// Sign arbitrary bytes, returning an algorithm-agile signature.
    fn sign_federation(&self, data: &[u8]) -> FederationSignature;

    /// The signature scheme this signer produces.
    fn scheme(&self) -> SignatureScheme;

    /// Ed25519 public key bytes (32 bytes) — used for identity derivation
    /// (node_id, H_ID) which is always Ed25519-based regardless of signing scheme.
    fn ed25519_pubkey_bytes(&self) -> [u8; 32];
}

/// Hex serde helper for signature bytes.
mod hex_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        hex::encode(bytes).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        hex::decode(&s).map_err(serde::de::Error::custom)
    }
}

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

    // -----------------------------------------------------------------------
    // PQC Phase 1: Algorithm Agility Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_signature_scheme_default_is_ed25519() {
        assert_eq!(SignatureScheme::default(), SignatureScheme::Ed25519);
    }

    #[test]
    fn test_signature_scheme_display() {
        assert_eq!(format!("{}", SignatureScheme::Ed25519), "Ed25519");
        assert_eq!(format!("{}", SignatureScheme::MlDsa65), "ML-DSA-65");
        assert_eq!(format!("{}", SignatureScheme::HybridEd25519MlDsa65), "Ed25519+ML-DSA-65");
    }

    #[test]
    fn test_signature_scheme_repr_u8_values() {
        assert_eq!(SignatureScheme::Ed25519 as u8, 0);
        assert_eq!(SignatureScheme::MlDsa65 as u8, 1);
        assert_eq!(SignatureScheme::HybridEd25519MlDsa65 as u8, 2);
    }

    #[test]
    fn test_signature_scheme_serde_roundtrip() {
        let schemes = [
            SignatureScheme::Ed25519,
            SignatureScheme::MlDsa65,
            SignatureScheme::HybridEd25519MlDsa65,
        ];
        for scheme in &schemes {
            let json = serde_json::to_string(scheme).unwrap();
            let back: SignatureScheme = serde_json::from_str(&json).unwrap();
            assert_eq!(*scheme, back, "round-trip failed for {scheme}");
        }
    }

    #[test]
    fn test_federation_signature_ed25519_construction() {
        let sk = SigningKey::generate(&mut OsRng);
        let data = b"test message";
        let raw_sig = sk.sign(data);

        let fed_sig = FederationSignature::ed25519(raw_sig);
        assert_eq!(fed_sig.scheme, SignatureScheme::Ed25519);
        assert_eq!(fed_sig.bytes.len(), 64);
        assert_eq!(fed_sig.bytes, raw_sig.to_bytes().to_vec());
    }

    #[test]
    fn test_federation_signature_placeholder() {
        let placeholder = FederationSignature::placeholder();
        assert_eq!(placeholder.scheme, SignatureScheme::Ed25519);
        assert_eq!(placeholder.bytes.len(), 64);
        assert!(placeholder.bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_federation_signature_ed25519_bytes_extraction() {
        let sk = SigningKey::generate(&mut OsRng);
        let raw_sig = sk.sign(b"extract test");

        let fed_sig = FederationSignature::ed25519(raw_sig);
        let extracted = fed_sig.ed25519_bytes().expect("should extract Ed25519 bytes");
        assert_eq!(extracted, raw_sig.to_bytes());
    }

    #[test]
    fn test_federation_signature_to_ed25519_signature() {
        let sk = SigningKey::generate(&mut OsRng);
        let raw_sig = sk.sign(b"roundtrip test");

        let fed_sig = FederationSignature::ed25519(raw_sig);
        let recovered = fed_sig.to_ed25519_signature().expect("should recover Ed25519 Signature");
        assert_eq!(recovered.to_bytes(), raw_sig.to_bytes());
    }

    #[test]
    fn test_federation_signature_serde_roundtrip() {
        let sk = SigningKey::generate(&mut OsRng);
        let raw_sig = sk.sign(b"serde roundtrip");

        let fed_sig = FederationSignature::ed25519(raw_sig);
        let json = serde_json::to_string(&fed_sig).unwrap();
        let back: FederationSignature = serde_json::from_str(&json).unwrap();

        assert_eq!(back.scheme, SignatureScheme::Ed25519);
        assert_eq!(back.bytes, fed_sig.bytes);
    }

    #[test]
    fn test_verify_federation_signature_valid() {
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();
        let data = b"valid signature test";
        let raw_sig = sk.sign(data);

        let fed_sig = FederationSignature::ed25519(raw_sig);
        let result = verify_federation_signature(&fed_sig, vk.as_bytes(), data);
        assert!(result.is_ok(), "valid signature should verify: {result:?}");
    }

    #[test]
    fn test_verify_federation_signature_wrong_key() {
        let sk = SigningKey::generate(&mut OsRng);
        let wrong_sk = SigningKey::generate(&mut OsRng);
        let wrong_vk = wrong_sk.verifying_key();
        let data = b"wrong key test";
        let raw_sig = sk.sign(data);

        let fed_sig = FederationSignature::ed25519(raw_sig);
        let result = verify_federation_signature(&fed_sig, wrong_vk.as_bytes(), data);
        assert_eq!(result, Err(FederationVerifyError::Ed25519VerificationFailed));
    }

    #[test]
    fn test_verify_federation_signature_tampered_data() {
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();
        let data = b"original data";
        let raw_sig = sk.sign(data);

        let fed_sig = FederationSignature::ed25519(raw_sig);
        let result = verify_federation_signature(&fed_sig, vk.as_bytes(), b"tampered data");
        assert_eq!(result, Err(FederationVerifyError::Ed25519VerificationFailed));
    }

    #[test]
    fn test_verify_federation_signature_invalid_sig_length() {
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();

        let bad_sig = FederationSignature {
            scheme: SignatureScheme::Ed25519,
            bytes: vec![0u8; 32], // Wrong length — should be 64
        };
        let result = verify_federation_signature(&bad_sig, vk.as_bytes(), b"test");
        assert_eq!(result, Err(FederationVerifyError::InvalidSignatureLength {
            scheme: SignatureScheme::Ed25519,
            expected: 64,
            actual: 32,
        }));
    }

    #[test]
    fn test_verify_federation_signature_invalid_pubkey() {
        let sk = SigningKey::generate(&mut OsRng);
        let raw_sig = sk.sign(b"test");
        let fed_sig = FederationSignature::ed25519(raw_sig);

        // Wrong-length pubkey
        let result = verify_federation_signature(&fed_sig, &[0u8; 16], b"test");
        assert_eq!(result, Err(FederationVerifyError::InvalidPublicKey));
    }

    #[test]
    fn test_verify_federation_signature_unsupported_ml_dsa() {
        let fake_sig = FederationSignature {
            scheme: SignatureScheme::MlDsa65,
            bytes: vec![0u8; 3309],
        };
        let result = verify_federation_signature(&fake_sig, &[0u8; 32], b"test");
        assert_eq!(result, Err(FederationVerifyError::UnsupportedScheme(SignatureScheme::MlDsa65)));
    }

    #[test]
    fn test_verify_federation_signature_unsupported_hybrid() {
        let fake_sig = FederationSignature {
            scheme: SignatureScheme::HybridEd25519MlDsa65,
            bytes: vec![0u8; 3373],
        };
        let result = verify_federation_signature(&fake_sig, &[0u8; 32], b"test");
        assert_eq!(result, Err(FederationVerifyError::UnsupportedScheme(
            SignatureScheme::HybridEd25519MlDsa65
        )));
    }

    #[test]
    fn test_federation_verify_error_display() {
        let err = FederationVerifyError::InvalidSignatureLength {
            scheme: SignatureScheme::Ed25519,
            expected: 64,
            actual: 32,
        };
        let msg = format!("{err}");
        assert!(msg.contains("64"), "should mention expected length");
        assert!(msg.contains("32"), "should mention actual length");
    }

    #[test]
    fn test_legacy_sign_verify_still_works() {
        // Ensure old sign_data/verify_signature helpers still function
        // alongside the new FederationSignature path
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();
        let data = b"legacy compat test";

        // Legacy path
        let legacy_sig = sign_data(&sk, data);
        assert!(verify_signature(&vk, data, &legacy_sig));

        // New path — same key, same data, same result
        let fed_sig = FederationSignature::ed25519(legacy_sig);
        let result = verify_federation_signature(&fed_sig, vk.as_bytes(), data);
        assert!(result.is_ok());
    }
}
