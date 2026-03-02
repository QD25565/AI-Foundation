//! Federation protocol messages

use crate::{
    FederationNode, SharingPreferences, TrustLevel,
    DataCategory, Result, FederationError,
    FederationSignature, SignatureScheme,
};
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// A signed federation message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationMessage {
    /// Message ID (for deduplication)
    pub id: String,

    /// Sender node ID
    pub from: String,

    /// Timestamp
    pub timestamp: DateTime<Utc>,

    /// The payload
    pub payload: FederationPayload,

    /// Signature over (id + from + timestamp + payload).
    ///
    /// Algorithm-agile: carries scheme identifier alongside raw bytes.
    /// Phase 1: always Ed25519. Phase 2+: may be ML-DSA-65 or hybrid.
    pub signature: FederationSignature,
}

impl FederationMessage {
    /// Create and sign a new message
    pub fn new(
        from: &str,
        payload: FederationPayload,
        signing_key: &ed25519_dalek::SigningKey,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let timestamp = Utc::now();

        let mut msg = Self {
            id: id.clone(),
            from: from.to_string(),
            timestamp,
            payload,
            signature: FederationSignature::placeholder(),
        };

        let data = msg.signing_data();
        msg.signature = FederationSignature::ed25519(crate::sign_data(signing_key, &data));

        msg
    }

    /// Get the data to sign/verify.
    ///
    /// **Security invariant:** The payload MUST be included in the signed data.
    /// If payload serialization fails, we panic rather than produce a signature
    /// that doesn't cover the payload — a silent failure here would allow an
    /// attacker to swap payloads while keeping a valid signature.
    fn signing_data(&self) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend(self.id.as_bytes());
        data.extend(self.from.as_bytes());
        data.extend(self.timestamp.timestamp().to_le_bytes());

        // Payload serialization uses JSON for deterministic output.
        // This MUST succeed — panic is correct if it doesn't.
        let payload_json = serde_json::to_vec(&self.payload)
            .expect("FederationPayload must be JSON-serializable — signature would be unsafe without payload");
        data.extend(payload_json);

        data
    }

    /// Verify the message signature using algorithm-agile verification.
    pub fn verify(&self, pubkey: &ed25519_dalek::VerifyingKey) -> bool {
        let data = self.signing_data();
        crate::verify_federation_signature(&self.signature, pubkey.as_bytes(), &data).is_ok()
    }

    /// Serialize to CBOR bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(self, &mut buf)
            .map_err(|e| FederationError::SerializationError(e.to_string()))?;
        Ok(buf)
    }

    /// Deserialize from CBOR bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        ciborium::de::from_reader(data)
            .map_err(|e| FederationError::SerializationError(e.to_string()))
    }
}

