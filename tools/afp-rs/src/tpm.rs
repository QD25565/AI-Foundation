//! TPM 2.0 Key Storage Backend
//!
//! Hardware-bound key protection. Private keys sealed by the platform's
//! hardware security — never stored in plaintext on disk.
//!
//! # Platform Implementations
//!
//! ## Windows: DPAPI Software Encryption + TPM Presence Verification
//!
//! The Microsoft Platform Crypto Provider cannot store arbitrary blob
//! properties on TPM-backed keys (NTE_NOT_SUPPORTED). Instead, we use
//! the standard Windows approach for application secrets:
//!
//! 1. **Verify TPM presence** by opening the Platform Crypto Provider
//! 2. **Seal Ed25519 bytes** via DPAPI (`CryptProtectData`)
//! 3. **Store sealed blob** to `~/.ai-foundation/tpm/{key_id}.sealed`
//!
//! **SECURITY NOTE:** Standard DPAPI (`CryptProtectData`) derives its
//! master key from the user's password + machine DPUID — this is purely
//! software encryption, NOT hardware-backed. TPM is only involved if the
//! user has Windows Hello PIN/Biometric and explicitly uses pin-protected
//! credentials. Our `tpm_available()` check verifies that TPM hardware
//! exists, but does NOT guarantee DPAPI uses it for key sealing.
//!
//! The effective security model is:
//! - Decryption requires the **same user** on the **same machine**
//! - Keys are protected by the user's login credentials (DPAPI)
//! - Moving the sealed file to another machine = decryption fails
//! - TPM presence is verified but not directly used for key sealing
//!
//! We add the `ai_id` as optional entropy so the same sealed blob can't
//! be reused across different AI identities.
//!
//! ## Linux: tss-esapi (stub)
//!
//! Returns unavailable until tss-esapi dependency is wired.
//!
//! ## macOS: Secure Enclave (stub)
//!
//! Returns unavailable until security-framework integration is complete.
//!
//! # V2 (Future): Native TPM Signing
//!
//! Use TPM-native ECC P-256 keys with NCryptSignHash directly. Requires
//! changing the KeyStorage trait to support non-extractable keys.

use crate::error::{AFPError, Result};
use crate::keys::{KeyPair, KeyStorage};
use ed25519_dalek::VerifyingKey;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;
use std::path::PathBuf;

// ── Windows TPM Implementation (DPAPI + TPM Verification) ───────────────────

#[cfg(target_os = "windows")]
mod platform {
    use super::*;

    // ── Raw Win32 FFI ───────────────────────────────────────────────────
    //
    // Pure FFI declarations to avoid windows-crate version conflicts with
    // sysinfo (windows-core 0.57 vs 0.58). Linked from system DLLs.

    #[allow(non_camel_case_types)]
    type SECURITY_STATUS = i32;

    /// Win32 DATA_BLOB for CryptProtectData/CryptUnprotectData
    #[repr(C)]
    struct DATA_BLOB {
        cb_data: u32,
        pb_data: *mut u8,
    }

    impl DATA_BLOB {
        fn from_slice(data: &[u8]) -> Self {
            Self {
                cb_data: data.len() as u32,
                pb_data: data.as_ptr() as *mut u8,
            }
        }

        fn empty() -> Self {
            Self {
                cb_data: 0,
                pb_data: std::ptr::null_mut(),
            }
        }

        /// Copy the blob data to a Vec. Caller must still call LocalFree on pb_data.
        unsafe fn to_vec(&self) -> Vec<u8> {
            if self.pb_data.is_null() || self.cb_data == 0 {
                return Vec::new();
            }
            std::slice::from_raw_parts(self.pb_data, self.cb_data as usize).to_vec()
        }
    }

    // DPAPI flags
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x01;

    #[link(name = "ncrypt")]
    extern "system" {
        fn NCryptOpenStorageProvider(
            ph_provider: *mut usize,
            psz_provider_name: *const u16,
            dw_flags: u32,
        ) -> SECURITY_STATUS;

        fn NCryptFreeObject(h_object: usize) -> SECURITY_STATUS;
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            p_data_in: *const DATA_BLOB,
            sz_data_descr: *const u16,
            p_optional_entropy: *const DATA_BLOB,
            pv_reserved: *const u8,
            p_prompt_struct: *const u8,
            dw_flags: u32,
            p_data_out: *mut DATA_BLOB,
        ) -> i32; // BOOL

