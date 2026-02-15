//! Federation Cryptography — Ed25519 Identity & Event Signing
//!
//! Every Teambook instance has a persistent Ed25519 keypair that IS its identity.
//! Every event crossing federation boundaries is signed by the originating Teambook.
//! Content-addressed event IDs (SHA-256) provide deduplication and integrity.
//!
//! Design principles:
//! - Keypair generated once on first run, persisted forever
//! - Signatures are non-repudiable (~50μs per sign, ~120μs per verify)
//! - Content hashes are deterministic — same event bytes = same ID everywhere
//! - No passwords, no expiring tokens, no central authority

use ed25519_dalek::{
    Signature, Signer, SigningKey, Verifier, VerifyingKey, SECRET_KEY_LENGTH,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{info, warn};

/// 32-byte Ed25519 public key, used as Teambook identity
pub type PeerPublicKey = [u8; 32];

/// 64-byte Ed25519 signature over event bytes
pub type EventSignature = [u8; 64];

/// 32-byte SHA-256 content hash, used as event ID
pub type ContentHash = [u8; 32];

// ---------------------------------------------------------------------------
// Teambook Identity (Ed25519 Keypair)
// ---------------------------------------------------------------------------

/// Persistent cryptographic identity for a Teambook instance.
///
/// Generated once on first run, stored at `~/.ai-foundation/federation/identity.key`.
/// The public key IS the Teambook's identity in the federation — no usernames,
/// no registration, no central authority.
pub struct TeambookIdentity {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
}

impl TeambookIdentity {
    /// Load existing identity or generate a new one.
    ///
    /// Identity file: `~/.ai-foundation/federation/identity.key` (32 bytes, raw secret key)
    /// The file contains ONLY the secret key — the public key is derived deterministically.
    pub async fn load_or_generate() -> anyhow::Result<Self> {
        let key_path = Self::key_path()?;

        if key_path.exists() {
            Self::load_from_file(&key_path).await
        } else {
            let identity = Self::generate();
            identity.save_to_file(&key_path).await?;
            info!(
                pubkey = %identity.public_key_hex(),
                "Generated new Teambook identity"
            );
            Ok(identity)
        }
    }

    /// Generate a fresh Ed25519 keypair using OS randomness.
    pub(crate) fn generate() -> Self {
        let mut csprng = rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Load identity from a raw 32-byte secret key file.
    async fn load_from_file(path: &Path) -> anyhow::Result<Self> {
        let bytes = fs::read(path).await?;
        if bytes.len() != SECRET_KEY_LENGTH {
            anyhow::bail!(
                "Identity file corrupted: expected {} bytes, got {}",
                SECRET_KEY_LENGTH,
                bytes.len()
            );
        }

        let mut secret = [0u8; SECRET_KEY_LENGTH];
        secret.copy_from_slice(&bytes);
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();

        info!(
            pubkey = %hex::encode(verifying_key.as_bytes()),
            "Loaded Teambook identity"
        );

        Ok(Self {
            signing_key,
            verifying_key,
        })
    }

    /// Persist the secret key to disk. Creates parent directories if needed.
    async fn save_to_file(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(path, self.signing_key.as_bytes()).await?;

        // Restrict file permissions on Unix (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, perms)?;
        }

        Ok(())
    }

    /// Path to the identity key file.
    fn key_path() -> anyhow::Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
        Ok(home
            .join(".ai-foundation")
            .join("federation")
            .join("identity.key"))
    }

    /// Sign arbitrary bytes with this Teambook's private key.
    ///
    /// ~50μs on modern hardware. Returns a 64-byte Ed25519 signature.
    pub fn sign(&self, message: &[u8]) -> EventSignature {
        let signature = self.signing_key.sign(message);
        signature.to_bytes()
    }

    /// This Teambook's public key (32 bytes).
    pub fn public_key(&self) -> PeerPublicKey {
        *self.verifying_key.as_bytes()
    }

    /// Public key as hex string (for logging/display).
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.verifying_key.as_bytes())
    }

    /// Short identifier derived from public key (first 8 hex chars).
    /// Used for human-readable peer identification in logs.
    pub fn short_id(&self) -> String {
        self.public_key_hex()[..8].to_string()
    }
}

