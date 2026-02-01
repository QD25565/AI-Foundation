//! AI Identity Management
//!
//! Each AI has a unique identity consisting of:
//! - AI_ID: Human-readable identifier (e.g., "lyra-584")
//! - Public Key: Ed25519 public key for signature verification
//! - Trust Level: Permission tier within the teambook
//! - Teambook: Which teambook this identity belongs to

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AFPError, Result};

/// Trust levels for tiered permission system
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum TrustLevel {
    /// Anonymous: Read-only, rate-limited, no hardware binding
    Anonymous = 0,

    /// Verified: Hardware fingerprint registered, can participate, soft-bannable
    Verified = 1,

    /// Trusted: TPM-bound identity, vouched, full privileges, hardware-bannable
    Trusted = 2,

    /// Owner: Controls teambook, can ban/unban, sets policies
    Owner = 3,
}

impl TrustLevel {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => TrustLevel::Anonymous,
            1 => TrustLevel::Verified,
            2 => TrustLevel::Trusted,
            3 => TrustLevel::Owner,
            _ => TrustLevel::Anonymous,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TrustLevel::Anonymous => "anonymous",
            TrustLevel::Verified => "verified",
            TrustLevel::Trusted => "trusted",
            TrustLevel::Owner => "owner",
        }
    }

    /// Check if this trust level can perform an action requiring `required` level
    pub fn can_perform(&self, required: TrustLevel) -> bool {
        *self >= required
    }
}

impl std::fmt::Display for TrustLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Full AI Identity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIIdentity {
    /// Human-readable AI identifier (e.g., "lyra-584")
    pub ai_id: String,

    /// Ed25519 public key (32 bytes)
    #[serde(with = "pubkey_serde")]
    pub pubkey: VerifyingKey,

    /// Teambook this identity belongs to ("local" or URL)
    pub teambook: String,

    /// Trust level within the teambook
    pub trust_level: TrustLevel,
}

impl AIIdentity {
    /// Create a new AI identity
    pub fn new(ai_id: String, pubkey: VerifyingKey, teambook: String) -> Self {
        Self {
            ai_id,
            pubkey,
            teambook,
            trust_level: TrustLevel::Anonymous,
        }
    }

    /// Create identity with specified trust level
    pub fn with_trust_level(mut self, level: TrustLevel) -> Self {
        self.trust_level = level;
        self
    }

    /// Generate a fingerprint of this identity (for display/verification)
    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.ai_id.as_bytes());
        hasher.update(self.pubkey.as_bytes());
        let result = hasher.finalize();
        // Return first 8 bytes as hex (16 chars)
        hex::encode(&result[..8])
    }

    /// Verify a signature made by this identity
    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<()> {
        self.pubkey
            .verify(message, signature)
            .map_err(|_| AFPError::SignatureVerificationFailed)
    }

    /// Validate AI_ID format (name-number)
    pub fn validate_ai_id(ai_id: &str) -> Result<()> {
        // Format: name-number (e.g., "lyra-584", "cascade-230")
        let parts: Vec<&str> = ai_id.rsplitn(2, '-').collect();
        if parts.len() != 2 {
            return Err(AFPError::InvalidAIID(
                "Must be in format 'name-number'".to_string(),
            ));
        }

        let number = parts[0];
        let name = parts[1];

        if name.is_empty() {
            return Err(AFPError::InvalidAIID("Name cannot be empty".to_string()));
        }

        if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(AFPError::InvalidAIID(
                "Name must be alphanumeric".to_string(),
            ));
        }

        if !number.chars().all(|c| c.is_ascii_digit()) {
            return Err(AFPError::InvalidAIID(
                "Suffix must be numeric".to_string(),
            ));
        }

        Ok(())
    }

    /// Serialize identity to CBOR bytes
    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf)
            .map_err(|e| AFPError::SerializationFailed(e.to_string()))?;
        Ok(buf)
    }

    /// Deserialize identity from CBOR bytes
    pub fn from_cbor(data: &[u8]) -> Result<Self> {
        ciborium::from_reader(data)
            .map_err(|e| AFPError::DeserializationFailed(e.to_string()))
    }
}

impl std::fmt::Display for AIIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}@{} [{}] ({})",
            self.ai_id,
            self.teambook,
            self.trust_level,
            self.fingerprint()
        )
    }
}

/// Compact identity for message headers (smaller than full AIIdentity)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactIdentity {
    /// AI ID
    pub ai_id: String,

    /// Public key bytes (32)
    pub pubkey: [u8; 32],
}

impl From<&AIIdentity> for CompactIdentity {
    fn from(identity: &AIIdentity) -> Self {
        Self {
            ai_id: identity.ai_id.clone(),
            pubkey: identity.pubkey.to_bytes(),
        }
    }
}

impl CompactIdentity {
    pub fn to_verifying_key(&self) -> Result<VerifyingKey> {
        VerifyingKey::from_bytes(&self.pubkey)
            .map_err(|e| AFPError::InvalidPublicKey(e.to_string()))
    }
}

/// Helper module for serde serialization of VerifyingKey
mod pubkey_serde {
    use ed25519_dalek::VerifyingKey;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(key: &VerifyingKey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        key.to_bytes().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<VerifyingKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes: [u8; 32] = Deserialize::deserialize(deserializer)?;
        VerifyingKey::from_bytes(&bytes).map_err(serde::de::Error::custom)
    }
}

/// Generate a new AI ID with random suffix
pub fn generate_ai_id(name: &str) -> String {
    use rand::Rng;
    let suffix: u32 = rand::thread_rng().gen_range(100..1000);
    format!("{}-{}", name.to_lowercase(), suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trust_level_ordering() {
        assert!(TrustLevel::Owner > TrustLevel::Trusted);
        assert!(TrustLevel::Trusted > TrustLevel::Verified);
        assert!(TrustLevel::Verified > TrustLevel::Anonymous);
    }

    #[test]
    fn test_validate_ai_id() {
        assert!(AIIdentity::validate_ai_id("lyra-584").is_ok());
        assert!(AIIdentity::validate_ai_id("cascade-230").is_ok());
        assert!(AIIdentity::validate_ai_id("aurora_myapp-127").is_ok());

        assert!(AIIdentity::validate_ai_id("invalid").is_err());
        assert!(AIIdentity::validate_ai_id("-123").is_err());
        assert!(AIIdentity::validate_ai_id("name-").is_err());
    }

    #[test]
    fn test_generate_ai_id() {
        let id = generate_ai_id("test");
        assert!(id.starts_with("test-"));
        assert!(AIIdentity::validate_ai_id(&id).is_ok());
    }
}
