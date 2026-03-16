//! Passkey-based manual pairing.
//!
//! Allows nodes to connect via a time-limited passkey that encodes
//! authenticated-encrypted connection information. Useful when automatic
//! discovery isn't available or for establishing initial trust.
//!
//! # Security
//!
//! The passkey payload (endpoint address + public key) is encrypted with
//! **AES-256-GCM** using a key derived from the passkey code via SHA-256.
//! Each encryption uses a fresh random 12-byte nonce stored alongside the
//! ciphertext — no nonce is ever reused. GCM authentication tags ensure
//! tampered payloads are rejected loudly, not silently ignored.
//!
//! Ciphertext wire format: `[12-byte nonce][ciphertext + 16-byte GCM tag]`.

use crate::{Endpoint, Result, FederationError};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Default passkey validity duration (5 minutes).
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// Passkey length (6 alphanumeric characters).
pub const PASSKEY_LENGTH: usize = 6;

/// Characters used in passkey generation (no I, O, 0, 1 to prevent confusion).
const PASSKEY_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// AES-256-GCM nonce length (96-bit / 12 bytes — GCM standard).
const NONCE_LEN: usize = 12;

// ─── Payload types ────────────────────────────────────────────────────────────

/// Information encoded in a passkey.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasskeyPayload {
    /// Node ID of the generator.
    pub node_id: String,
    /// Display name.
    pub display_name: String,
    /// Primary endpoint to connect to.
    pub endpoint: EndpointInfo,
    /// Public key for verification (hex encoded).
    pub pubkey_hex: String,
    /// When this passkey was created (Unix seconds).
    pub created_at: u64,
    /// When this passkey expires (Unix seconds).
    pub expires_at: u64,
}

/// Simplified endpoint info for passkey encoding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointInfo {
    /// Endpoint type.
    pub endpoint_type: String,
    /// Address (IP:port for QUIC, service name for mDNS, etc.).
    pub address: String,
    /// Optional certificate fingerprint (hex).
    pub cert_fingerprint: Option<String>,
}

impl EndpointInfo {
    /// Create from a QUIC endpoint.
    pub fn quic(addr: &str, cert_fingerprint: Option<&str>) -> Self {
        Self {
            endpoint_type: "quic".to_string(),
            address: addr.to_string(),
            cert_fingerprint: cert_fingerprint.map(String::from),
        }
    }

    /// Create from an mDNS service.
    pub fn mdns(service_name: &str) -> Self {
        Self {
            endpoint_type: "mdns".to_string(),
            address: service_name.to_string(),
            cert_fingerprint: None,
        }
    }

    /// Convert to an [`Endpoint`].
    pub fn to_endpoint(&self) -> Result<Endpoint> {
        match self.endpoint_type.as_str() {
            "quic" => {
                let addr = self
                    .address
                    .parse()
                    .map_err(|e| FederationError::Internal(format!("Invalid address: {e}")))?;

                if let Some(ref fp) = self.cert_fingerprint {
                    let bytes = hex::decode(fp).map_err(|e| {
                        FederationError::Internal(format!("Invalid fingerprint: {e}"))
                    })?;
                    let fp_array: [u8; 32] = bytes.try_into().map_err(|_| {
                        FederationError::Internal("Fingerprint wrong length".to_string())
                    })?;
                    Ok(Endpoint::quic_pinned(addr, fp_array))
                } else {
                    Ok(Endpoint::quic(addr))
                }
            }
            "mdns" => Ok(Endpoint::mdns(&self.address)),
            _ => Err(FederationError::Internal(format!(
                "Unknown endpoint type: {}",
                self.endpoint_type
            ))),
        }
    }
}

// ─── Generated passkey ────────────────────────────────────────────────────────

/// A generated passkey with its encrypted payload.
#[derive(Debug, Clone)]
pub struct GeneratedPasskey {
    /// The human-readable passkey code.
    pub code: String,
    /// Authenticated-encrypted payload: `[12-byte nonce][ciphertext + 16-byte GCM tag]`.
    pub encrypted_payload: Vec<u8>,
    /// When this expires (Unix seconds).
    pub expires_at: u64,
}

impl GeneratedPasskey {
    /// Check if this passkey has expired.
    pub fn is_expired(&self) -> bool {
        unix_now() > self.expires_at
    }

