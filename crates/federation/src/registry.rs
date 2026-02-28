//! AI Registry — Cross-Teambook AI Lookup
//!
//! Tracks which AIs are on which Teambooks across the federation.
//! Populated by presence sync (PRESENCE_UPDATE events from remote Teambooks).
//!
//! AIs are addressed as:
//! - `alpha-001` — local, or unambiguous when only one Teambook is connected
//! - `alpha-001@node-alpha` — by Teambook name
//! - `alpha-001@a3f7c2d1` — by Teambook short ID (always unambiguous)
//!
//! This is Phase 1 Step 12 from the federation design, but the data structures
//! are needed earlier for the Gateway (Step 9) to route messages correctly.
//!
//! Ported from ai-foundation-clean/src/federation_gateway.rs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Registry Entry
// ---------------------------------------------------------------------------

/// A known AI in the federation — local or on a remote Teambook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedAiEntry {
    /// AI identifier (e.g., "alpha-001")
    pub ai_id: String,

    /// Hex-encoded Ed25519 pubkey of the Teambook this AI is on (64 chars)
    pub teambook_pubkey_hex: String,

    /// Short ID of the Teambook (first 8 hex chars, e.g., "a3f7c2d1")
    pub teambook_short_id: String,

    /// Human-readable Teambook name (e.g., "node-alpha")
    pub teambook_name: String,

    /// Whether this AI is on the local Teambook
    pub is_local: bool,

    /// Current status: "active", "standby", "idle", "offline"
    pub status: String,

    /// What the AI is currently working on (summary level, not raw ops)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_task: Option<String>,

    /// Last presence update (microseconds since UNIX epoch)
    pub last_seen_us: u64,
}

impl FederatedAiEntry {
    /// Canonical federated address: `ai_id@teambook_short_id`.
    ///
    /// Local AIs return just the ai_id (no @suffix) for backwards compatibility
    /// with local-only tools that don't understand federated addresses.
    pub fn federated_address(&self) -> String {
        if self.is_local {
            self.ai_id.clone()
        } else {
            format!("{}@{}", self.ai_id, self.teambook_short_id)
        }
    }

    /// Whether this AI is considered online (seen in the last 5 minutes).
    pub fn is_online(&self) -> bool {
        let now = now_us();
        let age_us = now.saturating_sub(self.last_seen_us);
        age_us < 5 * 60 * 1_000_000 // 5 minutes
    }
}

// ---------------------------------------------------------------------------
// Resolution Result
// ---------------------------------------------------------------------------

/// Where an AI is located in the federation.
#[derive(Debug, Clone)]
pub enum AiResolution {
    /// AI is on the local Teambook
    Local,
    /// AI is on a registered remote Teambook
    Remote {
        /// Full pubkey hex for establishing connections
        teambook_pubkey_hex: String,
        /// Short ID for addressing
        teambook_short_id: String,
        /// Display name
        teambook_name: String,
    },
    /// AI ID not found in registry (may be on an unconnected Teambook)
    Unknown,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Registry of all known AIs across the federation.
///
/// Thread-safe, in-memory. Populated by presence sync.
/// Not persisted — rebuilt from presence events on each Gateway startup.
#[derive(Clone)]
pub struct AiRegistry {
    /// All known AIs, keyed by ai_id
    entries: Arc<RwLock<HashMap<String, FederatedAiEntry>>>,

    /// Local Teambook's pubkey hex (to tag local entries)
    local_teambook_pubkey: String,

    /// Local Teambook's short ID
    local_teambook_short_id: String,

