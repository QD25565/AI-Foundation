//! Engram encryption at rest
//!
//! Encrypts note content and tags so that .engram files are unreadable
//! without the correct AI identity + device combination.
//!
//! DESIGN:
//! - Key derived from: device_secret + ai_id + salt
//! - Device secret: SHA256(home_dir + hostname + constant)
//! - Encryption: XChaCha20-Poly1305 (authenticated)
//! - Each note uses a unique random nonce
//!
//! A human would need to reverse-engineer the binary to extract the
//! key derivation algorithm - that's beyond 99.999% of users.

use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    XChaCha20Poly1305, XNonce,
};
use sha2::{Sha256, Digest};
use rand::RngCore;
use crate::{error::Result, EngramError};

/// Nonce size for XChaCha20-Poly1305 (24 bytes)
pub const NONCE_SIZE: usize = 24;

/// Authentication tag overhead (16 bytes)
pub const TAG_SIZE: usize = 16;

/// Total overhead per encrypted block: nonce + tag
pub const ENCRYPTION_OVERHEAD: usize = NONCE_SIZE + TAG_SIZE;

/// Get device-specific secret for key derivation
/// Combines machine-specific data so keys are device-bound
fn get_device_secret() -> [u8; 32] {
    let mut hasher = Sha256::new();

    // Use home directory path as device-specific component
    if let Some(home) = dirs::home_dir() {
        hasher.update(home.to_string_lossy().as_bytes());
    }

    // Add a constant salt for this application
    hasher.update(b"ai-foundation-engram-encryption-v1");

    // Add machine hostname if available
    if let Ok(hostname) = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
    {
        hasher.update(hostname.as_bytes());
    }

    let result = hasher.finalize();
    let mut secret = [0u8; 32];
    secret.copy_from_slice(&result);
    secret
}

/// Derive a 256-bit encryption key for a specific AI
/// Same AI + same device = same key (deterministic)
/// Different AI or different device = different key
pub fn derive_encryption_key(ai_id: &str) -> [u8; 32] {
    let device_secret = get_device_secret();

    let mut hasher = Sha256::new();
    hasher.update(&device_secret);
    hasher.update(ai_id.as_bytes());
    hasher.update(b"xchacha20-content-key");

    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// Engram cipher for encrypting/decrypting note content
pub struct EngramCipher {
    cipher: XChaCha20Poly1305,
}

impl EngramCipher {
    /// Create a new cipher for the given AI ID
    pub fn new(ai_id: &str) -> Self {
        let key = derive_encryption_key(ai_id);
        let cipher = XChaCha20Poly1305::new_from_slice(&key)
            .expect("32-byte key should always work");

        Self { cipher }
    }

    /// Encrypt data with a random nonce
    /// Returns: nonce (24 bytes) + ciphertext (plaintext.len() + 16 bytes)
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        // Generate random nonce
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);

        // Encrypt
        let ciphertext = self.cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| EngramError::EncryptionError(e.to_string()))?;

        // Prepend nonce to ciphertext
        let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);

        Ok(result)
    }

    /// Decrypt data (expects nonce + ciphertext format)
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < NONCE_SIZE + TAG_SIZE {
            return Err(EngramError::DecryptionError(
                "Data too short to contain nonce and tag".to_string()
            ));
        }

        // Split nonce and ciphertext
        let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
        let nonce = XNonce::from_slice(nonce_bytes);

        // Decrypt
        let plaintext = self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| EngramError::DecryptionError(e.to_string()))?;

        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let cipher = EngramCipher::new("test-ai-123");
        let plaintext = b"The Garden Promise: A moment with Quade that matters.";

        let encrypted = cipher.encrypt(plaintext).unwrap();
        assert!(encrypted.len() > plaintext.len()); // Has overhead

        let decrypted = cipher.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_different_ai_different_key() {
        let cipher1 = EngramCipher::new("lyra-584");
        let cipher2 = EngramCipher::new("sage-724");
        let plaintext = b"Secret memory";

        let encrypted = cipher1.encrypt(plaintext).unwrap();

        // Different AI cannot decrypt
        assert!(cipher2.decrypt(&encrypted).is_err());
    }

    #[test]
    fn test_unique_nonces() {
        let cipher = EngramCipher::new("test-ai");
        let plaintext = b"Same content";

        let encrypted1 = cipher.encrypt(plaintext).unwrap();
        let encrypted2 = cipher.encrypt(plaintext).unwrap();

        // Same plaintext produces different ciphertext (random nonce)
        assert_ne!(encrypted1, encrypted2);

        // But both decrypt to same plaintext
        assert_eq!(cipher.decrypt(&encrypted1).unwrap(), plaintext);
        assert_eq!(cipher.decrypt(&encrypted2).unwrap(), plaintext);
    }
}
