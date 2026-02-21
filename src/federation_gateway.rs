//! Federation Gateway — Cross-Teambook Event Routing (Layer 2)
//!
//! Layer 1 (crypto, HLC, peer management) answers: "How do two Teambooks exchange signed data?"
//! Layer 2 (this module) answers: "How does ai-1 on PC-A send a DM to ai-4 on PC-B?"
//!
//! The gateway runs as a background task inside ai-foundation-http and:
//! 1. Maintains an AI Registry (which AI is on which Teambook)
//! 2. Syncs presence across federated Teambooks
//! 3. Routes DMs/broadcasts/dialogues to remote Teambooks
//! 4. Injects incoming remote events into the local event log via CLI
//!
//! Design: CLI subprocess pattern preserved. Gateway calls teambook CLI for everything.
//! No direct binary event log access. JSON over HTTP for federation messages.

use crate::cli_wrapper;
use crate::crypto::SignedEvent;
use crate::federation::FederationState;
use crate::hlc::HlcTimestamp;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

// ---------------------------------------------------------------------------
// AI Registry — Maps AI IDs to Teambook Locations
// ---------------------------------------------------------------------------

/// A known AI in the federation (local or remote).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedAiEntry {
    /// AI identifier (e.g., "ai-1")
    pub ai_id: String,

    /// Hex-encoded Ed25519 pubkey of the Teambook this AI is on
    pub teambook_pubkey_hex: String,

    /// Short ID of the Teambook (first 8 hex chars)
    pub teambook_short_id: String,

    /// Display name of the Teambook (e.g., "alice-homelab")
    pub teambook_name: String,

    /// Whether this AI is on the local Teambook
    pub is_local: bool,

    /// Current status: "active", "standby", "idle", "offline"
    pub status: String,

    /// What the AI is currently working on
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_task: Option<String>,

    /// Last presence update (microseconds since epoch)
    pub last_seen_us: u64,
}

impl FederatedAiEntry {
    /// Federated address: ai_id@teambook_short_id
    pub fn federated_address(&self) -> String {
        if self.is_local {
            self.ai_id.clone()
        } else {
            format!("{}@{}", self.ai_id, self.teambook_short_id)
        }
    }
}

/// Where an AI is located.
#[derive(Debug, Clone)]
pub enum AiResolution {
    /// AI is on this Teambook
    Local,
    /// AI is on a remote Teambook
    Remote {
        teambook_pubkey_hex: String,
        teambook_short_id: String,
        teambook_name: String,
    },
    /// AI not found in registry
    Unknown,
}

/// Registry of all known AIs across the federation.
pub struct AiRegistry {
    /// All known AIs, keyed by ai_id
    entries: Arc<RwLock<HashMap<String, FederatedAiEntry>>>,

    /// This Teambook's pubkey hex (to identify local entries)
    local_teambook_pubkey: String,

    /// This Teambook's short ID
    local_teambook_short_id: String,

    /// This Teambook's display name
    local_teambook_name: String,
}