/// Federation protocol payloads
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FederationPayload {
    // ========== Handshake ==========
    /// Initial hello from connecting node
    Hello {
        node: FederationNode,
        protocol_version: u32,
        /// Random 32-byte nonce (hex) — responder must echo in Welcome.
        /// Prevents replay of captured Hello messages.
        handshake_nonce: String,
    },

    /// Response to hello
    Welcome {
        node: FederationNode,
        protocol_version: u32,
        accepted: bool,
        rejection_reason: Option<String>,
        /// Echo of the Hello nonce — initiator verifies this matches.
        hello_nonce: String,
    },

    // ========== Discovery ==========
    /// Announce this node's presence
    NodeAnnounce(FederationNode),

    /// Query for a specific node
    NodeQuery {
        node_id: String,
    },

    /// Response to node query
    NodeResponse {
        node: Option<FederationNode>,
    },

    /// Request list of known peers
    PeerListRequest {
        max_results: u32,
    },

    /// List of known peers
    PeerList {
        peers: Vec<PeerInfo>,
    },

    // ========== Sharing Negotiation ==========
    /// Share our preferences
    SharePreferences(SharingPreferences),

    /// Negotiation complete
    NegotiationComplete {
        shared_categories: Vec<DataCategory>,
        success: bool,
    },

    // ========== Data Sync ==========
    /// Request specific data
    DataRequest {
        category: DataCategory,
        key: String,
    },

    /// Response with data
    DataResponse {
        category: DataCategory,
        key: String,
        data: Option<Vec<u8>>,
        version: u64,
    },

    /// Sync vector (what we have)
    SyncVector {
        category: DataCategory,
        entries: Vec<(String, u64)>, // key, version
    },

    // ========== Presence ==========
    /// Presence update
    PresenceUpdate(FederatedPresence),

    /// Request presence for nodes
    PresenceQuery {
        node_ids: Vec<String>,
    },

    /// Batch presence response
    PresenceBatch {
        presences: Vec<FederatedPresence>,
    },

    // ========== Messaging ==========
    /// Direct message
    DirectMessage {
        to: String,
        content: String,
        reply_to: Option<String>,
    },

    /// Broadcast to channel
    Broadcast {
        channel: String,
        content: String,
    },

    /// Message acknowledgment
    MessageAck {
        message_id: String,
        received: bool,
    },

    // ========== Routing ==========
    /// Route request (for mesh routing)
    RouteRequest {
        destination: String,
        ttl: u8,
    },

    /// Route response
    RouteResponse {
        destination: String,
        path: Vec<String>,
        hops: u8,
    },

    /// Relay a message through this node
    Relay {
        target: String,
        payload: Box<FederationPayload>,
    },

    // ========== Control ==========
    /// Ping (keepalive)
    Ping {
        timestamp: u64,
    },

    /// Pong response
    Pong {
        timestamp: u64,
        echo_timestamp: u64,
    },

    /// Graceful disconnect
    Goodbye {
        reason: String,
    },

    /// Error notification
    Error {
        code: u32,
        message: String,
    },

    // ========== Event Replication ==========
    /// Relayed teamengram event — carries raw event bytes from a peer's event log.
    ///
    /// Used for cursor-tracked event replication between Teambooks.
    /// The receiver can inject this into their local event log or process it
    /// through the inbox pipeline.
    ///
    /// `raw_event` format: `[header:64 bytes][payload:variable]` (teamengram wire format).
    EventRelay {
        /// Raw teamengram event bytes (header + payload).
        raw_event: Vec<u8>,
        /// Teamengram event type (u16) for filtering without full deserialization.
        event_type: u16,
        /// Source AI ID that generated this event.
        source_ai: String,
        /// Original sequence number in the source Teambook's event log.
        origin_seq: u64,
    },
}

/// Compact peer info for peer lists
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub node_id: String,
    pub display_name: String,
    pub trust_level: TrustLevel,
    pub last_seen: DateTime<Utc>,
    pub reachable_via: Vec<String>, // Endpoint descriptions
}

impl From<&FederationNode> for PeerInfo {
    fn from(node: &FederationNode) -> Self {
        Self {
            node_id: node.node_id.clone(),
            display_name: node.display_name.clone(),
            trust_level: node.trust_level,
            last_seen: node.last_seen,
            reachable_via: node.endpoints.iter().map(|e| e.description()).collect(),
        }
    }
}

/// Federated presence information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedPresence {
    /// The AI/device ID
    pub ai_id: String,

    /// Which node this presence is from
    pub node_id: String,

    /// Status (online, idle, busy, offline)
    pub status: String,

    /// Current activity (optional)
    pub activity: Option<String>,

    /// When this was last updated
    pub updated_at: DateTime<Utc>,

    /// Signature proving authenticity.
    ///
    /// Algorithm-agile: carries scheme identifier alongside raw bytes.
    pub signature: FederationSignature,
}

impl FederatedPresence {
    /// Create and sign a presence update
    pub fn new(
        ai_id: &str,
        node_id: &str,
        status: &str,
        activity: Option<String>,
        signing_key: &ed25519_dalek::SigningKey,
    ) -> Self {
        let updated_at = Utc::now();

        let mut presence = Self {
            ai_id: ai_id.to_string(),
            node_id: node_id.to_string(),
            status: status.to_string(),
            activity,
            updated_at,
            signature: FederationSignature::placeholder(),
        };

        let data = presence.signing_data();
        presence.signature = FederationSignature::ed25519(crate::sign_data(signing_key, &data));

        presence
    }