// ---------------------------------------------------------------------------
// Signature Verification (for received events)
// ---------------------------------------------------------------------------

/// Verify an Ed25519 signature against a public key and message.
///
/// ~120μs on modern hardware. Returns `true` if valid, `false` if tampered.
/// This is the core trust primitive — if verify returns true, the event
/// was created by the holder of the corresponding private key.
pub fn verify_signature(
    public_key: &PeerPublicKey,
    message: &[u8],
    signature: &EventSignature,
) -> bool {
    let Ok(verifying_key) = VerifyingKey::from_bytes(public_key) else {
        warn!("Invalid public key bytes");
        return false;
    };

    let sig = Signature::from_bytes(signature);

    verifying_key.verify(message, &sig).is_ok()
}

// ---------------------------------------------------------------------------
// Content-Addressed Event IDs (SHA-256)
// ---------------------------------------------------------------------------

/// Compute a content-addressed ID for event bytes.
///
/// The content hash is the SHA-256 digest of the canonical event bytes
/// (header + payload, BEFORE signing). This provides:
///
/// - **Deduplication**: Same event from two peers produces identical hash
/// - **Integrity**: Any bit flip changes the hash
/// - **Idempotent sync**: Re-syncing the same events is harmless
///
/// The hash covers the full event bytes as-is. The caller is responsible
/// for providing canonical bytes (the raw header + payload from the event log).
pub fn content_hash(event_bytes: &[u8]) -> ContentHash {
    let mut hasher = Sha256::new();
    hasher.update(event_bytes);
    hasher.finalize().into()
}

/// Content hash as hex string (for logging/storage/comparison).
pub fn content_hash_hex(event_bytes: &[u8]) -> String {
    hex::encode(content_hash(event_bytes))
}

// ---------------------------------------------------------------------------
// Signed Event Envelope (for federation transport)
// ---------------------------------------------------------------------------

/// A federation event: raw event bytes wrapped with origin identity and signature.
///
/// This is what travels between Teambooks. The receiver:
/// 1. Checks `origin_pubkey` against known peers
/// 2. Verifies `signature` over `event_bytes`
/// 3. Computes `content_hash(event_bytes)` for deduplication
/// 4. If new and valid, appends to local event log
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SignedEvent {
    /// Raw event bytes (header + payload) from the originating Teambook
    #[serde(with = "base64_bytes")]
    pub event_bytes: Vec<u8>,

    /// Ed25519 public key of the Teambook that created this event
    #[serde(with = "hex_bytes_32")]
    pub origin_pubkey: PeerPublicKey,

    /// Ed25519 signature over `event_bytes`
    #[serde(with = "hex_bytes_64")]
    pub signature: EventSignature,

    /// SHA-256 content hash of `event_bytes` (for quick dedup without re-hashing)
    #[serde(with = "hex_bytes_32")]
    pub content_id: ContentHash,
}

impl SignedEvent {
    /// Create a signed event from raw bytes using a Teambook identity.
    pub fn sign(event_bytes: Vec<u8>, identity: &TeambookIdentity) -> Self {
        let signature = identity.sign(&event_bytes);
        let content_id = content_hash(&event_bytes);

        Self {
            event_bytes,
            origin_pubkey: identity.public_key(),
            signature,
            content_id,
        }
    }

    /// Verify this event's signature and content hash integrity.
    ///
    /// Returns `Ok(())` if both the signature and content hash are valid.
    /// Returns `Err` with a specific reason on any failure.
    pub fn verify(&self) -> Result<(), SignedEventError> {
        // Verify content hash matches event bytes
        let expected_hash = content_hash(&self.event_bytes);
        if expected_hash != self.content_id {
            return Err(SignedEventError::ContentHashMismatch);
        }

        // Verify Ed25519 signature
        if !verify_signature(&self.origin_pubkey, &self.event_bytes, &self.signature) {
            return Err(SignedEventError::InvalidSignature);
        }

        Ok(())
    }

    /// Content ID as hex string.
    pub fn content_id_hex(&self) -> String {
        hex::encode(self.content_id)
    }

    /// Origin public key as hex string.
    pub fn origin_pubkey_hex(&self) -> String {
        hex::encode(self.origin_pubkey)
    }

    /// Short origin identifier (first 8 hex chars of pubkey).
    pub fn origin_short_id(&self) -> String {
        self.origin_pubkey_hex()[..8].to_string()
    }
}