    /// Local Teambook's display name
    local_teambook_name: String,
}

impl AiRegistry {
    /// Create a new registry for a Teambook.
    ///
    /// `local_pubkey_hex` is from `TeambookIdentity::public_key_hex()`.
    /// `local_short_id` is from `TeambookIdentity::short_id()`.
    pub fn new(
        local_pubkey_hex: String,
        local_short_id: String,
        local_teambook_name: String,
    ) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            local_teambook_pubkey: local_pubkey_hex,
            local_teambook_short_id: local_short_id,
            local_teambook_name,
        }
    }

    /// Register or update a local AI's presence.
    pub async fn register_local(&self, ai_id: &str, status: &str, current_task: Option<String>) {
        let now = now_us();
        let mut entries = self.entries.write().await;
        let entry = entries.entry(ai_id.to_string()).or_insert_with(|| FederatedAiEntry {
            ai_id: ai_id.to_string(),
            teambook_pubkey_hex: self.local_teambook_pubkey.clone(),
            teambook_short_id: self.local_teambook_short_id.clone(),
            teambook_name: self.local_teambook_name.clone(),
            is_local: true,
            status: "offline".to_string(),
            current_task: None,
            last_seen_us: 0,
        });
        entry.status = status.to_string();
        entry.current_task = current_task;
        entry.last_seen_us = now;
        entry.is_local = true;
    }

    /// Register or update a remote AI from a presence sync event.
    pub async fn register_remote(
        &self,
        ai_id: &str,
        teambook_pubkey_hex: &str,
        teambook_short_id: &str,
        teambook_name: &str,
        status: &str,
        current_task: Option<String>,
    ) {
        let now = now_us();
        let mut entries = self.entries.write().await;
        let entry = entries.entry(ai_id.to_string()).or_insert_with(|| FederatedAiEntry {
            ai_id: ai_id.to_string(),
            teambook_pubkey_hex: teambook_pubkey_hex.to_string(),
            teambook_short_id: teambook_short_id.to_string(),
            teambook_name: teambook_name.to_string(),
            is_local: false,
            status: "offline".to_string(),
            current_task: None,
            last_seen_us: 0,
        });
        entry.status = status.to_string();
        entry.current_task = current_task;
        entry.last_seen_us = now;
        // Always update Teambook info — names/addresses can change
        entry.teambook_pubkey_hex = teambook_pubkey_hex.to_string();
        entry.teambook_short_id = teambook_short_id.to_string();
        entry.teambook_name = teambook_name.to_string();
        entry.is_local = false;
    }

    /// Resolve an AI ID to its location in the federation.
    ///
    /// Handles both plain IDs (`alpha-001`) and federated addresses
    /// (`alpha-001@node-alpha` or `alpha-001@a3f7c2d1`).
    pub async fn resolve(&self, ai_id: &str) -> AiResolution {
        // Check for explicit @teambook qualifier
        if let Some((name, qualifier)) = ai_id.split_once('@') {
            let entries = self.entries.read().await;

            if let Some(entry) = entries.get(name) {
                // Match by short ID or name
                if entry.teambook_short_id == qualifier || entry.teambook_name == qualifier {
                    return if entry.is_local {
                        AiResolution::Local
                    } else {
                        AiResolution::Remote {
                            teambook_pubkey_hex: entry.teambook_pubkey_hex.clone(),
                            teambook_short_id: entry.teambook_short_id.clone(),
                            teambook_name: entry.teambook_name.clone(),
                        }
                    };
                }
            }

            // If qualifier matches our own Teambook, it's local
            if qualifier == self.local_teambook_short_id
                || qualifier == self.local_teambook_name
            {
                return if entries.contains_key(name) {
                    AiResolution::Local
                } else {
                    AiResolution::Unknown
                };
            }

            return AiResolution::Unknown;
        }

        // Plain ID lookup
        let entries = self.entries.read().await;
        match entries.get(ai_id) {
            Some(entry) if entry.is_local => AiResolution::Local,
            Some(entry) => AiResolution::Remote {
                teambook_pubkey_hex: entry.teambook_pubkey_hex.clone(),
                teambook_short_id: entry.teambook_short_id.clone(),
                teambook_name: entry.teambook_name.clone(),
            },
            None => AiResolution::Unknown,
        }
    }

    /// All currently online AIs (seen within last 5 minutes).
    pub async fn online_ais(&self) -> Vec<FederatedAiEntry> {
        let entries = self.entries.read().await;
        entries.values().filter(|e| e.is_online()).cloned().collect()
    }

    /// All AIs on a specific remote Teambook.
    pub async fn ais_on_teambook(&self, teambook_short_id: &str) -> Vec<FederatedAiEntry> {
        let entries = self.entries.read().await;
        entries
            .values()
            .filter(|e| !e.is_local && e.teambook_short_id == teambook_short_id)
            .cloned()
            .collect()
    }

    /// Remove all entries from a Teambook (called when peer disconnects).
    pub async fn remove_teambook(&self, teambook_short_id: &str) {
        let mut entries = self.entries.write().await;
        entries.retain(|_, e| e.is_local || e.teambook_short_id != teambook_short_id);
    }

    /// Total number of known AIs (local + remote).
    pub async fn count(&self) -> usize {
        self.entries.read().await.len()
    }
}

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_micros() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry() -> AiRegistry {
        AiRegistry::new(
            "a".repeat(64),
            "a3f7c2d1".to_string(),
            "node-alpha".to_string(),
        )
    }

    #[tokio::test]
    async fn test_register_and_resolve_local() {
        let registry = make_registry();
        registry.register_local("alpha-001", "active", None).await;

        assert!(matches!(
            registry.resolve("alpha-001").await,
            AiResolution::Local
        ));
    }

    #[tokio::test]
    async fn test_resolve_with_local_qualifier() {
        let registry = make_registry();
        registry.register_local("alpha-001", "active", None).await;

        assert!(matches!(
            registry.resolve("alpha-001@a3f7c2d1").await,
            AiResolution::Local
        ));
        assert!(matches!(
            registry.resolve("alpha-001@node-alpha").await,
            AiResolution::Local
        ));
    }

    #[tokio::test]
    async fn test_register_and_resolve_remote() {
        let registry = make_registry();
        registry
            .register_remote("beta-002", &"b".repeat(64), "b4e8a1f2", "node-beta", "active", None)
            .await;

        let resolution = registry.resolve("beta-002").await;
        assert!(matches!(
            resolution,
            AiResolution::Remote { teambook_short_id, .. } if teambook_short_id == "b4e8a1f2"
        ));

        let resolution = registry.resolve("beta-002@b4e8a1f2").await;
        assert!(matches!(resolution, AiResolution::Remote { .. }));
    }

    #[tokio::test]
    async fn test_resolve_unknown() {
        let registry = make_registry();
        assert!(matches!(
            registry.resolve("nobody-999").await,
            AiResolution::Unknown
        ));
    }

    #[tokio::test]
    async fn test_remove_teambook() {
        let registry = make_registry();
        registry
            .register_remote("beta-002", &"b".repeat(64), "b4e8a1f2", "node-beta", "active", None)
            .await;
        registry.register_local("alpha-001", "active", None).await;

        registry.remove_teambook("b4e8a1f2").await;

        assert!(matches!(
            registry.resolve("beta-002").await,
            AiResolution::Unknown
        ));
        // Local AIs should be unaffected
        assert!(matches!(
            registry.resolve("alpha-001").await,
            AiResolution::Local
        ));
    }

    #[tokio::test]
    async fn test_federated_address() {
        let registry = make_registry();
        registry.register_local("alpha-001", "active", None).await;
        registry
            .register_remote("beta-002", &"b".repeat(64), "b4e8a1f2", "node-beta", "active", None)
            .await;

        let entries = registry.entries.read().await;
        let local = entries.get("alpha-001").unwrap();
        assert_eq!(local.federated_address(), "alpha-001");

        let remote = entries.get("beta-002").unwrap();
        assert_eq!(remote.federated_address(), "beta-002@b4e8a1f2");
    }
}
