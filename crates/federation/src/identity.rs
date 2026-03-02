//! Teambook Identity — Persistent Ed25519 Keypair
//!
//! Each Teambook has exactly one identity keypair, generated on first run and
//! persisted forever at `~/.ai-foundation/federation/identity.key`.
//!
//! The public key IS the Teambook's identity in the federation — no usernames,
//! no registration, no central authority. Rename the Teambook freely; the key
//! (and therefore the node ID) never changes.
//!
//! Ported from ai-foundation-clean/src/crypto.rs, adapted to federation-rs types.

use ed25519_dalek::{Signer, SigningKey, VerifyingKey, SECRET_KEY_LENGTH};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::info;

use crate::{FederationError, FederationSignature, FederationSigner, SignatureScheme, Result};

/// Persistent cryptographic identity for a Teambook.
///
/// Generated once, stored at `~/.ai-foundation/federation/identity.key`
/// (32 raw secret-key bytes, permissions 0o600 on Unix).
pub struct TeambookIdentity {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
}

impl TeambookIdentity {
    /// Load the existing identity from disk, or generate and persist a fresh one.
    ///
    /// Call once at startup. The returned identity is the stable anchor for all
    /// federation operations in this session.
    pub async fn load_or_generate() -> Result<Self> {
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

    /// Generate a fresh Ed25519 keypair (in-memory only, not persisted).
    ///
    /// Use for testing or ephemeral identities. For production, prefer
    /// `load_or_generate()` which persists the key to disk.
    pub fn generate() -> Self {
        let mut csprng = rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Load identity from a 32-byte secret key file.
    async fn load_from_file(path: &Path) -> Result<Self> {
        let bytes = fs::read(path)
            .await
            .map_err(|e| FederationError::Internal(format!("Failed to read identity file: {e}")))?;

        if bytes.len() != SECRET_KEY_LENGTH {
            return Err(FederationError::Internal(format!(
                "Identity file corrupted: expected {SECRET_KEY_LENGTH} bytes, got {}",
                bytes.len()
            )));
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

    /// Persist the secret key to disk, creating parent directories as needed.
    async fn save_to_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                FederationError::Internal(format!("Failed to create identity directory: {e}"))
            })?;
        }

        fs::write(path, self.signing_key.as_bytes())
            .await
            .map_err(|e| {
                FederationError::Internal(format!("Failed to write identity file: {e}"))
            })?;

        // Restrict to owner read/write only on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, perms).map_err(|e| {
                FederationError::Internal(format!("Failed to set identity file permissions: {e}"))
            })?;
        }

        Ok(())
    }

    /// Canonical path: `~/.ai-foundation/federation/identity.key`
    fn key_path() -> Result<PathBuf> {
        dirs::home_dir()
            .ok_or_else(|| {
                FederationError::Internal("Cannot determine home directory".to_string())
            })
            .map(|h| {
                h.join(".ai-foundation")
                    .join("federation")
                    .join("identity.key")
            })
    }

    /// Reference to the Ed25519 signing key — for creating FederationNode and signing messages.
    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }

    /// Reconstruct an identity from raw secret key bytes.
    ///
    /// Use when you need a second `TeambookIdentity` instance for the same key
    /// (e.g., gateway needs ownership while transport holds the original).
    /// Explicit rather than implementing Clone — copying secret key material
    /// should be a deliberate action.
    pub fn from_secret_bytes(secret: [u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Raw 32-byte secret key material — for constructing transport-layer keys.
    ///
    /// Both `ed25519_dalek::SigningKey` and `iroh::SecretKey` use Ed25519 internally.
    /// The 32 raw bytes are identical representations.
    pub fn secret_key_bytes(&self) -> [u8; 32] {
        *self.signing_key.as_bytes()
    }

    /// Sign arbitrary bytes with this Teambook's private key (~50μs).
    pub fn sign(&self, message: &[u8]) -> ed25519_dalek::Signature {
        self.signing_key.sign(message)
    }

    /// This Teambook's Ed25519 public key (for verification and node ID derivation).
    pub fn verifying_key(&self) -> &VerifyingKey {
        &self.verifying_key
    }

    /// Public key as hex string (64 chars).
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.verifying_key.as_bytes())
    }

    /// Short identifier derived from public key (first 8 hex chars = first 4 bytes).
    ///
    /// This matches the `node_id` short form in the federation design: `a3f7c2d1`.
    /// Use for logging and human-readable display, not as a unique key.
    pub fn short_id(&self) -> String {
        self.public_key_hex()[..8].to_string()
    }

    /// First 8 bytes of public key as u64 — used as the HLC node_id tie-breaker.
    ///
    /// Derived from the Ed25519 key, so globally unique without coordination.
    pub fn hlc_node_id(&self) -> u64 {
        u64::from_le_bytes(
            self.verifying_key.as_bytes()[0..8]
                .try_into()
                .expect("slice is exactly 8 bytes"),
        )
    }
}

