//! Key Storage Abstraction
//!
//! Provides secure storage for signing keys with fallback chain:
//! 1. TPM 2.0 (Windows/Linux) - Keys never leave hardware
//! 2. OS Keychain (Windows Credential Manager, macOS Keychain, Linux Secret Service)
//! 3. Encrypted file - AES-256-GCM with password
//!
//! The abstraction ensures that signing operations work regardless of
//! storage backend, and private keys are protected at rest.
//!
//! ## PQC Migration (Phase 1: Algorithm Agility)
//!
//! The [`KeyMaterial`] enum wraps the signing key to support multiple
//! signature algorithms. Currently only Ed25519 is active.
//!
//! **Phase 2** will add `KeyMaterial::MlDsa65` for ML-DSA-65 (FIPS 204)
//! post-quantum signatures, and `KeyMaterial::HybridEd25519MlDsa65` for
//! composite Ed25519 + ML-DSA-65 signing. The `KeyPair` public API
//! remains stable across phases — callers are unaffected.
//!
//! **Phase 3** drops Ed25519 from new signatures (retained for identity
//! derivation and legacy verification only).

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use zeroize::Zeroize;

use crate::error::{AFPError, Result};

/// Identifies which signature algorithm a [`KeyPair`] uses.
///
/// Phase 1: Only `Ed25519` is active. Phase 2 adds `MlDsa65` and
/// `HybridEd25519MlDsa65`. Serialized as `u8` for compact wire representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SignatureAlgorithm {
    /// Classical Ed25519 (64-byte signatures, 32-byte keys)
    Ed25519 = 0,
    // Phase 2:
    // /// Post-quantum ML-DSA-65 / FIPS 204 (3,309-byte signatures, 1,952-byte pubkeys)
    // MlDsa65 = 1,
    // /// Hybrid: Ed25519 + ML-DSA-65 (both must verify). Composite signature = 3,373 bytes.
    // HybridEd25519MlDsa65 = 2,
}

/// Algorithm-agile key material wrapper.
///
/// Wraps the concrete signing key so that [`KeyPair`] can support multiple
/// signature algorithms without changing its public API. All match arms
/// currently resolve to the single `Ed25519` variant; Phase 2 adds ML-DSA-65.
#[derive(Clone)]
pub enum KeyMaterial {
    /// Classical Ed25519 signing key (32-byte seed)
    Ed25519(SigningKey),
    // Phase 2:
    // /// Post-quantum ML-DSA-65 signing key (4,032 bytes)
    // MlDsa65(ml_dsa::MlDsa65SigningKey),
    // /// Hybrid: holds both keys, signs with both algorithms
    // HybridEd25519MlDsa65 { ed25519: SigningKey, ml_dsa: ml_dsa::MlDsa65SigningKey },
}

/// Key pair for signing (public key can be shared, private key is protected)
#[derive(Clone)]
pub struct KeyPair {
    /// Algorithm-agile key material
    key: KeyMaterial,
}

impl KeyPair {
    /// Generate a new random Ed25519 key pair
    pub fn generate() -> Self {
        let mut rng = rand::thread_rng();
        let signing_key = SigningKey::generate(&mut rng);
        Self { key: KeyMaterial::Ed25519(signing_key) }
    }

    /// Create from existing Ed25519 signing key bytes
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self> {
        let signing_key = SigningKey::from_bytes(bytes);
        Ok(Self { key: KeyMaterial::Ed25519(signing_key) })
    }

    /// Which signature algorithm this key pair uses
    pub fn algorithm(&self) -> SignatureAlgorithm {
        match &self.key {
            KeyMaterial::Ed25519(_) => SignatureAlgorithm::Ed25519,
        }
    }

    /// Get the public key
    pub fn public_key(&self) -> VerifyingKey {
        match &self.key {
            KeyMaterial::Ed25519(sk) => sk.verifying_key(),
        }
    }

