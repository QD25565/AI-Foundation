//! Federation protocol messages

use crate::{
    FederationNode, NodeCapabilities, SharingPreferences, TrustLevel,
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

    /// Get the data to sign/verify
    fn signing_data(&self) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend(self.id.as_bytes());
        data.extend(self.from.as_bytes());
        data.extend(self.timestamp.timestamp().to_le_bytes());

        // Serialize payload (deterministic)
        if let Ok(payload_bytes) = ciborium::ser::into_writer(&self.payload, Vec::new()) {
            // ciborium writes to the vec, we need different approach
        }
        // Use serde_json for now (deterministic enough for signatures)
        if let Ok(payload_json) = serde_json::to_vec(&self.payload) {
            data.extend(payload_json);
        }

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
}