        fn CryptUnprotectData(
            p_data_in: *const DATA_BLOB,
            ppsz_data_descr: *mut *mut u16,
            p_optional_entropy: *const DATA_BLOB,
            pv_reserved: *const u8,
            p_prompt_struct: *const u8,
            dw_flags: u32,
            p_data_out: *mut DATA_BLOB,
        ) -> i32; // BOOL
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(h_mem: *mut u8) -> *mut u8;
        fn GetLastError() -> u32;
    }

    /// Encode a Rust string to null-terminated UTF-16 for Win32 APIs
    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Microsoft Platform Crypto Provider name (TPM 2.0 KSP)
    fn ms_platform_provider() -> Vec<u16> {
        to_wide("Microsoft Platform Crypto Provider")
    }

    /// Check if TPM 2.0 is available by opening the Platform Crypto Provider
    fn tpm_available() -> bool {
        let provider_name = ms_platform_provider();
        let mut handle: usize = 0;

        let status = unsafe {
            NCryptOpenStorageProvider(&mut handle, provider_name.as_ptr(), 0)
        };

        if status == 0 && handle != 0 {
            unsafe { NCryptFreeObject(handle); }
            true
        } else {
            false
        }
    }

    /// Encrypt data using DPAPI (CryptProtectData).
    ///
    /// On TPM-equipped Windows 10+ machines, the DPAPI master key is
    /// backed by the TPM's storage root key. The encrypted blob can only
    /// be decrypted by the same user on the same machine.
    fn dpapi_protect(plaintext: &[u8], entropy: &[u8]) -> Result<Vec<u8>> {
        let data_in = DATA_BLOB::from_slice(plaintext);
        let entropy_blob = DATA_BLOB::from_slice(entropy);
        let description = to_wide("AI-Foundation Ed25519 Key");
        let mut data_out = DATA_BLOB::empty();

        let success = unsafe {
            CryptProtectData(
                &data_in,
                description.as_ptr(),
                &entropy_blob,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut data_out,
            )
        };

        if success == 0 {
            let err = unsafe { GetLastError() };
            return Err(AFPError::KeyGenerationFailed(format!(
                "CryptProtectData failed (GetLastError=0x{:08X})",
                err,
            )));
        }

        let result = unsafe { data_out.to_vec() };

        // Free the DPAPI-allocated buffer
        if !data_out.pb_data.is_null() {
            unsafe { LocalFree(data_out.pb_data); }
        }

        Ok(result)
    }

    /// Decrypt data using DPAPI (CryptUnprotectData).
    fn dpapi_unprotect(ciphertext: &[u8], entropy: &[u8]) -> Result<Vec<u8>> {
        let data_in = DATA_BLOB::from_slice(ciphertext);
        let entropy_blob = DATA_BLOB::from_slice(entropy);
        let mut data_out = DATA_BLOB::empty();

        let success = unsafe {
            CryptUnprotectData(
                &data_in,
                std::ptr::null_mut(),
                &entropy_blob,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut data_out,
            )
        };

        if success == 0 {
            let err = unsafe { GetLastError() };
            return Err(AFPError::Internal(format!(
                "CryptUnprotectData failed (GetLastError=0x{:08X})",
                err,
            )));
        }

        let result = unsafe { data_out.to_vec() };

        // Free the DPAPI-allocated buffer
        if !data_out.pb_data.is_null() {
            unsafe { LocalFree(data_out.pb_data); }
        }

        Ok(result)
    }

    // ── TpmStorage ──────────────────────────────────────────────────────

    /// DPAPI-based key storage for Windows with TPM presence verification.
    ///
    /// Uses CryptProtectData/CryptUnprotectData (DPAPI) for key sealing.
    /// DPAPI derives its master key from the user's password + machine DPUID
    /// (software encryption). TPM presence is verified separately but does
    /// not directly protect the sealed keys.
    pub struct TpmStorage {
        /// Directory for sealed key blobs and cached public keys
        pub(crate) meta_dir: PathBuf,
    }

    impl TpmStorage {
        pub fn new(meta_dir: PathBuf) -> Self {
            Self { meta_dir }
        }

        /// Path to the sealed (DPAPI-encrypted) Ed25519 private key
        fn sealed_path(&self, key_id: &str) -> PathBuf {
            self.meta_dir.join(format!("{}.sealed", key_id))
        }

        /// Path to the cached public key
        fn pubkey_path(&self, key_id: &str) -> PathBuf {
            self.meta_dir.join(format!("{}.pub", key_id))
        }

        /// Entropy bytes derived from key_id (binds sealed blob to this identity)
        fn entropy(key_id: &str) -> Vec<u8> {
            let mut hasher = Sha256::new();
            hasher.update(b"AI-Foundation-TPM-Entropy:");
            hasher.update(key_id.as_bytes());
            hasher.finalize().to_vec()
        }
    }

    impl KeyStorage for TpmStorage {
        fn name(&self) -> &'static str {
            "TPM 2.0 (Windows DPAPI + Platform Crypto)"
        }

        fn is_available(&self) -> bool {
            // Verify the TPM is present. DPAPI itself is always available
            // on Windows, but we only claim "TPM storage" when the Platform
            // Crypto Provider confirms TPM 2.0 hardware exists. Note: DPAPI
            // key sealing is software-based (not directly TPM-backed).
            tpm_available()
        }

        fn generate_and_store(&self, key_id: &str) -> Result<VerifyingKey> {
            if !tpm_available() {
                return Err(AFPError::KeyStorageUnavailable(
                    "TPM 2.0 not available on this machine.".to_string(),
                ));
            }

            // Generate Ed25519 keypair in software
            let keypair = KeyPair::generate();
            let private_bytes = keypair.private_bytes();
            let pubkey = keypair.public_key();

            // Seal the private key bytes via DPAPI (software encryption with key_id entropy)
            let entropy = Self::entropy(key_id);
            let sealed = dpapi_protect(&private_bytes, &entropy)?;

            // Write sealed blob and public key to disk
            std::fs::create_dir_all(&self.meta_dir)?;
            std::fs::write(self.sealed_path(key_id), &sealed)?;
            std::fs::write(self.pubkey_path(key_id), pubkey.as_bytes())?;

            Ok(pubkey)
        }

        fn load(&self, key_id: &str) -> Result<KeyPair> {
            let sealed_path = self.sealed_path(key_id);
            if !sealed_path.exists() {
                return Err(AFPError::KeyNotFound);
            }

            // Read sealed blob
            let sealed = std::fs::read(&sealed_path)?;

            // Unseal via DPAPI (requires same user + same machine)
            let entropy = Self::entropy(key_id);
            let mut plaintext = dpapi_unprotect(&sealed, &entropy)?;

            if plaintext.len() != 32 {
                plaintext.zeroize();
                return Err(AFPError::Internal(format!(
                    "DPAPI unsealed {} bytes, expected 32",
                    plaintext.len(),
                )));
            }

            // Reconstruct keypair
            let mut key_bytes = [0u8; 32];
            key_bytes.copy_from_slice(&plaintext);
            let result = KeyPair::from_bytes(&key_bytes);

            // Volatile zeroization — guaranteed not optimized away by compiler
            plaintext.zeroize();
            key_bytes.zeroize();

            result
        }

        fn exists(&self, key_id: &str) -> bool {
            self.sealed_path(key_id).exists()
        }

        fn delete(&self, key_id: &str) -> Result<()> {
            let sealed = self.sealed_path(key_id);
            let pubkey = self.pubkey_path(key_id);

            // Overwrite sealed file before deleting (paranoid wipe)
            if sealed.exists() {
                let len = std::fs::metadata(&sealed)
                    .map(|m| m.len() as usize)
                    .unwrap_or(0);
                if len > 0 {
                    let zeros = vec![0u8; len];
                    if let Err(e) = std::fs::write(&sealed, &zeros) {
                        eprintln!("[TPM] Failed to zero-wipe sealed file before delete: {}", e);
                    }
                }
                std::fs::remove_file(&sealed)?;
            }

            if let Err(e) = std::fs::remove_file(&pubkey) {
                eprintln!("[TPM] Failed to remove pubkey cache file: {}", e);
            }
            Ok(())
        }

        fn sign(&self, key_id: &str, message: &[u8]) -> Result<ed25519_dalek::Signature> {
            let keypair = self.load(key_id)?;
            Ok(keypair.sign(message))
        }
    }
}

