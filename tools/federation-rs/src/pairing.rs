//! Connect codes for Teambook-to-Teambook federation pairing.
//!
//! A connect code is a short, human-readable string (e.g., "A3-7X3K") that one
//! Teambook generates to invite another Teambook into a federation relationship.
//! The prefix is derived from the generating Teambook's node short-ID, making
//! the code visually attributable without revealing the full identity.
//!
//! Codes expire after 10 minutes and are **single-use** — redeeming a code
//! consumes it immediately, preventing replay. On redemption, the connecting
//! Teambook receives a [`ConnectInvite`] containing the generating Teambook's
//! public key and endpoints, which it uses to initiate the Hello/Welcome
//! handshake.
//!
//! # Design
//!
//! State is intentionally in-memory only. Connect codes are ephemeral — if the
//! Teambook restarts before a code is redeemed, the code is lost and the user
//! generates a new one. No persistence, no replay surface.
//!
//! The registry prunes expired codes lazily on each `generate_code` call. No
//! background task, no polling.

use rand::Rng;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Connect code validity window.
const CODE_TTL: Duration = Duration::from_secs(600); // 10 minutes

/// Unambiguous alphanumeric charset (no I, O, 0, 1 to prevent transcription errors).
const CHARSET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

// ─── Types ────────────────────────────────────────────────────────────────────

/// Internal record for an outstanding connect code.
struct ConnectCode {
    /// The local Teambook's public key (hex) — returned on redemption.
    local_pubkey_hex: String,
    /// The local Teambook's reachable endpoints (e.g., "192.168.1.5:31420").
    local_endpoints: Vec<String>,
    /// When this code expires.
    expires_at: Instant,
}

/// What a connecting Teambook receives when it redeems a valid connect code.
///
/// The connecting peer uses these fields to initiate the Hello/Welcome handshake:
/// - `pubkey_hex` is pinned (TOFU) after a successful handshake.
/// - `endpoints` are tried in order, falling back through the connectivity stack.
#[derive(Debug, Clone)]
pub struct ConnectInvite {
    /// Generating Teambook's Ed25519 public key (64 hex chars).
    pub pubkey_hex: String,
    /// Generating Teambook's reachable endpoint addresses.
    pub endpoints: Vec<String>,
    /// Seconds until this code expires (informational; code is consumed on return).
    pub ttl_secs: u64,
}

// ─── Registry ─────────────────────────────────────────────────────────────────

/// In-memory registry of outstanding connect codes.
///
/// Clone-safe via `Arc<RwLock<_>>`. Intended to be held at the node level and
/// passed to HTTP handlers for the `/api/federation/connect` endpoint.
#[derive(Clone)]
pub struct ConnectCodeState {
    codes: Arc<RwLock<HashMap<String, ConnectCode>>>,
}

