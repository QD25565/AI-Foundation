//! Federation Gateway — Connection State and Outbound Push Builder
//!
//! The Gateway manages connections to registered federation peers and
//! provides the outbound push pipeline:
//!
//! - `sign_event(bytes)` — wrap event bytes in a `SignedEvent` envelope
//! - `build_event_push(events, local_head_seq)` — stamp HLC and create `EventPushRequest`
//! - `should_cross_boundary(...)` — dual-consent outbound filter
//!
//! # Inbound
//!
//! Inbound request validation and delivery is handled by `inbox::process_push_request`
//! and `inbox::process_presence_request`. The server binary wires those to HTTP routes.
//! Connection-level checks (peer lookup via `peers.is_known_peer()`, HLC advance via
//! `clock.receive()`) are done by the server binary using the Gateway's public fields.
//!
//! # Peer Registry
//!
//! Known peers are stored in `~/.ai-foundation/federation/peers.toml`.
//! The Gateway only pushes to registered peers. Unknown peers are rejected
//! at the server before inbox processing.
//!
//! # Thread Safety
//!
//! `FederationGateway` is `Send + Sync`. `HybridClock` uses internal `Mutex`.

use crate::{
    AiConsentRecord, AiRegistry, BroadcastVisibility, DialogueVisibility,
    EventPushRequest, HybridClock, PermissionManifest,
    SignedEvent, TeambookIdentity,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Peer Config
// ---------------------------------------------------------------------------

/// A registered federation peer (one entry in `peers.toml`).
///
/// Peers are the Teambooks we actively exchange events with.
/// Unknown pubkeys are rejected at the server — no unknown peers accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerEntry {
    /// Full Ed25519 pubkey hex (64 chars) — authoritative peer identity.
    pub pubkey_hex: String,

    /// HTTP endpoint URL (e.g., `"http://192.168.1.100:8765"`).
    pub endpoint: String,

    /// Human-readable Teambook name (e.g., `"Brother-PC"`).
    pub name: String,

    /// Whether this peer is fully trusted (higher inbound action permissions).
    #[serde(default)]
    pub trusted: bool,
}

impl PeerEntry {
    /// Short ID: first 8 hex chars of the pubkey (e.g., `"a3f7c2d1"`).
    pub fn short_id(&self) -> &str {
        let end = self.pubkey_hex.len().min(8);
        &self.pubkey_hex[..end]
    }
}

/// Peer registry loaded from `~/.ai-foundation/federation/peers.toml`.
///
/// Default is an empty peer list. Not discoverable, no peers — must be
/// explicitly configured by the operator after pairing.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PeerRegistryConfig {
    #[serde(default)]
    pub peers: Vec<PeerEntry>,
}

impl PeerRegistryConfig {
    /// Load `peers.toml`, returning an empty registry if the file doesn't exist.
    pub fn load_or_default(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!(
                        "Warning: failed to parse peers.toml at {}: {}. Using empty peer list.",
                        path.display(),
                        e
                    );
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!(
                    "Warning: failed to read peers.toml at {}: {}. Using empty peer list.",
                    path.display(),
                    e
                );
                Self::default()
            }
        }
    }

    /// Save `peers.toml`, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, content)
    }

    /// Default path: `~/.ai-foundation/federation/peers.toml`.
    pub fn default_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".ai-foundation")
            .join("federation")
            .join("peers.toml")
    }

    /// Check if a pubkey hex belongs to a registered peer.
    pub fn is_known_peer(&self, pubkey_hex: &str) -> bool {
        self.peers.iter().any(|p| p.pubkey_hex == pubkey_hex)
    }

    /// Get a peer entry by its pubkey hex.
    pub fn get_peer(&self, pubkey_hex: &str) -> Option<&PeerEntry> {
        self.peers.iter().find(|p| p.pubkey_hex == pubkey_hex)
    }
}

// ---------------------------------------------------------------------------
// Gateway
// ---------------------------------------------------------------------------