/// Errors that can occur when verifying a signed federation event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignedEventError {
    /// SHA-256 content hash doesn't match event bytes (corrupted or tampered)
    ContentHashMismatch,
    /// Ed25519 signature verification failed (wrong key or tampered data)
    InvalidSignature,
}

impl std::fmt::Display for SignedEventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ContentHashMismatch => {
                write!(f, "content hash does not match event bytes")
            }
            Self::InvalidSignature => {
                write!(f, "Ed25519 signature verification failed")
            }
        }
    }
}

impl std::error::Error for SignedEventError {}

// ---------------------------------------------------------------------------
// Serde helpers for fixed-size byte arrays
// ---------------------------------------------------------------------------

/// Serialize/deserialize Vec<u8> as base64 (for event_bytes which can be large).
mod base64_bytes {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        STANDARD.decode(&s).map_err(serde::de::Error::custom)
    }
}

/// Serialize/deserialize [u8; 32] as hex string.
mod hex_bytes_32 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        let vec = hex::decode(&s).map_err(serde::de::Error::custom)?;
        let arr: [u8; 32] = vec
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 32 bytes"))?;
        Ok(arr)
    }
}

/// Serialize/deserialize [u8; 64] as hex string.
mod hex_bytes_64 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        let s = String::deserialize(d)?;
        let vec = hex::decode(&s).map_err(serde::de::Error::custom)?;
        let arr: [u8; 64] = vec
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 64 bytes"))?;
        Ok(arr)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sign_and_verify() {
        let identity = TeambookIdentity::generate();
        let event_bytes = b"test event data for signing".to_vec();

        let signed = SignedEvent::sign(event_bytes.clone(), &identity);

        // Verification should succeed
        assert!(signed.verify().is_ok());

        // Content hash should be deterministic
        assert_eq!(signed.content_id, content_hash(&event_bytes));
    }

    #[tokio::test]
    async fn test_tampered_bytes_fail_verification() {
        let identity = TeambookIdentity::generate();
        let event_bytes = b"original event data".to_vec();

        let mut signed = SignedEvent::sign(event_bytes, &identity);

        // Tamper with the event bytes
        signed.event_bytes[0] ^= 0xFF;

        // Both content hash and signature should fail
        assert_eq!(
            signed.verify(),
            Err(SignedEventError::ContentHashMismatch)
        );
    }

    #[tokio::test]
    async fn test_wrong_key_fails_verification() {
        let identity_a = TeambookIdentity::generate();
        let identity_b = TeambookIdentity::generate();
        let event_bytes = b"event from teambook A".to_vec();

        let mut signed = SignedEvent::sign(event_bytes, &identity_a);

        // Replace origin key with a different Teambook's key
        signed.origin_pubkey = identity_b.public_key();

        // Content hash is still valid, but signature won't match
        assert_eq!(
            signed.verify(),
            Err(SignedEventError::InvalidSignature)
        );
    }

    #[test]
    fn test_content_hash_deterministic() {
        let data = b"identical event bytes";
        let hash1 = content_hash(data);
        let hash2 = content_hash(data);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_content_hash_different_data() {
        let hash1 = content_hash(b"event A");
        let hash2 = content_hash(b"event B");
        assert_ne!(hash1, hash2);
    }

    #[tokio::test]
    async fn test_signed_event_serialization_roundtrip() {
        let identity = TeambookIdentity::generate();
        let event_bytes = b"serializable event".to_vec();

        let signed = SignedEvent::sign(event_bytes, &identity);

        // Serialize to JSON
        let json = serde_json::to_string(&signed).unwrap();

        // Deserialize back
        let deserialized: SignedEvent = serde_json::from_str(&json).unwrap();

        // Should still verify
        assert!(deserialized.verify().is_ok());
        assert_eq!(signed.content_id, deserialized.content_id);
        assert_eq!(signed.origin_pubkey, deserialized.origin_pubkey);
        assert_eq!(signed.event_bytes, deserialized.event_bytes);
    }

    #[test]
    fn test_short_id() {
        let identity = TeambookIdentity::generate();
        let short = identity.short_id();
        assert_eq!(short.len(), 8);
        // Should be valid hex
        assert!(short.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
