//! Federation protocol messages

use crate::{
    FederationNode, SharingPreferences, TrustLevel,
    DataCategory, Result, FederationError,
};
use ed25519_dalek::Signature;
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

    /// Signature over (id + from + timestamp + payload)
    #[serde(with = "signature_serde")]
    pub signature: Signature,
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

        // Create unsigned version for signing
        let mut msg = Self {
            id: id.clone(),
            from: from.to_string(),
            timestamp,
            payload,
            signature: Signature::from_bytes(&[0u8; 64]), // Placeholder
        };

        // Sign
        let data = msg.signing_data();
        msg.signature = crate::sign_data(signing_key, &data);

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

    /// Verify the message signature
    pub fn verify(&self, pubkey: &ed25519_dalek::VerifyingKey) -> bool {
        let data = self.signing_data();
        crate::verify_signature(pubkey, &data, &self.signature)
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
    },

    /// Response to hello
    Welcome {
        node: FederationNode,
        protocol_version: u32,
        accepted: bool,
        rejection_reason: Option<String>,
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

    /// Signature proving authenticity
    #[serde(with = "signature_serde")]
    pub signature: Signature,
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
            signature: Signature::from_bytes(&[0u8; 64]),
        };

        // Sign
        let data = presence.signing_data();
        presence.signature = crate::sign_data(signing_key, &data);

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

    /// Verify the presence signature
    pub fn verify(&self, pubkey: &ed25519_dalek::VerifyingKey) -> bool {
        let data = self.signing_data();
        crate::verify_signature(pubkey, &data, &self.signature)
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
        let pubkey_arr: [u8; 32] = pubkey_bytes
            .try_into()
            .map_err(|_| SignedEventError::InvalidPublicKey)?;
        let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&pubkey_arr)
            .map_err(|_| SignedEventError::InvalidPublicKey)?;

        // Verify content hash
        let expected_hash = hex::encode(content_hash(&self.event_bytes));
        if expected_hash != self.content_id {
            return Err(SignedEventError::ContentHashMismatch);
        }

        // Decode and verify signature
        let sig_bytes = hex::decode(&self.signature)
            .map_err(|_| SignedEventError::InvalidSignature)?;
        let sig_arr: [u8; 64] = sig_bytes
            .try_into()
            .map_err(|_| SignedEventError::InvalidSignature)?;
        let signature = ed25519_dalek::Signature::from_bytes(&sig_arr);

        use ed25519_dalek::Verifier;
        verifying_key
            .verify(&self.event_bytes, &signature)
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

/// Serde support for Signature
mod signature_serde {
    use ed25519_dalek::Signature;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(sig: &Signature, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        hex::encode(sig.to_bytes()).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Signature, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex_str = String::deserialize(deserializer)?;
        let bytes = hex::decode(&hex_str).map_err(serde::de::Error::custom)?;
        let bytes: [u8; 64] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("Invalid signature length"))?;
        Ok(Signature::from_bytes(&bytes))
    }
}

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
            "beta-002",
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
}
