//! Federation Node - represents a Teambook in the mesh

use crate::{Endpoint, SharingPreferences, TrustLevel, Result, FederationError, node_id_from_pubkey};
use ed25519_dalek::{SigningKey, VerifyingKey, Signature};
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// A node in the federation mesh (represents a Teambook)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationNode {
    /// Unique identifier (hash of public key)
    pub node_id: String,

    /// Human-readable display name
    pub display_name: String,

    /// Ed25519 public key for verification
    #[serde(with = "pubkey_serde")]
    pub pubkey: VerifyingKey,

    /// Hardware fingerprint for identity binding
    pub hardware_fingerprint: Option<String>,

    /// How to reach this node
    pub endpoints: Vec<Endpoint>,

    /// What this node can do
    pub capabilities: NodeCapabilities,

    /// Current trust level
    pub trust_level: TrustLevel,

    /// What this node shares
    pub sharing_prefs: SharingPreferences,

    /// When this node was first seen
    pub first_seen: DateTime<Utc>,

    /// When this node was last seen
    pub last_seen: DateTime<Utc>,

    /// Optional profile data (from profile-cli)
    pub profile: Option<NodeProfile>,
}

impl FederationNode {
    /// Create a new local node (generates keys)
    pub fn new_local(display_name: &str, signing_key: &SigningKey) -> Self {
        let verifying_key = signing_key.verifying_key();
        let node_id = node_id_from_pubkey(&verifying_key);
        let now = Utc::now();

        Self {
            node_id,
            display_name: display_name.to_string(),
            pubkey: verifying_key,
            hardware_fingerprint: None,
            endpoints: Vec::new(),
            capabilities: NodeCapabilities::default(),
            trust_level: TrustLevel::Owner, // Local node is always Owner
            sharing_prefs: SharingPreferences::default(),
            first_seen: now,
            last_seen: now,
            profile: None,
        }
    }

    /// Create from discovered peer data
    pub fn from_discovery(
        node_id: String,
        display_name: String,
        pubkey: VerifyingKey,
        endpoints: Vec<Endpoint>,
    ) -> Self {
        let now = Utc::now();

        Self {
            node_id,
            display_name,
            pubkey,
            hardware_fingerprint: None,
            endpoints,
            capabilities: NodeCapabilities::default(),
            trust_level: TrustLevel::Anonymous,
            sharing_prefs: SharingPreferences::minimal(),
            first_seen: now,
            last_seen: now,
            profile: None,
        }
    }

    /// Add an endpoint to this node
    pub fn add_endpoint(&mut self, endpoint: Endpoint) {
        if !self.endpoints.contains(&endpoint) {
            self.endpoints.push(endpoint);
        }
    }

    /// Update last seen timestamp
    pub fn touch(&mut self) {
        self.last_seen = Utc::now();
    }

    /// Check if node meets minimum trust level
    pub fn meets_trust(&self, required: TrustLevel) -> bool {
        self.trust_level >= required
    }

    /// Upgrade trust level (can only go up)
    pub fn upgrade_trust(&mut self, new_level: TrustLevel) {
        if new_level > self.trust_level {
            self.trust_level = new_level;
        }
    }

    /// Set hardware fingerprint
    pub fn set_fingerprint(&mut self, fingerprint: String) {
        self.hardware_fingerprint = Some(fingerprint);
        // Having a fingerprint upgrades from Anonymous to Verified
        if self.trust_level == TrustLevel::Anonymous {
            self.trust_level = TrustLevel::Verified;
        }
    }

    /// Verify that a signature came from this node
    pub fn verify_signature(&self, data: &[u8], signature: &Signature) -> bool {
        crate::verify_signature(&self.pubkey, data, signature)
    }
}

/// What a node is capable of
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCapabilities {
    /// Maximum simultaneous connections
    pub max_connections: u32,

    /// Can this node relay messages for others?
    pub supports_relay: bool,

    /// Can this node cache data for the mesh?
    pub supports_cache: bool,

    /// Bandwidth limit in kbps (None = unlimited)
    pub bandwidth_limit_kbps: Option<u32>,

    /// Supported transport types
    pub supported_transports: Vec<crate::TransportType>,

    /// Protocol version
    pub protocol_version: u32,
}

impl Default for NodeCapabilities {
    fn default() -> Self {
        Self {
            max_connections: 32,
            supports_relay: true,
            supports_cache: true,
            bandwidth_limit_kbps: None,
            supported_transports: vec![
                crate::TransportType::QuicInternet,
                crate::TransportType::QuicLan,
                crate::TransportType::Mdns,
            ],
            protocol_version: 1,
        }
    }
}

/// Profile data for visual identity (from profile-cli)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeProfile {
    /// AI identifier
    pub ai_id: String,

    /// Display name
    pub name: String,

    /// Pronouns (optional)
    pub pronouns: Option<String>,

    /// Short tagline
    pub tagline: Option<String>,

    /// Primary color (hex)
    pub primary_color: Option<String>,

    /// Secondary color (hex)
    pub secondary_color: Option<String>,

    /// Avatar type (for procedural generation)
    pub avatar_type: Option<String>,
}

/// Serde support for VerifyingKey
mod pubkey_serde {
    use ed25519_dalek::VerifyingKey;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(key: &VerifyingKey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let bytes = key.as_bytes();
        hex::encode(bytes).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<VerifyingKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex_str = String::deserialize(deserializer)?;
        let bytes = hex::decode(&hex_str).map_err(serde::de::Error::custom)?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("Invalid key length"))?;
        VerifyingKey::from_bytes(&bytes).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;

    #[test]
    fn test_new_local_node() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let node = FederationNode::new_local("Test Node", &signing_key);

        assert!(!node.node_id.is_empty());
        assert_eq!(node.display_name, "Test Node");
        assert_eq!(node.trust_level, TrustLevel::Owner);
        assert!(node.endpoints.is_empty());
    }

    #[test]
    fn test_trust_upgrade() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let mut node = FederationNode::new_local("Test", &signing_key);
        node.trust_level = TrustLevel::Anonymous;

        node.upgrade_trust(TrustLevel::Verified);
        assert_eq!(node.trust_level, TrustLevel::Verified);

        // Can't downgrade
        node.upgrade_trust(TrustLevel::Anonymous);
        assert_eq!(node.trust_level, TrustLevel::Verified);
    }

    #[test]
    fn test_fingerprint_upgrade() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        let mut node = FederationNode::from_discovery(
            "test".to_string(),
            "Test".to_string(),
            verifying_key,
            vec![],
        );

        assert_eq!(node.trust_level, TrustLevel::Anonymous);

        node.set_fingerprint("abc123".to_string());
        assert_eq!(node.trust_level, TrustLevel::Verified);
    }
}