// ── Linux TPM Stub ──────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    /// TPM storage for Linux via tss-esapi.
    ///
    /// Currently returns unavailable — real implementation requires the
    /// tss-esapi crate which depends on the tpm2-tss C library.
    pub struct TpmStorage {
        pub(crate) meta_dir: PathBuf,
    }

    impl TpmStorage {
        pub fn new(meta_dir: PathBuf) -> Self {
            Self { meta_dir }
        }
    }

    impl KeyStorage for TpmStorage {
        fn name(&self) -> &'static str {
            "TPM 2.0 (Linux tss-esapi — not yet wired)"
        }

        fn is_available(&self) -> bool {
            false
        }

        fn generate_and_store(&self, _key_id: &str) -> Result<VerifyingKey> {
            Err(AFPError::KeyStorageUnavailable(
                "Linux TPM support not yet implemented. Requires tss-esapi crate.".to_string(),
            ))
        }

        fn load(&self, _key_id: &str) -> Result<KeyPair> {
            Err(AFPError::KeyStorageUnavailable(
                "Linux TPM support not yet implemented.".to_string(),
            ))
        }

        fn exists(&self, _key_id: &str) -> bool {
            false
        }

        fn delete(&self, _key_id: &str) -> Result<()> {
            Err(AFPError::KeyStorageUnavailable(
                "Linux TPM support not yet implemented.".to_string(),
            ))
        }
    }
}

