//! Pairing system for human device linking.
//!
//! Generates short codes (e.g., QD-7X3K) that expire after 10 minutes.
//! On successful pairing, returns an auth token mapped to an H_ID.
//! The H_ID is used as AI_ID when calling CLI tools — the backend
//! doesn't distinguish human from AI, it's just an identifier.

use rand::Rng;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const CODE_TTL: Duration = Duration::from_secs(600); // 10 minutes

struct PairingCode {
    h_id: String,
    expires_at: Instant,
}

#[derive(Clone)]
pub struct PairingState {
    codes: Arc<RwLock<HashMap<String, PairingCode>>>,
    tokens: Arc<RwLock<HashMap<String, String>>>, // token -> h_id
}

impl PairingState {
    pub fn new() -> Self {
        Self {
            codes: Arc::new(RwLock::new(HashMap::new())),
            tokens: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Generate a pairing code for an H_ID.
    /// Code format: XX-YYYY where XX is derived from h_id, YYYY is random alphanumeric.
    pub async fn generate_code(&self, h_id: &str) -> String {
        let mut rng = rand::rngs::OsRng;

        // Derive prefix from h_id (e.g., "human-alice" -> "AL")
        let raw_prefix = h_id
            .trim_start_matches("human-")
            .to_uppercase();
        let prefix = if raw_prefix.len() >= 2 {
            &raw_prefix[..2]
        } else {
            &raw_prefix
        };

        // Unambiguous alphanumeric charset (no I, O, 0, 1)
        let charset = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
        let suffix: String = (0..4)
            .map(|_| charset[rng.gen_range(0..charset.len())] as char)
            .collect();

        let code = format!("{}-{}", prefix, suffix);

        let mut codes = self.codes.write().await;
        // Prune expired codes
        codes.retain(|_, v| v.expires_at > Instant::now());
        codes.insert(
            code.clone(),
            PairingCode {
                h_id: h_id.to_string(),
                expires_at: Instant::now() + CODE_TTL,
            },
        );

        code
    }

    /// Validate a pairing code. On success, consumes the code and returns (h_id, token).
    pub async fn validate_code(&self, code: &str) -> Option<(String, String)> {
        let mut codes = self.codes.write().await;
        if let Some(pc) = codes.remove(code) {
            if pc.expires_at > Instant::now() {
                let token = generate_token();
                self.tokens
                    .write()
                    .await
                    .insert(token.clone(), pc.h_id.clone());
                return Some((pc.h_id, token));
            }
        }
        None
    }

    /// Resolve an auth token to its H_ID.
    pub async fn resolve_token(&self, token: &str) -> Option<String> {
        self.tokens.read().await.get(token).cloned()
    }
}

/// Generate a cryptographically random 32-character token.
fn generate_token() -> String {
    let mut rng = rand::rngs::OsRng;
    let charset = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..32)
        .map(|_| charset[rng.gen_range(0..charset.len())] as char)
        .collect()
}
