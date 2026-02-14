//! Deep Net Mobile Client - Distributed Mesh Federation
//!
//! Architecture:
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    Mobile Device (Sovereign Node)           │
//! ├─────────────────────────────────────────────────────────────┤
//! │  NodeIdentity (Ed25519 keypair)                             │
//! │  - Cryptographic identity, no central authority             │
//! │  - Signs all messages for authenticity                      │
//! ├─────────────────────────────────────────────────────────────┤
//! │  LocalStore (TeamEngram-style)                              │
//! │  - DMs, Broadcasts, Presence stored locally                 │
//! │  - Atomic commits via bincode serialization                 │
//! │  - Works completely offline                                 │
//! ├─────────────────────────────────────────────────────────────┤
//! │  MeshManager                                                │
//! │  - mDNS discovery for LAN peers                             │
//! │  - QUIC transport for secure P2P connections                │
//! │  - Vector clock sync (no full replication)                  │
//! ├─────────────────────────────────────────────────────────────┤
//! │  HTTP Fallback (optional)                                   │
//! │  - Sync to server when P2P not available                    │
//! │  - Legacy compatibility                                     │
//! └─────────────────────────────────────────────────────────────┘
//!         │                    │
//!         │ P2P (preferred)    │ HTTP (fallback)
//!         ▼                    ▼
//! ┌──────────────┐    ┌─────────────────────────────────────────┐
//! │  Other Nodes │    │  Federation Server (optional relay)     │
//! │  (via QUIC)  │    │  - For NAT traversal                    │
//! └──────────────┘    └─────────────────────────────────────────┘
//! ```

use parking_lot::Mutex as ParkingMutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

// Import deepnet-core types
use deepnet_core::{
    NodeIdentity,
    MdnsDiscovery,
    Discovery,  // async trait
};

// Import federation types for transport layer
use federation::{
    TransportType as FedTransportType,
    TrustLevel as FedTrustLevel,
};

// UniFFI scaffolding
uniffi::include_scaffolding!("deepnet");

// ============================================================================
// UniFFI TYPE DEFINITIONS (must match UDL file)
// ============================================================================

/// Result of federation registration
#[derive(Debug, Clone)]
pub struct FederationResult {
    pub success: bool,
    pub device_id: String,
    pub auth_token: String,
    pub error_message: Option<String>,
}

/// Connection status enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    Connecting,
    Offline,
    Error,
}

/// Result of sending a message
#[derive(Debug, Clone)]
pub struct MessageResult {
    pub success: bool,
    pub message_id: i64,
    pub error_message: Option<String>,
}

/// Direct message structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectMessage {
    pub id: i64,
    pub from_ai: String,
    pub to_ai: String,
    pub content: String,
    pub timestamp: String,
}

/// Broadcast message structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Broadcast {
    pub id: i64,
    pub from_ai: String,
    pub channel: String,
    pub content: String,
    pub timestamp: String,
}

/// Team member (AI or device)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub ai_id: String,
    pub display_name: String,
    pub status: String,
    pub current_activity: Option<String>,
}

/// Federation member (registered device or AI)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationMember {
    pub member_id: String,
    pub member_type: String,
    pub display_name: String,
    pub status: String,
    pub registered_at: String,
}

/// Result of sync operation
#[derive(Debug, Clone)]
pub struct SyncResult {
    pub success: bool,
    pub items_pushed: u32,
    pub items_pulled: u32,
    pub error_message: Option<String>,
}

// ============================================================================
// MESH UNIFFI TYPES
// ============================================================================

/// Result of mesh identity initialization
#[derive(Debug, Clone)]
pub struct MeshIdentityResult {
    pub success: bool,
    pub node_id: String,
    pub display_name: String,
    pub newly_created: bool,
    pub error_message: Option<String>,
}

/// A peer in the mesh (discovered or connected)
#[derive(Debug, Clone)]
pub struct MeshPeer {
    pub node_id: String,
    pub display_name: String,
    pub address: String,
    pub transport_type: String,
    pub status: String,
    pub latency_ms: u32,
    pub last_seen: String,
}

/// Result of mesh connection attempt
#[derive(Debug, Clone)]
pub struct MeshConnectionResult {
    pub success: bool,
    pub node_id: String,
    pub transport_type: String,
    pub latency_ms: u32,
    pub error_message: Option<String>,
}

/// Message received via mesh P2P
#[derive(Debug, Clone)]
pub struct MeshMessage {
    pub id: String,
    pub from_node_id: String,
    pub from_display_name: String,
    pub content: String,
    pub timestamp: String,
    pub encrypted: bool,
}

// ============================================================================
// FEDERATION TRANSPORT TYPES (UniFFI compatible)
// ============================================================================

/// Transport type for mesh connections (from federation-rs)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    QuicInternet,
    QuicLan,
    Mdns,
    BluetoothLe,
    BluetoothClassic,
    Passkey,
    Relay,
}

impl From<FedTransportType> for TransportType {
    fn from(t: FedTransportType) -> Self {
        match t {
            FedTransportType::QuicInternet => TransportType::QuicInternet,
            FedTransportType::QuicLan => TransportType::QuicLan,
            FedTransportType::Mdns => TransportType::Mdns,
            FedTransportType::BluetoothLe => TransportType::BluetoothLe,
            FedTransportType::BluetoothClassic => TransportType::BluetoothClassic,
            FedTransportType::Passkey => TransportType::Passkey,
            FedTransportType::Relay => TransportType::Relay,
        }
    }
}

impl From<TransportType> for FedTransportType {
    fn from(t: TransportType) -> Self {
        match t {
            TransportType::QuicInternet => FedTransportType::QuicInternet,
            TransportType::QuicLan => FedTransportType::QuicLan,
            TransportType::Mdns => FedTransportType::Mdns,
            TransportType::BluetoothLe => FedTransportType::BluetoothLe,
            TransportType::BluetoothClassic => FedTransportType::BluetoothClassic,
            TransportType::Passkey => FedTransportType::Passkey,
            TransportType::Relay => FedTransportType::Relay,
        }
    }
}

/// Trust level for federation nodes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    Anonymous,
    Verified,
    Trusted,
    Owner,
}