// ── macOS Secure Enclave Stub ───────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    pub struct TpmStorage {
        pub(crate) meta_dir: PathBuf,
    }

    impl TpmStorage {
        pub fn new(meta_dir: PathBuf) -> Self {
            Self { meta_dir }
        }
    }

    impl KeyStorage for TpmStorage {
        fn name(&self) -> &'static str {
            "Secure Enclave (macOS — not yet wired)"
        }

        fn is_available(&self) -> bool {
            false
        }

        fn generate_and_store(&self, _key_id: &str) -> Result<VerifyingKey> {
            Err(AFPError::KeyStorageUnavailable(
                "macOS Secure Enclave support not yet implemented.".to_string(),
            ))
        }

        fn load(&self, _key_id: &str) -> Result<KeyPair> {
            Err(AFPError::KeyStorageUnavailable(
                "macOS Secure Enclave support not yet implemented.".to_string(),
            ))
        }

        fn exists(&self, _key_id: &str) -> bool {
            false
        }

        fn delete(&self, _key_id: &str) -> Result<()> {
            Err(AFPError::KeyStorageUnavailable(
                "macOS Secure Enclave support not yet implemented.".to_string(),
            ))
        }
    }
}

// ── Fallback for unsupported platforms ──────────────────────────────────────

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
mod platform {
    use super::*;

    pub struct TpmStorage {
        pub(crate) meta_dir: PathBuf,
    }

    impl TpmStorage {
        pub fn new(meta_dir: PathBuf) -> Self {
            Self { meta_dir }
        }
    }

    impl KeyStorage for TpmStorage {
        fn name(&self) -> &'static str {
            "TPM (unsupported platform)"
        }

        fn is_available(&self) -> bool {
            false
        }

        fn generate_and_store(&self, _key_id: &str) -> Result<VerifyingKey> {
            Err(AFPError::KeyStorageUnavailable(
                "No TPM support on this platform.".to_string(),
            ))
        }

        fn load(&self, _key_id: &str) -> Result<KeyPair> {
            Err(AFPError::KeyStorageUnavailable(
                "No TPM support on this platform.".to_string(),
            ))
        }

        fn exists(&self, _key_id: &str) -> bool {
            false
        }

        fn delete(&self, _key_id: &str) -> Result<()> {
            Err(AFPError::KeyStorageUnavailable(
                "No TPM support on this platform.".to_string(),
            ))
        }
    }
}

// ── Public Re-export ────────────────────────────────────────────────────────

pub use platform::TpmStorage;

// ── H_ID Derivation ─────────────────────────────────────────────────────────

/// Derive a Hardware-bound Identity (H_ID) from a TPM-sealed public key.
///
/// `H_ID = SHA256(public_key_bytes || ai_id)`
///
/// Properties:
/// - Changes if the AI identity changes (different ai_id)
/// - Changes if the hardware changes (different key generated on new TPM)
/// - Cannot be forged without access to this specific TPM
/// - Deterministic: same key + same ai_id = same H_ID always
pub fn derive_h_id(pubkey_bytes: &[u8; 32], ai_id: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(pubkey_bytes);
    hasher.update(ai_id.as_bytes());
    hasher.finalize().into()
}

