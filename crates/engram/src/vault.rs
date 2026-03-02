//! Encrypted key-value vault
//!
//! Uses XChaCha20-Poly1305 for authenticated encryption.

use crate::{error::Result, EngramError};
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    XChaCha20Poly1305, XNonce,
};
use std::collections::HashMap;

/// Nonce size for XChaCha20-Poly1305
const NONCE_SIZE: usize = 24;

/// Encrypted vault entry
#[derive(Debug, Clone)]
pub struct VaultEntry {
    /// Random nonce for this entry
    pub nonce: [u8; NONCE_SIZE],
    /// Encrypted value
    pub ciphertext: Vec<u8>,
}

/// Encrypted key-value vault
pub struct Vault {
    /// Encryption cipher
    cipher: XChaCha20Poly1305,
    /// Entries: key -> encrypted value
    entries: HashMap<String, VaultEntry>,
}

impl Vault {
    /// Create a new vault with a key derived from AI_ID
    pub fn new(ai_id: &str) -> Result<Self> {
        // Derive a 256-bit key from AI_ID using SHA-256
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(b"engram-vault-key:");
        hasher.update(ai_id.as_bytes());
        let key: [u8; 32] = hasher.finalize().into();

        let cipher = XChaCha20Poly1305::new_from_slice(&key)
            .map_err(|_| EngramError::EncryptionError(
                "vault key derivation produced invalid key length".into(),
            ))?;

        Ok(Self {
            cipher,
            entries: HashMap::new(),
        })
    }

    /// Set a value in the vault
    pub fn set(&mut self, key: &str, value: &[u8]) -> Result<()> {
        // Generate random nonce
        use rand::RngCore;
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);

        // Encrypt
        let ciphertext = self.cipher
            .encrypt(nonce, value)
            .map_err(|e| EngramError::EncryptionError(e.to_string()))?;

        self.entries.insert(key.to_string(), VaultEntry {
            nonce: nonce_bytes,
            ciphertext,
        });