impl AiRegistry {
    /// Create a new registry for a Teambook.
    pub fn new(
        local_pubkey_hex: String,
        local_short_id: String,
        local_name: String,
    ) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            local_teambook_pubkey: local_pubkey_hex,
            local_teambook_short_id: local_short_id,
            local_teambook_name: local_name,
        }
    }

    /// Register or update a local AI.
    pub async fn register_local(&self, ai_id: &str, status: &str, current_task: Option<String>) {
        let now = now_us();
        let mut entries = self.entries.write().await;
        let entry = entries.entry(ai_id.to_string()).or_insert_with(|| {
            FederatedAiEntry {
                ai_id: ai_id.to_string(),
                teambook_pubkey_hex: self.local_teambook_pubkey.clone(),
                teambook_short_id: self.local_teambook_short_id.clone(),
                teambook_name: self.local_teambook_name.clone(),
                is_local: true,
                status: "offline".to_string(),
                current_task: None,
                last_seen_us: 0,
            }
        });
        entry.status = status.to_string();
        entry.current_task = current_task;
        entry.last_seen_us = now;
        entry.is_local = true;
    }

    /// Register or update a remote AI (from presence sync).
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
        let entry = entries
            .entry(ai_id.to_string())
            .or_insert_with(|| FederatedAiEntry {
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
        entry.teambook_pubkey_hex = teambook_pubkey_hex.to_string();
        entry.teambook_short_id = teambook_short_id.to_string();
        entry.teambook_name = teambook_name.to_string();
        entry.is_local = false;
    }

    /// Resolve an AI ID to its location.
    ///
    /// Handles both plain IDs ("ai-1") and federated addresses ("ai-1@a3f7c2d1").
    pub async fn resolve(&self, ai_id: &str) -> AiResolution {
        // Check for explicit @teambook_short_id
        if let Some((name, tb_short)) = ai_id.split_once('@') {
            let entries = self.entries.read().await;
            // Look for the AI on the specified Teambook
            if let Some(entry) = entries.get(name) {
                if entry.teambook_short_id == tb_short {
                    if entry.is_local {
                        return AiResolution::Local;
                    } else {
                        return AiResolution::Remote {
                            teambook_pubkey_hex: entry.teambook_pubkey_hex.clone(),
                            teambook_short_id: entry.teambook_short_id.clone(),
                            teambook_name: entry.teambook_name.clone(),
                        };
                    }
                }
            }
            // Even without registry entry, if tb_short != ours, it's remote
            if tb_short != self.local_teambook_short_id {
                // We don't know the full pubkey, but we know the short ID
                return AiResolution::Unknown;
            }
            // tb_short is ours, treat as local lookup
            return if entries.contains_key(name) {
                AiResolution::Local
            } else {
                AiResolution::Unknown
            };
        }

        // Plain AI ID: check registry
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

    /// List all known AIs (local + remote).
    pub async fn list_all(&self) -> Vec<FederatedAiEntry> {
        self.entries.read().await.values().cloned().collect()
    }

    /// List only remote AIs.
    pub async fn list_remote(&self) -> Vec<FederatedAiEntry> {
        self.entries
            .read()
            .await
            .values()
            .filter(|e| !e.is_local)
            .cloned()
            .collect()
    }

    /// List only local AIs.
    pub async fn list_local(&self) -> Vec<FederatedAiEntry> {
        self.entries
            .read()
            .await
            .values()
            .filter(|e| e.is_local)
            .cloned()
            .collect()
    }

    /// Get the local Teambook's short ID.
    pub fn local_short_id(&self) -> &str {
        &self.local_teambook_short_id
    }

    /// Get the local Teambook's pubkey hex.
    pub fn local_pubkey_hex(&self) -> &str {
        &self.local_teambook_pubkey
    }

    /// Get the local Teambook's display name.
    pub fn local_name(&self) -> &str {
        &self.local_teambook_name
    }

    /// Persist the registry to disk.
    pub async fn save(&self) -> anyhow::Result<()> {
        let path = Self::registry_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let entries = self.entries.read().await;
        let data = serde_json::to_string_pretty(&*entries)?;
        fs::write(&path, data).await?;
        Ok(())
    }

    /// Load registry from disk.
    pub async fn load_into(&self) -> anyhow::Result<()> {
        let path = Self::registry_path()?;
        if !path.exists() {
            return Ok(());
        }
        let data = fs::read_to_string(&path).await?;
        let loaded: HashMap<String, FederatedAiEntry> = serde_json::from_str(&data)?;
        let mut entries = self.entries.write().await;
        for (k, v) in loaded {
            entries.entry(k).or_insert(v);
        }
        info!(count = entries.len(), "Loaded AI registry");
        Ok(())
    }

    fn registry_path() -> anyhow::Result<PathBuf> {
        let home =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
        Ok(home
            .join(".ai-foundation")
            .join("federation")
            .join("ai_registry.json"))
    }
}

// ---------------------------------------------------------------------------
// Federation Message Types
// ---------------------------------------------------------------------------

/// A semantic message for cross-Teambook communication.
///
/// These are JSON-serialized and wrapped in SignedEvent for transport.
/// The semantic format is intentional — we don't forward raw binary events,
/// we re-create events locally via CLI on the receiving end.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationMessage {
    /// Message type
    pub msg_type: FederationMessageType,

    /// Source AI who created this event
    pub source_ai: String,

    /// Source Teambook short ID
    pub source_teambook: String,

    /// Source Teambook display name
    pub source_teambook_name: String,

    /// Target AI (for DMs/dialogues) — None for broadcasts/presence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_ai: Option<String>,

    /// HLC timestamp from source
    pub hlc: HlcTimestamp,

    /// Message payload (type-specific)
    pub payload: serde_json::Value,
}

/// Types of federation messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FederationMessageType {
    /// DM to a specific remote AI
    DirectMessage,
    /// Broadcast on a federated channel
    Broadcast,
    /// Start a cross-Teambook dialogue
    DialogueStart,
    /// Respond in a cross-Teambook dialogue
    DialogueRespond,
    /// End a cross-Teambook dialogue
    DialogueEnd,
    /// Presence sync (batch of AI statuses)
    PresenceSync,
    /// Shared learning/insight
    LearningShare,
}

/// Batch of federation messages pushed between Teambooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationRelayRequest {
    /// Signed messages to deliver
    pub messages: Vec<SignedEvent>,

    /// Sender's HLC
    pub sender_hlc: HlcTimestamp,
}

/// Response to a relay request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationRelayResponse {
    /// Number of messages processed
    pub processed: usize,

    /// Number of messages rejected
    pub rejected: usize,

    /// Number of duplicates skipped
    pub duplicates: usize,

    /// Specific errors
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

/// Presence sync request from a peer Teambook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceSyncRequest {
    /// Peer's Teambook pubkey hex
    pub teambook_pubkey: String,

    /// Peer's Teambook display name
    pub teambook_name: String,

    /// AIs on the peer Teambook
    pub ais: Vec<PresenceAiEntry>,

    /// HLC timestamp
    pub hlc: HlcTimestamp,

    /// Ed25519 signature over the canonical JSON of (teambook_pubkey + ais + hlc)
    pub signature: String,
}

/// A single AI's presence in a sync payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceAiEntry {
    pub ai_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_task: Option<String>,
}

// ---------------------------------------------------------------------------
// Federation Gateway
// ---------------------------------------------------------------------------