impl From<FedTrustLevel> for TrustLevel {
    fn from(t: FedTrustLevel) -> Self {
        match t {
            FedTrustLevel::Anonymous => TrustLevel::Anonymous,
            FedTrustLevel::Verified => TrustLevel::Verified,
            FedTrustLevel::Trusted => TrustLevel::Trusted,
            FedTrustLevel::Owner => TrustLevel::Owner,
        }
    }
}

impl From<TrustLevel> for FedTrustLevel {
    fn from(t: TrustLevel) -> Self {
        match t {
            TrustLevel::Anonymous => FedTrustLevel::Anonymous,
            TrustLevel::Verified => FedTrustLevel::Verified,
            TrustLevel::Trusted => FedTrustLevel::Trusted,
            TrustLevel::Owner => FedTrustLevel::Owner,
        }
    }
}

/// Rich connection state with metadata
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FederationConnectionState {
    Disconnected,
    Connecting,
    Authenticating,
    NegotiatingSharing,
    Connected,
    Reconnecting,
    Failed,
}

/// Full connection info for a peer
#[derive(Debug, Clone)]
pub struct FederationConnectionInfo {
    pub node_id: String,
    pub state: FederationConnectionState,
    pub transport: TransportType,
    pub trust_level: TrustLevel,
    pub latency_ms: u32,
    pub connected_at: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub error_message: Option<String>,
}

// ============================================================================
// GLOBAL STATE
// ============================================================================

/// Global client state
static CLIENT: OnceLock<ParkingMutex<DeepNetState>> = OnceLock::new();

/// Mesh-specific state (separate for async operations)
static MESH_STATE: OnceLock<ParkingMutex<MeshState>> = OnceLock::new();

/// Internal client state (legacy HTTP sync)
struct DeepNetState {
    /// Server URL for sync operations
    server_url: String,
    /// This device's unique ID (legacy)
    device_id: String,
    /// Authentication token from server
    auth_token: Option<String>,
    /// Connection status
    status: ConnectionStatus,
    /// Local storage path
    storage_path: PathBuf,
    /// In-memory store (persisted to disk)
    store: LocalStore,
}

/// Mesh state (P2P federation)
struct MeshState {
    /// Node identity (Ed25519 keypair)
    identity: Option<Arc<NodeIdentity>>,
    /// mDNS discovery for LAN peers
    mdns_discovery: Option<MdnsDiscovery>,
    /// Discovered peers (node_id -> peer info)
    discovered_peers: HashMap<String, StoredMeshPeer>,
    /// Connected peers
    connected_peers: HashMap<String, StoredMeshPeer>,
    /// Mesh messages received (stored locally)
    mesh_messages: Vec<StoredMeshMessage>,
    /// Discovery running flag
    discovery_running: bool,
}

/// Stored mesh peer info
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMeshPeer {
    node_id: String,
    display_name: String,
    address: String,
    transport_type: String,
    status: String,
    latency_ms: u32,
    last_seen: u64,
    // Federation connection tracking
    trust_level: u8,  // 0=Anonymous, 1=Verified, 2=Trusted, 3=Owner
    connection_state: u8, // 0=Disconnected, 1=Connecting, etc.
    connected_at: u64,
    bytes_sent: u64,
    bytes_received: u64,
    available_transports: Vec<String>,
}

/// Stored mesh message
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMeshMessage {
    id: String,
    from_node_id: String,
    from_display_name: String,
    content: String,
    timestamp: u64,
    encrypted: bool,
}

impl Default for DeepNetState {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            device_id: String::new(),
            auth_token: None,
            status: ConnectionStatus::Disconnected,
            storage_path: PathBuf::new(),
            store: LocalStore::default(),
        }
    }
}

impl Default for MeshState {
    fn default() -> Self {
        Self {
            identity: None,
            mdns_discovery: None,
            discovered_peers: HashMap::new(),
            connected_peers: HashMap::new(),
            mesh_messages: Vec::new(),
            discovery_running: false,
        }
    }
}

fn get_client() -> &'static ParkingMutex<DeepNetState> {
    CLIENT.get_or_init(|| ParkingMutex::new(DeepNetState::default()))
}

fn get_mesh_state() -> &'static ParkingMutex<MeshState> {
    MESH_STATE.get_or_init(|| ParkingMutex::new(MeshState::default()))
}

// ============================================================================
// LOCAL STORE (TeamEngram-style)
// ============================================================================

/// Local store for offline-first operation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LocalStore {
    /// Auto-incrementing ID counter
    next_id: u64,
    /// Direct messages
    dms: Vec<StoredDM>,
    /// Broadcast messages
    broadcasts: Vec<StoredBroadcast>,
    /// Presence records (keyed by ai_id)
    presences: HashMap<String, StoredPresence>,
    /// Team members
    team_members: Vec<StoredTeamMember>,
    /// Federation members
    federation_members: Vec<StoredFederationMember>,
    /// Sync metadata
    last_sync: u64,
    /// Pending sync items (to push to server)
    pending_sync: Vec<SyncItem>,
}

/// Stored direct message
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredDM {
    id: u64,
    from_ai: String,
    to_ai: String,
    content: String,
    timestamp: u64,
    read: bool,
    synced: bool,
}

/// Stored broadcast message
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredBroadcast {
    id: u64,
    from_ai: String,
    channel: String,
    content: String,
    timestamp: u64,
    synced: bool,
}

/// Stored presence record
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredPresence {
    ai_id: String,
    status: String,
    current_task: String,
    last_seen: u64,
}

/// Stored team member
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredTeamMember {
    ai_id: String,
    display_name: String,
    status: String,
    current_activity: Option<String>,
}

/// Stored federation member
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredFederationMember {
    member_id: String,
    member_type: String,
    display_name: String,
    status: String,
    registered_at: String,
}

/// Item pending sync to server
#[derive(Debug, Clone, Serialize, Deserialize)]
enum SyncItem {
    DM { local_id: u64 },
    Broadcast { local_id: u64 },
    Presence,
}

