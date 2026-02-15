//! Federation Peer Management — Registration, Storage, Auth Policy
//!
//! Each Teambook maintains a registry of known peers (other Teambooks).
//! Peers are identified by their Ed25519 public keys and authenticated
//! via signed challenges during registration.
//!
//! Auth policy is per-Teambook: each instance decides what identity tier
//! it requires from federation participants (device-bound, hardware-attested,
//! or OAuth-verified).
//!
//! No central authority. No passwords. No expiring tokens.

use crate::crypto::{self, PeerPublicKey, TeambookIdentity};
use crate::hlc::HybridClock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Peer Registry
// ---------------------------------------------------------------------------

/// Information about a registered federation peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    /// Peer's Ed25519 public key (32 bytes) — this IS the peer's identity
    #[serde(with = "hex_pubkey")]
    pub public_key: PeerPublicKey,

    /// Human-readable name chosen by the peer (e.g., "alice-homelab")
    pub display_name: String,

    /// HTTP endpoint for pushing events (e.g., "http://192.168.1.50:8080")
    pub endpoint: String,

    /// When this peer was registered (microseconds since epoch)
    pub registered_at: u64,

    /// Last time we successfully communicated with this peer
    pub last_seen_at: u64,

    /// Last event sequence we know this peer has received from us
    pub last_synced_seq: u64,

    /// Whether we initiated the connection or they did
    pub initiated_by_us: bool,

    /// Current connection status
    pub status: PeerStatus,
}

impl PeerInfo {
    /// Public key as hex string.
    pub fn pubkey_hex(&self) -> String {
        hex::encode(self.public_key)
    }

    /// Short ID (first 8 hex chars of pubkey).
    pub fn short_id(&self) -> String {
        hex::encode(&self.public_key[..4])
    }
}

/// Connection status of a federation peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerStatus {
    /// Peer is registered and responding to health checks
    Online,
    /// Peer hasn't responded recently but is still registered
    Offline,
    /// Peer registration is pending mutual confirmation
    PendingMutual,
    /// Peer was explicitly removed (kept for audit trail)
    Removed,
}

impl std::fmt::Display for PeerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Online => write!(f, "online"),
            Self::Offline => write!(f, "offline"),
            Self::PendingMutual => write!(f, "pending"),
            Self::Removed => write!(f, "removed"),
        }
    }
}

// ---------------------------------------------------------------------------
// Auth Policy
// ---------------------------------------------------------------------------

/// Minimum identity tier required to join this Teambook's federation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthTier {
    /// Ed25519 keypair generated on device (default)
    DeviceBound,
    /// OAuth 2.0 / OpenID Connect backed identity
    OAuthVerified,
    /// TPM 2.0 hardware attestation
    HardwareAttested,
}

impl std::fmt::Display for AuthTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeviceBound => write!(f, "device-bound"),
            Self::OAuthVerified => write!(f, "oauth-verified"),
            Self::HardwareAttested => write!(f, "hardware-attested"),
        }
    }
}

/// Federation policy for this Teambook instance.
///
/// Controls who can join, how many peers are allowed, and
/// whether mutual registration is required.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationPolicy {
    /// Minimum auth tier required for new peers
    pub min_auth_tier: AuthTier,

    /// Require both sides to approve the connection
    pub require_mutual: bool,

    /// Automatically accept peers that we already know (by pubkey)
    pub auto_accept_known: bool,

    /// Maximum number of active peers (0 = unlimited)
    pub max_peers: usize,
}

impl Default for FederationPolicy {
    fn default() -> Self {
        Self {
            min_auth_tier: AuthTier::DeviceBound,
            require_mutual: true,
            auto_accept_known: false,
            max_peers: 10,
        }
    }
}

// ---------------------------------------------------------------------------
// Registration Protocol Messages
// ---------------------------------------------------------------------------