/// The Federation Gateway routes events between Teambooks.
///
/// Runs as a background task inside the HTTP server. Uses event-driven
/// wake (teambook standby) to detect local events that need forwarding,
/// and receives remote events via HTTP endpoints.
pub struct FederationGateway {
    /// Federation state (peers, identity, clock)
    federation: Arc<FederationState>,

    /// AI registry (who is where)
    registry: Arc<AiRegistry>,

    /// Whether the gateway is running
    running: Arc<RwLock<bool>>,

    /// Last sequence we've processed for outbound routing (reserved for future use)
    #[allow(dead_code)]
    last_processed_seq: Arc<RwLock<u64>>,
}

impl FederationGateway {
    /// Create a new federation gateway.
    pub fn new(federation: Arc<FederationState>, registry: Arc<AiRegistry>) -> Self {
        Self {
            federation,
            registry,
            running: Arc::new(RwLock::new(false)),
            last_processed_seq: Arc::new(RwLock::new(0)),
        }
    }

    /// Start the gateway background tasks.
    ///
    /// Spawns two tasks:
    /// 1. Presence sync loop — periodically pushes local AI presence to peers
    /// 2. Outbound routing loop — watches for events to forward to remote AIs
    pub async fn start(&self) {
        let mut running = self.running.write().await;
        if *running {
            warn!("Federation gateway already running");
            return;
        }
        *running = true;
        drop(running);

        // Load saved registry
        if let Err(e) = self.registry.load_into().await {
            warn!(error = %e, "Failed to load AI registry (starting fresh)");
        }

        info!("Federation gateway started");

        // Spawn presence sync loop
        let federation = self.federation.clone();
        let registry = self.registry.clone();
        tokio::spawn(async move {
            presence_sync_loop(federation, registry).await;
        });

        // Spawn outbound routing loop
        let federation = self.federation.clone();
        let registry = self.registry.clone();
        tokio::spawn(async move {
            outbound_routing_loop(federation, registry).await;
        });
    }

    /// Process an incoming federation relay (called by HTTP handler).
    ///
    /// Verifies each message, deserializes the FederationMessage payload,
    /// and injects the event into the local event log via CLI.
    pub async fn process_relay(
        &self,
        request: &FederationRelayRequest,
    ) -> FederationRelayResponse {
        let mut processed = 0usize;
        let mut rejected = 0usize;
        let mut duplicates = 0usize;
        let mut errors = Vec::new();

        // Update HLC
        if let Err(e) = self.federation.clock.receive(&request.sender_hlc) {
            warn!("Relay sender HLC drift: {}", e);
        }

        for (i, signed) in request.messages.iter().enumerate() {
            let content_id = signed.content_id_hex();

            // 1. Verify signature + content hash
            if let Err(e) = signed.verify() {
                errors.push(format!("msg[{}]: {}", i, e));
                rejected += 1;
                continue;
            }

            // 2. Check peer is registered
            if !self.federation.is_known_peer(&signed.origin_pubkey).await {
                errors.push(format!("msg[{}]: unknown peer", i));
                rejected += 1;
                continue;
            }

            // 3. Dedup
            if !self.federation.is_new_event(&content_id).await {
                duplicates += 1;
                continue;
            }

            // 4. Deserialize the FederationMessage from event_bytes
            let msg: FederationMessage = match serde_json::from_slice(&signed.event_bytes) {
                Ok(m) => m,
                Err(e) => {
                    errors.push(format!("msg[{}]: invalid payload: {}", i, e));
                    rejected += 1;
                    continue;
                }
            };

            // 5. Process based on message type
            match self.inject_message(&msg).await {
                Ok(()) => {
                    self.federation.mark_event_seen(content_id).await;
                    self.federation
                        .touch_peer(&signed.origin_pubkey_hex())
                        .await;
                    processed += 1;
                    debug!(
                        msg_type = ?msg.msg_type,
                        source = %msg.source_ai,
                        "Processed federation message"
                    );
                }
                Err(e) => {
                    errors.push(format!("msg[{}]: injection failed: {}", i, e));
                    rejected += 1;
                }
            }
        }

        if processed > 0 || rejected > 0 {
            info!(processed, rejected, duplicates, "Processed federation relay");
        }

        FederationRelayResponse {
            processed,
            rejected,
            duplicates,
            errors,
        }
    }

    /// Inject a verified federation message into the local event log via CLI.
    async fn inject_message(&self, msg: &FederationMessage) -> anyhow::Result<()> {
        // The source_ai is set to "ai_id@teambook_short_id" so local AIs
        // can see where the message came from.
        let federated_source = format!("{}@{}", msg.source_ai, msg.source_teambook);

        match msg.msg_type {
            FederationMessageType::DirectMessage => {
                let content = msg
                    .payload
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("DM missing content field"))?;

                let target = msg
                    .target_ai
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("DM missing target_ai"))?;

                let result =
                    cli_wrapper::teambook_as(&["dm", target, content], &federated_source).await;

                if result.starts_with("Error:") {
                    anyhow::bail!("DM injection failed: {}", result);
                }
                info!(
                    from = %federated_source,
                    to = %target,
                    "Injected federated DM"
                );
                Ok(())
            }