    /// Get the private key bytes (use with caution!)
    pub fn private_bytes(&self) -> [u8; 32] {
        match &self.key {
            KeyMaterial::Ed25519(sk) => sk.to_bytes(),
        }
    }

    /// Sign a message
    pub fn sign(&self, message: &[u8]) -> Signature {
        match &self.key {
            KeyMaterial::Ed25519(sk) => sk.sign(message),
        }
    }

    /// Verify a signature (convenience method)
    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<()> {
        use ed25519_dalek::Verifier;
        match &self.key {
            KeyMaterial::Ed25519(sk) => {
                sk.verifying_key()
                    .verify(message, signature)
                    .map_err(|_| AFPError::SignatureVerificationFailed)
            }
        }
    }

    /// Access the underlying key material (for storage backends)
    pub(crate) fn key_material(&self) -> &KeyMaterial {
        &self.key
    }
}

/// Abstract trait for key storage backends
pub trait KeyStorage: Send + Sync {
    /// Get the name of this storage backend
    fn name(&self) -> &'static str;

    /// Check if this storage backend is available
    fn is_available(&self) -> bool;

    /// Generate a new key pair and store it
    fn generate_and_store(&self, key_id: &str) -> Result<VerifyingKey>;

    /// Load an existing key pair
    fn load(&self, key_id: &str) -> Result<KeyPair>;

    /// Check if a key exists
    fn exists(&self, key_id: &str) -> bool;

    /// Delete a key
    fn delete(&self, key_id: &str) -> Result<()>;

    /// Sign a message using the stored key
    fn sign(&self, key_id: &str, message: &[u8]) -> Result<Signature> {
        let keypair = self.load(key_id)?;
        Ok(keypair.sign(message))
    }
}

/// Keychain-based storage (uses OS credential manager)
pub struct KeychainStorage {
    service_name: String,
}

impl KeychainStorage {
    pub fn new(service_name: &str) -> Self {
        Self {
            service_name: service_name.to_string(),
        }
    }

    fn keyring_entry(&self, key_id: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service_name, key_id)
            .map_err(|e| AFPError::KeyStorageUnavailable(format!("keyring entry creation failed: {}", e)))
    }
}

impl KeyStorage for KeychainStorage {
    fn name(&self) -> &'static str {
        "OS Keychain"
    }

    fn is_available(&self) -> bool {
        // Test by actually storing and retrieving a test value using
        // SEPARATE Entry objects - this catches mock backends that don't
        // persist across Entry::new() calls
        let entry1 = match keyring::Entry::new(&self.service_name, "__afp_test__") {
            Ok(e) => e,
            Err(_) => return false,
        };

        // Try to set a test password
        if entry1.set_password("__test_value_12345__").is_err() {
            return false;
        }

        // Create a NEW Entry object to verify persistence
        let entry2 = match keyring::Entry::new(&self.service_name, "__afp_test__") {
            Ok(e) => e,
            Err(_) => return false,
        };

        // Try to retrieve from the NEW entry
        let result = entry2.get_password();
        let _ = entry2.delete_credential(); // Clean up

        match result {
            Ok(val) => val == "__test_value_12345__",
            Err(_) => false,
        }
    }

    fn generate_and_store(&self, key_id: &str) -> Result<VerifyingKey> {
        let keypair = KeyPair::generate();
        let mut private_bytes = keypair.private_bytes();

        // Store as base64
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &private_bytes,
        );
        private_bytes.zeroize();

        self.keyring_entry(key_id)?
            .set_password(&encoded)
            .map_err(|e| AFPError::KeyStorageUnavailable(e.to_string()))?;

        Ok(keypair.public_key())
    }

    fn load(&self, key_id: &str) -> Result<KeyPair> {
        let encoded = self
            .keyring_entry(key_id)?
            .get_password()
            .map_err(|_| AFPError::KeyNotFound)?;

        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &encoded,
        )
        .map_err(|e| AFPError::Internal(format!("Base64 decode failed: {}", e)))?;

        if bytes.len() != 32 {
            return Err(AFPError::Internal("Invalid key length".to_string()));
        }

        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&bytes);
        KeyPair::from_bytes(&key_bytes)
    }

    fn exists(&self, key_id: &str) -> bool {
        self.keyring_entry(key_id)
            .map(|entry| entry.get_password().is_ok())
            .unwrap_or(false)
    }

    fn delete(&self, key_id: &str) -> Result<()> {
        self.keyring_entry(key_id)?
            .delete_credential()
            .map_err(|e| AFPError::Internal(e.to_string()))
    }
}