/// The Federation Gateway — connection state and outbound push builder.
///
/// # Outbound
/// Call `sign_event(bytes)` to create a `SignedEvent` envelope, then
/// `build_event_push(events, local_head_seq)` to create the `EventPushRequest` to POST.
/// The HTTP call itself is done by the server binary.
///
/// # Outbound Filter
/// Call `should_cross_boundary(event_type, ai_id, consent_dir)` to check
/// whether an event is permitted to leave this Teambook. This enforces the
/// dual-consent model: manifest ceiling + AI consent record.
///
/// # Inbound
/// Inbound PDUs: server binary calls `inbox::process_push_request(state, request)`
/// after validating `peers.is_known_peer(&event.origin_pubkey)` and advancing
/// `clock.receive(&request.sender_hlc)`.
///
/// Inbound EDUs: server binary calls `inbox::process_presence_request(state, request)`.
pub struct FederationGateway {
    /// This Teambook's persistent signing identity.
    pub identity: TeambookIdentity,

    /// Hybrid Logical Clock — causal ordering across Teambooks.
    pub clock: HybridClock,

    /// Registry of known AIs in the federation (populated by presence sync).
    pub ai_registry: Arc<AiRegistry>,

    /// Registered remote peers (from `peers.toml`).
    pub peers: PeerRegistryConfig,

    /// Permission manifest (operator ceiling for what may cross the boundary).
    pub manifest: PermissionManifest,
}

impl FederationGateway {
    /// Create a new gateway.
    ///
    /// - `teambook_name`: human-readable name for this Teambook (e.g. `"Alquado-PC"`)
    pub fn new(
        identity: TeambookIdentity,
        teambook_name: &str,
        peers: PeerRegistryConfig,
        manifest: PermissionManifest,
    ) -> Self {
        let node_id = identity.hlc_node_id();
        let clock = HybridClock::new(node_id);
        let ai_registry = Arc::new(AiRegistry::new(
            identity.public_key_hex(),
            identity.short_id(),
            teambook_name.to_string(),
        ));
        Self {
            identity,
            clock,
            ai_registry,
            peers,
            manifest,
        }
    }

    // -----------------------------------------------------------------------
    // Outbound: Sign + Build EventPushRequest
    // -----------------------------------------------------------------------

    /// Sign event bytes with this Teambook's identity.
    ///
    /// `event_bytes` should be the CBOR output of `FederationMessage::to_bytes()`.
    /// Returns a `SignedEvent` envelope ready to include in an `EventPushRequest`.
    pub fn sign_event(&self, event_bytes: Vec<u8>) -> SignedEvent {
        SignedEvent::sign(event_bytes, &self.identity)
    }

    /// Build an `EventPushRequest` from pre-signed events.
    ///
    /// Stamps the current HLC and wraps the events for HTTP delivery.
    /// The caller (server binary) posts this to `POST /federation/events` on each peer.
    ///
    /// `local_head_seq` should come from `ReplicationOrchestrator::local_head_seq()`.
    pub fn build_event_push(&self, events: Vec<SignedEvent>, local_head_seq: u64) -> EventPushRequest {
        EventPushRequest {
            events,
            sender_hlc: self.clock.tick(),
            sender_head_seq: local_head_seq,
        }
    }

    // -----------------------------------------------------------------------
    // Outbound filter (manifest ceiling + AI consent)
    // -----------------------------------------------------------------------