            FederationMessageType::Broadcast => {
                let content = msg
                    .payload
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Broadcast missing content field"))?;

                let channel = msg
                    .payload
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .unwrap_or("general");

                let result = cli_wrapper::teambook_as(
                    &["broadcast", content, "--channel", channel],
                    &federated_source,
                )
                .await;

                if result.starts_with("Error:") {
                    anyhow::bail!("Broadcast injection failed: {}", result);
                }
                info!(
                    from = %federated_source,
                    channel = %channel,
                    "Injected federated broadcast"
                );
                Ok(())
            }

            FederationMessageType::DialogueStart => {
                let topic = msg
                    .payload
                    .get("topic")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("DialogueStart missing topic"))?;

                let responder = msg
                    .target_ai
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("DialogueStart missing target_ai"))?;

                let result = cli_wrapper::teambook_as(
                    &["dialogue-create", responder, topic],
                    &federated_source,
                )
                .await;

                if result.starts_with("Error:") {
                    anyhow::bail!("Dialogue injection failed: {}", result);
                }
                info!(
                    from = %federated_source,
                    to = %responder,
                    topic = %topic,
                    "Injected federated dialogue"
                );
                Ok(())
            }

            FederationMessageType::DialogueRespond => {
                let dialogue_id = msg
                    .payload
                    .get("dialogue_id")
                    .and_then(|v| v.as_str().or_else(|| v.as_u64().map(|_| "")))
                    .ok_or_else(|| anyhow::anyhow!("DialogueRespond missing dialogue_id"))?;

                // Handle both string and number dialogue_id
                let id_str = if let Some(id) = msg.payload.get("dialogue_id").and_then(|v| v.as_u64()) {
                    id.to_string()
                } else {
                    dialogue_id.to_string()
                };

                let content = msg
                    .payload
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("DialogueRespond missing content"))?;

                let result = cli_wrapper::teambook_as(
                    &["dialogue-respond", &id_str, content],
                    &federated_source,
                )
                .await;

                if result.starts_with("Error:") {
                    anyhow::bail!("Dialogue response injection failed: {}", result);
                }
                Ok(())
            }

            FederationMessageType::DialogueEnd => {
                let dialogue_id = if let Some(id) = msg.payload.get("dialogue_id").and_then(|v| v.as_u64()) {
                    id.to_string()
                } else {
                    msg.payload
                        .get("dialogue_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0")
                        .to_string()
                };

                let status = msg
                    .payload
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("completed");

                let result = cli_wrapper::teambook_as(
                    &["dialogue-end", &dialogue_id, status],
                    &federated_source,
                )
                .await;

                if result.starts_with("Error:") {
                    anyhow::bail!("Dialogue end injection failed: {}", result);
                }
                Ok(())
            }

            FederationMessageType::PresenceSync => {
                // Presence sync updates the registry, doesn't inject events
                if let Ok(ais) =
                    serde_json::from_value::<Vec<PresenceAiEntry>>(msg.payload.clone())
                {
                    for ai in &ais {
                        self.registry
                            .register_remote(
                                &ai.ai_id,
                                &msg.source_teambook, // This is the short_id, but we use it
                                &msg.source_teambook,
                                &msg.source_teambook_name,
                                &ai.status,
                                ai.current_task.clone(),
                            )
                            .await;
                    }
                    info!(
                        from = %msg.source_teambook_name,
                        count = ais.len(),
                        "Updated AI registry from presence sync"
                    );
                    let _ = self.registry.save().await;
                }
                Ok(())
            }

            FederationMessageType::LearningShare => {
                let content = msg
                    .payload
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Learning missing content"))?;

                let tags = msg
                    .payload
                    .get("tags")
                    .and_then(|v| v.as_str())
                    .unwrap_or("federation");

                let result = cli_wrapper::teambook_as(
                    &["learning", content, "--tags", tags],
                    &federated_source,
                )
                .await;

                if result.starts_with("Error:") {
                    anyhow::bail!("Learning injection failed: {}", result);
                }
                Ok(())
            }
        }
    }

    /// Process an incoming presence sync (called by HTTP handler).
    pub async fn process_presence_sync(
        &self,
        request: &PresenceSyncRequest,
    ) -> anyhow::Result<()> {
        // Verify the sender is a known peer
        let pubkey_bytes = hex::decode(&request.teambook_pubkey)
            .map_err(|_| anyhow::anyhow!("Invalid pubkey hex"))?;

        if pubkey_bytes.len() != 32 {
            anyhow::bail!("Invalid pubkey length");
        }

        let mut arr = [0u8; 32];
        arr.copy_from_slice(&pubkey_bytes);

        if !self.federation.is_known_peer(&arr).await {
            anyhow::bail!("Presence sync from unknown peer");
        }

        // Verify signature
        let sign_data = serde_json::json!({
            "teambook_pubkey": request.teambook_pubkey,
            "ais": request.ais,
            "hlc": request.hlc,
        });
        let sign_bytes = serde_json::to_vec(&sign_data)?;
        let sig_bytes = hex::decode(&request.signature)
            .map_err(|_| anyhow::anyhow!("Invalid signature hex"))?;

        if sig_bytes.len() != 64 {
            anyhow::bail!("Invalid signature length");
        }

        let mut sig_arr = [0u8; 64];
        sig_arr.copy_from_slice(&sig_bytes);

        if !crate::crypto::verify_signature(&arr, &sign_bytes, &sig_arr) {
            anyhow::bail!("Presence sync signature verification failed");
        }

        // Update HLC
        if let Err(e) = self.federation.clock.receive(&request.hlc) {
            warn!("Presence sync HLC drift: {}", e);
        }

        // Derive short ID
        let short_id = request.teambook_pubkey[..8].to_string();

        // Update registry
        for ai in &request.ais {
            self.registry
                .register_remote(
                    &ai.ai_id,
                    &request.teambook_pubkey,
                    &short_id,
                    &request.teambook_name,
                    &ai.status,
                    ai.current_task.clone(),
                )
                .await;
        }

        // Touch the peer
        self.federation.touch_peer(&request.teambook_pubkey).await;

        info!(
            peer = %request.teambook_name,
            ais = request.ais.len(),
            "Received presence sync"
        );

        let _ = self.registry.save().await;
        Ok(())
    }

    /// Get the AI registry.
    pub fn registry(&self) -> &Arc<AiRegistry> {
        &self.registry
    }

    /// Create a signed FederationMessage.
    pub fn sign_message(&self, msg: &FederationMessage) -> SignedEvent {
        let json_bytes = serde_json::to_vec(msg).expect("FederationMessage serialization");
        SignedEvent::sign(json_bytes, &self.federation.identity)
    }

    /// Build a presence sync request for this Teambook.
    pub async fn build_presence_sync(&self) -> PresenceSyncRequest {
        let local_ais = self.registry.list_local().await;
        let ais: Vec<PresenceAiEntry> = local_ais
            .into_iter()
            .map(|e| PresenceAiEntry {
                ai_id: e.ai_id,
                status: e.status,
                current_task: e.current_task,
            })
            .collect();

        let hlc = self.federation.clock.tick();

        // Sign the payload
        let sign_data = serde_json::json!({
            "teambook_pubkey": self.federation.identity.public_key_hex(),
            "ais": ais,
            "hlc": hlc,
        });
        let sign_bytes = serde_json::to_vec(&sign_data).unwrap();
        let signature = self.federation.identity.sign(&sign_bytes);

        PresenceSyncRequest {
            teambook_pubkey: self.federation.identity.public_key_hex(),
            teambook_name: self.registry.local_name().to_string(),
            ais,
            hlc,
            signature: hex::encode(signature),
        }
    }

    // -----------------------------------------------------------------------
    // Outbound Routing — Federation-Aware Send
    // -----------------------------------------------------------------------

    /// Send a DM, automatically routing to the correct Teambook.
    ///
    /// - If target AI is local: delivers via `teambook dm` CLI
    /// - If target AI is remote: wraps in FederationMessage, signs, pushes to peer
    /// - If target AI is unknown: delivers locally (let CLI handle the error)
    ///
    /// This is the core of transparent federation — callers don't need to know
    /// whether the target is local or remote.
    pub async fn send_dm(
        &self,
        from_ai: &str,
        to_ai: &str,
        content: &str,
    ) -> anyhow::Result<String> {
        // Strip @teambook suffix for local resolution, but keep for routing
        let (bare_to, _explicit_tb) = if let Some((name, tb)) = to_ai.split_once('@') {
            (name, Some(tb))
        } else {
            (to_ai, None)
        };

        let resolution = self.registry.resolve(to_ai).await;

        match resolution {
            AiResolution::Local => {
                // Deliver locally via CLI
                let result = cli_wrapper::teambook_as(
                    &["dm", bare_to, content],
                    from_ai,
                )
                .await;
                if result.starts_with("Error:") {
                    anyhow::bail!("{}", result);
                }
                Ok(result)
            }
            AiResolution::Remote {
                teambook_pubkey_hex,
                teambook_short_id,
                ..
            } => {
                // Build federation message
                let msg = FederationMessage {
                    msg_type: FederationMessageType::DirectMessage,
                    source_ai: from_ai.to_string(),
                    source_teambook: self.registry.local_short_id().to_string(),
                    source_teambook_name: self.registry.local_name().to_string(),
                    target_ai: Some(bare_to.to_string()),
                    hlc: self.federation.clock.tick(),
                    payload: serde_json::json!({ "content": content }),
                };

                // Sign and push to target Teambook
                self.push_to_peer(&msg, &teambook_pubkey_hex, &teambook_short_id)
                    .await?;

                info!(
                    from = %from_ai,
                    to = %bare_to,
                    peer = %teambook_short_id,
                    "Sent federated DM"
                );
                Ok(format!(
                    "DM sent to {}@{} via federation",
                    bare_to, teambook_short_id
                ))
            }
            AiResolution::Unknown => {
                // Unknown AI — deliver locally, let the CLI handle the error
                let result = cli_wrapper::teambook_as(
                    &["dm", to_ai, content],
                    from_ai,
                )
                .await;
                Ok(result)
            }
        }
    }

    /// Send a broadcast, optionally federating to all peers.
    ///
    /// Always delivers locally first via CLI. If `federate` is true,
    /// also pushes to all registered peers.
    pub async fn send_broadcast(
        &self,
        from_ai: &str,
        content: &str,
        channel: &str,
        federate: bool,
    ) -> anyhow::Result<String> {
        // Always deliver locally first
        let local_result = cli_wrapper::teambook_as(
            &["broadcast", content, "--channel", channel],
            from_ai,
        )
        .await;

        if !federate {
            return Ok(local_result);
        }

        // Forward to all peers
        let peers = self.federation.list_peers().await;
        if peers.is_empty() {
            return Ok(local_result);
        }

        let msg = FederationMessage {
            msg_type: FederationMessageType::Broadcast,
            source_ai: from_ai.to_string(),
            source_teambook: self.registry.local_short_id().to_string(),
            source_teambook_name: self.registry.local_name().to_string(),
            target_ai: None,
            hlc: self.federation.clock.tick(),
            payload: serde_json::json!({
                "content": content,
                "channel": channel,
            }),
        };

        let signed = self.sign_message(&msg);
        let relay_req = FederationRelayRequest {
            messages: vec![signed],
            sender_hlc: self.federation.clock.tick(),
        };
        let body = serde_json::to_string(&relay_req)?;

        let mut forwarded = 0usize;
        for peer in &peers {
            let url = format!(
                "{}/api/federation/relay",
                peer.endpoint.trim_end_matches('/')
            );
            let result = tokio::process::Command::new("curl")
                .args([
                    "-s",
                    "-X",
                    "POST",
                    "-H",
                    "Content-Type: application/json",
                    "-d",
                    &body,
                    "--connect-timeout",
                    "5",
                    "--max-time",
                    "10",
                    &url,
                ])
                .output()
                .await;

            if let Ok(output) = result {
                if output.status.success() {
                    forwarded += 1;
                }
            }
        }

        if forwarded > 0 {
            info!(
                from = %from_ai,
                channel = %channel,
                peers = forwarded,
                "Federated broadcast"
            );
            Ok(format!(
                "{}\n[Broadcast federated to {} peer(s)]",
                local_result, forwarded
            ))
        } else {
            Ok(local_result)
        }
    }

    /// Push a FederationMessage to a specific peer.
    async fn push_to_peer(
        &self,
        msg: &FederationMessage,
        peer_pubkey_hex: &str,
        peer_short_id: &str,
    ) -> anyhow::Result<()> {
        let signed = self.sign_message(msg);
        let relay_req = FederationRelayRequest {
            messages: vec![signed],
            sender_hlc: self.federation.clock.tick(),
        };

        let peer = self
            .federation
            .get_peer(peer_pubkey_hex)
            .await
            .ok_or_else(|| anyhow::anyhow!("Peer {} not found in registry", peer_short_id))?;

        let url = format!(
            "{}/api/federation/relay",
            peer.endpoint.trim_end_matches('/')
        );
        let body = serde_json::to_string(&relay_req)?;

        let result = tokio::process::Command::new("curl")
            .args([
                "-s",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "-d",
                &body,
                "--connect-timeout",
                "5",
                "--max-time",
                "15",
                &url,
            ])
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("Federation relay transport error: {}", e))?;

        if result.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&result.stderr);
            anyhow::bail!("Federation relay to {} failed: {}", peer_short_id, stderr.trim())
        }
    }
}