/// File-based storage with encryption
pub struct FileStorage {
    base_path: PathBuf,
    encryption_key: Option<[u8; 32]>,
}

impl FileStorage {
    /// Create file storage with optional encryption
    pub fn new(base_path: PathBuf, password: Option<&str>) -> Self {
        let encryption_key = password.map(|p| {
            let mut hasher = Sha256::new();
            hasher.update(b"AFP_FILE_KEY:");
            hasher.update(p.as_bytes());
            hasher.finalize().into()
        });

        Self {
            base_path,
            encryption_key,
        }
    }

    fn key_path(&self, key_id: &str) -> PathBuf {
        self.base_path.join(format!("{}.key", key_id))
    }

    /// AES-256-GCM authenticated encryption for key files.
    ///
    /// File format: `[nonce:12][ciphertext+tag:N+16]`
    /// For Ed25519 private keys (32 bytes): total file = 60 bytes.
    fn encrypt(&self, data: &[u8]) -> Vec<u8> {
        match &self.encryption_key {
            Some(key) => {
                use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
                use aes_gcm::aead::Aead;
                use rand::RngCore;

                let cipher = Aes256Gcm::new(aes_gcm::Key::<Aes256Gcm>::from_slice(key));
                let mut nonce_bytes = [0u8; 12];
                rand::thread_rng().fill_bytes(&mut nonce_bytes);
                let nonce = Nonce::from_slice(&nonce_bytes);

                let ciphertext = cipher.encrypt(nonce, data)
                    .expect("AES-256-GCM encryption failed");

                let mut out = Vec::with_capacity(12 + ciphertext.len());
                out.extend_from_slice(&nonce_bytes);
                out.extend_from_slice(&ciphertext);
                out
            }
            None => data.to_vec(),
        }
    }

    fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        match &self.encryption_key {
            Some(key) => {
                // New AES-256-GCM format: [nonce:12][ciphertext+tag]
                // Minimum size: 12 (nonce) + 16 (tag) = 28 bytes
                if data.len() >= 28 {
                    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
                    use aes_gcm::aead::Aead;

                    let cipher = Aes256Gcm::new(aes_gcm::Key::<Aes256Gcm>::from_slice(key));
                    let nonce = Nonce::from_slice(&data[..12]);
                    cipher.decrypt(nonce, &data[12..])
                        .map_err(|_| AFPError::Internal(
                            "Decryption failed (wrong password or corrupted key file)".to_string()
                        ))
                } else {
                    // File too small for any valid format
                    Err(AFPError::Internal("Key file too small".to_string()))
                }
            }
            None => Ok(data.to_vec()),
        }
    }
}