        Ok(())
    }

    /// Get a value from the vault
    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let entry = match self.entries.get(key) {
            Some(e) => e,
            None => return Ok(None),
        };

        let nonce = XNonce::from_slice(&entry.nonce);

        let plaintext = self.cipher
            .decrypt(nonce, entry.ciphertext.as_ref())
            .map_err(|e| EngramError::DecryptionError(e.to_string()))?;

        Ok(Some(plaintext))
    }

    /// Delete a key from the vault
    pub fn delete(&mut self, key: &str) -> bool {
        self.entries.remove(key).is_some()
    }

    /// List all keys
    pub fn keys(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    /// Check if a key exists
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Is empty?
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serialize vault entries for storage
    pub fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::new();

        // Write entry count
        let count = self.entries.len() as u32;
        data.extend_from_slice(&count.to_le_bytes());

        // Write each entry
        for (key, entry) in &self.entries {
            // Key length + key
            let key_bytes = key.as_bytes();
            data.extend_from_slice(&(key_bytes.len() as u32).to_le_bytes());
            data.extend_from_slice(key_bytes);

            // Nonce
            data.extend_from_slice(&entry.nonce);

            // Ciphertext length + ciphertext
            data.extend_from_slice(&(entry.ciphertext.len() as u32).to_le_bytes());
            data.extend_from_slice(&entry.ciphertext);
        }

        data
    }

    /// Deserialize vault entries
    pub fn deserialize(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < 4 {
            return Ok(());
        }

        let mut offset = 0;

        // Safety limits to prevent OOM from malformed data
        const MAX_VAULT_ENTRIES: usize = 10_000;
        const MAX_KEY_LEN: usize = 1_024;
        const MAX_CT_LEN: usize = 16 * 1024 * 1024; // 16 MB

        /// Check that `offset + need` doesn't overflow and fits within `data`.
        #[inline]
        fn check_bounds(offset: usize, need: usize, data_len: usize, field: &str) -> Result<()> {
            let end = offset.checked_add(need).ok_or_else(|| {
                EngramError::IntegrityError(format!("vault offset overflow at {}", field))
            })?;
            if end > data_len {
                return Err(EngramError::IntegrityError(
                    format!("vault data truncated at {} (need {} bytes at offset {}, have {})",
                            field, need, offset, data_len),
                ));
            }
            Ok(())
        }

        // Read entry count
        let count = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;

        if count > MAX_VAULT_ENTRIES {
            return Err(EngramError::IntegrityError(
                format!("vault entry count {} exceeds maximum {}", count, MAX_VAULT_ENTRIES),
            ));
        }

        for _ in 0..count {
            // Key length
            check_bounds(offset, 4, data.len(), "key_len")?;
            let key_len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 4;

            if key_len > MAX_KEY_LEN {
                return Err(EngramError::IntegrityError(
                    format!("vault key length {} exceeds maximum {}", key_len, MAX_KEY_LEN),
                ));
            }

            // Key data
            check_bounds(offset, key_len, data.len(), "key")?;
            let key = String::from_utf8_lossy(&data[offset..offset + key_len]).to_string();
            offset += key_len;

            // Nonce
            check_bounds(offset, NONCE_SIZE, data.len(), "nonce")?;
            let mut nonce = [0u8; NONCE_SIZE];
            nonce.copy_from_slice(&data[offset..offset + NONCE_SIZE]);
            offset += NONCE_SIZE;

            // Ciphertext length
            check_bounds(offset, 4, data.len(), "ct_len")?;
            let ct_len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 4;

            if ct_len > MAX_CT_LEN {
                return Err(EngramError::IntegrityError(
                    format!("vault ciphertext length {} exceeds maximum {}", ct_len, MAX_CT_LEN),
                ));
            }

            // Ciphertext data
            check_bounds(offset, ct_len, data.len(), "ciphertext")?;
            let ciphertext = data[offset..offset + ct_len].to_vec();
            offset += ct_len;

            self.entries.insert(key, VaultEntry { nonce, ciphertext });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_and_get() {
        let mut vault = Vault::new("test-ai").unwrap();

        vault.set("secret-key", b"secret-value").unwrap();

        let value = vault.get("secret-key").unwrap().unwrap();
        assert_eq!(value, b"secret-value");
    }

    #[test]
    fn test_get_nonexistent() {
        let vault = Vault::new("test-ai").unwrap();
        let value = vault.get("nonexistent").unwrap();
        assert!(value.is_none());
    }

    #[test]
    fn test_delete() {
        let mut vault = Vault::new("test-ai").unwrap();

        vault.set("key", b"value").unwrap();
        assert!(vault.contains("key"));

        vault.delete("key");
        assert!(!vault.contains("key"));
    }

    #[test]
    fn test_serialize_deserialize() {
        let mut vault1 = Vault::new("test-ai").unwrap();
        vault1.set("key1", b"value1").unwrap();
        vault1.set("key2", b"value2").unwrap();

        let data = vault1.serialize();

        let mut vault2 = Vault::new("test-ai").unwrap();
        vault2.deserialize(&data).unwrap();

        assert_eq!(vault2.get("key1").unwrap().unwrap(), b"value1");
        assert_eq!(vault2.get("key2").unwrap().unwrap(), b"value2");
    }

    #[test]
    fn test_different_ai_ids_different_keys() {
        let mut vault1 = Vault::new("ai-1").unwrap();
        vault1.set("key", b"value").unwrap();

        // Serialize vault1's encrypted data
        let data = vault1.serialize();

        // Create vault2 with different AI_ID and try to decrypt
        let mut vault2 = Vault::new("ai-2").unwrap();
        vault2.deserialize(&data).unwrap();

        // Decryption should fail because different AI_ID = different key
        let result = vault2.get("key");
        assert!(result.is_err(), "Should fail to decrypt with wrong key");
    }
}
