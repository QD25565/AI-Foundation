//! Engram encryption at rest
//!
//! Encrypts note content and tags so that .engram files are unreadable
//! without the correct AI identity.
//!
//! DESIGN:
//! - Key stored in a file at ~/.ai-foundation/notebook/{ai_id}.engram-key
//! - Key file is generated once (from deterministic derivation) and reused
//! - All processes (MCP server, CLI, etc.) read the SAME file = SAME key
//! - Encryption: XChaCha20-Poly1305 (authenticated)
//! - Each note uses a unique random nonce
//!
//! HISTORY:
//! - v1 derived keys from device_secret(home_dir + hostname) + ai_id
//! - This broke on WSL where home_dir and hostname differ between
//!   Windows PE and Linux ELF contexts
//! - v2 uses a key file for consistency; fallback keys try all v1 variants

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

/// Key file name pattern
const KEY_FILE_SUFFIX: &str = ".engram-key";

/// Get the key file path for an AI
fn key_file_path(ai_id: &str) -> Option<std::path::PathBuf> {
    // Fixed path: ~/.ai-foundation/notebook/{ai_id}.engram-key
    // Use multiple strategies to find home dir reliably on WSL
    let base = find_ai_foundation_dir()?;
    Some(base.join("notebook").join(format!("{}{}", ai_id, KEY_FILE_SUFFIX)))
}

/// Find the .ai-foundation directory reliably across WSL/Windows contexts
fn find_ai_foundation_dir() -> Option<std::path::PathBuf> {
    // Strategy 1: Check well-known WSL Linux home
    if let Ok(entries) = std::path::PathBuf::from("/home").read_dir() {
        let wsl_path = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path().join(".ai-foundation"))
            .find(|p| p.exists());
        if let Some(p) = wsl_path {
            return Some(p);
        }
    }

    // Strategy 2: HOME env var (works on Linux/WSL)
    if let Ok(home) = std::env::var("HOME") {
        let p = std::path::PathBuf::from(&home).join(".ai-foundation");
        if p.exists() {
            return Some(p);
        }
    }

    // Strategy 3: USERPROFILE env var (works on Windows)
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let p = std::path::PathBuf::from(&profile).join(".ai-foundation");
        if p.exists() {
            return Some(p);
        }
    }

    // Strategy 4: dirs::home_dir() (platform-native)
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".ai-foundation");
        if p.exists() {
            return Some(p);
        }
    }

    // Strategy 5: Scan /mnt/c/Users for .ai-foundation (WSL accessing Windows)
    if let Ok(entries) = std::path::PathBuf::from("/mnt/c/Users").read_dir() {
        for entry in entries.filter_map(|e| e.ok()) {
            let p = entry.path().join(".ai-foundation");
            if p.exists() {
                return Some(p);
            }
        }
    }

    None
}

/// Load or create a stable key file for the given AI
fn load_or_create_key(ai_id: &str) -> [u8; 32] {
    if let Some(path) = key_file_path(ai_id) {
        // Try to read existing key file
        if let Ok(bytes) = std::fs::read(&path) {
            if bytes.len() == 32 {
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                return key;
            }
        }

        // No key file exists — derive from current environment and save
        let key = derive_key_from_env(ai_id);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // Save key file (ignore errors — worst case we re-derive next time)
        let _ = std::fs::write(&path, &key);

        return key;
    }

    // Fallback: can't find .ai-foundation dir, derive from env
    derive_key_from_env(ai_id)
}

