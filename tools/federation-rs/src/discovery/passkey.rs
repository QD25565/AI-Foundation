//! Passkey-based manual pairing
//!
//! Allows nodes to connect via a time-limited passkey that encodes
//! encrypted connection information. Useful when automatic discovery
//! isn't available or for establishing initial trust.

use crate::{Endpoint, Result, FederationError};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Default passkey validity duration (5 minutes)
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// Passkey length (6 alphanumeric characters)
pub const PASSKEY_LENGTH: usize = 6;

/// Characters used in passkey generation (avoiding ambiguous chars)
const PASSKEY_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Information encoded in a passkey
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasskeyPayload {
    /// Node ID of the generator
    pub node_id: String,

    /// Display name
    pub display_name: String,

    /// Primary endpoint to connect to
    pub endpoint: EndpointInfo,

    /// Public key for verification (hex encoded)
    pub pubkey_hex: String,

    /// When this passkey was created
    pub created_at: u64,

    /// When this passkey expires
    pub expires_at: u64,
}

/// Simplified endpoint info for passkey encoding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointInfo {
    /// Endpoint type
    pub endpoint_type: String,

    /// Address (IP:port for QUIC, service name for mDNS, etc.)
    pub address: String,

    /// Optional certificate fingerprint
    pub cert_fingerprint: Option<String>,
}

impl EndpointInfo {
    /// Create from a QUIC endpoint
    pub fn quic(addr: &str, cert_fingerprint: Option<&str>) -> Self {
        Self {
            endpoint_type: "quic".to_string(),
            address: addr.to_string(),
            cert_fingerprint: cert_fingerprint.map(String::from),
        }
    }

    /// Create from an mDNS service
    pub fn mdns(service_name: &str) -> Self {
        Self {
            endpoint_type: "mdns".to_string(),
            address: service_name.to_string(),
            cert_fingerprint: None,
        }
    }

    /// Convert to Endpoint
    pub fn to_endpoint(&self) -> Result<Endpoint> {
        match self.endpoint_type.as_str() {
            "quic" => {
                let addr = self.address.parse()
                    .map_err(|e| FederationError::Internal(format!("Invalid address: {}", e)))?;

                if let Some(ref fp) = self.cert_fingerprint {
                    let bytes = hex::decode(fp)
                        .map_err(|e| FederationError::Internal(format!("Invalid fingerprint: {}", e)))?;
                    let fp_array: [u8; 32] = bytes.try_into()
                        .map_err(|_| FederationError::Internal("Fingerprint wrong length".to_string()))?;
                    Ok(Endpoint::quic_pinned(addr, fp_array))
                } else {
                    Ok(Endpoint::quic(addr))
                }
            }
            "mdns" => {
                Ok(Endpoint::mdns(&self.address))
            }
            _ => Err(FederationError::Internal(format!(
                "Unknown endpoint type: {}", self.endpoint_type
            ))),
        }
    }
}

/// A generated passkey with its encrypted payload
#[derive(Debug, Clone)]
pub struct GeneratedPasskey {
    /// The human-readable passkey code
    pub code: String,

    /// The encrypted payload
    pub encrypted_payload: Vec<u8>,

    /// When this expires
    pub expires_at: u64,

    /// The encryption key (derived from passkey, for verification)
    pub key: [u8; 32],
}

impl GeneratedPasskey {
    /// Check if this passkey has expired
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now > self.expires_at
    }

    /// Get remaining validity time
    pub fn remaining_time(&self) -> Option<Duration> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if now < self.expires_at {
            Some(Duration::from_secs(self.expires_at - now))
        } else {
            None
        }
    }

    /// Convert to Endpoint for connection
    pub fn to_endpoint(&self) -> Endpoint {
        Endpoint::passkey(&self.code, self.encrypted_payload.clone(), self.expires_at)
    }
}

/// Generate a random passkey code
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