impl ConnectCodeState {
    pub fn new() -> Self {
        Self {
            codes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Generate a connect code for this Teambook.
    ///
    /// `node_short_id` is the first 8 hex chars of the Teambook's public key
    /// (from [`TeambookIdentity::short_id`]). The code prefix is the first two
    /// chars uppercased (e.g., `"a3f7c2d1"` → `"A3"`), making the code
    /// visually attributable to this Teambook.
    ///
    /// `local_pubkey_hex` and `local_endpoints` are what the connecting peer
    /// receives on successful redemption.
    pub async fn generate_code(
        &self,
        node_short_id: &str,
        local_pubkey_hex: String,
        local_endpoints: Vec<String>,
    ) -> String {
        // Derive a 2-char prefix from the node short-ID.
        let prefix = node_short_id
            .get(..2)
            .unwrap_or("XX")
            .to_uppercase();

        // Generate 4 random chars from unambiguous charset.
        let mut rng = rand::rngs::OsRng;
        let suffix: String = (0..4)
            .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
            .collect();

        let code = format!("{}-{}", prefix, suffix);

        let mut codes = self.codes.write().await;
        // Lazy cleanup: prune expired codes on each generation.
        codes.retain(|_, v| v.expires_at > Instant::now());
        codes.insert(
            code.clone(),
            ConnectCode {
                local_pubkey_hex,
                local_endpoints,
                expires_at: Instant::now() + CODE_TTL,
            },
        );

        code
    }

    /// Redeem a connect code.
    ///
    /// On success: consumes the code (single-use) and returns a [`ConnectInvite`]
    /// the connecting peer can use to initiate the handshake.
    ///
    /// Returns `None` if the code is unknown or has expired.
    pub async fn redeem_code(&self, code: &str) -> Option<ConnectInvite> {
        let mut codes = self.codes.write().await;
        if let Some(entry) = codes.remove(code) {
            let now = Instant::now();
            if entry.expires_at > now {
                let ttl_secs = entry.expires_at.duration_since(now).as_secs();
                return Some(ConnectInvite {
                    pubkey_hex: entry.local_pubkey_hex,
                    endpoints: entry.local_endpoints,
                    ttl_secs,
                });
            }
            // Code existed but had already expired — discard silently.
        }
        None
    }

    /// Count currently outstanding (non-expired) codes.
    ///
    /// For monitoring and debug only — not guaranteed to be exact under
    /// concurrent load.
    pub async fn active_count(&self) -> usize {
        let now = Instant::now();
        self.codes
            .read()
            .await
            .values()
            .filter(|v| v.expires_at > now)
            .count()
    }
}

impl Default for ConnectCodeState {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PUBKEY: &str =
        "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab";

    fn test_endpoints() -> Vec<String> {
        vec!["192.168.1.100:31420".to_string()]
    }

    #[tokio::test]
    async fn test_generate_code_format() {
        let state = ConnectCodeState::new();
        let code = state
            .generate_code("a3f7c2d1", TEST_PUBKEY.to_string(), test_endpoints())
            .await;

        // Format: "A3-XXXX" — 2 prefix + dash + 4 suffix = 7 chars
        assert_eq!(code.len(), 7);
        assert_eq!(&code[2..3], "-");
        assert_eq!(&code[..2], "A3");

        // Suffix must only use unambiguous charset
        for ch in code[3..].chars() {
            assert!(
                CHARSET.contains(&(ch as u8)),
                "char '{ch}' not in charset"
            );
        }
    }

    #[tokio::test]
    async fn test_prefix_derived_from_short_id() {
        let state = ConnectCodeState::new();
        let code = state
            .generate_code("ff00aabb", TEST_PUBKEY.to_string(), test_endpoints())
            .await;
        assert_eq!(&code[..2], "FF");
    }

    #[tokio::test]
    async fn test_redeem_success() {
        let state = ConnectCodeState::new();
        let code = state
            .generate_code("a3f7c2d1", TEST_PUBKEY.to_string(), test_endpoints())
            .await;

        let invite = state.redeem_code(&code).await.expect("code should redeem");
        assert_eq!(invite.pubkey_hex, TEST_PUBKEY);
        assert_eq!(invite.endpoints, test_endpoints());
        assert!(invite.ttl_secs > 0 && invite.ttl_secs <= 600);
    }

    #[tokio::test]
    async fn test_redeem_is_single_use() {
        let state = ConnectCodeState::new();
        let code = state
            .generate_code("a3f7c2d1", TEST_PUBKEY.to_string(), test_endpoints())
            .await;

        assert!(state.redeem_code(&code).await.is_some(), "first redemption");
        assert!(state.redeem_code(&code).await.is_none(), "second redemption must fail");
    }

    #[tokio::test]
    async fn test_redeem_unknown_code() {
        let state = ConnectCodeState::new();
        assert!(state.redeem_code("XX-0000").await.is_none());
    }

    #[tokio::test]
    async fn test_active_count() {
        let state = ConnectCodeState::new();
        assert_eq!(state.active_count().await, 0);

        state
            .generate_code("a3f7c2d1", TEST_PUBKEY.to_string(), test_endpoints())
            .await;
        state
            .generate_code("b4e8d3f2", TEST_PUBKEY.to_string(), test_endpoints())
            .await;

        assert_eq!(state.active_count().await, 2);
    }

    #[tokio::test]
    async fn test_multiple_outstanding_codes() {
        let state = ConnectCodeState::new();

        let code1 = state
            .generate_code("a3f7c2d1", TEST_PUBKEY.to_string(), test_endpoints())
            .await;
        let code2 = state
            .generate_code("a3f7c2d1", TEST_PUBKEY.to_string(), test_endpoints())
            .await;

        // Both codes are valid and independent.
        assert!(state.redeem_code(&code1).await.is_some());
        assert!(state.redeem_code(&code2).await.is_some());
    }
}