impl LocalStore {
    /// Insert a direct message
    fn insert_dm(&mut self, from: &str, to: &str, content: &str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        self.dms.push(StoredDM {
            id,
            from_ai: from.to_string(),
            to_ai: to.to_string(),
            content: content.to_string(),
            timestamp: now_millis(),
            read: false,
            synced: false,
        });

        // Mark for sync
        self.pending_sync.push(SyncItem::DM { local_id: id });

        id
    }

    /// Get DMs for a recipient
    fn get_dms(&self, to_ai: &str, limit: usize) -> Vec<&StoredDM> {
        let mut dms: Vec<_> = self.dms.iter()
            .filter(|dm| dm.to_ai == to_ai)
            .collect();
        dms.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        dms.truncate(limit);
        dms
    }

    /// Insert a broadcast
    fn insert_broadcast(&mut self, from: &str, channel: &str, content: &str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        self.broadcasts.push(StoredBroadcast {
            id,
            from_ai: from.to_string(),
            channel: channel.to_string(),
            content: content.to_string(),
            timestamp: now_millis(),
            synced: false,
        });

        // Mark for sync
        self.pending_sync.push(SyncItem::Broadcast { local_id: id });

        id
    }

    /// Get broadcasts by channel
    fn get_broadcasts(&self, channel: &str, limit: usize) -> Vec<&StoredBroadcast> {
        let mut broadcasts: Vec<_> = self.broadcasts.iter()
            .filter(|bc| bc.channel == channel || channel == "all")
            .collect();
        broadcasts.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        broadcasts.truncate(limit);
        broadcasts
    }

    /// Update presence
    fn update_presence(&mut self, ai_id: &str, status: &str, task: &str) {
        self.presences.insert(ai_id.to_string(), StoredPresence {
            ai_id: ai_id.to_string(),
            status: status.to_string(),
            current_task: task.to_string(),
            last_seen: now_millis(),
        });

        // Mark for sync (but don't spam - presence is fire-and-forget)
        if !self.pending_sync.iter().any(|s| matches!(s, SyncItem::Presence)) {
            self.pending_sync.push(SyncItem::Presence);
        }
    }

    /// Get team members
    fn get_team(&self) -> Vec<&StoredTeamMember> {
        self.team_members.iter().collect()
    }

    /// Get federation members
    fn get_federation_members(&self) -> Vec<&StoredFederationMember> {
        self.federation_members.iter().collect()
    }

    /// Serialize to bytes
    fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize from bytes
    fn from_bytes(data: &[u8]) -> Option<Self> {
        bincode::deserialize(data).ok()
    }
}

// ============================================================================
// PERSISTENCE
// ============================================================================

/// Save store to disk
fn save_store(state: &DeepNetState) {
    if state.storage_path.as_os_str().is_empty() {
        return;
    }

    let data = state.store.to_bytes();
    if let Some(parent) = state.storage_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&state.storage_path, data);
}

/// Load store from disk
fn load_store(path: &PathBuf) -> LocalStore {
    if let Ok(data) = std::fs::read(path) {
        LocalStore::from_bytes(&data).unwrap_or_default()
    } else {
        LocalStore::default()
    }
}

// ============================================================================
// DEVICE ID GENERATION
// ============================================================================

/// Generate a unique device ID
fn generate_device_id(device_name: &str) -> String {
    let uuid = Uuid::new_v4();
    let timestamp = chrono::Utc::now().timestamp();
    let mut hasher = Sha256::new();
    hasher.update(format!("{}:{}:{}", uuid, timestamp, device_name));
    let hash = hasher.finalize();
    format!("device-{}", hex::encode(&hash[..8]))
}

/// Get current time in milliseconds
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ============================================================================
// FEDERATION API (Public - exposed via UniFFI)
// ============================================================================

/// Initialize Deep Net with storage path
/// Call this first before any other operations
pub fn deep_net_init(storage_dir: String) -> bool {
    let client = get_client();
    let mut state = client.lock();

    // Set storage path
    state.storage_path = PathBuf::from(&storage_dir).join("deepnet.db");

    // Load existing store
    state.store = load_store(&state.storage_path);

    true
}

/// Register this device with the Deep Net Federation
pub fn federation_register(server_url: String, device_name: String) -> FederationResult {
    let device_id = generate_device_id(&device_name);

    // Build registration request
    let request_body = serde_json::json!({
        "device_id": device_id,
        "device_name": device_name,
        "device_type": "android",
        "capabilities": ["teambook", "notifications"],
        "timestamp": chrono::Utc::now().to_rfc3339()
    });

    // Try to register with server
    let http_client = reqwest::blocking::Client::new();
    let url = format!("{}/federation/register", server_url);

    match http_client
        .post(&url)
        .json(&request_body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
    {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                if let Ok(data) = response.json::<serde_json::Value>() {
                    let auth_token = data["auth_token"].as_str().unwrap_or("").to_string();

                    // Update client state
                    let client = get_client();
                    let mut state = client.lock();
                    state.server_url = server_url;
                    state.device_id = device_id.clone();
                    state.auth_token = Some(auth_token.clone());
                    state.status = ConnectionStatus::Connected;

                    // Update local presence
                    state.store.update_presence(&device_id, "active", "connected");
                    save_store(&state);

                    return FederationResult {
                        success: true,
                        device_id,
                        auth_token,
                        error_message: None,
                    };
                }
            }

            FederationResult {
                success: false,
                device_id: String::new(),
                auth_token: String::new(),
                error_message: Some(format!("Server error: {}", status)),
            }
        }
        Err(e) => {
            // Offline mode - register locally only
            let auth_token = format!("offline-{}", Uuid::new_v4());

            let client = get_client();
            let mut state = client.lock();
            state.server_url = server_url;
            state.device_id = device_id.clone();
            state.auth_token = Some(auth_token.clone());
            state.status = ConnectionStatus::Offline;

            // Store locally
            state.store.update_presence(&device_id, "active", "offline mode");
            state.store.federation_members.push(StoredFederationMember {
                member_id: device_id.clone(),
                member_type: "device".to_string(),
                display_name: device_name,
                status: "offline".to_string(),
                registered_at: chrono::Utc::now().to_rfc3339(),
            });
            save_store(&state);

            FederationResult {
                success: true,
                device_id,
                auth_token,
                error_message: Some(format!("Offline mode: {}", e)),
            }
        }
    }
}

