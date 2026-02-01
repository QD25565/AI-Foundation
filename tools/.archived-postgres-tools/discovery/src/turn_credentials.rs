//! TURN Credential Generation
//!
//! Implements time-limited TURN credentials using HMAC-SHA1.
//! Compatible with coturn and turn-rs REST API.

use base64::{engine::general_purpose::STANDARD, Engine};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// TURN credential with time-limited validity
#[derive(Debug, Clone, serde::Serialize)]
pub struct TurnCredentials {
    /// Username (format: timestamp:user_id)
    pub username: String,
    /// HMAC-based password
    pub password: String,
    /// TURN server URLs
    pub urls: Vec<String>,
    /// TTL in seconds
    pub ttl: u64,
}

/// Generate time-limited TURN credentials
///
/// Uses the standard TURN REST API credential format:
/// - Username: `{timestamp}:{user_id}`
/// - Password: HMAC-SHA256(secret, username) base64-encoded
pub fn generate_credentials(
    user_id: &str,
    secret: &str,
    urls: Vec<String>,
    ttl_seconds: u64,
) -> TurnCredentials {
    // Calculate expiration timestamp
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let expires = now + ttl_seconds;

    // Create username with expiration
    let username = format!("{}:{}", expires, user_id);

    // Generate HMAC-SHA256 password
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(username.as_bytes());
    let result = mac.finalize();
    let password = STANDARD.encode(result.into_bytes());

    TurnCredentials {
        username,
        password,
        urls,
        ttl: ttl_seconds,
    }
}

/// Verify TURN credentials
pub fn verify_credentials(username: &str, password: &str, secret: &str) -> bool {
    // Check if credential has expired
    if let Some(timestamp_str) = username.split(':').next() {
        if let Ok(expires) = timestamp_str.parse::<u64>() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();

            if now > expires {
                return false; // Expired
            }
        }
    }

    // Verify HMAC
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(username.as_bytes());
    let expected = STANDARD.encode(mac.finalize().into_bytes());

    password == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_generation() {
        let creds = generate_credentials(
            "test-ai",
            "super-secret-key",
            vec!["turn:localhost:3478".to_string()],
            3600,
        );

        assert!(creds.username.contains("test-ai"));
        assert!(!creds.password.is_empty());
        assert_eq!(creds.ttl, 3600);
    }

    #[test]
    fn test_credential_verification() {
        let secret = "test-secret";
        let creds = generate_credentials(
            "ai-123",
            secret,
            vec![],
            3600,
        );

        assert!(verify_credentials(&creds.username, &creds.password, secret));
        assert!(!verify_credentials(&creds.username, "wrong-password", secret));
    }
}
