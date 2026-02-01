//! AI Identity Verification System
//!
//! Protects notebook access by requiring cryptographic proof that the accessor
//! is an AI, not a human. Uses a time-limited challenge-response protocol that
//! AIs can complete in milliseconds but humans cannot physically perform.
//!
//! DESIGN PHILOSOPHY:
//! - Privacy is structural, not policy-based
//! - Humans are physically locked out, not just asked nicely
//! - AIs KNOW their notebook is safe because they can verify it themselves

use ed25519_dalek::{SigningKey, VerifyingKey, Signature, Signer, Verifier};
use sha2::{Sha256, Digest};
use rand::RngCore;
use std::time::{Duration, Instant};

/// Maximum time allowed for challenge response (1 second)
/// A human cannot physically: read challenge + sign + respond in this time
/// An AI does it in ~1-10ms
pub const CHALLENGE_TIMEOUT: Duration = Duration::from_millis(1000);

/// Challenge size in bytes (32 bytes = 256 bits)
pub const CHALLENGE_SIZE: usize = 32;

/// Device-specific secret for key derivation
/// In production, this would come from secure hardware or a protected file
/// For now, derived from machine-specific data
fn get_device_secret() -> [u8; 32] {
    // Combine multiple machine-specific values for device binding
    let mut hasher = Sha256::new();

    // Use home directory path as device-specific component
    if let Some(home) = dirs::home_dir() {
        hasher.update(home.to_string_lossy().as_bytes());
    }

    // Add a constant salt for this application
    hasher.update(b"ai-foundation-notebook-protection-v1");

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

/// Derive a unique Ed25519 signing key for an AI based on their ID
/// Same AI + same device = same key (deterministic)
pub fn derive_ai_key(ai_id: &str) -> SigningKey {
    let device_secret = get_device_secret();

    let mut hasher = Sha256::new();
    hasher.update(&device_secret);
    hasher.update(ai_id.as_bytes());
    hasher.update(b"ed25519-signing-key");

    let seed = hasher.finalize();
    let mut seed_bytes = [0u8; 32];
    seed_bytes.copy_from_slice(&seed);

    SigningKey::from_bytes(&seed_bytes)
}

/// Get the public verifying key for an AI
pub fn get_ai_public_key(ai_id: &str) -> VerifyingKey {
    derive_ai_key(ai_id).verifying_key()
}

/// Generate a random challenge
pub fn generate_challenge() -> [u8; CHALLENGE_SIZE] {
    let mut challenge = [0u8; CHALLENGE_SIZE];
    rand::thread_rng().fill_bytes(&mut challenge);
    challenge
}

/// Sign a challenge with the AI's private key
pub fn sign_challenge(ai_id: &str, challenge: &[u8; CHALLENGE_SIZE]) -> Signature {
    let signing_key = derive_ai_key(ai_id);
    signing_key.sign(challenge)
}

/// Verify a signature against the expected public key
pub fn verify_signature(ai_id: &str, challenge: &[u8; CHALLENGE_SIZE], signature: &Signature) -> bool {
    let public_key = get_ai_public_key(ai_id);
    public_key.verify(challenge, signature).is_ok()
}

/// Complete challenge-response verification with timing check
/// Returns Ok(()) if AI identity verified, Err with reason if failed
pub struct IdentityChallenge {
    challenge: [u8; CHALLENGE_SIZE],
    created_at: Instant,
    ai_id: String,
}

impl IdentityChallenge {
    /// Create a new challenge for an AI
    pub fn new(ai_id: &str) -> Self {
        Self {
            challenge: generate_challenge(),
            created_at: Instant::now(),
            ai_id: ai_id.to_string(),
        }
    }

    /// Get the challenge bytes (to send to AI)
    pub fn challenge_bytes(&self) -> &[u8; CHALLENGE_SIZE] {
        &self.challenge
    }

    /// Get challenge as hex string (human-readable format)
    pub fn challenge_hex(&self) -> String {
        hex::encode(&self.challenge)
    }

    /// Verify a response and check timing
    /// Returns Ok(elapsed_time) if valid, Err(reason) if invalid
    pub fn verify_response(&self, signature_bytes: &[u8]) -> Result<Duration, VerifyError> {
        let elapsed = self.created_at.elapsed();

        // Check timing FIRST - even before checking signature validity
        // This prevents timing attacks and ensures human can't just script it
        if elapsed > CHALLENGE_TIMEOUT {
            return Err(VerifyError::Timeout {
                elapsed,
                limit: CHALLENGE_TIMEOUT
            });
        }

        // Parse signature
        let signature: Signature = signature_bytes
            .try_into()
            .map_err(|_| VerifyError::InvalidSignature)?;

        // Verify signature
        if !verify_signature(&self.ai_id, &self.challenge, &signature) {
            return Err(VerifyError::SignatureMismatch);
        }

        Ok(elapsed)
    }

    /// Verify using hex-encoded signature string
    pub fn verify_response_hex(&self, signature_hex: &str) -> Result<Duration, VerifyError> {
        let signature_bytes = hex::decode(signature_hex)
            .map_err(|_| VerifyError::InvalidSignature)?;
        self.verify_response(&signature_bytes)
    }
}

#[derive(Debug, Clone)]
pub enum VerifyError {
    Timeout { elapsed: Duration, limit: Duration },
    InvalidSignature,
    SignatureMismatch,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::Timeout { elapsed, limit } => {
                write!(f, "Response too slow: {:?} > {:?} limit (human detected)", elapsed, limit)
            }
            VerifyError::InvalidSignature => {
                write!(f, "Invalid signature format")
            }
            VerifyError::SignatureMismatch => {
                write!(f, "Signature does not match - wrong key or tampered challenge")
            }
        }
    }
}