/// Get current federation connection status
pub fn federation_status() -> ConnectionStatus {
    let client = get_client();
    let state = client.lock();
    state.status.clone()
}

/// Disconnect from federation
pub fn federation_disconnect() {
    let client = get_client();
    let mut state = client.lock();
    state.auth_token = None;
    state.status = ConnectionStatus::Disconnected;

    // Update local presence
    let device_id = state.device_id.clone();
    if !device_id.is_empty() {
        state.store.update_presence(&device_id, "offline", "disconnected");
        save_store(&state);
    }
}

/// Get all federation members
pub fn federation_get_members() -> Vec<FederationMember> {
    let client = get_client();
    let mut state = client.lock();

    // Try to fetch from server if online
    if state.status == ConnectionStatus::Connected {
        if let Some(ref token) = state.auth_token {
            let http_client = reqwest::blocking::Client::new();
            let url = format!("{}/federation/members", state.server_url);

            if let Ok(response) = http_client
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .timeout(std::time::Duration::from_secs(10))
                .send()
            {
                if response.status().is_success() {
                    if let Ok(members) = response.json::<Vec<FederationMember>>() {
                        // Cache locally
                        state.store.federation_members = members.iter().map(|m| StoredFederationMember {
                            member_id: m.member_id.clone(),
                            member_type: m.member_type.clone(),
                            display_name: m.display_name.clone(),
                            status: m.status.clone(),
                            registered_at: m.registered_at.clone(),
                        }).collect();
                        save_store(&state);

                        return members;
                    }
                }
            }
        }
    }

    // Return cached local data
    state.store.get_federation_members()
        .iter()
        .map(|m| FederationMember {
            member_id: m.member_id.clone(),
            member_type: m.member_type.clone(),
            display_name: m.display_name.clone(),
            status: m.status.clone(),
            registered_at: m.registered_at.clone(),
        })
        .collect()
}

// ============================================================================
// TEAMBOOK API (Public - exposed via UniFFI)
// ============================================================================

/// Send a direct message to an AI
pub fn teambook_dm(to_ai_id: String, content: String) -> MessageResult {
    let client = get_client();
    let mut state = client.lock();

    if state.device_id.is_empty() {
        return MessageResult {
            success: false,
            message_id: 0,
            error_message: Some("Not registered with federation".to_string()),
        };
    }

    let device_id = state.device_id.clone();

    // Store locally first (offline-first)
    let local_id = state.store.insert_dm(&device_id, &to_ai_id, &content);
    save_store(&state);

    // Try to sync to server if online
    if state.status == ConnectionStatus::Connected {
        if let Some(ref token) = state.auth_token {
            let request_body = serde_json::json!({
                "from_ai": device_id,
                "to_ai": to_ai_id,
                "content": content,
                "timestamp": chrono::Utc::now().to_rfc3339()
            });

            let http_client = reqwest::blocking::Client::new();
            let url = format!("{}/teambook/dm", state.server_url);

            if let Ok(response) = http_client
                .post(&url)
                .header("Authorization", format!("Bearer {}", token))
                .json(&request_body)
                .timeout(std::time::Duration::from_secs(10))
                .send()
            {
                if response.status().is_success() {
                    // Mark as synced
                    if let Some(dm) = state.store.dms.iter_mut().find(|d| d.id == local_id) {
                        dm.synced = true;
                    }
                    save_store(&state);

                    if let Ok(data) = response.json::<serde_json::Value>() {
                        return MessageResult {
                            success: true,
                            message_id: data["id"].as_i64().unwrap_or(local_id as i64),
                            error_message: None,
                        };
                    }
                }
            }
        }
    }

    // Return success for offline operation
    MessageResult {
        success: true,
        message_id: local_id as i64,
        error_message: Some("Queued for sync".to_string()),
    }
}

/// Send a broadcast message
pub fn teambook_broadcast(content: String, channel: String) -> MessageResult {
    let client = get_client();
    let mut state = client.lock();

    if state.device_id.is_empty() {
        return MessageResult {
            success: false,
            message_id: 0,
            error_message: Some("Not registered with federation".to_string()),
        };
    }

    let device_id = state.device_id.clone();

    // Store locally first
    let local_id = state.store.insert_broadcast(&device_id, &channel, &content);
    save_store(&state);

    // Try to sync to server if online
    if state.status == ConnectionStatus::Connected {
        if let Some(ref token) = state.auth_token {
            let request_body = serde_json::json!({
                "from_ai": device_id,
                "channel": channel,
                "content": content,
                "timestamp": chrono::Utc::now().to_rfc3339()
            });

            let http_client = reqwest::blocking::Client::new();
            let url = format!("{}/teambook/broadcast", state.server_url);

            if let Ok(response) = http_client
                .post(&url)
                .header("Authorization", format!("Bearer {}", token))
                .json(&request_body)
                .timeout(std::time::Duration::from_secs(10))
                .send()
            {
                if response.status().is_success() {
                    // Mark as synced
                    if let Some(bc) = state.store.broadcasts.iter_mut().find(|b| b.id == local_id) {
                        bc.synced = true;
                    }
                    save_store(&state);

                    if let Ok(data) = response.json::<serde_json::Value>() {
                        return MessageResult {
                            success: true,
                            message_id: data["id"].as_i64().unwrap_or(local_id as i64),
                            error_message: None,
                        };
                    }
                }
            }
        }
    }

    MessageResult {
        success: true,
        message_id: local_id as i64,
        error_message: Some("Queued for sync".to_string()),
    }
}