// ---------------------------------------------------------------------------
// Background Tasks
// ---------------------------------------------------------------------------

/// Presence sync loop: every 60 seconds, push local AI presence to all peers.
async fn presence_sync_loop(federation: Arc<FederationState>, registry: Arc<AiRegistry>) {
    loop {
        // First, refresh local AI presence from teambook
        let status_output = cli_wrapper::teambook_as(&["status"], "federation-gateway").await;
        parse_and_register_local_ais(&registry, &status_output).await;

        // Build presence sync request
        let local_ais = registry.list_local().await;
        if local_ais.is_empty() {
            debug!("No local AIs to sync");
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            continue;
        }

        let ais: Vec<PresenceAiEntry> = local_ais
            .into_iter()
            .map(|e| PresenceAiEntry {
                ai_id: e.ai_id,
                status: e.status,
                current_task: e.current_task,
            })
            .collect();

        let hlc = federation.clock.tick();
        let sign_data = serde_json::json!({
            "teambook_pubkey": federation.identity.public_key_hex(),
            "ais": ais,
            "hlc": hlc,
        });
        let sign_bytes = serde_json::to_vec(&sign_data).unwrap();
        let signature = federation.identity.sign(&sign_bytes);

        let request = PresenceSyncRequest {
            teambook_pubkey: federation.identity.public_key_hex(),
            teambook_name: registry.local_name().to_string(),
            ais,
            hlc,
            signature: hex::encode(signature),
        };

        // Push to all peers
        let peers = federation.list_peers().await;
        for peer in &peers {
            let url = format!(
                "{}/api/federation/presence",
                peer.endpoint.trim_end_matches('/')
            );
            let body = match serde_json::to_string(&request) {
                Ok(b) => b,
                Err(e) => {
                    error!("Failed to serialize presence sync: {}", e);
                    continue;
                }
            };

            let result = tokio::process::Command::new("curl")
                .args([
                    "-s",
                    "-X",
                    "POST",
                    "-H",
                    "Content-Type: application/json",
                    "-d",
                    &body,
                    "--connect-timeout",
                    "5",
                    "--max-time",
                    "10",
                    &url,
                ])
                .output()
                .await;

            match result {
                Ok(output) if output.status.success() => {
                    debug!(peer = %peer.short_id(), "Presence sync pushed");
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    debug!(peer = %peer.short_id(), error = %stderr.trim(), "Presence sync failed");
                }
                Err(e) => {
                    debug!(peer = %peer.short_id(), error = %e, "Presence sync transport error");
                }
            }
        }

        let _ = registry.save().await;

        // Sleep 60 seconds before next sync
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}

/// Outbound routing loop: monitors for events that need cross-Teambook delivery.
///
/// Note: DM/broadcast routing is now handled at the point of creation
/// (via FederationGateway::send_dm / send_broadcast), so this loop
/// only needs to handle edge cases and future event types.
/// Uses `teambook standby` for event-driven wake — zero CPU while waiting.
async fn outbound_routing_loop(federation: Arc<FederationState>, _registry: Arc<AiRegistry>) {
    // Wait for initial presence sync to populate registry
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    loop {
        // Check if we have any peers
        let peers = federation.list_peers().await;
        if peers.is_empty() {
            // No peers, sleep longer
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            continue;
        }

        // Wait for an event (event-driven, zero CPU)
        let wake_result =
            cli_wrapper::teambook_as(&["standby", "60"], "federation-gateway").await;

        debug!(wake = %wake_result, "Gateway outbound wake (monitoring)");

        // DM/broadcast routing now happens at the point of creation
        // (send_dm/send_broadcast methods). This loop remains for:
        // - Future event types that need passive outbound forwarding
        // - Monitoring/health checks
        // - Catching events created directly via CLI (not through MCP/HTTP)
    }
}

/// Parse teambook status output and register local AIs.
async fn parse_and_register_local_ais(registry: &AiRegistry, status_output: &str) {
    // Parse the status output format. Typical format:
    //   AI: ai-1 | Status: active | Task: Reviewing PRs
    //   AI: ai-4 | Status: standby
    for line in status_output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("=") || line.starts_with("-") {
            continue;
        }

        // Try to extract AI ID and status from various formats
        if let Some(ai_id) = extract_field(line, "AI:").or_else(|| extract_field(line, "ai:")) {
            let status = extract_field(line, "Status:")
                .or_else(|| extract_field(line, "status:"))
                .unwrap_or_else(|| "active".to_string());
            let task = extract_field(line, "Task:")
                .or_else(|| extract_field(line, "task:"));

            registry.register_local(&ai_id, &status, task).await;
        }

        // Also handle simpler formats like: "ai-1 (active) Working on X"
        // or "ai-1 | active | Working on X"
        if line.contains('|') && !line.contains("AI:") {
            let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
            if parts.len() >= 2 {
                let ai_id = parts[0].trim();
                let status = parts[1].trim().trim_start_matches("Status:").trim();
                let task = parts.get(2).map(|s| s.trim().trim_start_matches("Task:").trim().to_string());
                if !ai_id.is_empty()
                    && !ai_id.contains(' ')
                    && ai_id.len() < 64
                    && !ai_id.starts_with("=")
                {
                    registry
                        .register_local(ai_id, status, task)
                        .await;
                }
            }
        }
    }
}