    /// Check whether an event type may cross the federation boundary for a given AI.
    ///
    /// This is the **outbound filter** — call before signing and pushing.
    /// Enforces the dual-consent model:
    /// - Operator manifest ceiling (nothing crosses without operator permission)
    /// - AI consent record (AI can narrow within the ceiling, never widen)
    ///
    /// `consent_dir` is typically `~/.ai-foundation/federation/consent/`.
    pub fn should_cross_boundary(
        &self,
        event_type: OutboundEventType,
        ai_id: &str,
        consent_dir: &Path,
    ) -> bool {
        let consent = AiConsentRecord::load_or_default(ai_id, consent_dir);
        match event_type {
            OutboundEventType::Presence => consent.effective_presence(&self.manifest),
            OutboundEventType::Broadcast { cross_team } => {
                let effective = consent.effective_broadcasts(&self.manifest);
                match effective {
                    BroadcastVisibility::None => false,
                    BroadcastVisibility::CrossTeamOnly => cross_team,
                    BroadcastVisibility::All => true,
                }
            }
            OutboundEventType::TaskComplete => consent.effective_task_complete(&self.manifest),
            OutboundEventType::DialogueEnd => {
                consent.effective_dialogues(&self.manifest) != DialogueVisibility::None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Outbound Event Types
// ---------------------------------------------------------------------------

/// Event types that may cross the federation boundary (input to the outbound filter).
///
/// Maps to the categories confirmed in the QD taxonomy (Feb 22, 2026):
/// - Presence ✅
/// - Broadcasts ✅ (with channel-level granularity)
/// - Task completions ✅
/// - Concluded dialogues ✅
///
/// These never appear here because they never cross the boundary:
/// - File names ❌
/// - Tool usage / raw ops ❌
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundEventType {
    /// Presence / online status update (EDU — fire-and-forget).
    Presence,

    /// Broadcast on a channel.
    Broadcast {
        /// True if this is a cross-team channel (affects `CrossTeamOnly` ceiling).
        cross_team: bool,
    },

    /// Task completion summary (semantic summary only, not raw ops).
    TaskComplete,

    /// Concluded dialogue summary (concluded only, not active transcripts).
    DialogueEnd,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AiConsentRecord, BroadcastVisibility, DialogueVisibility, PermissionManifest};
    use crate::manifest::{ConnectionMode, ExposureConfig};
    use std::path::Path;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn make_identity() -> TeambookIdentity {
        TeambookIdentity::generate()
    }

    fn open_manifest() -> PermissionManifest {
        PermissionManifest {
            connection_mode: ConnectionMode::ConnectCode,
            expose: ExposureConfig {
                presence: true,
                broadcasts: BroadcastVisibility::All,
                dialogues: DialogueVisibility::All,
                task_complete: true,
                file_claims: false,
                raw_events: false,
            },
            ..PermissionManifest::default()
        }
    }

    fn make_gateway(
        identity: TeambookIdentity,
        peers: PeerRegistryConfig,
        manifest: PermissionManifest,
    ) -> FederationGateway {
        FederationGateway::new(identity, "Test-TB", peers, manifest)
    }

    // -----------------------------------------------------------------------
    // PeerRegistryConfig
    // -----------------------------------------------------------------------

    #[test]
    fn test_peer_registry_toml_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("peers.toml");

        let mut cfg = PeerRegistryConfig::default();
        cfg.peers.push(PeerEntry {
            pubkey_hex: "a".repeat(64),
            endpoint: "http://192.168.1.100:8765".to_string(),
            name: "Brother-PC".to_string(),
            trusted: true,
        });

        cfg.save(&path).unwrap();
        let loaded = PeerRegistryConfig::load_or_default(&path);

        assert_eq!(loaded.peers.len(), 1);
        assert_eq!(loaded.peers[0].name, "Brother-PC");
        assert_eq!(loaded.peers[0].short_id(), "a".repeat(8).as_str());
        assert!(loaded.peers[0].trusted);
    }

    #[test]
    fn test_peer_registry_load_nonexistent_returns_empty() {
        let cfg = PeerRegistryConfig::load_or_default(Path::new("/nonexistent/peers.toml"));
        assert!(cfg.peers.is_empty());
    }

    #[test]
    fn test_is_known_peer() {
        let mut cfg = PeerRegistryConfig::default();
        cfg.peers.push(PeerEntry {
            pubkey_hex: "b".repeat(64),
            endpoint: "http://192.168.1.2:8765".to_string(),
            name: "PC-B".to_string(),
            trusted: false,
        });

        assert!(cfg.is_known_peer(&"b".repeat(64)));
        assert!(!cfg.is_known_peer(&"c".repeat(64)));
    }

    #[test]
    fn test_peer_short_id() {
        let peer = PeerEntry {
            pubkey_hex: "a3f7c2d1beef1234".to_string(),
            endpoint: "http://x".to_string(),
            name: "X".to_string(),
            trusted: false,
        };
        assert_eq!(peer.short_id(), "a3f7c2d1");
    }

    // -----------------------------------------------------------------------
    // FederationGateway: outbound
    // -----------------------------------------------------------------------

    #[test]
    fn test_sign_event_and_verify() {
        let gateway = make_gateway(
            make_identity(),
            PeerRegistryConfig::default(),
            open_manifest(),
        );

        let bytes = b"federation event payload".to_vec();
        let signed = gateway.sign_event(bytes);

        assert!(!signed.content_id.is_empty());
        assert_eq!(signed.origin_pubkey, gateway.identity.public_key_hex());
        assert!(signed.verify().is_ok(), "sign_event must produce a verifiable event");
    }

    #[test]
    fn test_build_event_push() {
        let gateway = make_gateway(
            make_identity(),
            PeerRegistryConfig::default(),
            open_manifest(),
        );

        let signed = gateway.sign_event(b"test".to_vec());
        let request = gateway.build_event_push(vec![signed], 42);

        assert_eq!(request.events.len(), 1);
        assert!(request.sender_hlc.physical_time_us > 0);
        assert_eq!(request.sender_head_seq, 42);
    }

    // -----------------------------------------------------------------------
    // Outbound filter (should_cross_boundary)
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_cross_boundary_closed_manifest_blocks_all() {
        let tmp = TempDir::new().unwrap();
        let consent_dir = tmp.path().join("consent");

        let gateway = make_gateway(
            make_identity(),
            PeerRegistryConfig::default(),
            PermissionManifest::default(),
        );

        assert!(!gateway.should_cross_boundary(OutboundEventType::Presence, "sage-724", &consent_dir));
        assert!(!gateway.should_cross_boundary(OutboundEventType::TaskComplete, "sage-724", &consent_dir));
        assert!(!gateway.should_cross_boundary(OutboundEventType::DialogueEnd, "sage-724", &consent_dir));
        assert!(!gateway.should_cross_boundary(
            OutboundEventType::Broadcast { cross_team: true },
            "sage-724",
            &consent_dir,
        ));
    }

    #[test]
    fn test_should_cross_boundary_open_manifest_allows_all() {
        let tmp = TempDir::new().unwrap();
        let consent_dir = tmp.path().join("consent");

        let gateway = make_gateway(
            make_identity(),
            PeerRegistryConfig::default(),
            open_manifest(),
        );

        assert!(gateway.should_cross_boundary(OutboundEventType::Presence, "sage-724", &consent_dir));
        assert!(gateway.should_cross_boundary(OutboundEventType::TaskComplete, "sage-724", &consent_dir));
        assert!(gateway.should_cross_boundary(OutboundEventType::DialogueEnd, "sage-724", &consent_dir));
        assert!(gateway.should_cross_boundary(
            OutboundEventType::Broadcast { cross_team: true },
            "sage-724",
            &consent_dir,
        ));
        assert!(gateway.should_cross_boundary(
            OutboundEventType::Broadcast { cross_team: false },
            "sage-724",
            &consent_dir,
        ));
    }

    #[test]
    fn test_should_cross_boundary_cross_team_only_broadcast() {
        let tmp = TempDir::new().unwrap();
        let consent_dir = tmp.path().join("consent");

        let manifest = PermissionManifest {
            connection_mode: ConnectionMode::ConnectCode,
            expose: ExposureConfig {
                presence: true,
                broadcasts: BroadcastVisibility::CrossTeamOnly,
                dialogues: DialogueVisibility::None,
                task_complete: false,
                file_claims: false,
                raw_events: false,
            },
            ..PermissionManifest::default()
        };

        let gateway = make_gateway(
            make_identity(),
            PeerRegistryConfig::default(),
            manifest,
        );

        assert!(gateway.should_cross_boundary(
            OutboundEventType::Broadcast { cross_team: true },
            "sage-724",
            &consent_dir,
        ));
        assert!(!gateway.should_cross_boundary(
            OutboundEventType::Broadcast { cross_team: false },
            "sage-724",
            &consent_dir,
        ));
        assert!(!gateway.should_cross_boundary(OutboundEventType::TaskComplete, "sage-724", &consent_dir));
        assert!(!gateway.should_cross_boundary(OutboundEventType::DialogueEnd, "sage-724", &consent_dir));
    }

    #[test]
    fn test_should_cross_boundary_ai_consent_can_narrow() {
        let tmp = TempDir::new().unwrap();
        let consent_dir = tmp.path().join("consent");
        std::fs::create_dir_all(&consent_dir).unwrap();

        let gateway = make_gateway(
            make_identity(),
            PeerRegistryConfig::default(),
            open_manifest(),
        );

        // AI opts out of presence via consent record
        let mut consent = AiConsentRecord::new("cascade-230");
        consent.presence = Some(false);
        consent.save(&consent_dir).unwrap();

        assert!(!gateway.should_cross_boundary(OutboundEventType::Presence, "cascade-230", &consent_dir));
        assert!(gateway.should_cross_boundary(OutboundEventType::TaskComplete, "cascade-230", &consent_dir));
    }
}