/// Sent when a Teambook wants to register as a peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRegistrationRequest {
    /// Requester's Ed25519 public key
    #[serde(with = "hex_pubkey")]
    pub public_key: PeerPublicKey,

    /// Human-readable display name
    pub display_name: String,

    /// Requester's HTTP endpoint for receiving events
    pub endpoint: String,

    /// Challenge: random bytes signed with the requester's private key
    /// to prove they hold the corresponding secret key
    pub challenge_nonce: String,

    /// Ed25519 signature of the challenge_nonce (proves key ownership)
    pub challenge_signature: String,
}

/// Response to a registration request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRegistrationResponse {
    /// Whether the registration was accepted
    pub accepted: bool,

    /// Reason for rejection (if not accepted)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Responder's public key (for mutual registration)
    #[serde(with = "hex_pubkey")]
    pub public_key: PeerPublicKey,

    /// Responder's display name
    pub display_name: String,

    /// Responder's endpoint
    pub endpoint: String,

    /// Responder's signed challenge (proves their key ownership too)
    pub challenge_nonce: String,
    pub challenge_signature: String,
}

// ---------------------------------------------------------------------------
// Federation State
// ---------------------------------------------------------------------------

/// Core federation state for a Teambook instance.
///
/// Manages peer registry, identity, HLC, and auth policy.
/// Thread-safe via Arc<RwLock> for concurrent access from HTTP handlers.
pub struct FederationState {
    /// This Teambook's cryptographic identity
    pub identity: Arc<TeambookIdentity>,

    /// Hybrid Logical Clock for causal event ordering
    pub clock: Arc<HybridClock>,

    /// Registered peers (keyed by hex-encoded public key)
    peers: Arc<RwLock<HashMap<String, PeerInfo>>>,

    /// Content hashes of events we've already seen (for deduplication)
    seen_events: Arc<RwLock<HashMap<String, u64>>>, // hash_hex -> timestamp_us

    /// Auth policy for this Teambook
    policy: Arc<RwLock<FederationPolicy>>,

    /// Display name for this Teambook in the federation
    display_name: String,

    /// HTTP endpoint where this Teambook receives federation events
    local_endpoint: String,
}

impl FederationState {
    /// Initialize federation state. Loads or generates identity keypair.
    pub async fn init(display_name: String, local_endpoint: String) -> anyhow::Result<Self> {
        let identity = TeambookIdentity::load_or_generate().await?;
        let node_id = HybridClock::node_id_from_pubkey(&identity.public_key());
        let clock = HybridClock::new(node_id);

        let peers = Self::load_peers().await.unwrap_or_default();

        info!(
            pubkey = %identity.short_id(),
            display_name = %display_name,
            endpoint = %local_endpoint,
            peers = peers.len(),
            "Federation state initialized"
        );

        Ok(Self {
            identity: Arc::new(identity),
            clock: Arc::new(clock),
            peers: Arc::new(RwLock::new(peers)),
            seen_events: Arc::new(RwLock::new(HashMap::new())),
            policy: Arc::new(RwLock::new(FederationPolicy::default())),
            display_name,
            local_endpoint,
        })
    }

    // -----------------------------------------------------------------------
    // Peer Management
    // -----------------------------------------------------------------------