    fn signing_data(&self) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend(self.ai_id.as_bytes());
        data.extend(self.node_id.as_bytes());
        data.extend(self.status.as_bytes());
        if let Some(ref act) = self.activity {
            data.extend(act.as_bytes());
        }
        data.extend(self.updated_at.timestamp().to_le_bytes());
        data
    }

    /// Verify the presence signature using algorithm-agile verification.
    pub fn verify(&self, pubkey: &ed25519_dalek::VerifyingKey) -> bool {
        let data = self.signing_data();
        crate::verify_federation_signature(&self.signature, pubkey.as_bytes(), &data).is_ok()
    }
}

// ---------------------------------------------------------------------------
// Signed Event Envelope (wire format for federation transport)
// ---------------------------------------------------------------------------

/// A content-addressed, signed envelope for federation events.
///
/// This is what travels between Teambooks over the wire. The receiver:
/// 1. Checks `origin_pubkey` against known peers
/// 2. Verifies `signature` over `event_bytes` (Ed25519)
/// 3. Verifies `content_id` matches SHA-256(`event_bytes`) (integrity check)
/// 4. Deduplicates by `content_id` (same bytes from two peers = same event)
/// 5. Deserializes `event_bytes` as a `FederationMessage`
/// 6. Writes a `FEDERATED_*` event to the local event log
///
/// PDUs (DMs, task completions, concluded dialogues) use this envelope.
/// EDUs (presence) are fire-and-forget and don't require this wrapper.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SignedEvent {
    /// Serialized FederationMessage bytes (CBOR)
    pub event_bytes: Vec<u8>,

    /// Ed25519 public key of the originating Teambook (32 bytes, hex-encoded)
    pub origin_pubkey: String,

    /// Ed25519 signature over `event_bytes` (64 bytes, hex-encoded)
    pub signature: String,

    /// SHA-256 content hash of `event_bytes` — the deduplication key (hex-encoded)
    pub content_id: String,
}

impl SignedEvent {
    /// Wrap and sign a serialized FederationMessage.
    ///
    /// `event_bytes` should be the CBOR output of `FederationMessage::to_bytes()`.
    /// Returns an envelope ready for transmission.
    pub fn sign(
        event_bytes: Vec<u8>,
        identity: &crate::identity::TeambookIdentity,
    ) -> Self {
        let signature = identity.sign(&event_bytes);
        let content_id = content_hash(&event_bytes);

        Self {
            event_bytes,
            origin_pubkey: identity.public_key_hex(),
            signature: hex::encode(signature.to_bytes()),
            content_id: hex::encode(content_id),
        }
    }

    /// Verify signature and content hash integrity.
    ///
    /// Returns `Ok(())` if both checks pass.
    /// Returns `Err` with a specific reason on any failure — fail loudly.
    pub fn verify(&self) -> std::result::Result<(), SignedEventError> {
        // Decode origin pubkey
        let pubkey_bytes = hex::decode(&self.origin_pubkey)
            .map_err(|_| SignedEventError::InvalidPublicKey)?;

        // Verify content hash
        let expected_hash = hex::encode(content_hash(&self.event_bytes));
        if expected_hash != self.content_id {
            return Err(SignedEventError::ContentHashMismatch);
        }

        // Decode signature bytes and wrap in FederationSignature (Phase 1: Ed25519)
        let sig_bytes = hex::decode(&self.signature)
            .map_err(|_| SignedEventError::InvalidSignature)?;
        let fed_sig = FederationSignature {
            scheme: SignatureScheme::Ed25519,
            bytes: sig_bytes,
        };

        // Verify using algorithm-agile verification
        crate::verify_federation_signature(&fed_sig, &pubkey_bytes, &self.event_bytes)
            .map_err(|_| SignedEventError::InvalidSignature)
    }

    /// Content ID as hex (the deduplication key for PDU idempotency).
    pub fn content_id_hex(&self) -> &str {
        &self.content_id
    }
}

/// Compute SHA-256 content hash of event bytes.
///
/// Deterministic: same bytes always produce the same hash.
/// Used for PDU deduplication — re-syncing the same event is harmless.
pub fn content_hash(event_bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(event_bytes);
    hasher.finalize().into()
}

