//! Encryption at Rest for .teamengram Event Log
//!
//! AES-256-GCM authenticated encryption for event payloads.
//! Encryption is transparent to higher layers — the event_log writer encrypts
//! on append and the reader decrypts on read.
//!
//! Design:
//! - **Event payloads only** — headers stay plaintext (needed for seeking, compaction, routing)
//! - **Compress-then-encrypt** — compression runs before encryption (correct order)
//! - **Per-event nonce** — `sequence(8) || event_type(2) || 0x0000(2)` = 12 bytes.
//!   Uniqueness guaranteed by monotonically increasing sequence numbers.
//! - **HKDF-SHA256 key derivation** — from TPM-sealed key material + H_ID salt
//! - **Backward compatible** — FLAG_ENCRYPTED in event header flags; unencrypted files
//!   remain readable. Mixed encrypted/unencrypted events in the same log are supported
//!   (migration path: new events encrypted, old events readable).
//!
//! Overhead: 16 bytes per event (GCM authentication tag). Negligible at ~100 events/sec.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};

/// Flag indicating event payload is encrypted (bit 1 in EventHeader.flags).
/// Coexists with FLAG_COMPRESSED (bit 0, 0x0001).
pub const FLAG_ENCRYPTED: u16 = 0x0002;

/// AES-GCM authentication tag size (bytes)
pub const GCM_TAG_SIZE: usize = 16;

/// AES-GCM nonce size (96 bits / 12 bytes)
pub const NONCE_SIZE: usize = 12;

/// Encryption errors
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("encryption failed")]
    EncryptionFailed,

    #[error("decryption failed (wrong key or corrupted data)")]
    DecryptionFailed,

    #[error("key derivation failed: {0}")]
    KeyDerivationFailed(String),
}

/// AES-256-GCM encryption context for teamengram event payloads.
///
/// Thread-safe. Wrap in `Arc` for shared use across EventLogWriter and EventLogReader.
pub struct TeamEngramCrypto {
    cipher: Aes256Gcm,
}

impl TeamEngramCrypto {
    /// Create from a raw 32-byte AES-256 key.
    ///
    /// Use `from_key_material` for production (derives key via HKDF).
    /// Use `new` directly only in tests or when key is already derived.
    pub fn new(key_bytes: &[u8; 32]) -> Self {
        let key = Key::<Aes256Gcm>::from_slice(key_bytes);
        Self {
            cipher: Aes256Gcm::new(key),
        }
    }

    /// Derive AES-256-GCM encryption key from input key material using HKDF-SHA256.
    ///
    /// # Arguments
    /// * `ikm` — Input key material (e.g., TPM-sealed Ed25519 private key, 32 bytes)
    /// * `salt` — Identity binding (e.g., H_ID bytes from `SHA256(pubkey || ai_id)`)
    ///
    /// The derived key is bound to both the hardware identity (via key material)
    /// and the AI identity (via H_ID salt). Different AIs on the same machine
    /// get different encryption keys.
    pub fn from_key_material(ikm: &[u8], salt: &[u8]) -> Result<Self, CryptoError> {
        use hkdf::Hkdf;
        use sha2::Sha256;

        // Minimum lengths to prevent weak key derivation.
        // Production: ikm = 32-byte TPM-sealed key, salt = 32-byte H_ID hash.
        const MIN_IKM_LEN: usize = 16;
        const MIN_SALT_LEN: usize = 16;

        if ikm.len() < MIN_IKM_LEN {
            return Err(CryptoError::KeyDerivationFailed(
                format!("input key material too short ({} bytes, minimum {})", ikm.len(), MIN_IKM_LEN),
            ));
        }
        if salt.len() < MIN_SALT_LEN {
            return Err(CryptoError::KeyDerivationFailed(
                format!("salt too short ({} bytes, minimum {})", salt.len(), MIN_SALT_LEN),
            ));
        }

        let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
        let mut key_bytes = [0u8; 32];
        hk.expand(b"teamengram-aes256gcm-v1", &mut key_bytes)
            .map_err(|e| CryptoError::KeyDerivationFailed(e.to_string()))?;
        let crypto = Self::new(&key_bytes);
        // Best-effort zeroization (compiler may optimize away; use `zeroize` crate for volatile)
        key_bytes.fill(0);
        Ok(crypto)
    }