    /// Get remaining validity time.
    pub fn remaining_time(&self) -> Option<Duration> {
        let now = unix_now();
        if now < self.expires_at {
            Some(Duration::from_secs(self.expires_at - now))
        } else {
            None
        }
    }

    /// Convert to an [`Endpoint`] for connection.
    pub fn to_endpoint(&self) -> Endpoint {
        Endpoint::passkey(&self.code, self.encrypted_payload.clone(), self.expires_at)
    }
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Generate a random 6-character passkey code.
pub fn generate_passkey_code() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..PASSKEY_LENGTH)
        .map(|_| {
            let idx = rng.gen_range(0..PASSKEY_CHARS.len());
            PASSKEY_CHARS[idx] as char
        })
        .collect()
}

/// Derive an AES-256 key from a passkey code.
///
/// Uses SHA-256 with a domain separator. The passkey code provides the entropy;
/// the domain separator prevents key reuse across protocol contexts.
pub fn derive_key_from_passkey(code: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(code.as_bytes());
    hasher.update(b"ai-foundation-passkey-v1");
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// Generate a passkey encoding connection info for sharing.
///
/// The payload is encrypted with AES-256-GCM. The generated passkey code is
/// the only thing that needs to be shared out-of-band — the encrypted payload
/// is transferred separately (e.g. as a QR code or HTTP response body).
pub fn generate_passkey(
    node_id: &str,
    display_name: &str,
    endpoint_info: EndpointInfo,
    pubkey_hex: &str,
    ttl: Duration,
) -> Result<GeneratedPasskey> {
    let now = unix_now();
    let expires_at = now + ttl.as_secs();

    let payload = PasskeyPayload {
        node_id: node_id.to_string(),
        display_name: display_name.to_string(),
        endpoint: endpoint_info,
        pubkey_hex: pubkey_hex.to_string(),
        created_at: now,
        expires_at,
    };

    let payload_json = serde_json::to_vec(&payload)
        .map_err(|e| FederationError::SerializationError(e.to_string()))?;

    let code = generate_passkey_code();
    let key = derive_key_from_passkey(&code);
    let encrypted_payload = aes_gcm_encrypt(&payload_json, &key)?;

    Ok(GeneratedPasskey {
        code,
        encrypted_payload,
        expires_at,
    })
}

/// Decode a passkey to get connection info.
///
/// Returns an error if the passkey has expired, the ciphertext is malformed,
/// or authentication fails (wrong code or tampered payload).
pub fn decode_passkey(
    code: &str,
    encrypted_payload: &[u8],
    expires_at: u64,
) -> Result<PasskeyPayload> {
    if unix_now() > expires_at {
        return Err(FederationError::Internal("Passkey has expired".to_string()));
    }

    let key = derive_key_from_passkey(code);
    let payload_json = aes_gcm_decrypt(encrypted_payload, &key)?;

    serde_json::from_slice(&payload_json).map_err(|e| {
        FederationError::SerializationError(format!("Invalid passkey data: {e}"))
    })
}

// ─── Display helpers ──────────────────────────────────────────────────────────

/// Validate that a passkey code uses the correct format and charset.
pub fn validate_passkey_format(code: &str) -> bool {
    code.len() == PASSKEY_LENGTH
        && code.chars().all(|c| PASSKEY_CHARS.contains(&(c as u8)))
}

/// Format a 6-char passkey for human display (e.g., "ABC123" → "ABC-123").
pub fn format_passkey_display(code: &str) -> String {
    if code.len() == 6 {
        format!("{}-{}", &code[0..3], &code[3..6])
    } else {
        code.to_string()
    }
}

/// Parse a passkey from display format back to raw code.
pub fn parse_passkey_display(display: &str) -> String {
    display.replace('-', "").replace(' ', "").to_uppercase()
}

// ─── AES-256-GCM helpers ──────────────────────────────────────────────────────

/// Encrypt `data` with AES-256-GCM.
///
/// Output: `[12-byte nonce][ciphertext + 16-byte GCM auth tag]`.
/// Each call generates a fresh nonce — never reused.
fn aes_gcm_encrypt(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|_| FederationError::Internal("Encryption failed".to_string()))?;

    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt AES-256-GCM ciphertext produced by [`aes_gcm_encrypt`].
///
/// Fails loudly on authentication failure — never ignores tampering silently.
fn aes_gcm_decrypt(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    if data.len() < NONCE_LEN {
        return Err(FederationError::Internal(
            "Passkey ciphertext too short".to_string(),
        ));
    }

    let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher.decrypt(nonce, ciphertext).map_err(|_| {
        FederationError::Internal(
            "Passkey decryption failed (wrong code or tampered payload)".to_string(),
        )
    })
}

// ─── Private helpers ──────────────────────────────────────────────────────────

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PUBKEY: &str =
        "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab";

    #[test]
    fn test_generate_passkey_code() {
        let code = generate_passkey_code();
        assert_eq!(code.len(), PASSKEY_LENGTH);
        assert!(validate_passkey_format(&code));
    }

    #[test]
    fn test_passkey_format_display() {
        let code = "ABC123";
        let display = format_passkey_display(code);
        assert_eq!(display, "ABC-123");
        let parsed = parse_passkey_display(&display);
        assert_eq!(parsed, code);
    }

    #[test]
    fn test_key_derivation_deterministic() {
        let key1 = derive_key_from_passkey("ABC123");
        let key2 = derive_key_from_passkey("ABC123");
        assert_eq!(key1, key2, "Same code must produce same key");

        let key3 = derive_key_from_passkey("XYZ789");
        assert_ne!(key1, key3, "Different codes must produce different keys");
    }

    #[test]
    fn test_aes_gcm_roundtrip() {
        let key = derive_key_from_passkey("TEST12");
        let plaintext = b"Hello, Teambook federation!";

        let ciphertext = aes_gcm_encrypt(plaintext, &key).unwrap();

        // Must be longer: nonce (12) + plaintext + GCM tag (16)
        assert!(ciphertext.len() >= NONCE_LEN + plaintext.len() + 16);

        let decrypted = aes_gcm_decrypt(&ciphertext, &key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_aes_gcm_tamper_detection() {
        let key = derive_key_from_passkey("TEST12");
        let mut ciphertext = aes_gcm_encrypt(b"Sensitive endpoint data", &key).unwrap();

        // Flip a byte in the ciphertext body (past the nonce)
        ciphertext[NONCE_LEN] ^= 0xFF;

        assert!(
            aes_gcm_decrypt(&ciphertext, &key).is_err(),
            "Tampered ciphertext must fail authentication"
        );
    }

    #[test]
    fn test_aes_gcm_unique_nonces() {
        let key = derive_key_from_passkey("TEST12");
        let ct1 = aes_gcm_encrypt(b"Same message", &key).unwrap();
        let ct2 = aes_gcm_encrypt(b"Same message", &key).unwrap();

        // Probabilistically guaranteed by OsRng — nonce reuse would be catastrophic
        assert_ne!(ct1, ct2, "Each encryption must use a fresh nonce");
    }

    #[test]
    fn test_generate_and_decode_passkey_roundtrip() {
        let endpoint_info = EndpointInfo::quic("192.168.1.100:31420", None);

        let generated = generate_passkey(
            "node-123",
            "Test Node",
            endpoint_info,
            TEST_PUBKEY,
            Duration::from_secs(300),
        )
        .unwrap();

        assert!(!generated.is_expired());
        assert!(generated.remaining_time().is_some());

        let decoded = decode_passkey(
            &generated.code,
            &generated.encrypted_payload,
            generated.expires_at,
        )
        .unwrap();

        assert_eq!(decoded.node_id, "node-123");
        assert_eq!(decoded.display_name, "Test Node");
        assert_eq!(decoded.pubkey_hex, TEST_PUBKEY);
    }

    #[test]
    fn test_decode_wrong_code_fails() {
        let endpoint_info = EndpointInfo::quic("192.168.1.100:31420", None);

        let generated = generate_passkey(
            "node-123",
            "Test Node",
            endpoint_info,
            TEST_PUBKEY,
            Duration::from_secs(300),
        )
        .unwrap();

        // Wrong passkey code → wrong key → authentication failure
        let result = decode_passkey(
            "WRONG1",
            &generated.encrypted_payload,
            generated.expires_at,
        );
        assert!(result.is_err(), "Wrong code must fail decryption");
    }
}