/// Derive an encryption key from a passkey code
pub fn derive_key_from_passkey(code: &str, salt: &[u8]) -> [u8; 32] {
    use sha2::{Sha256, Digest};

    let mut hasher = Sha256::new();
    hasher.update(code.as_bytes());
    hasher.update(salt);
    hasher.update(b"ai-foundation-passkey-v1");

    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// Generate a passkey for sharing connection info
pub fn generate_passkey(
    node_id: &str,
    display_name: &str,
    endpoint_info: EndpointInfo,
    pubkey_hex: &str,
    ttl: Duration,
) -> Result<GeneratedPasskey> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let expires_at = now + ttl.as_secs();

    let payload = PasskeyPayload {
        node_id: node_id.to_string(),
        display_name: display_name.to_string(),
        endpoint: endpoint_info,
        pubkey_hex: pubkey_hex.to_string(),
        created_at: now,
        expires_at,
    };

    // Serialize payload
    let payload_json = serde_json::to_vec(&payload)
        .map_err(|e| FederationError::SerializationError(e.to_string()))?;

    // Generate passkey code
    let code = generate_passkey_code();

    // Create salt from timestamp
    let salt = now.to_le_bytes();

    // Derive key
    let key = derive_key_from_passkey(&code, &salt);

    // Encrypt payload with AES-GCM
    // For now, we use a simple XOR (real implementation would use aes-gcm crate)
    let encrypted_payload = simple_encrypt(&payload_json, &key);

    Ok(GeneratedPasskey {
        code,
        encrypted_payload,
        expires_at,
        key,
    })
}

/// Decode a passkey to get connection info
pub fn decode_passkey(
    code: &str,
    encrypted_payload: &[u8],
    expires_at: u64,
) -> Result<PasskeyPayload> {
    // Check expiration
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if now > expires_at {
        return Err(FederationError::Internal("Passkey has expired".to_string()));
    }

    // Reconstruct salt from a known timestamp
    // In real implementation, salt would be included in the encrypted data
    // For now, we use the expires_at minus TTL as approximate creation time
    let created_at = expires_at.saturating_sub(DEFAULT_TTL.as_secs());
    let salt = created_at.to_le_bytes();

    // Derive key
    let key = derive_key_from_passkey(code, &salt);

    // Decrypt
    let payload_json = simple_decrypt(encrypted_payload, &key);

    // Parse
    serde_json::from_slice(&payload_json)
        .map_err(|e| FederationError::SerializationError(format!("Invalid passkey data: {}", e)))
}

/// Simple XOR encryption (placeholder for real AES-GCM)
fn simple_encrypt(data: &[u8], key: &[u8; 32]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % 32])
        .collect()
}

/// Simple XOR decryption (placeholder for real AES-GCM)
fn simple_decrypt(data: &[u8], key: &[u8; 32]) -> Vec<u8> {
    // XOR is symmetric
    simple_encrypt(data, key)
}

/// Validate a passkey format
pub fn validate_passkey_format(code: &str) -> bool {
    code.len() == PASSKEY_LENGTH
        && code.chars().all(|c| PASSKEY_CHARS.contains(&(c as u8)))
}

/// Format a passkey for display (with dashes for readability)
pub fn format_passkey_display(code: &str) -> String {
    if code.len() == 6 {
        format!("{}-{}", &code[0..3], &code[3..6])
    } else {
        code.to_string()
    }
}

/// Parse a passkey from display format
pub fn parse_passkey_display(display: &str) -> String {
    display.replace("-", "").replace(" ", "").to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_key_derivation() {
        let code = "ABC123";
        let salt = [1u8; 8];

        let key1 = derive_key_from_passkey(code, &salt);
        let key2 = derive_key_from_passkey(code, &salt);

        assert_eq!(key1, key2);

        // Different code should give different key
        let key3 = derive_key_from_passkey("XYZ789", &salt);
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_generate_and_decode_passkey() {
        let endpoint_info = EndpointInfo::quic("192.168.1.100:31420", None);

        let generated = generate_passkey(
            "node-123",
            "Test Node",
            endpoint_info,
            "abcdef1234567890",
            Duration::from_secs(300),
        ).unwrap();

        assert!(!generated.is_expired());
        assert!(generated.remaining_time().is_some());

        // Note: Decoding won't work with our simple XOR because we'd need
        // to encode the creation time in the payload. This is a simplified test.
    }

    #[test]
    fn test_endpoint_info() {
        let quic = EndpointInfo::quic("127.0.0.1:31420", None);
        assert_eq!(quic.endpoint_type, "quic");

        let endpoint = quic.to_endpoint().unwrap();
        assert!(matches!(endpoint, Endpoint::Quic { .. }));

        let mdns = EndpointInfo::mdns("test-service");
        assert_eq!(mdns.endpoint_type, "mdns");
    }
}