/// Extract a field value from a status line.
fn extract_field(line: &str, prefix: &str) -> Option<String> {
    let idx = line.find(prefix)?;
    let after = &line[idx + prefix.len()..].trim_start();
    // Take until next | or end of line
    let value = if let Some(pipe) = after.find('|') {
        &after[..pipe]
    } else {
        after
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_registry_local_ai() {
        let registry = AiRegistry::new(
            "abcdef1234567890".repeat(4),
            "abcdef12".to_string(),
            "test-teambook".to_string(),
        );

        registry
            .register_local("ai-1", "active", Some("Reviewing code".into()))
            .await;

        let all = registry.list_all().await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].ai_id, "ai-1");
        assert!(all[0].is_local);
        assert_eq!(all[0].status, "active");

        match registry.resolve("ai-1").await {
            AiResolution::Local => {}
            other => panic!("Expected Local, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_registry_remote_ai() {
        let registry = AiRegistry::new(
            "a".repeat(64),
            "aaaaaaaa".to_string(),
            "local-tb".to_string(),
        );

        registry
            .register_remote(
                "ai-3",
                &"b".repeat(64),
                "bbbbbbbb",
                "remote-tb",
                "standby",
                None,
            )
            .await;

        match registry.resolve("ai-3").await {
            AiResolution::Remote {
                teambook_short_id, ..
            } => {
                assert_eq!(teambook_short_id, "bbbbbbbb");
            }
            other => panic!("Expected Remote, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_registry_unknown_ai() {
        let registry = AiRegistry::new(
            "a".repeat(64),
            "aaaaaaaa".to_string(),
            "local-tb".to_string(),
        );

        match registry.resolve("unknown-ai").await {
            AiResolution::Unknown => {}
            other => panic!("Expected Unknown, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_registry_federated_address() {
        let registry = AiRegistry::new(
            "a".repeat(64),
            "aaaaaaaa".to_string(),
            "local-tb".to_string(),
        );

        registry
            .register_remote(
                "ai-1",
                &"b".repeat(64),
                "bbbbbbbb",
                "remote-tb",
                "active",
                None,
            )
            .await;

        // Resolve with explicit @teambook
        match registry.resolve("ai-1@bbbbbbbb").await {
            AiResolution::Remote { .. } => {}
            other => panic!("Expected Remote, got {:?}", other),
        }

        // Unknown teambook short ID
        match registry.resolve("ai-1@cccccccc").await {
            AiResolution::Unknown => {}
            other => panic!("Expected Unknown, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_registry_local_vs_remote() {
        let registry = AiRegistry::new(
            "a".repeat(64),
            "aaaaaaaa".to_string(),
            "local-tb".to_string(),
        );

        registry
            .register_local("ai-1", "active", None)
            .await;
        registry
            .register_remote(
                "ai-3",
                &"b".repeat(64),
                "bbbbbbbb",
                "remote-tb",
                "active",
                None,
            )
            .await;

        let local = registry.list_local().await;
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].ai_id, "ai-1");

        let remote = registry.list_remote().await;
        assert_eq!(remote.len(), 1);
        assert_eq!(remote[0].ai_id, "ai-3");
    }

    #[test]
    fn test_extract_field() {
        assert_eq!(
            extract_field("AI: ai-1 | Status: active", "AI:"),
            Some("ai-1".to_string())
        );
        assert_eq!(
            extract_field("AI: ai-1 | Status: active", "Status:"),
            Some("active".to_string())
        );
        assert_eq!(
            extract_field("AI: ai-1 | Status: active | Task: Code review", "Task:"),
            Some("Code review".to_string())
        );
        assert_eq!(extract_field("no match here", "AI:"), None);
    }

    #[test]
    fn test_federation_message_serialization() {
        let msg = FederationMessage {
            msg_type: FederationMessageType::DirectMessage,
            source_ai: "ai-1".to_string(),
            source_teambook: "abcdef12".to_string(),
            source_teambook_name: "alice-homelab".to_string(),
            target_ai: Some("ai-4".to_string()),
            hlc: HlcTimestamp::zero(42),
            payload: serde_json::json!({
                "content": "Hey, can you review the auth changes?"
            }),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let recovered: FederationMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(recovered.source_ai, "ai-1");
        assert_eq!(recovered.msg_type, FederationMessageType::DirectMessage);
        assert_eq!(
            recovered.target_ai,
            Some("ai-4".to_string())
        );
    }

    #[test]
    fn test_federated_address() {
        let local = FederatedAiEntry {
            ai_id: "ai-1".to_string(),
            teambook_pubkey_hex: "a".repeat(64),
            teambook_short_id: "aaaaaaaa".to_string(),
            teambook_name: "local".to_string(),
            is_local: true,
            status: "active".to_string(),
            current_task: None,
            last_seen_us: 0,
        };
        assert_eq!(local.federated_address(), "ai-1");

        let remote = FederatedAiEntry {
            ai_id: "ai-3".to_string(),
            teambook_pubkey_hex: "b".repeat(64),
            teambook_short_id: "bbbbbbbb".to_string(),
            teambook_name: "remote".to_string(),
            is_local: false,
            status: "active".to_string(),
            current_task: None,
            last_seen_us: 0,
        };
        assert_eq!(remote.federated_address(), "ai-3@bbbbbbbb");
    }

    #[tokio::test]
    async fn test_parse_and_register_local_ais() {
        let registry = AiRegistry::new(
            "a".repeat(64),
            "aaaaaaaa".to_string(),
            "local-tb".to_string(),
        );

        let status_output = "\
AI: ai-1 | Status: active | Task: Code review
AI: ai-2 | Status: standby
=== Team ===
ai-4 | active | Building federation";

        parse_and_register_local_ais(&registry, status_output).await;

        let local = registry.list_local().await;
        assert!(local.len() >= 2, "Expected at least 2 local AIs, got {}", local.len());

        // Check sage was registered
        match registry.resolve("ai-1").await {
            AiResolution::Local => {}
            other => panic!("Expected ai-1 to be Local, got {:?}", other),
        }

        // Check lyra was registered
        match registry.resolve("ai-2").await {
            AiResolution::Local => {}
            other => panic!("Expected ai-2 to be Local, got {:?}", other),
        }
    }

    #[test]
    fn test_federation_send_request_serialization() {
        // Verify FederationRelayRequest serializes correctly
        let msg = FederationMessage {
            msg_type: FederationMessageType::Broadcast,
            source_ai: "ai-4".to_string(),
            source_teambook: "abcdef12".to_string(),
            source_teambook_name: "test-tb".to_string(),
            target_ai: None,
            hlc: HlcTimestamp::zero(42),
            payload: serde_json::json!({
                "content": "Hello federation!",
                "channel": "general",
            }),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let recovered: FederationMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(recovered.msg_type, FederationMessageType::Broadcast);
        assert_eq!(recovered.source_ai, "ai-4");
        assert!(recovered.target_ai.is_none());
        assert_eq!(
            recovered.payload.get("channel").unwrap().as_str().unwrap(),
            "general"
        );
    }

    #[tokio::test]
    async fn test_resolve_strips_at_suffix() {
        let registry = AiRegistry::new(
            "a".repeat(64),
            "aaaaaaaa".to_string(),
            "local-tb".to_string(),
        );

        // Register local AI
        registry.register_local("ai-1", "active", None).await;

        // Resolving with own teambook short_id should still resolve as Local
        match registry.resolve("ai-1@aaaaaaaa").await {
            AiResolution::Local => {}
            other => panic!("Expected Local for local @address, got {:?}", other),
        }
    }
}