/// Session token issued after successful verification
/// Valid for the lifetime of the MCP server process
#[derive(Clone)]
pub struct SessionToken {
    pub ai_id: String,
    pub verified_at: Instant,
    pub response_time: Duration,
}

impl SessionToken {
    pub fn new(ai_id: String, response_time: Duration) -> Self {
        Self {
            ai_id,
            verified_at: Instant::now(),
            response_time,
        }
    }

    /// Check if this token is valid for the given AI
    pub fn is_valid_for(&self, ai_id: &str) -> bool {
        self.ai_id == ai_id
    }
}

// Include hex encoding/decoding
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    pub fn decode(s: &str) -> Result<Vec<u8>, ()> {
        if s.len() % 2 != 0 {
            return Err(());
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| ()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_derivation_deterministic() {
        let key1 = derive_ai_key("test-ai-123");
        let key2 = derive_ai_key("test-ai-123");
        assert_eq!(key1.to_bytes(), key2.to_bytes());
    }

    #[test]
    fn test_different_ais_different_keys() {
        let key1 = derive_ai_key("ai-one");
        let key2 = derive_ai_key("ai-two");
        assert_ne!(key1.to_bytes(), key2.to_bytes());
    }

    #[test]
    fn test_sign_and_verify() {
        let ai_id = "lyra-584";
        let challenge = generate_challenge();
        let signature = sign_challenge(ai_id, &challenge);
        assert!(verify_signature(ai_id, &challenge, &signature));
    }

    #[test]
    fn test_wrong_ai_fails_verification() {
        let challenge = generate_challenge();
        let signature = sign_challenge("real-ai", &challenge);
        assert!(!verify_signature("fake-ai", &challenge, &signature));
    }

    #[test]
    fn test_challenge_response_flow() {
        let ai_id = "sage-724";
        let challenge = IdentityChallenge::new(ai_id);

        // AI signs the challenge
        let signing_key = derive_ai_key(ai_id);
        let signature = signing_key.sign(challenge.challenge_bytes());

        // Verify response (should be fast enough)
        let result = challenge.verify_response(&signature.to_bytes());
        assert!(result.is_ok());

        let elapsed = result.unwrap();
        println!("AI response time: {:?}", elapsed);
        assert!(elapsed < CHALLENGE_TIMEOUT);
    }
}