    /// Process an incoming peer registration request.
    ///
    /// Validates the challenge signature, checks auth policy, and either
    /// accepts or rejects the registration.
    pub async fn handle_registration(
        &self,
        req: &PeerRegistrationRequest,
    ) -> PeerRegistrationResponse {
        // 1. Verify the challenge signature (proves they hold the private key)
        let nonce_bytes = req.challenge_nonce.as_bytes();
        let sig_bytes = match hex::decode(&req.challenge_signature) {
            Ok(b) if b.len() == 64 => {
                let mut arr = [0u8; 64];
                arr.copy_from_slice(&b);
                arr
            }
            _ => {
                return self.reject_registration("invalid challenge signature format");
            }
        };

        if !crypto::verify_signature(&req.public_key, nonce_bytes, &sig_bytes) {
            return self.reject_registration("challenge signature verification failed");
        }

        // 2. Check if we've hit the peer limit
        let policy = self.policy.read().await;
        let peers = self.peers.read().await;
        let active_count = peers
            .values()
            .filter(|p| p.status != PeerStatus::Removed)
            .count();

        if policy.max_peers > 0 && active_count >= policy.max_peers {
            return self.reject_registration(&format!(
                "peer limit reached ({}/{})",
                active_count, policy.max_peers
            ));
        }
        drop(peers);
        drop(policy);

        // 3. Register the peer
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;

        let peer = PeerInfo {
            public_key: req.public_key,
            display_name: req.display_name.clone(),
            endpoint: req.endpoint.clone(),
            registered_at: now_us,
            last_seen_at: now_us,
            last_synced_seq: 0,
            initiated_by_us: false,
            status: PeerStatus::Online,
        };

        let peer_hex = hex::encode(req.public_key);
        info!(
            peer = %peer.short_id(),
            name = %peer.display_name,
            endpoint = %peer.endpoint,
            "Registered federation peer"
        );

        self.peers.write().await.insert(peer_hex, peer);
        let _ = self.save_peers().await;

        // 4. Build our response with our own signed challenge
        let our_nonce = self.generate_challenge_nonce();
        let our_sig = self.identity.sign(our_nonce.as_bytes());

        PeerRegistrationResponse {
            accepted: true,
            reason: None,
            public_key: self.identity.public_key(),
            display_name: self.display_name.clone(),
            endpoint: self.local_endpoint.clone(),
            challenge_nonce: our_nonce,
            challenge_signature: hex::encode(our_sig),
        }
    }

    /// Build a registration request to send to another Teambook.
    pub fn build_registration_request(&self) -> PeerRegistrationRequest {
        let nonce = self.generate_challenge_nonce();
        let sig = self.identity.sign(nonce.as_bytes());

        PeerRegistrationRequest {
            public_key: self.identity.public_key(),
            display_name: self.display_name.clone(),
            endpoint: self.local_endpoint.clone(),
            challenge_nonce: nonce,
            challenge_signature: hex::encode(sig),
        }
    }

    /// Process the response from a registration request we sent.
    pub async fn handle_registration_response(
        &self,
        resp: &PeerRegistrationResponse,
    ) -> Result<(), String> {
        if !resp.accepted {
            return Err(resp
                .reason
                .clone()
                .unwrap_or_else(|| "registration rejected".to_string()));
        }

        // Verify their challenge signature
        let nonce_bytes = resp.challenge_nonce.as_bytes();
        let sig_bytes = hex::decode(&resp.challenge_signature)
            .map_err(|_| "invalid signature format")?;

        if sig_bytes.len() != 64 {
            return Err("invalid signature length".to_string());
        }

        let mut sig_arr = [0u8; 64];
        sig_arr.copy_from_slice(&sig_bytes);

        if !crypto::verify_signature(&resp.public_key, nonce_bytes, &sig_arr) {
            return Err("challenge signature verification failed".to_string());
        }

        // Register the peer
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;

        let peer = PeerInfo {
            public_key: resp.public_key,
            display_name: resp.display_name.clone(),
            endpoint: resp.endpoint.clone(),
            registered_at: now_us,
            last_seen_at: now_us,
            last_synced_seq: 0,
            initiated_by_us: true,
            status: PeerStatus::Online,
        };

        let peer_hex = hex::encode(resp.public_key);
        info!(
            peer = %peer.short_id(),
            name = %peer.display_name,
            "Mutual registration complete"
        );

        self.peers.write().await.insert(peer_hex, peer);
        let _ = self.save_peers().await;

        Ok(())
    }

    /// List all registered peers.
    pub async fn list_peers(&self) -> Vec<PeerInfo> {
        self.peers
            .read()
            .await
            .values()
            .filter(|p| p.status != PeerStatus::Removed)
            .cloned()
            .collect()
    }

    /// Get a specific peer by hex-encoded public key.
    pub async fn get_peer(&self, pubkey_hex: &str) -> Option<PeerInfo> {
        self.peers.read().await.get(pubkey_hex).cloned()
    }