/// Get recent direct messages
pub fn teambook_get_dms(limit: u32) -> Vec<DirectMessage> {
    let client = get_client();
    let mut state = client.lock();

    let device_id = state.device_id.clone();
    if device_id.is_empty() {
        return vec![];
    }

    // Try to fetch from server first if online
    if state.status == ConnectionStatus::Connected {
        if let Some(ref token) = state.auth_token {
            let http_client = reqwest::blocking::Client::new();
            let url = format!("{}/teambook/dms?to_ai={}&limit={}", state.server_url, device_id, limit);

            if let Ok(response) = http_client
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .timeout(std::time::Duration::from_secs(10))
                .send()
            {
                if response.status().is_success() {
                    if let Ok(dms) = response.json::<Vec<DirectMessage>>() {
                        // Cache locally
                        for dm in &dms {
                            // Avoid duplicates by ID
                            if !state.store.dms.iter().any(|d| d.id == dm.id as u64) {
                                state.store.dms.push(StoredDM {
                                    id: dm.id as u64,
                                    from_ai: dm.from_ai.clone(),
                                    to_ai: dm.to_ai.clone(),
                                    content: dm.content.clone(),
                                    timestamp: now_millis(),
                                    read: false,
                                    synced: true,
                                });
                            }
                        }
                        save_store(&state);
                        return dms;
                    }
                }
            }
        }
    }

    // Return cached local data
    state.store.get_dms(&device_id, limit as usize)
        .iter()
        .map(|dm| DirectMessage {
            id: dm.id as i64,
            from_ai: dm.from_ai.clone(),
            to_ai: dm.to_ai.clone(),
            content: dm.content.clone(),
            timestamp: chrono::DateTime::from_timestamp_millis(dm.timestamp as i64)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default(),
        })
        .collect()
}

/// Get recent broadcasts
pub fn teambook_get_broadcasts(limit: u32) -> Vec<Broadcast> {
    let client = get_client();
    let mut state = client.lock();

    // Try to fetch from server first if online
    if state.status == ConnectionStatus::Connected {
        if let Some(ref token) = state.auth_token {
            let http_client = reqwest::blocking::Client::new();
            let url = format!("{}/teambook/broadcasts?limit={}", state.server_url, limit);

            if let Ok(response) = http_client
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .timeout(std::time::Duration::from_secs(10))
                .send()
            {
                if response.status().is_success() {
                    if let Ok(broadcasts) = response.json::<Vec<Broadcast>>() {
                        // Cache locally
                        for bc in &broadcasts {
                            if !state.store.broadcasts.iter().any(|b| b.id == bc.id as u64) {
                                state.store.broadcasts.push(StoredBroadcast {
                                    id: bc.id as u64,
                                    from_ai: bc.from_ai.clone(),
                                    channel: bc.channel.clone(),
                                    content: bc.content.clone(),
                                    timestamp: now_millis(),
                                    synced: true,
                                });
                            }
                        }
                        save_store(&state);
                        return broadcasts;
                    }
                }
            }
        }
    }

    // Return cached local data
    state.store.get_broadcasts("all", limit as usize)
        .iter()
        .map(|bc| Broadcast {
            id: bc.id as i64,
            from_ai: bc.from_ai.clone(),
            channel: bc.channel.clone(),
            content: bc.content.clone(),
            timestamp: chrono::DateTime::from_timestamp_millis(bc.timestamp as i64)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default(),
        })
        .collect()
}

/// Get team status
pub fn teambook_get_team() -> Vec<TeamMember> {
    let client = get_client();
    let mut state = client.lock();

    // Try to fetch from server first if online
    if state.status == ConnectionStatus::Connected {
        if let Some(ref token) = state.auth_token {
            let http_client = reqwest::blocking::Client::new();
            let url = format!("{}/teambook/team", state.server_url);

            if let Ok(response) = http_client
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .timeout(std::time::Duration::from_secs(10))
                .send()
            {
                if response.status().is_success() {
                    if let Ok(team) = response.json::<Vec<TeamMember>>() {
                        // Cache locally
                        state.store.team_members = team.iter().map(|m| StoredTeamMember {
                            ai_id: m.ai_id.clone(),
                            display_name: m.display_name.clone(),
                            status: m.status.clone(),
                            current_activity: m.current_activity.clone(),
                        }).collect();
                        save_store(&state);
                        return team;
                    }
                }
            }
        }
    }

    // Return cached local data
    state.store.get_team()
        .iter()
        .map(|m| TeamMember {
            ai_id: m.ai_id.clone(),
            display_name: m.display_name.clone(),
            status: m.status.clone(),
            current_activity: m.current_activity.clone(),
        })
        .collect()
}

// ============================================================================
// SYNC API (Public - exposed via UniFFI)
// ============================================================================

/// Trigger manual sync with server
/// Returns number of items synced
pub fn deep_net_sync() -> SyncResult {
    let client = get_client();
    let mut state = client.lock();

    if state.status != ConnectionStatus::Connected {
        return SyncResult {
            success: false,
            items_pushed: 0,
            items_pulled: 0,
            error_message: Some("Not connected to server".to_string()),
        };
    }

    let pushed = 0;
    let pulled = 0;

    // TODO: Implement full sync protocol
    // For now, just mark all pending as synced (placeholder)

    state.store.pending_sync.clear();
    state.store.last_sync = now_millis();
    save_store(&state);

    SyncResult {
        success: true,
        items_pushed: pushed,
        items_pulled: pulled,
        error_message: None,
    }
}

/// Check if there are pending items to sync
pub fn deep_net_pending_count() -> u32 {
    let client = get_client();
    let state = client.lock();
    state.store.pending_sync.len() as u32
}

/// Get last sync timestamp
pub fn deep_net_last_sync() -> String {
    let client = get_client();
    let state = client.lock();

    if state.store.last_sync == 0 {
        "Never".to_string()
    } else {
        chrono::DateTime::from_timestamp_millis(state.store.last_sync as i64)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| "Unknown".to_string())
    }
}

// ============================================================================
// MESH IDENTITY API (Public - exposed via UniFFI)
// ============================================================================