impl KeyStorage for FileStorage {
    fn name(&self) -> &'static str {
        "Encrypted File"
    }

    fn is_available(&self) -> bool {
        // Check if we can write to the base path
        if !self.base_path.exists() {
            if let Err(_) = std::fs::create_dir_all(&self.base_path) {
                return false;
            }
        }
        true
    }

    fn generate_and_store(&self, key_id: &str) -> Result<VerifyingKey> {
        let keypair = KeyPair::generate();
        let mut private_bytes = keypair.private_bytes();

        // Encrypt and store
        let encrypted = self.encrypt(&private_bytes);
        private_bytes.zeroize();

        std::fs::create_dir_all(&self.base_path)?;
        std::fs::write(self.key_path(key_id), &encrypted)?;

        Ok(keypair.public_key())
    }

    fn load(&self, key_id: &str) -> Result<KeyPair> {
        let path = self.key_path(key_id);
        if !path.exists() {
            return Err(AFPError::KeyNotFound);
        }

        let encrypted = std::fs::read(&path)?;
        let decrypted = self.decrypt(&encrypted)?;

        if decrypted.len() != 32 {
            return Err(AFPError::Internal("Invalid key length".to_string()));
        }

        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&decrypted);
        let result = KeyPair::from_bytes(&key_bytes);
        key_bytes.zeroize(); // Volatile zeroization — guaranteed not optimized away
        result
    }

    fn exists(&self, key_id: &str) -> bool {
        self.key_path(key_id).exists()
    }

    fn delete(&self, key_id: &str) -> Result<()> {
        let path = self.key_path(key_id);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

/// Composite storage that tries multiple backends in order
pub struct FallbackStorage {
    backends: Vec<Box<dyn KeyStorage>>,
}

impl FallbackStorage {
    pub fn new() -> Self {
        Self {
            backends: Vec::new(),
        }
    }

    pub fn add_backend(mut self, backend: Box<dyn KeyStorage>) -> Self {
        self.backends.push(backend);
        self
    }

    /// Create the default fallback chain for the current platform
    ///
    /// Priority order:
    /// 1. TPM 2.0 — keys sealed by hardware, never on disk in plaintext
    /// 2. OS Keychain — Windows Credential Manager / macOS Keychain / Linux Secret Service
    /// 3. Encrypted file — AES-256-GCM authenticated encryption on disk
    pub fn default_chain(ai_id: &str) -> Self {
        let mut chain = Self::new();

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

        // 1. Try TPM 2.0 first (keys never leave hardware)
        let tpm_meta_dir = home.join(".ai-foundation").join("tpm");
        let tpm = crate::tpm::TpmStorage::new(tpm_meta_dir);
        if tpm.is_available() {
            chain = chain.add_backend(Box::new(tpm));
        }

        // 2. Try OS Keychain
        let keychain = KeychainStorage::new("ai-foundation");
        if keychain.is_available() {
            chain = chain.add_backend(Box::new(keychain));
        }

        // 3. Fall back to encrypted file
        let key_dir = home.join(".ai-foundation").join("keys");
        let file_storage = FileStorage::new(key_dir, Some(ai_id));
        chain = chain.add_backend(Box::new(file_storage));

        chain
    }

    fn first_available(&self) -> Result<&dyn KeyStorage> {
        self.backends
            .iter()
            .find(|b| b.is_available())
            .map(|b| b.as_ref())
            .ok_or_else(|| AFPError::KeyStorageUnavailable("No backends available".to_string()))
    }
}

impl KeyStorage for FallbackStorage {
    fn name(&self) -> &'static str {
        "Fallback Chain"
    }

    fn is_available(&self) -> bool {
        self.backends.iter().any(|b| b.is_available())
    }

    fn generate_and_store(&self, key_id: &str) -> Result<VerifyingKey> {
        self.first_available()?.generate_and_store(key_id)
    }

    fn load(&self, key_id: &str) -> Result<KeyPair> {
        // Try each backend until we find the key
        for backend in &self.backends {
            if backend.is_available() && backend.exists(key_id) {
                return backend.load(key_id);
            }
        }
        Err(AFPError::KeyNotFound)
    }

    fn exists(&self, key_id: &str) -> bool {
        self.backends
            .iter()
            .any(|b| b.is_available() && b.exists(key_id))
    }

    fn delete(&self, key_id: &str) -> Result<()> {
        for backend in &self.backends {
            if backend.is_available() && backend.exists(key_id) {
                return backend.delete(key_id);
            }
        }
        Ok(())
    }
}

