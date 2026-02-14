//! Node Identity - Ed25519 based sovereign identity for mesh nodes
//!
//! Each node in the Deep Net mesh has a unique identity derived from an Ed25519 keypair.
//! The public key serves as the Node ID, ensuring no central authority is needed.

use ed25519_dalek::{SigningKey, VerifyingKey, Signature, Signer, Verifier};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// 32-byte Node ID derived from Ed25519 public key
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub [u8; 32]);

impl NodeId {
    /// Create a NodeId from raw bytes
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Get the raw bytes
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Convert to hex string for display
    pub fn to_hex(&self) -> String {
        hex::encode(&self.0)
    }

    /// Parse from hex string
    pub fn from_hex(s: &str) -> Result<Self, IdentityError> {
        let bytes = hex::decode(s).map_err(|_| IdentityError::InvalidHex)?;
        if bytes.len() != 32 {
            return Err(IdentityError::InvalidLength);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }

    /// Short display form (first 8 chars of hex)
    pub fn short(&self) -> String {
        self.to_hex()[..8].to_string()
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({}...)", &self.to_hex()[..8])
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.short())
    }
}

/// Capabilities a node can advertise
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Can send/receive direct messages
    DirectMessage,
    /// Can broadcast to mesh
    Broadcast,
    /// Can share files
    FileShare,
    /// Publishes presence information
    Presence,
    /// Can relay messages for other nodes
    Relay,
    /// Supports end-to-end encrypted channels
    E2EEncryption,
}

/// Public identity information for a node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeManifest {
    /// Unique node identifier (public key)
    pub node_id: NodeId,
    /// Human-readable display name (not unique)
    pub display_name: String,
    /// Capabilities this node supports
    pub capabilities: Vec<Capability>,
    /// Unix timestamp when identity was created
    pub created_at: u64,
    /// Protocol version
    pub protocol_version: u32,
    /// Optional public metadata
    pub metadata: Option<NodeMetadata>,
}

/// Optional public metadata for node discovery
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeMetadata {
    /// Device type (mobile, desktop, server)
    pub device_type: Option<String>,
    /// Application version
    pub app_version: Option<String>,
    /// Geographic region hint (for latency-based routing)
    pub region: Option<String>,
}

/// Complete node identity with private key
pub struct NodeIdentity {
    /// The signing key (private)
    signing_key: SigningKey,
    /// Public manifest
    pub manifest: NodeManifest,
}

impl NodeIdentity {
    /// Generate a new random identity
    pub fn generate(display_name: String) -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let node_id = NodeId::from_bytes(verifying_key.to_bytes());

        let manifest = NodeManifest {
            node_id,
            display_name,
            capabilities: vec![
                Capability::DirectMessage,
                Capability::Broadcast,
                Capability::Presence,
            ],
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            protocol_version: 1,
            metadata: None,
        };

        Self {
            signing_key,
            manifest,
        }
    }

    /// Get the node ID
    pub fn node_id(&self) -> NodeId {
        self.manifest.node_id
    }

    /// Sign a message
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }

    /// Get the verifying (public) key
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Export identity to bytes (for secure storage)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.signing_key.as_bytes());
        bytes.extend_from_slice(&bincode::serialize(&self.manifest).unwrap());
        bytes
    }

    /// Import identity from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, IdentityError> {
        if bytes.len() < 32 {
            return Err(IdentityError::InvalidLength);
        }

        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&bytes[..32]);
        let signing_key = SigningKey::from_bytes(&key_bytes);

        let manifest: NodeManifest = bincode::deserialize(&bytes[32..])
            .map_err(|_| IdentityError::DeserializationFailed)?;

        // Verify the manifest's node_id matches the key
        let expected_id = NodeId::from_bytes(signing_key.verifying_key().to_bytes());
        if manifest.node_id != expected_id {
            return Err(IdentityError::KeyMismatch);
        }

        Ok(Self {
            signing_key,
            manifest,
        })
    }

    /// Update display name
    pub fn set_display_name(&mut self, name: String) {
        self.manifest.display_name = name;
    }

    /// Update capabilities
    pub fn set_capabilities(&mut self, capabilities: Vec<Capability>) {
        self.manifest.capabilities = capabilities;
    }
}

/// Verify a signature from a node
pub fn verify_signature(
    node_id: &NodeId,
    message: &[u8],
    signature: &Signature,
) -> Result<(), IdentityError> {
    let verifying_key = VerifyingKey::from_bytes(node_id.as_bytes())
        .map_err(|_| IdentityError::InvalidPublicKey)?;

    verifying_key
        .verify(message, signature)
        .map_err(|_| IdentityError::InvalidSignature)
}

/// Identity-related errors
#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("Invalid hex encoding")]
    InvalidHex,

    #[error("Invalid byte length")]
    InvalidLength,

    #[error("Invalid public key")]
    InvalidPublicKey,

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Key mismatch in manifest")]
    KeyMismatch,

    #[error("Deserialization failed")]
    DeserializationFailed,
}

// Hex encoding helper (inline to avoid extra dependency)
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    pub fn decode(s: &str) -> Result<Vec<u8>, ()> {
        if s.len() % 2 != 0 {
            return Err(());
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| ()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_generation() {
        let identity = NodeIdentity::generate("TestNode".to_string());
        assert_eq!(identity.manifest.display_name, "TestNode");
        assert!(!identity.manifest.capabilities.is_empty());
    }

    #[test]
    fn test_sign_and_verify() {
        let identity = NodeIdentity::generate("Signer".to_string());
        let message = b"Hello, Deep Net!";

        let signature = identity.sign(message);
        assert!(verify_signature(&identity.node_id(), message, &signature).is_ok());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let identity = NodeIdentity::generate("Serializable".to_string());
        let bytes = identity.to_bytes();
        let restored = NodeIdentity::from_bytes(&bytes).unwrap();

        assert_eq!(identity.node_id(), restored.node_id());
        assert_eq!(identity.manifest.display_name, restored.manifest.display_name);
    }

    #[test]
    fn test_node_id_hex() {
        let identity = NodeIdentity::generate("HexTest".to_string());
        let hex = identity.node_id().to_hex();
        let parsed = NodeId::from_hex(&hex).unwrap();

        assert_eq!(identity.node_id(), parsed);
    }
}
