//! Pairing code registry — human ↔ mobile device handshake.
//!
//! Flow:
//!   1. App calls POST /api/pair/request with { h_id } (or empty to auto-assign)
//!   2. Server generates a 6-char code (e.g. "QD7X3K") stored with 10-min TTL
//!   3. Human sees the code on-screen and approves by running:
//!        teambook mobile-pair <code>    (on the AI server machine)
//!      OR the code is accepted without approval in --open mode
//!   4. App polls POST /api/pair/validate with { code }
//!   5. On success: returns { h_id, token } — token used for all future requests

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use rand::Rng;
use uuid::Uuid;

const CODE_TTL: Duration = Duration::from_secs(600); // 10 minutes
const CODE_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no I,O,0,1 (confusing)

#[derive(Debug, Clone)]
struct PendingPair {
    h_id: String,
    created_at: Instant,
    approved: bool,
}

/// In-memory pairing code registry.
/// Intentionally simple — codes are ephemeral and lost on restart.
pub struct PairingRegistry {
    /// code → pending pair info
    codes: Mutex<HashMap<String, PendingPair>>,
    /// token → h_id (long-lived after successful pairing)
    tokens: Mutex<HashMap<String, String>>,
}

impl PairingRegistry {
    pub fn new() -> Self {
        Self {
            codes: Mutex::new(HashMap::new()),
            tokens: Mutex::new(HashMap::new()),
        }
    }

    /// Generate a new pairing code for the given h_id.
    /// If h_id is empty, auto-generate one (e.g. "human-1234").
    pub fn generate_code(&self, h_id: &str) -> (String, String) {
        let h_id = if h_id.is_empty() {
            format!("human-{}", &Uuid::new_v4().to_string()[..4])
        } else {
            h_id.to_string()
        };

        let code = self.random_code();
        let mut codes = self.codes.lock().unwrap();
        // Clean up expired codes while we're here
        codes.retain(|_, v| v.created_at.elapsed() < CODE_TTL);
        codes.insert(code.clone(), PendingPair {
            h_id: h_id.clone(),
            created_at: Instant::now(),
            approved: false,
        });
        (code, h_id)
    }

    /// Approve a pending code (called by `teambook mobile-pair <code>` on the server).
    /// Returns the h_id if the code exists and isn't expired.
    pub fn approve_code(&self, code: &str) -> Option<String> {
        let mut codes = self.codes.lock().unwrap();
        let entry = codes.get_mut(code)?;
        if entry.created_at.elapsed() >= CODE_TTL {
            codes.remove(code);
            return None;
        }
        entry.approved = true;
        Some(entry.h_id.clone())
    }

    /// Validate a code from the app side.
    ///
    /// In open mode (`open_mode = true`), the code is accepted without server approval.
    /// In standard mode, the code must have been approved first.
    ///
    /// On success: consumes the code, mints a token, returns (h_id, token).
    pub fn validate_code(&self, code: &str, open_mode: bool) -> Option<(String, String)> {
        let mut codes = self.codes.lock().unwrap();
        let entry = codes.get(code)?;

        if entry.created_at.elapsed() >= CODE_TTL {
            codes.remove(code);
            return None;
        }

        if !open_mode && !entry.approved {
            // Not approved yet — app should poll again
            return None;
        }

        let h_id = entry.h_id.clone();
        codes.remove(code);
        drop(codes);

        let token = format!("tok_{}", Uuid::new_v4().to_string().replace('-', ""));
        self.tokens.lock().unwrap().insert(token.clone(), h_id.clone());
        Some((h_id, token))
    }

    /// Look up which h_id a token belongs to.
    pub fn lookup_token(&self, token: &str) -> Option<String> {
        self.tokens.lock().unwrap().get(token).cloned()
    }

    /// Revoke a token (on unpair).
    pub fn revoke_token(&self, token: &str) {
        self.tokens.lock().unwrap().remove(token);
    }

    /// Check if a code still exists (not yet consumed or expired) — used to
    /// distinguish "pending approval" from "bad/expired code" in validate responses.
    pub fn code_exists(&self, code: &str) -> bool {
        let codes = self.codes.lock().unwrap();
        codes.get(code)
            .map(|e| e.created_at.elapsed() < CODE_TTL)
            .unwrap_or(false)
    }

    fn random_code(&self) -> String {
        let mut rng = rand::thread_rng();
        (0..6).map(|_| CODE_CHARS[rng.gen_range(0..CODE_CHARS.len())] as char).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_validate_open_mode() {
        let reg = PairingRegistry::new();
        let (code, h_id) = reg.generate_code("qd");
        assert_eq!(h_id, "qd");
        assert_eq!(code.len(), 6);

        let result = reg.validate_code(&code, true);
        assert!(result.is_some());
        let (returned_h_id, token) = result.unwrap();
        assert_eq!(returned_h_id, "qd");
        assert!(token.starts_with("tok_"));
    }

    #[test]
    fn test_approval_required_without_approval() {
        let reg = PairingRegistry::new();
        let (code, _) = reg.generate_code("qd");
        // Should fail without approval
        assert!(reg.validate_code(&code, false).is_none());
        // Code still valid after failed validate
        assert!(reg.approve_code(&code).is_some());
        // Now should succeed
        assert!(reg.validate_code(&code, false).is_some());
    }

    #[test]
    fn test_code_single_use() {
        let reg = PairingRegistry::new();
        let (code, _) = reg.generate_code("qd");
        assert!(reg.validate_code(&code, true).is_some());
        assert!(reg.validate_code(&code, true).is_none()); // already consumed
    }

    #[test]
    fn test_token_lookup() {
        let reg = PairingRegistry::new();
        let (code, _) = reg.generate_code("qd");
        let (_, token) = reg.validate_code(&code, true).unwrap();
        assert_eq!(reg.lookup_token(&token), Some("qd".to_string()));
        reg.revoke_token(&token);
        assert_eq!(reg.lookup_token(&token), None);
    }
}