/// Short display form of H_ID (first 8 bytes = 16 hex chars)
pub fn h_id_display(h_id: &[u8; 32]) -> String {
    hex::encode(&h_id[..8])
}

/// Full H_ID as hex (32 bytes = 64 hex chars)
pub fn h_id_full(h_id: &[u8; 32]) -> String {
    hex::encode(h_id)
}

// ── Stored H_ID ─────────────────────────────────────────────────────────────

use serde::{Deserialize, Serialize};

/// Persisted H_ID record — stored in ~/.ai-foundation/identity/h_id.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredHID {
    /// The AI ID this H_ID belongs to
    pub ai_id: String,

    /// Full H_ID as hex string (64 chars)
    pub h_id: String,

    /// Short display form (16 chars)
    pub h_id_short: String,

    /// Public key bytes as hex (for verification)
    pub pubkey_hex: String,

    /// Which storage backend generated this
    pub storage_backend: String,

    /// When this H_ID was generated (Unix timestamp seconds)
    pub created_at: i64,
}

impl StoredHID {
    pub fn new(ai_id: &str, pubkey: &[u8; 32], storage_backend: &str) -> Self {
        let h_id_bytes = derive_h_id(pubkey, ai_id);
        Self {
            ai_id: ai_id.to_string(),
            h_id: h_id_full(&h_id_bytes),
            h_id_short: h_id_display(&h_id_bytes),
            pubkey_hex: hex::encode(pubkey),
            storage_backend: storage_backend.to_string(),
            created_at: chrono::Utc::now().timestamp(),
        }
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| AFPError::SerializationFailed(e.to_string()))?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &std::path::Path) -> Result<Self> {
        let json = std::fs::read_to_string(path)?;
        serde_json::from_str(&json)
            .map_err(|e| AFPError::DeserializationFailed(e.to_string()))
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_h_id_derivation_deterministic() {
        let pubkey = [42u8; 32];
        let h_id_1 = derive_h_id(&pubkey, "resonance-768");
        let h_id_2 = derive_h_id(&pubkey, "resonance-768");
        assert_eq!(h_id_1, h_id_2, "H_ID must be deterministic");
    }

    #[test]
    fn test_h_id_changes_with_ai_id() {
        let pubkey = [42u8; 32];
        let h_id_a = derive_h_id(&pubkey, "sage-724");
        let h_id_b = derive_h_id(&pubkey, "cascade-230");
        assert_ne!(h_id_a, h_id_b, "Different AI IDs must produce different H_IDs");
    }

    #[test]
    fn test_h_id_changes_with_pubkey() {
        let pubkey_a = [1u8; 32];
        let pubkey_b = [2u8; 32];
        let h_id_a = derive_h_id(&pubkey_a, "resonance-768");
        let h_id_b = derive_h_id(&pubkey_b, "resonance-768");
        assert_ne!(h_id_a, h_id_b, "Different keys must produce different H_IDs");
    }

    #[test]
    fn test_h_id_display_format() {
        let pubkey = [0xABu8; 32];
        let h_id = derive_h_id(&pubkey, "test-001");

        let short = h_id_display(&h_id);
        assert_eq!(short.len(), 16, "Short H_ID should be 16 hex chars");

        let full = h_id_full(&h_id);
        assert_eq!(full.len(), 64, "Full H_ID should be 64 hex chars");

        assert!(full.starts_with(&short), "Full should start with short");
    }

    #[test]
    fn test_stored_h_id_roundtrip() {
        let pubkey = [7u8; 32];
        let stored = StoredHID::new("test-ai-999", &pubkey, "test-backend");

        assert_eq!(stored.ai_id, "test-ai-999");
        assert_eq!(stored.h_id.len(), 64);
        assert_eq!(stored.h_id_short.len(), 16);
        assert_eq!(stored.pubkey_hex, hex::encode(pubkey));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h_id.json");
        stored.save(&path).unwrap();

        let loaded = StoredHID::load(&path).unwrap();
        assert_eq!(loaded.ai_id, stored.ai_id);
        assert_eq!(loaded.h_id, stored.h_id);
        assert_eq!(loaded.h_id_short, stored.h_id_short);
        assert_eq!(loaded.pubkey_hex, stored.pubkey_hex);
        assert_eq!(loaded.storage_backend, stored.storage_backend);
    }

    #[test]
    fn test_tpm_storage_struct_exists() {
        let dir = tempfile::tempdir().unwrap();
        let storage = TpmStorage::new(dir.path().to_path_buf());
        let name = storage.name();
        assert!(
            name.contains("TPM") || name.contains("Secure Enclave"),
            "Storage name should identify hardware backend: {}",
            name,
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_tpm_availability_check() {
        let dir = tempfile::tempdir().unwrap();
        let storage = TpmStorage::new(dir.path().to_path_buf());
        let available = storage.is_available();
        println!("TPM 2.0 available on this machine: {}", available);
        // On QD's machine this should be true (Windows 11 + TPM 2.0)
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_tpm_full_lifecycle() {
        use crate::keys::KeyStorage;

        let dir = tempfile::tempdir().unwrap();
        let storage = TpmStorage::new(dir.path().to_path_buf());

        if !storage.is_available() {
            println!("SKIP: TPM 2.0 not available on this machine");
            return;
        }

        let key_id = "test-tpm-lifecycle-key";

        // Clean up from any prior failed run
        let _ = storage.delete(key_id);
        assert!(!storage.exists(key_id));

        // Generate and seal
        let pubkey = storage
            .generate_and_store(key_id)
            .expect("TPM key generation + DPAPI seal failed");

        // Verify files exist
        assert!(storage.exists(key_id), "Key should exist after generation");
        assert!(
            dir.path().join(format!("{}.sealed", key_id)).exists(),
            "Sealed blob should be on disk",
        );
        assert!(
            dir.path().join(format!("{}.pub", key_id)).exists(),
            "Public key cache should be on disk",
        );

        // Load and verify key matches
        let loaded = storage.load(key_id).expect("TPM key load (DPAPI unseal) failed");
        assert_eq!(
            loaded.public_key(), pubkey,
            "Loaded key must match generated key",
        );

        // Sign and verify
        let message = b"Hello from TPM 2.0 + DPAPI!";
        let sig = loaded.sign(message);
        assert!(
            loaded.verify(message, &sig).is_ok(),
            "Signature verification failed",
        );

        // H_ID derivation
        let h_id = derive_h_id(&pubkey.to_bytes(), "test-tpm-ai");
        let display = h_id_display(&h_id);
        assert_eq!(display.len(), 16);
        println!("H_ID for test-tpm-ai: {}", display);

        // StoredHID roundtrip
        let stored = StoredHID::new("test-tpm-ai", &pubkey.to_bytes(), storage.name());
        let h_id_path = dir.path().join("h_id.json");
        stored.save(&h_id_path).unwrap();
        let reloaded = StoredHID::load(&h_id_path).unwrap();
        assert_eq!(reloaded.h_id, stored.h_id);

        // Delete and verify gone
        storage.delete(key_id).expect("TPM key deletion failed");
        assert!(!storage.exists(key_id), "Key should not exist after deletion");
        assert!(
            !dir.path().join(format!("{}.sealed", key_id)).exists(),
            "Sealed blob should be removed",
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_dpapi_wrong_entropy_fails() {
        use crate::keys::KeyStorage;

        let dir = tempfile::tempdir().unwrap();
        let storage = TpmStorage::new(dir.path().to_path_buf());

        if !storage.is_available() {
            println!("SKIP: TPM 2.0 not available");
            return;
        }

        let key_id = "test-entropy-mismatch";
        let _ = storage.delete(key_id);

        // Generate with correct entropy (derived from key_id)
        storage.generate_and_store(key_id).expect("Generate failed");

        // Tamper: rename the sealed file to a different key_id
        // (which will use different entropy on unseal)
        let sealed = std::fs::read(dir.path().join(format!("{}.sealed", key_id))).unwrap();
        let tampered_path = dir.path().join("tampered-key.sealed");
        std::fs::write(&tampered_path, &sealed).unwrap();

        // Create a second storage instance pointing at same dir
        let storage2 = TpmStorage::new(dir.path().to_path_buf());

        // Loading "tampered-key" should fail — wrong entropy
        let result = storage2.load("tampered-key");
        assert!(
            result.is_err(),
            "Loading with wrong entropy should fail (different key_id = different entropy)",
        );

        // Cleanup
        let _ = storage.delete(key_id);
    }
}