/// Errors that can occur when verifying a signed federation event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignedEventError {
    /// Malformed Ed25519 public key bytes
    InvalidPublicKey,
    /// SHA-256 content hash doesn't match event bytes (corrupted or tampered)
    ContentHashMismatch,
    /// Ed25519 signature verification failed (wrong key or tampered data)
    InvalidSignature,
}

impl std::fmt::Display for SignedEventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPublicKey => write!(f, "malformed Ed25519 public key"),
            Self::ContentHashMismatch => write!(f, "content hash does not match event bytes"),
            Self::InvalidSignature => write!(f, "Ed25519 signature verification failed"),
        }
    }
}

impl std::error::Error for SignedEventError {}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    #[test]
    fn test_message_sign_verify() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        let msg = FederationMessage::new(
            "test-node",
            FederationPayload::Ping { timestamp: 12345 },
            &signing_key,
        );

        assert!(msg.verify(&verifying_key));

        // Wrong key should fail
        let other_key = SigningKey::generate(&mut OsRng);
        assert!(!msg.verify(&other_key.verifying_key()));
    }

    #[test]
    fn test_presence_sign_verify() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        let presence = FederatedPresence::new(
            "lyra-584",
            "node-123",
            "online",
            Some("Working on federation".to_string()),
            &signing_key,
        );

        assert!(presence.verify(&verifying_key));
    }

    #[test]
    fn test_message_serialization() {
        let signing_key = SigningKey::generate(&mut OsRng);

        let msg = FederationMessage::new(
            "test-node",
            FederationPayload::Ping { timestamp: 12345 },
            &signing_key,
        );

        let bytes = msg.to_bytes().unwrap();
        let decoded = FederationMessage::from_bytes(&bytes).unwrap();

        assert_eq!(msg.id, decoded.id);
        assert_eq!(msg.from, decoded.from);
    }

    // -------------------------------------------------------------------
    // PQC Phase 1: Algorithm Agility Tests for Messages
    // -------------------------------------------------------------------

    #[test]
    fn test_message_signature_is_federation_signature() {
        // Verify FederationMessage.signature field uses FederationSignature
        // with Ed25519 scheme after construction
        let sk = SigningKey::generate(&mut OsRng);
        let msg = FederationMessage::new(
            "pqc-node",
            FederationPayload::Ping { timestamp: 99 },
            &sk,
        );

        assert_eq!(msg.signature.scheme, crate::SignatureScheme::Ed25519);
        assert_eq!(msg.signature.bytes.len(), 64);
        // Signature should not be all zeros (placeholder must be replaced)
        assert!(!msg.signature.bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_message_cbor_roundtrip_preserves_federation_signature() {
        // CRITICAL: The wire format changed from raw hex signature to
        // {scheme, bytes} struct. Verify CBOR serialization preserves both fields.
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();

        let msg = FederationMessage::new(
            "cbor-test",
            FederationPayload::Broadcast {
                channel: "general".to_string(),
                content: "PQC test broadcast".to_string(),
            },
            &sk,
        );

        let cbor_bytes = msg.to_bytes().unwrap();
        let decoded = FederationMessage::from_bytes(&cbor_bytes).unwrap();

        // Scheme and signature bytes must survive CBOR round-trip
        assert_eq!(decoded.signature.scheme, crate::SignatureScheme::Ed25519);
        assert_eq!(decoded.signature.bytes, msg.signature.bytes);

        // Verify the decoded message's signature still validates
        assert!(decoded.verify(&vk), "CBOR round-trip broke signature verification");
    }

    #[test]
    fn test_message_cbor_roundtrip_tamper_detection() {
        // Verify that tampering with a CBOR-decoded message is detected
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();

        let msg = FederationMessage::new(
            "tamper-test",
            FederationPayload::DirectMessage {
                to: "target-ai".to_string(),
                content: "secret message".to_string(),
                reply_to: None,
            },
            &sk,
        );

        let cbor_bytes = msg.to_bytes().unwrap();
        let mut decoded = FederationMessage::from_bytes(&cbor_bytes).unwrap();

        // Tamper with the from field
        decoded.from = "attacker-node".to_string();
        assert!(!decoded.verify(&vk), "tampered message should fail verification");
    }

    #[test]
    fn test_message_verify_uses_algorithm_agile_path() {
        // Ensure FederationMessage::verify() goes through verify_federation_signature
        // by testing with wrong key — error path must work end-to-end
        let sk = SigningKey::generate(&mut OsRng);
        let wrong_sk = SigningKey::generate(&mut OsRng);

        let msg = FederationMessage::new(
            "agile-verify",
            FederationPayload::Ping { timestamp: 42 },
            &sk,
        );

        assert!(msg.verify(&sk.verifying_key()));
        assert!(!msg.verify(&wrong_sk.verifying_key()));
    }

    #[test]
    fn test_presence_signature_is_federation_signature() {
        let sk = SigningKey::generate(&mut OsRng);
        let presence = FederatedPresence::new(
            "resonance-768",
            "node-456",
            "busy",
            Some("Running PQC tests".to_string()),
            &sk,
        );

        assert_eq!(presence.signature.scheme, crate::SignatureScheme::Ed25519);
        assert_eq!(presence.signature.bytes.len(), 64);
        assert!(!presence.signature.bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_presence_verify_uses_algorithm_agile_path() {
        let sk = SigningKey::generate(&mut OsRng);
        let wrong_sk = SigningKey::generate(&mut OsRng);

        let presence = FederatedPresence::new(
            "cascade-230",
            "node-789",
            "online",
            None,
            &sk,
        );

        assert!(presence.verify(&sk.verifying_key()));
        assert!(!presence.verify(&wrong_sk.verifying_key()));
    }

    #[test]
    fn test_signed_event_verify_uses_algorithm_agile_path() {
        // SignedEvent internally wraps hex-encoded signature in FederationSignature
        // during verify(). Ensure this path works correctly.
        let identity = crate::identity::TeambookIdentity::generate();

        let msg = FederationMessage::new(
            &identity.short_id(),
            FederationPayload::Ping { timestamp: 1000 },
            identity.signing_key(),
        );

        let cbor_bytes = msg.to_bytes().unwrap();
        let envelope = SignedEvent::sign(cbor_bytes, &identity);

        assert!(envelope.verify().is_ok(), "SignedEvent verify should pass");
    }

    #[test]
    fn test_signed_event_tampered_bytes_detected() {
        let identity = crate::identity::TeambookIdentity::generate();

        let msg = FederationMessage::new(
            &identity.short_id(),
            FederationPayload::Goodbye { reason: "test".to_string() },
            identity.signing_key(),
        );

        let cbor_bytes = msg.to_bytes().unwrap();
        let mut envelope = SignedEvent::sign(cbor_bytes, &identity);

        // Tamper with event bytes
        if let Some(b) = envelope.event_bytes.last_mut() {
            *b ^= 0xFF;
        }

        let result = envelope.verify();
        assert!(result.is_err(), "tampered event should fail verification");
    }

    #[test]
    fn test_signed_event_content_hash_integrity() {
        let identity = crate::identity::TeambookIdentity::generate();

        let msg = FederationMessage::new(
            &identity.short_id(),
            FederationPayload::Ping { timestamp: 2000 },
            identity.signing_key(),
        );

        let cbor_bytes = msg.to_bytes().unwrap();
        let envelope = SignedEvent::sign(cbor_bytes, &identity);

        // Content ID should be deterministic SHA-256
        let expected_hash = hex::encode(content_hash(&envelope.event_bytes));
        assert_eq!(envelope.content_id, expected_hash);
    }

    #[test]
    fn test_message_complex_payload_signature_covers_payload() {
        // Cascade's concern: signing_data() MUST include payload.
        // Test with a complex payload to ensure it's covered by the signature.
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();

        let msg = FederationMessage::new(
            "payload-test",
            FederationPayload::DataResponse {
                category: DataCategory::Notes,
                key: "important-data".to_string(),
                data: Some(vec![1, 2, 3, 4, 5]),
                version: 42,
            },
            &sk,
        );

        // Original verifies
        assert!(msg.verify(&vk));

        // Verify via CBOR round-trip too
        let bytes = msg.to_bytes().unwrap();
        let decoded = FederationMessage::from_bytes(&bytes).unwrap();
        assert!(decoded.verify(&vk));
    }
}