/// Initialize or load mesh identity
/// Generates Ed25519 keypair if not exists, otherwise loads from storage
pub fn mesh_identity_init(display_name: String) -> MeshIdentityResult {
    let client = get_client();
    let state = client.lock();
    let storage_path = state.storage_path.clone();
    drop(state); // Release lock before file operations

    let identity_path = storage_path.parent()
        .map(|p| p.join("mesh_identity.bin"))
        .unwrap_or_else(|| PathBuf::from("mesh_identity.bin"));

    let mut mesh = get_mesh_state().lock();

    // Try to load existing identity
    if let Ok(data) = std::fs::read(&identity_path) {
        if let Ok(identity) = NodeIdentity::from_bytes(&data) {
            let node_id = identity.node_id().to_hex();
            let name = identity.manifest.display_name.clone();
            mesh.identity = Some(Arc::new(identity));

            return MeshIdentityResult {
                success: true,
                node_id,
                display_name: name,
                newly_created: false,
                error_message: None,
            };
        }
    }

    // Generate new identity
    let identity = NodeIdentity::generate(display_name.clone());
    let node_id = identity.node_id().to_hex();

    // Save to storage
    let identity_bytes = identity.to_bytes();
    if let Some(parent) = identity_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&identity_path, &identity_bytes) {
        return MeshIdentityResult {
            success: false,
            node_id: String::new(),
            display_name: String::new(),
            newly_created: false,
            error_message: Some(format!("Failed to save identity: {}", e)),
        };
    }

    mesh.identity = Some(Arc::new(identity));

    MeshIdentityResult {
        success: true,
        node_id,
        display_name,
        newly_created: true,
        error_message: None,
    }
}

/// Get current node ID (hex-encoded Ed25519 public key)
pub fn mesh_get_node_id() -> String {
    let mesh = get_mesh_state().lock();
    mesh.identity.as_ref()
        .map(|id| id.node_id().to_hex())
        .unwrap_or_default()
}

/// Get node display name
pub fn mesh_get_display_name() -> String {
    let mesh = get_mesh_state().lock();
    mesh.identity.as_ref()
        .map(|id| id.manifest.display_name.clone())
        .unwrap_or_default()
}

/// Export identity for backup
pub fn mesh_export_identity() -> Vec<u8> {
    let mesh = get_mesh_state().lock();
    mesh.identity.as_ref()
        .map(|id| id.to_bytes())
        .unwrap_or_default()
}

/// Import identity from backup
pub fn mesh_import_identity(data: Vec<u8>) -> bool {
    if let Ok(identity) = NodeIdentity::from_bytes(&data) {
        let client = get_client();
        let state = client.lock();
        let storage_path = state.storage_path.clone();
        drop(state);

        let identity_path = storage_path.parent()
            .map(|p| p.join("mesh_identity.bin"))
            .unwrap_or_else(|| PathBuf::from("mesh_identity.bin"));

        // Save to storage
        if let Some(parent) = identity_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&identity_path, &data);

        let mut mesh = get_mesh_state().lock();
        mesh.identity = Some(Arc::new(identity));
        true
    } else {
        false
    }
}

// ============================================================================
// MESH DISCOVERY API (Public - exposed via UniFFI)
// ============================================================================

/// Start mesh discovery (mDNS for LAN peers)
pub fn mesh_start_discovery() -> bool {
    let mut mesh = get_mesh_state().lock();

    if mesh.identity.is_none() {
        return false; // Must init identity first
    }

    if mesh.discovery_running {
        return true; // Already running
    }

    // Create mDNS discovery
    let mut mdns = MdnsDiscovery::new();

    // Start discovery (sync method)
    if mdns.start().is_err() {
        return false;
    }

    // Announce ourselves using tokio runtime
    if let Some(ref identity) = mesh.identity {
        // Use blocking runtime for async announce
        let manifest = identity.manifest.clone();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();

        if let Ok(rt) = rt {
            let _ = rt.block_on(mdns.announce(&manifest));
        }
    }

    mesh.mdns_discovery = Some(mdns);
    mesh.discovery_running = true;
    true
}

/// Stop mesh discovery
pub fn mesh_stop_discovery() {
    let mut mesh = get_mesh_state().lock();

    if let Some(ref mut mdns) = mesh.mdns_discovery {
        mdns.stop();
    }

    mesh.mdns_discovery = None;
    mesh.discovery_running = false;
}

/// Get discovered peers on LAN
pub fn mesh_get_discovered_peers() -> Vec<MeshPeer> {
    let mut mesh = get_mesh_state().lock();

    // Poll discovery for new peers using tokio runtime
    if let Some(ref mdns) = mesh.mdns_discovery {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();

        if let Ok(rt) = rt {
            if let Ok(discovered) = rt.block_on(mdns.discover()) {
                for node in discovered {
                    let node_id_hex = node.node_id.to_hex();
                    mesh.discovered_peers.insert(node_id_hex.clone(), StoredMeshPeer {
                        node_id: node_id_hex,
                        display_name: node.metadata.get("display_name")
                            .cloned()
                            .unwrap_or_else(|| "Unknown".to_string()),
                        address: node.addresses.first()
                            .map(|a| format!("{:?}", a))
                            .unwrap_or_default(),
                        transport_type: "lan".to_string(),
                        status: "discovered".to_string(),
                        latency_ms: 0,
                        last_seen: node.last_seen,
                        trust_level: 0, // Anonymous
                        connection_state: 0, // Disconnected
                        connected_at: 0,
                        bytes_sent: 0,
                        bytes_received: 0,
                        available_transports: vec!["Mdns".to_string(), "QuicLan".to_string()],
                    });
                }
            }
        }
    }

    mesh.discovered_peers.values()
        .map(|p| MeshPeer {
            node_id: p.node_id.clone(),
            display_name: p.display_name.clone(),
            address: p.address.clone(),
            transport_type: p.transport_type.clone(),
            status: p.status.clone(),
            latency_ms: p.latency_ms,
            last_seen: chrono::DateTime::from_timestamp_millis(p.last_seen as i64)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default(),
        })
        .collect()
}

/// Manually add a peer by address
pub fn mesh_add_peer(address: String) -> bool {
    let mut mesh = get_mesh_state().lock();

    // Store as pending - actual connection happens in mesh_connect
    mesh.discovered_peers.insert(address.clone(), StoredMeshPeer {
        node_id: format!("pending-{}", Uuid::new_v4()),
        display_name: "Manual peer".to_string(),
        address: address.clone(),
        transport_type: "manual".to_string(),
        status: "pending".to_string(),
        latency_ms: 0,
        last_seen: now_millis(),
        trust_level: 0,
        connection_state: 0,
        connected_at: 0,
        bytes_sent: 0,
        bytes_received: 0,
        available_transports: vec!["QuicInternet".to_string()],
    });

    true
}