    /// Encrypt an event payload.
    ///
    /// Nonce is deterministically derived from the event's sequence number
    /// and type. Since sequence numbers are monotonically increasing and never
    /// reused, nonce uniqueness is guaranteed without random generation.
    ///
    /// Returns ciphertext (`plaintext.len() + 16` bytes with GCM auth tag appended).
    pub fn encrypt_payload(
        &self,
        plaintext: &[u8],
        sequence: u64,
        event_type: u16,
    ) -> Result<Vec<u8>, CryptoError> {
        let nonce = Self::event_nonce(sequence, event_type);
        self.cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|_| CryptoError::EncryptionFailed)
    }

    /// Decrypt an event payload.
    ///
    /// Verifies the GCM authentication tag — returns error if the data was
    /// tampered with or the wrong key is used.
    pub fn decrypt_payload(
        &self,
        ciphertext: &[u8],
        sequence: u64,
        event_type: u16,
    ) -> Result<Vec<u8>, CryptoError> {
        let nonce = Self::event_nonce(sequence, event_type);
        self.cipher
            .decrypt(Nonce::from_slice(&nonce), ciphertext)
            .map_err(|_| CryptoError::DecryptionFailed)
    }

    /// Encrypt B+Tree page data area.
    ///
    /// Nonce: `page_id(8 LE) || txn_id_low32(4 LE)`.
    /// Shadow paging guarantees page_id is never reused for different content
    /// at the same txn_id, so nonce uniqueness holds.
    pub fn encrypt_page_data(
        &self,
        plaintext: &[u8],
        page_id: u64,
        txn_id: u64,
    ) -> Result<Vec<u8>, CryptoError> {
        let nonce = Self::page_nonce(page_id, txn_id);
        self.cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|_| CryptoError::EncryptionFailed)
    }

    /// Decrypt B+Tree page data area.
    pub fn decrypt_page_data(
        &self,
        ciphertext: &[u8],
        page_id: u64,
        txn_id: u64,
    ) -> Result<Vec<u8>, CryptoError> {
        let nonce = Self::page_nonce(page_id, txn_id);
        self.cipher
            .decrypt(Nonce::from_slice(&nonce), ciphertext)
            .map_err(|_| CryptoError::DecryptionFailed)
    }

    /// Build deterministic nonce for event log payloads (12 bytes).
    ///
    /// `[0..8) sequence (u64 LE)  |  [8..10) event_type (u16 LE)  |  [10..12) zero`
    #[inline]
    fn event_nonce(sequence: u64, event_type: u16) -> [u8; NONCE_SIZE] {
        let mut nonce = [0u8; NONCE_SIZE];
        nonce[0..8].copy_from_slice(&sequence.to_le_bytes());
        nonce[8..10].copy_from_slice(&event_type.to_le_bytes());
        nonce
    }

    /// Build deterministic nonce for B+Tree pages (12 bytes).
    ///
    /// `[0..8) page_id (u64 LE)  |  [8..12) txn_id lower 32 bits (u32 LE)`
    ///
    /// **Truncation note:** Only the lower 32 bits of `txn_id` are used.
    /// This means two txn_ids that differ only in their upper 32 bits would
    /// produce the same nonce for the same page_id. In practice this is safe:
    /// shadow paging never writes the same page_id twice within a single
    /// transaction, and reaching 2^32 transactions (~4 billion) would require
    /// decades of continuous operation at our write rates. If txn_id space
    /// is ever extended beyond u32 range, this nonce construction must be
    /// revisited (e.g., hash-based nonce derivation).
    #[inline]
    fn page_nonce(page_id: u64, txn_id: u64) -> [u8; NONCE_SIZE] {
        let mut nonce = [0u8; NONCE_SIZE];
        nonce[0..8].copy_from_slice(&page_id.to_le_bytes());
        nonce[8..12].copy_from_slice(&(txn_id as u32).to_le_bytes());
        nonce
    }
}

impl std::fmt::Debug for TeamEngramCrypto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TeamEngramCrypto")
            .field("algorithm", &"AES-256-GCM")
            .finish()
    }
}

/// Name of the encryption key file within the data directory.
pub const ENCRYPTION_KEY_FILE: &str = "encryption.key";