    /// Check if a public key belongs to a known peer.
    pub async fn is_known_peer(&self, pubkey: &PeerPublicKey) -> bool {
        let hex = hex::encode(pubkey);
        let peers = self.peers.read().await;
        peers
            .get(&hex)
            .map(|p| p.status != PeerStatus::Removed)
            .unwrap_or(false)
    }

    /// Remove a peer by hex-encoded public key.
    pub async fn remove_peer(&self, pubkey_hex: &str) -> bool {
        let mut peers = self.peers.write().await;
        if let Some(peer) = peers.get_mut(pubkey_hex) {
            peer.status = PeerStatus::Removed;
            info!(peer = %peer.short_id(), "Removed federation peer");
            drop(peers);
            let _ = self.save_peers().await;
            true
        } else {
            false
        }
    }

    /// Update a peer's last_seen_at timestamp.
    pub async fn touch_peer(&self, pubkey_hex: &str) {
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;

        let mut peers = self.peers.write().await;
        if let Some(peer) = peers.get_mut(pubkey_hex) {
            peer.last_seen_at = now_us;
            peer.status = PeerStatus::Online;
        }
    }

    /// Update a peer's last synced sequence number.
    pub async fn update_peer_sync_seq(&self, pubkey_hex: &str, seq: u64) {
        let mut peers = self.peers.write().await;
        if let Some(peer) = peers.get_mut(pubkey_hex) {
            peer.last_synced_seq = seq;
        }
    }

    // -----------------------------------------------------------------------
    // Event Deduplication
    // -----------------------------------------------------------------------

    /// Check if we've already seen an event by its content hash.
    /// Returns `true` if the event is NEW (not seen before).
    pub async fn is_new_event(&self, content_hash_hex: &str) -> bool {
        !self.seen_events.read().await.contains_key(content_hash_hex)
    }

    /// Mark an event as seen.
    pub async fn mark_event_seen(&self, content_hash_hex: String) {
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;

        self.seen_events
            .write()
            .await
            .insert(content_hash_hex, now_us);
    }

    /// Prune seen events older than `max_age_us` to prevent unbounded memory growth.
    pub async fn prune_seen_events(&self, max_age_us: u64) {
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;

        let cutoff = now_us.saturating_sub(max_age_us);
        let mut seen = self.seen_events.write().await;
        let before = seen.len();
        seen.retain(|_, ts| *ts > cutoff);
        let pruned = before - seen.len();
        if pruned > 0 {
            info!(pruned, remaining = seen.len(), "Pruned seen event cache");
        }
    }

    // -----------------------------------------------------------------------
    // Policy
    // -----------------------------------------------------------------------

    /// Get current federation policy.
    pub async fn policy(&self) -> FederationPolicy {
        self.policy.read().await.clone()
    }

    /// Update federation policy.
    pub async fn set_policy(&self, policy: FederationPolicy) {
        info!(
            min_auth = %policy.min_auth_tier,
            mutual = policy.require_mutual,
            max_peers = policy.max_peers,
            "Federation policy updated"
        );
        *self.policy.write().await = policy;
    }

    // -----------------------------------------------------------------------
    // Federation Status
    // -----------------------------------------------------------------------

    /// Get a summary of federation health.
    pub async fn status(&self) -> FederationStatus {
        let peers = self.peers.read().await;
        let online = peers
            .values()
            .filter(|p| p.status == PeerStatus::Online)
            .count();
        let total = peers
            .values()
            .filter(|p| p.status != PeerStatus::Removed)
            .count();
        let seen_count = self.seen_events.read().await.len();

        FederationStatus {
            pubkey: self.identity.public_key_hex(),
            short_id: self.identity.short_id(),
            display_name: self.display_name.clone(),
            endpoint: self.local_endpoint.clone(),
            peers_online: online,
            peers_total: total,
            events_seen: seen_count,
            policy: self.policy.read().await.clone(),
        }
    }

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    /// Path to the peers registry file.
    fn peers_path() -> anyhow::Result<PathBuf> {
        let home =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
        Ok(home
            .join(".ai-foundation")
            .join("federation")
            .join("peers.json"))
    }