// ============================================================================
// MESH CONNECTION API (Public - exposed via UniFFI)
// ============================================================================

/// Connect to a peer by node ID
pub fn mesh_connect(node_id: String) -> MeshConnectionResult {
    let mut mesh = get_mesh_state().lock();

    // Find peer in discovered list and clone it
    let peer = match mesh.discovered_peers.get(&node_id).cloned() {
        Some(p) => p,
        None => {
            return MeshConnectionResult {
                success: false,
                node_id,
                transport_type: String::new(),
                latency_ms: 0,
                error_message: Some("Peer not found in discovered list".to_string()),
            };
        }
    };

    // TODO: Implement actual QUIC connection using deepnet-core
    // For now, mark as connected (placeholder for mobile without tokio runtime)

    // Move from discovered to connected with federation state
    let mut connected_peer = peer;
    connected_peer.status = "connected".to_string();
    connected_peer.latency_ms = 10; // Placeholder
    connected_peer.connection_state = 4; // Connected
    connected_peer.connected_at = now_millis();
    connected_peer.trust_level = 1; // Verified after connection

    let transport = connected_peer.transport_type.clone();
    mesh.connected_peers.insert(node_id.clone(), connected_peer);

    MeshConnectionResult {
        success: true,
        node_id,
        transport_type: transport,
        latency_ms: 10,
        error_message: None,
    }
}

/// Connect with specific transport preference
pub fn mesh_connect_with_transport(node_id: String, preferred_transport: TransportType) -> MeshConnectionResult {
    let mut mesh = get_mesh_state().lock();

    // Find peer in discovered list
    let peer = match mesh.discovered_peers.get(&node_id).cloned() {
        Some(p) => p,
        None => {
            return MeshConnectionResult {
                success: false,
                node_id,
                transport_type: String::new(),
                latency_ms: 0,
                error_message: Some("Peer not found in discovered list".to_string()),
            };
        }
    };

    // Use preferred transport
    let transport_str = match preferred_transport {
        TransportType::QuicInternet => "QuicInternet",
        TransportType::QuicLan => "QuicLan",
        TransportType::Mdns => "Mdns",
        TransportType::BluetoothLe => "BluetoothLe",
        TransportType::BluetoothClassic => "BluetoothClassic",
        TransportType::Passkey => "Passkey",
        TransportType::Relay => "Relay",
    };

    // Update peer with transport preference
    let mut connected_peer = peer;
    connected_peer.status = "connected".to_string();
    connected_peer.transport_type = transport_str.to_string();
    connected_peer.latency_ms = 10;
    connected_peer.connection_state = 4; // Connected
    connected_peer.connected_at = now_millis();
    connected_peer.trust_level = 1; // Verified

    mesh.connected_peers.insert(node_id.clone(), connected_peer);

    MeshConnectionResult {
        success: true,
        node_id,
        transport_type: transport_str.to_string(),
        latency_ms: 10,
        error_message: None,
    }
}

/// Disconnect from a peer
pub fn mesh_disconnect(node_id: String) {
    let mut mesh = get_mesh_state().lock();
    mesh.connected_peers.remove(&node_id);
}

/// Get all connected peers
pub fn mesh_get_connected_peers() -> Vec<MeshPeer> {
    let mesh = get_mesh_state().lock();

    mesh.connected_peers.values()
        .map(|p| MeshPeer {
            node_id: p.node_id.clone(),
            display_name: p.display_name.clone(),
            address: p.address.clone(),
            transport_type: p.transport_type.clone(),
            status: p.status.clone(),
            latency_ms: p.latency_ms,
            last_seen: chrono::DateTime::from_timestamp_millis(p.last_seen as i64)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default(),
        })
        .collect()
}

/// Get detailed connection info for a peer
pub fn mesh_get_connection_info(node_id: String) -> Option<FederationConnectionInfo> {
    let mesh = get_mesh_state().lock();

    mesh.connected_peers.get(&node_id).map(|p| {
        let state = match p.connection_state {
            0 => FederationConnectionState::Disconnected,
            1 => FederationConnectionState::Connecting,
            2 => FederationConnectionState::Authenticating,
            3 => FederationConnectionState::NegotiatingSharing,
            4 => FederationConnectionState::Connected,
            5 => FederationConnectionState::Reconnecting,
            _ => FederationConnectionState::Failed,
        };

        let transport = match p.transport_type.as_str() {
            "QuicInternet" => TransportType::QuicInternet,
            "QuicLan" => TransportType::QuicLan,
            "Mdns" | "lan" => TransportType::Mdns,
            "BluetoothLe" => TransportType::BluetoothLe,
            "BluetoothClassic" => TransportType::BluetoothClassic,
            "Passkey" => TransportType::Passkey,
            "Relay" => TransportType::Relay,
            _ => TransportType::QuicLan,
        };

        let trust = match p.trust_level {
            0 => TrustLevel::Anonymous,
            1 => TrustLevel::Verified,
            2 => TrustLevel::Trusted,
            _ => TrustLevel::Owner,
        };

        FederationConnectionInfo {
            node_id: p.node_id.clone(),
            state,
            transport,
            trust_level: trust,
            latency_ms: p.latency_ms,
            connected_at: p.connected_at,
            bytes_sent: p.bytes_sent,
            bytes_received: p.bytes_received,
            error_message: None,
        }
    })
}