impl Default for FallbackStorage {
    fn default() -> Self {
        Self::new()
    }
}

/// Stored identity (combines key storage with identity info)
///
/// **PQC Note:** `pubkey` stays `[u8; 32]` (Ed25519) in Phase 1. Phase 2 adds
/// a separate `pqc_pubkey: Option<Vec<u8>>` field for ML-DSA-65 public keys
/// (1,952 bytes). The Ed25519 pubkey is retained permanently for H_ID and
/// node_id derivation (see PQC-TRANSITION-ARCHITECTURE.md Decisions 1 & 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredIdentity {
    pub ai_id: String,
    pub pubkey: [u8; 32],
    pub teambook: String,
    pub created_at: i64,
    // Phase 2: pub pqc_pubkey: Option<Vec<u8>>,
    // Phase 2: pub algorithm: SignatureAlgorithm,
}

impl StoredIdentity {
    pub fn save_metadata(path: &PathBuf, identity: &StoredIdentity) -> Result<()> {
        let json = serde_json::to_string_pretty(identity)
            .map_err(|e| AFPError::SerializationFailed(e.to_string()))?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load_metadata(path: &PathBuf) -> Result<StoredIdentity> {
        let json = std::fs::read_to_string(path)?;
        serde_json::from_str(&json)
            .map_err(|e| AFPError::DeserializationFailed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_keypair_generation() {
        let kp = KeyPair::generate();
        let message = b"Hello, world!";
        let signature = kp.sign(message);
        assert!(kp.verify(message, &signature).is_ok());
    }

    #[test]
    fn test_keypair_algorithm() {
        let kp = KeyPair::generate();
        assert_eq!(kp.algorithm(), SignatureAlgorithm::Ed25519);
    }

    #[test]
    fn test_key_material_ed25519_roundtrip() {
        let kp = KeyPair::generate();
        let bytes = kp.private_bytes();
        let restored = KeyPair::from_bytes(&bytes).unwrap();
        assert_eq!(kp.public_key(), restored.public_key());
        assert_eq!(restored.algorithm(), SignatureAlgorithm::Ed25519);

        // Cross-verify: sign with original, verify with restored
        let msg = b"PQC migration test";
        let sig = kp.sign(msg);
        assert!(restored.verify(msg, &sig).is_ok());
    }

    #[test]
    fn test_signature_algorithm_serde() {
        let algo = SignatureAlgorithm::Ed25519;
        let json = serde_json::to_string(&algo).unwrap();
        let restored: SignatureAlgorithm = serde_json::from_str(&json).unwrap();
        assert_eq!(algo, restored);
    }

    #[test]
    fn test_file_storage() {
        let dir = tempdir().unwrap();
        let storage = FileStorage::new(dir.path().to_path_buf(), Some("test-password"));

        assert!(storage.is_available());

        // Generate and store
        let pubkey = storage.generate_and_store("test-key").unwrap();

        // Load
        let loaded = storage.load("test-key").unwrap();
        assert_eq!(loaded.public_key(), pubkey);

        // Sign with loaded key
        let message = b"Test message";
        let sig = loaded.sign(message);
        assert!(loaded.verify(message, &sig).is_ok());

        // Delete
        storage.delete("test-key").unwrap();
        assert!(!storage.exists("test-key"));
    }

    #[test]
    fn test_file_storage_encryption() {
        let dir = tempdir().unwrap();

        // Store with encryption
        let storage1 = FileStorage::new(dir.path().to_path_buf(), Some("password123"));
        storage1.generate_and_store("encrypted-key").unwrap();

        // Load with wrong password — AES-256-GCM auth tag rejects wrong key
        let storage2 = FileStorage::new(dir.path().to_path_buf(), Some("wrong-password"));
        let result = storage2.load("encrypted-key");
        assert!(result.is_err(), "AES-GCM should reject wrong password");
    }
}