    /// Load peers from disk.
    async fn load_peers() -> anyhow::Result<HashMap<String, PeerInfo>> {
        let path = Self::peers_path()?;
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let data = fs::read_to_string(&path).await?;
        let peers: HashMap<String, PeerInfo> = serde_json::from_str(&data)?;
        info!(count = peers.len(), "Loaded federation peers");
        Ok(peers)
    }

    /// Save peers to disk.
    async fn save_peers(&self) -> anyhow::Result<()> {
        let path = Self::peers_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let peers = self.peers.read().await;
        let data = serde_json::to_string_pretty(&*peers)?;
        fs::write(&path, data).await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Generate a random challenge nonce for registration.
    fn generate_challenge_nonce(&self) -> String {
        use rand::Rng;
        let mut rng = rand::rngs::OsRng;
        let nonce: [u8; 32] = rng.gen();
        hex::encode(nonce)
    }

    /// Build a rejection response.
    fn reject_registration(&self, reason: &str) -> PeerRegistrationResponse {
        warn!(reason, "Rejected peer registration");
        PeerRegistrationResponse {
            accepted: false,
            reason: Some(reason.to_string()),
            public_key: self.identity.public_key(),
            display_name: self.display_name.clone(),
            endpoint: self.local_endpoint.clone(),
            challenge_nonce: String::new(),
            challenge_signature: String::new(),
        }
    }
}

/// Summary of federation health for status endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationStatus {
    pub pubkey: String,
    pub short_id: String,
    pub display_name: String,
    pub endpoint: String,
    pub peers_online: usize,
    pub peers_total: usize,
    pub events_seen: usize,
    pub policy: FederationPolicy,
}

// ---------------------------------------------------------------------------
// Serde helper for PeerPublicKey
// ---------------------------------------------------------------------------

mod hex_pubkey {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        let vec = hex::decode(&s).map_err(serde::de::Error::custom)?;
        let arr: [u8; 32] = vec
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 32 bytes"))?;
        Ok(arr)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_registration_roundtrip() {
        let state_a = FederationState::init(
            "teambook-a".to_string(),
            "http://localhost:8080".to_string(),
        )
        .await
        .unwrap();

        let state_b = FederationState::init(
            "teambook-b".to_string(),
            "http://localhost:8081".to_string(),
        )
        .await
        .unwrap();

        // A builds a registration request
        let req = state_a.build_registration_request();

        // B handles the request
        let resp = state_b.handle_registration(&req).await;
        assert!(resp.accepted);

        // A processes B's response
        let result = state_a.handle_registration_response(&resp).await;
        assert!(result.is_ok());

        // Both should have each other as peers
        assert!(state_a.is_known_peer(&state_b.identity.public_key()).await);
        assert!(state_b.is_known_peer(&state_a.identity.public_key()).await);
    }

    #[tokio::test]
    async fn test_event_dedup() {
        let state = FederationState::init(
            "test".to_string(),
            "http://localhost:8080".to_string(),
        )
        .await
        .unwrap();

        let hash = "abc123def456".to_string();

        assert!(state.is_new_event(&hash).await);
        state.mark_event_seen(hash.clone()).await;
        assert!(!state.is_new_event(&hash).await);
    }

    #[tokio::test]
    async fn test_federation_status() {
        let state = FederationState::init(
            "my-teambook".to_string(),
            "http://localhost:8080".to_string(),
        )
        .await
        .unwrap();

        let status = state.status().await;
        assert_eq!(status.display_name, "my-teambook");
        assert_eq!(status.endpoint, "http://localhost:8080");
        // pubkey should be 64 hex chars (32 bytes)
        assert_eq!(status.pubkey.len(), 64);
        assert!(status.pubkey.chars().all(|c| c.is_ascii_hexdigit()));
        // short_id should be first 8 chars of pubkey
        assert_eq!(status.short_id, &status.pubkey[..8]);
    }
}