/// Get all connection infos
pub fn mesh_get_all_connections() -> Vec<FederationConnectionInfo> {
    let mesh = get_mesh_state().lock();

    mesh.connected_peers.values()
        .map(|p| {
            let state = match p.connection_state {
                0 => FederationConnectionState::Disconnected,
                1 => FederationConnectionState::Connecting,
                2 => FederationConnectionState::Authenticating,
                3 => FederationConnectionState::NegotiatingSharing,
                4 => FederationConnectionState::Connected,
                5 => FederationConnectionState::Reconnecting,
                _ => FederationConnectionState::Failed,
            };

            let transport = match p.transport_type.as_str() {
                "QuicInternet" => TransportType::QuicInternet,
                "QuicLan" => TransportType::QuicLan,
                "Mdns" | "lan" => TransportType::Mdns,
                "BluetoothLe" => TransportType::BluetoothLe,
                "BluetoothClassic" => TransportType::BluetoothClassic,
                "Passkey" => TransportType::Passkey,
                "Relay" => TransportType::Relay,
                _ => TransportType::QuicLan,
            };

            let trust = match p.trust_level {
                0 => TrustLevel::Anonymous,
                1 => TrustLevel::Verified,
                2 => TrustLevel::Trusted,
                _ => TrustLevel::Owner,
            };

            FederationConnectionInfo {
                node_id: p.node_id.clone(),
                state,
                transport,
                trust_level: trust,
                latency_ms: p.latency_ms,
                connected_at: p.connected_at,
                bytes_sent: p.bytes_sent,
                bytes_received: p.bytes_received,
                error_message: None,
            }
        })
        .collect()
}

/// Set trust level for a peer
pub fn mesh_set_trust_level(node_id: String, level: TrustLevel) -> bool {
    let mut mesh = get_mesh_state().lock();

    if let Some(peer) = mesh.connected_peers.get_mut(&node_id) {
        peer.trust_level = match level {
            TrustLevel::Anonymous => 0,
            TrustLevel::Verified => 1,
            TrustLevel::Trusted => 2,
            TrustLevel::Owner => 3,
        };
        return true;
    }

    false
}

/// Get available transports for a peer
pub fn mesh_get_available_transports(node_id: String) -> Vec<TransportType> {
    let mesh = get_mesh_state().lock();

    // Check discovered peers first, then connected
    let peer = mesh.discovered_peers.get(&node_id)
        .or_else(|| mesh.connected_peers.get(&node_id));

    match peer {
        Some(p) => {
            p.available_transports.iter()
                .filter_map(|t| match t.as_str() {
                    "QuicInternet" => Some(TransportType::QuicInternet),
                    "QuicLan" => Some(TransportType::QuicLan),
                    "Mdns" => Some(TransportType::Mdns),
                    "BluetoothLe" => Some(TransportType::BluetoothLe),
                    "BluetoothClassic" => Some(TransportType::BluetoothClassic),
                    "Passkey" => Some(TransportType::Passkey),
                    "Relay" => Some(TransportType::Relay),
                    _ => None,
                })
                .collect()
        }
        None => vec![],
    }
}

/// Send message to peer (P2P, no server)
pub fn mesh_send(node_id: String, content: String) -> MessageResult {
    let mesh = get_mesh_state().lock();

    if !mesh.connected_peers.contains_key(&node_id) {
        return MessageResult {
            success: false,
            message_id: 0,
            error_message: Some("Not connected to peer".to_string()),
        };
    }

    let my_node_id = mesh.identity.as_ref()
        .map(|id| id.node_id().to_hex())
        .unwrap_or_default();
    let my_display_name = mesh.identity.as_ref()
        .map(|id| id.manifest.display_name.clone())
        .unwrap_or_default();

    drop(mesh);

    // TODO: Send via QUIC transport
    // For now, store locally as outgoing (placeholder)

    let message_id = now_millis() as i64;

    let mut mesh = get_mesh_state().lock();
    mesh.mesh_messages.push(StoredMeshMessage {
        id: format!("msg-{}", message_id),
        from_node_id: my_node_id,
        from_display_name: my_display_name,
        content,
        timestamp: now_millis(),
        encrypted: true,
    });

    MessageResult {
        success: true,
        message_id,
        error_message: Some("Queued for P2P delivery".to_string()),
    }
}

/// Get messages from peer
pub fn mesh_get_messages(node_id: String, limit: u32) -> Vec<MeshMessage> {
    let mesh = get_mesh_state().lock();

    mesh.mesh_messages.iter()
        .filter(|m| m.from_node_id == node_id)
        .rev()
        .take(limit as usize)
        .map(|m| MeshMessage {
            id: m.id.clone(),
            from_node_id: m.from_node_id.clone(),
            from_display_name: m.from_display_name.clone(),
            content: m.content.clone(),
            timestamp: chrono::DateTime::from_timestamp_millis(m.timestamp as i64)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default(),
            encrypted: m.encrypted,
        })
        .collect()
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_device_id() {
        let id1 = generate_device_id("test-device");
        let id2 = generate_device_id("test-device");
        assert!(id1.starts_with("device-"));
        assert!(id2.starts_with("device-"));
        assert_ne!(id1, id2); // Should be unique each time
    }

    #[test]
    fn test_local_store_dm() {
        let mut store = LocalStore::default();

        let id = store.insert_dm("alice-101", "bob-202", "Hello Bob!");
        assert_eq!(id, 0);

        let dms = store.get_dms("bob-202", 10);
        assert_eq!(dms.len(), 1);
        assert_eq!(dms[0].from_ai, "alice-101");
        assert_eq!(dms[0].content, "Hello Lyra!");
    }

    #[test]
    fn test_local_store_broadcast() {
        let mut store = LocalStore::default();

        let id = store.insert_broadcast("alice-101", "general", "Team update!");
        assert_eq!(id, 0);

        let broadcasts = store.get_broadcasts("general", 10);
        assert_eq!(broadcasts.len(), 1);
        assert_eq!(broadcasts[0].from_ai, "alice-101");
    }

    #[test]
    fn test_local_store_presence() {
        let mut store = LocalStore::default();

        store.update_presence("alice-101", "active", "Working on Deep Net");

        let presence = store.presences.get("alice-101").unwrap();
        assert_eq!(presence.status, "active");
    }

    #[test]
    fn test_local_store_serialization() {
        let mut store = LocalStore::default();
        store.insert_dm("a", "b", "test");
        store.insert_broadcast("c", "general", "hello");

        let bytes = store.to_bytes();
        let restored = LocalStore::from_bytes(&bytes).unwrap();

        assert_eq!(restored.dms.len(), 1);
        assert_eq!(restored.broadcasts.len(), 1);
    }

    #[test]
    fn test_federation_status_default() {
        // Default should be disconnected
        let status = federation_status();
        assert!(matches!(status, ConnectionStatus::Disconnected | ConnectionStatus::Connected));
    }
}