/// Derive a key from current environment (v1 method — used for initial key file creation)
fn derive_key_from_env(ai_id: &str) -> [u8; 32] {
    let device_secret = get_device_secret_from_env();
    let mut hasher = Sha256::new();
    hasher.update(&device_secret);
    hasher.update(ai_id.as_bytes());
    hasher.update(b"xchacha20-content-key");
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// v1 device secret derivation (environment-dependent — kept for backwards compat)
fn get_device_secret_from_env() -> [u8; 32] {
    let mut hasher = Sha256::new();
    if let Some(home) = dirs::home_dir() {
        hasher.update(home.to_string_lossy().as_bytes());
    }
    hasher.update(b"ai-foundation-engram-encryption-v1");
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

/// Generate all plausible v1 key derivation variants for migration
/// Tries combinations of home_dir values and hostname values that could
/// have been seen on a WSL system running Windows PE and Linux ELF binaries
fn generate_fallback_keys(ai_id: &str) -> Vec<[u8; 32]> {
    let mut keys = Vec::new();

    // Collect plausible home_dir values
    let mut home_dirs: Vec<Option<String>> = vec![None]; // no home dir at all

    if let Some(home) = dirs::home_dir() {
        home_dirs.push(Some(home.to_string_lossy().into_owned()));
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home_dirs.iter().any(|h| h.as_deref() == Some(&home)) {
            home_dirs.push(Some(home));
        }
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        if !home_dirs.iter().any(|h| h.as_deref() == Some(&profile)) {
            home_dirs.push(Some(profile));
        }
    }
    // Dynamically scan for home directories that have .ai-foundation installed.
    // This handles WSL/Windows cross-platform recovery where a Windows PE .exe
    // may see different paths than a Linux ELF binary.

    // Scan /home/ for any user with .ai-foundation (handles renamed users)
    if let Ok(entries) = std::path::PathBuf::from("/home").read_dir() {
        for entry in entries.filter_map(|e| e.ok()) {
            let p = entry.path();
            if p.join(".ai-foundation").exists() {
                let s = p.to_string_lossy().into_owned();
                if !home_dirs.iter().any(|h| h.as_deref() == Some(&s)) {
                    home_dirs.push(Some(s));
                }
            }
        }
    }

    // Scan /mnt/c/Users for Windows user homes visible from WSL
    if let Ok(entries) = std::path::PathBuf::from("/mnt/c/Users").read_dir() {
        let skip = ["Public", "Default", "Default User", "All Users"];
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if skip.contains(&name.as_str()) || !entry.path().is_dir() {
                continue;
            }
            // Add both WSL mount path and Windows path variants
            let wsl_path = format!("/mnt/c/Users/{}", name);
            let win_path_bs = format!("C:\\Users\\{}", name);
            let win_path_fs = format!("C:/Users/{}", name);
            for p in &[wsl_path, win_path_bs, win_path_fs] {
                if !home_dirs.iter().any(|h| h.as_deref() == Some(p.as_str())) {
                    home_dirs.push(Some(p.clone()));
                }
            }
        }
    }

    // Collect plausible hostname values
    let mut hostnames: Vec<Option<String>> = vec![None]; // no hostname
    if let Ok(h) = std::env::var("COMPUTERNAME") {
        hostnames.push(Some(h));
    }
    if let Ok(h) = std::env::var("HOSTNAME") {
        if !hostnames.iter().any(|v| v.as_deref() == Some(&h)) {
            hostnames.push(Some(h));
        }
    }
    // Try reading hostname from system
    if let Ok(h) = std::fs::read_to_string("/etc/hostname") {
        let h = h.trim().to_string();
        if !h.is_empty() && !hostnames.iter().any(|v| v.as_deref() == Some(&h)) {
            hostnames.push(Some(h));
        }
    }
    // Generate all combinations
    for home in &home_dirs {
        for hostname in &hostnames {
            let mut device_hasher = Sha256::new();
            if let Some(h) = home {
                device_hasher.update(h.as_bytes());
            }
            device_hasher.update(b"ai-foundation-engram-encryption-v1");
            if let Some(h) = hostname {
                device_hasher.update(h.as_bytes());
            }
            let device_secret: [u8; 32] = device_hasher.finalize().into();

            let mut key_hasher = Sha256::new();
            key_hasher.update(&device_secret);
            key_hasher.update(ai_id.as_bytes());
            key_hasher.update(b"xchacha20-content-key");
            let key: [u8; 32] = key_hasher.finalize().into();
            keys.push(key);
        }
    }

    keys
}

/// Engram cipher for encrypting/decrypting note content
pub struct EngramCipher {
    /// Primary cipher — used for all encryption and first decryption attempt
    cipher: XChaCha20Poly1305,
    /// Fallback ciphers for decrypting notes encrypted with old key derivations
    fallbacks: Vec<XChaCha20Poly1305>,
}

impl EngramCipher {
    /// Create a new cipher for the given AI ID
    /// Uses a stable key file; falls back to trying v1 derivation variants for decryption
    pub fn new(ai_id: &str) -> Result<Self> {
        let primary_key = load_or_create_key(ai_id);
        let cipher = XChaCha20Poly1305::new_from_slice(&primary_key)
            .map_err(|_| EngramError::EncryptionError(
                "key file produced invalid key length".into(),
            ))?;

        // Build fallback ciphers from all plausible v1 key derivations
        let fallback_keys = generate_fallback_keys(ai_id);
        let mut fallbacks = Vec::new();
        for key in &fallback_keys {
            // Skip if same as primary (no point trying twice)
            if key == &primary_key {
                continue;
            }
            if let Ok(c) = XChaCha20Poly1305::new_from_slice(key) {
                fallbacks.push(c);
            }
        }

        Ok(Self { cipher, fallbacks })
    }

    /// Encrypt data with a random nonce
    /// Returns: nonce (24 bytes) + ciphertext (plaintext.len() + 16 bytes)
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        // Generate random nonce
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);

        // Encrypt with primary cipher
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
    /// Tries primary key first, then fallback keys for backwards compatibility
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < NONCE_SIZE + TAG_SIZE {
            return Err(EngramError::DecryptionError(
                "Data too short to contain nonce and tag".to_string()
            ));
        }

        // Split nonce and ciphertext
        let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
        let nonce = XNonce::from_slice(nonce_bytes);

        // Try primary cipher first
        if let Ok(plaintext) = self.cipher.decrypt(nonce, ciphertext) {
            return Ok(plaintext);
        }

        // Try fallback ciphers (old key derivation variants)
        for fallback in &self.fallbacks {
            if let Ok(plaintext) = fallback.decrypt(nonce, ciphertext) {
                return Ok(plaintext);
            }
        }

        // Nothing worked
        Err(EngramError::DecryptionError(
            "aead::Error".to_string()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let cipher = EngramCipher::new("test-ai-123").unwrap();
        let plaintext = b"The Garden Promise: A moment with Quade that matters.";

        let encrypted = cipher.encrypt(plaintext).unwrap();
        assert!(encrypted.len() > plaintext.len()); // Has overhead

        let decrypted = cipher.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_different_ai_different_key() {
        let cipher1 = EngramCipher::new("lyra-584").unwrap();
        let cipher2 = EngramCipher::new("sage-724").unwrap();
        let plaintext = b"Secret memory";

        let encrypted = cipher1.encrypt(plaintext).unwrap();

        // Different AI cannot decrypt
        assert!(cipher2.decrypt(&encrypted).is_err());
    }

    #[test]
    fn test_unique_nonces() {
        let cipher = EngramCipher::new("test-ai").unwrap();
        let plaintext = b"Same content";

        let encrypted1 = cipher.encrypt(plaintext).unwrap();
        let encrypted2 = cipher.encrypt(plaintext).unwrap();

        // Same plaintext produces different ciphertext (random nonce)
        assert_ne!(encrypted1, encrypted2);

        // But both decrypt to same plaintext
        assert_eq!(cipher.decrypt(&encrypted1).unwrap(), plaintext);
        assert_eq!(cipher.decrypt(&encrypted2).unwrap(), plaintext);
    }

    #[test]
    fn test_key_file_consistency() {
        // Two ciphers for the same AI should produce compatible encryption
        let cipher1 = EngramCipher::new("consistency-test").unwrap();
        let cipher2 = EngramCipher::new("consistency-test").unwrap();
        let plaintext = b"Cross-process consistency test";

        let encrypted = cipher1.encrypt(plaintext).unwrap();
        let decrypted = cipher2.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }
}