/// Load encryption context from the data directory.
///
/// Looks for `encryption.key` (32 raw bytes) in `data_dir`.
/// Returns `Ok(Some(crypto))` if key exists and is valid,
/// `Ok(None)` if no key file exists (encryption disabled),
/// or `Err` if the file exists but is malformed.
pub fn load_encryption_key(data_dir: &std::path::Path) -> Result<Option<TeamEngramCrypto>, CryptoError> {
    let key_path = data_dir.join(ENCRYPTION_KEY_FILE);

    // Atomic read — no TOCTOU race between exists() and read().
    // Handle NotFound as "encryption disabled", all other IO errors propagate.
    let key_bytes = match std::fs::read(&key_path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(CryptoError::KeyDerivationFailed(
            format!("failed to read {}: {}", key_path.display(), e)
        )),
    };

    if key_bytes.len() != 32 {
        return Err(CryptoError::KeyDerivationFailed(
            format!("encryption.key must be exactly 32 bytes, got {}", key_bytes.len())
        ));
    }

    let mut key = [0u8; 32];
    key.copy_from_slice(&key_bytes);
    let crypto = TeamEngramCrypto::new(&key);

    // Zero the temporary copy
    key.fill(0);

    Ok(Some(crypto))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = i as u8;
        }
        key
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let crypto = TeamEngramCrypto::new(&test_key());
        let plaintext = b"Hello, encrypted teamengram!";
        let sequence = 42u64;
        let event_type = 0x0002u16; // DM

        let ciphertext = crypto
            .encrypt_payload(plaintext, sequence, event_type)
            .unwrap();

        assert_eq!(ciphertext.len(), plaintext.len() + GCM_TAG_SIZE);
        assert_ne!(&ciphertext[..plaintext.len()], &plaintext[..]);

        let decrypted = crypto
            .decrypt_payload(&ciphertext, sequence, event_type)
            .unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn wrong_key_fails_decryption() {
        let crypto1 = TeamEngramCrypto::new(&test_key());
        let mut wrong_key = test_key();
        wrong_key[0] ^= 0xFF;
        let crypto2 = TeamEngramCrypto::new(&wrong_key);

        let ciphertext = crypto1.encrypt_payload(b"secret", 1, 0x0002).unwrap();
        let result = crypto2.decrypt_payload(&ciphertext, 1, 0x0002);
        assert!(result.is_err());
    }

    #[test]
    fn wrong_sequence_fails_decryption() {
        let crypto = TeamEngramCrypto::new(&test_key());
        let ciphertext = crypto.encrypt_payload(b"secret", 1, 0x0002).unwrap();
        let result = crypto.decrypt_payload(&ciphertext, 2, 0x0002);
        assert!(result.is_err());
    }

    #[test]
    fn wrong_event_type_fails_decryption() {
        let crypto = TeamEngramCrypto::new(&test_key());
        let ciphertext = crypto.encrypt_payload(b"secret", 1, 0x0002).unwrap();
        let result = crypto.decrypt_payload(&ciphertext, 1, 0x0001);
        assert!(result.is_err());
    }

    #[test]
    fn tampered_ciphertext_fails_decryption() {
        let crypto = TeamEngramCrypto::new(&test_key());
        let mut ciphertext = crypto
            .encrypt_payload(b"secret data", 1, 0x0002)
            .unwrap();
        ciphertext[0] ^= 0x01;
        let result = crypto.decrypt_payload(&ciphertext, 1, 0x0002);
        assert!(result.is_err());
    }

    #[test]
    fn empty_payload_encrypts() {
        let crypto = TeamEngramCrypto::new(&test_key());
        let ciphertext = crypto.encrypt_payload(b"", 1, 0x0001).unwrap();
        assert_eq!(ciphertext.len(), GCM_TAG_SIZE);

        let decrypted = crypto.decrypt_payload(&ciphertext, 1, 0x0001).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn hkdf_key_derivation() {
        let ikm = b"fake-tpm-sealed-private-key-bytes!!";
        let salt = b"fake-h_id-sha256-hash-32-bytes!!";

        let crypto = TeamEngramCrypto::from_key_material(ikm, salt).unwrap();
        let ciphertext = crypto.encrypt_payload(b"test", 1, 0x0001).unwrap();
        let decrypted = crypto.decrypt_payload(&ciphertext, 1, 0x0001).unwrap();
        assert_eq!(&decrypted, b"test");
    }

    #[test]
    fn hkdf_different_salt_different_key() {
        let ikm = b"same-key-material-for-both-tests";
        let salt_a = b"salt-aaaa-16bytes";  // >=16 bytes
        let salt_b = b"salt-bbbb-16bytes";  // >=16 bytes
        let crypto1 = TeamEngramCrypto::from_key_material(ikm, salt_a).unwrap();
        let crypto2 = TeamEngramCrypto::from_key_material(ikm, salt_b).unwrap();

        let ciphertext = crypto1.encrypt_payload(b"test", 1, 0x0001).unwrap();
        let result = crypto2.decrypt_payload(&ciphertext, 1, 0x0001);
        assert!(result.is_err());
    }

    #[test]
    fn hkdf_rejects_short_ikm() {
        let short_ikm = b"too-short"; // 9 bytes < 16
        let salt = b"fake-h_id-sha256-hash-32-bytes!!";
        let result = TeamEngramCrypto::from_key_material(short_ikm, salt);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("input key material too short"));
    }

    #[test]
    fn hkdf_rejects_short_salt() {
        let ikm = b"fake-tpm-sealed-private-key-bytes!!";
        let short_salt = b"tiny"; // 4 bytes < 16
        let result = TeamEngramCrypto::from_key_material(ikm, short_salt);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("salt too short"));
    }

    #[test]
    fn hkdf_accepts_minimum_lengths() {
        let ikm = &[0xABu8; 16]; // exactly 16 bytes
        let salt = &[0xCDu8; 16]; // exactly 16 bytes
        let crypto = TeamEngramCrypto::from_key_material(ikm, salt).unwrap();
        let ct = crypto.encrypt_payload(b"ok", 1, 0x0001).unwrap();
        let pt = crypto.decrypt_payload(&ct, 1, 0x0001).unwrap();
        assert_eq!(&pt, b"ok");
    }

    #[test]
    fn page_encrypt_decrypt_round_trip() {
        let crypto = TeamEngramCrypto::new(&test_key());
        let data = vec![0xABu8; 4064];
        let encrypted = crypto.encrypt_page_data(&data, 7, 100).unwrap();
        assert_eq!(encrypted.len(), data.len() + GCM_TAG_SIZE);

        let decrypted = crypto.decrypt_page_data(&encrypted, 7, 100).unwrap();
        assert_eq!(decrypted, data);
    }

    #[test]
    fn page_wrong_page_id_fails() {
        let crypto = TeamEngramCrypto::new(&test_key());
        let encrypted = crypto.encrypt_page_data(&[0xCDu8; 100], 1, 1).unwrap();
        let result = crypto.decrypt_page_data(&encrypted, 2, 1);
        assert!(result.is_err());
    }

    #[test]
    fn nonce_uniqueness() {
        let n1 = TeamEngramCrypto::event_nonce(1, 0x0001);
        let n2 = TeamEngramCrypto::event_nonce(2, 0x0001);
        let n3 = TeamEngramCrypto::event_nonce(1, 0x0002);
        assert_ne!(n1, n2);
        assert_ne!(n1, n3);
        assert_ne!(n2, n3);
    }

    #[test]
    fn page_nonce_uniqueness() {
        let n1 = TeamEngramCrypto::page_nonce(1, 1);
        let n2 = TeamEngramCrypto::page_nonce(2, 1);
        let n3 = TeamEngramCrypto::page_nonce(1, 2);
        assert_ne!(n1, n2);
        assert_ne!(n1, n3);
    }

    #[test]
    fn large_payload_round_trip() {
        let crypto = TeamEngramCrypto::new(&test_key());
        let plaintext = vec![0x42u8; 65535];
        let ciphertext = crypto.encrypt_payload(&plaintext, 999999, 0x0303).unwrap();
        let decrypted = crypto.decrypt_payload(&ciphertext, 999999, 0x0303).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    // ── load_encryption_key tests ───────────────────────────────────────

    #[test]
    fn load_encryption_key_no_file_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let result = super::load_encryption_key(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn load_encryption_key_valid_file() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join(super::ENCRYPTION_KEY_FILE);
        std::fs::write(&key_path, &test_key()).unwrap();

        let crypto = super::load_encryption_key(tmp.path()).unwrap();
        assert!(crypto.is_some());

        // Verify the loaded key works by encrypting/decrypting
        let crypto = crypto.unwrap();
        let ct = crypto.encrypt_payload(b"test", 1, 0x0001).unwrap();
        let pt = crypto.decrypt_payload(&ct, 1, 0x0001).unwrap();
        assert_eq!(&pt, b"test");
    }

    #[test]
    fn load_encryption_key_wrong_size_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join(super::ENCRYPTION_KEY_FILE);

        // Write 16 bytes instead of 32
        std::fs::write(&key_path, &[0u8; 16]).unwrap();
        let result = super::load_encryption_key(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("32 bytes"));
    }

    #[test]
    fn load_encryption_key_matches_direct_construction() {
        let tmp = tempfile::tempdir().unwrap();
        let key_bytes = test_key();
        let key_path = tmp.path().join(super::ENCRYPTION_KEY_FILE);
        std::fs::write(&key_path, &key_bytes).unwrap();

        // Load from file
        let from_file = super::load_encryption_key(tmp.path()).unwrap().unwrap();
        // Construct directly
        let direct = TeamEngramCrypto::new(&key_bytes);

        // Both should produce identical ciphertext for same inputs
        let ct1 = from_file.encrypt_payload(b"same", 42, 0x0002).unwrap();
        let ct2 = direct.encrypt_payload(b"same", 42, 0x0002).unwrap();
        assert_eq!(ct1, ct2);
    }
}