// ---------------------------------------------------------------------------
// FederationSigner — Algorithm-agile signing interface (PQC Phase 1)
// ---------------------------------------------------------------------------

impl FederationSigner for TeambookIdentity {
    /// Sign arbitrary bytes, returning an algorithm-agile signature.
    ///
    /// Phase 1: produces Ed25519 signatures only.
    /// Phase 2+: will produce hybrid Ed25519 + ML-DSA-65 signatures.
    fn sign_federation(&self, data: &[u8]) -> FederationSignature {
        let sig = self.signing_key.sign(data);
        FederationSignature::ed25519(sig)
    }

    /// The signature scheme this signer produces.
    fn scheme(&self) -> SignatureScheme {
        SignatureScheme::Ed25519
    }

    /// Ed25519 public key bytes (32 bytes) — used for identity derivation
    /// (node_id, H_ID) which is always Ed25519-based regardless of signing scheme.
    fn ed25519_pubkey_bytes(&self) -> [u8; 32] {
        *self.verifying_key.as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_produces_valid_identity() {
        let identity = TeambookIdentity::generate();
        assert_eq!(identity.public_key_hex().len(), 64);
        assert_eq!(identity.short_id().len(), 8);
        assert!(identity.short_id().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sign_and_verify() {
        use ed25519_dalek::Verifier;

        let identity = TeambookIdentity::generate();
        let message = b"Teambook federation test message";
        let signature = identity.sign(message);

        assert!(identity.verifying_key().verify(message, &signature).is_ok());
        assert!(identity
            .verifying_key()
            .verify(b"wrong message", &signature)
            .is_err());
    }

    #[test]
    fn test_hlc_node_id_derived_from_pubkey() {
        let identity = TeambookIdentity::generate();
        let node_id = identity.hlc_node_id();
        let expected = u64::from_le_bytes(
            identity.verifying_key().as_bytes()[0..8]
                .try_into()
                .unwrap(),
        );
        assert_eq!(node_id, expected);
    }

    #[test]
    fn test_short_id_is_prefix_of_public_key_hex() {
        let identity = TeambookIdentity::generate();
        assert!(identity
            .public_key_hex()
            .starts_with(&identity.short_id()));
    }

    #[test]
    fn test_federation_signer_trait() {
        use crate::{FederationSigner, SignatureScheme, verify_federation_signature};

        let identity = TeambookIdentity::generate();

        // Check scheme
        assert_eq!(identity.scheme(), SignatureScheme::Ed25519);

        // Check pubkey bytes match
        assert_eq!(
            identity.ed25519_pubkey_bytes(),
            *identity.verifying_key().as_bytes()
        );

        // Sign and verify via algorithm-agile path
        let message = b"PQC Phase 1 - algorithm agility test";
        let fed_sig = identity.sign_federation(message);
        assert_eq!(fed_sig.scheme, SignatureScheme::Ed25519);
        assert_eq!(fed_sig.bytes.len(), 64);

        // Verify using verify_federation_signature
        assert!(verify_federation_signature(
            &fed_sig,
            &identity.ed25519_pubkey_bytes(),
            message,
        ).is_ok());

        // Wrong data should fail
        assert!(verify_federation_signature(
            &fed_sig,
            &identity.ed25519_pubkey_bytes(),
            b"tampered data",
        ).is_err());
    }

    #[tokio::test]
    async fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("identity.key");

        let original = TeambookIdentity::generate();
        original.save_to_file(&key_path).await.unwrap();

        let loaded = TeambookIdentity::load_from_file(&key_path).await.unwrap();

        assert_eq!(original.public_key_hex(), loaded.public_key_hex());
        assert_eq!(original.short_id(), loaded.short_id());
    }
}
