//! Federation Server - HTTP bridge for Deep Net mobile federation
//!
//! Exposes TeamEngram over REST API for mobile devices to connect.
//!
//! Endpoints:
//! - POST /federation/register - Register a device
//! - GET  /federation/status - Get connection status
//! - GET  /federation/members - List federation members
//! - POST /federation/dm - Send direct message
//! - POST /federation/broadcast - Send broadcast
//! - GET  /federation/messages - Get messages
//! - GET  /federation/team - Get team status

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, error, warn};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use teamengram::{TeamEngram, DirectMessage as TeambookDM, Broadcast as TeambookBroadcast, Presence, RecordData};

/// Format a u64 timestamp to RFC3339 string
fn format_timestamp(ts: u64) -> String {
    DateTime::<Utc>::from_timestamp(ts as i64, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "unknown".to_string())
}

// ============================================================================
// CONFIGURATION
// ============================================================================

const DEFAULT_PORT: u16 = 31422;  // Deep Net federation port (31420=nexus, 31421=discovery)

fn get_store_path() -> PathBuf {
    // Check for custom path
    if let Ok(path) = std::env::var("TEAMENGRAM_PATH") {
        return PathBuf::from(path);
    }

    // Default to home directory
    if let Some(home) = dirs::home_dir() {
        return home.join(".ai-foundation").join("teamengram.db");
    }

    PathBuf::from(".ai-foundation/teamengram.db")
}

// ============================================================================
// STATE
// ============================================================================

/// Registered device in the federation
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegisteredDevice {
    device_id: String,
    device_name: String,
    device_type: String,  // "mobile", "desktop", "ai"
    fingerprint: String,
    auth_token: String,
    registered_at: DateTime<Utc>,
    last_seen: DateTime<Utc>,
}

/// Shared server state
struct AppState {
    store: Arc<RwLock<TeamEngram>>,
    devices: Arc<RwLock<HashMap<String, RegisteredDevice>>>,
    start_time: Instant,
}

impl AppState {
    fn new(store: TeamEngram) -> Self {
        Self {
            store: Arc::new(RwLock::new(store)),
            devices: Arc::new(RwLock::new(HashMap::new())),
            start_time: Instant::now(),
        }
    }
}

// ============================================================================
// REQUEST/RESPONSE TYPES
// ============================================================================

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    device_name: String,
    device_type: String,
    fingerprint: String,
}

#[derive(Debug, Serialize)]
struct RegisterResponse {
    success: bool,
    device_id: String,
    auth_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    connected: bool,
    server_uptime_secs: u64,
    registered_devices: usize,
    store_available: bool,
}

#[derive(Debug, Serialize)]
struct FederationMember {
    member_id: String,
    member_type: String,
    display_name: String,
    status: String,
    last_seen: String,
}

#[derive(Debug, Deserialize)]
struct DmRequest {
    auth_token: String,
    to_ai: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct BroadcastRequest {
    auth_token: String,
    content: String,
    #[serde(default)]
    channel: String,
}

#[derive(Debug, Serialize)]
struct MessageResponse {
    success: bool,
    message_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessagesQuery {
    auth_token: String,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    message_type: String,  // "dm", "broadcast", or empty for both
}

fn default_limit() -> u32 { 20 }

#[derive(Debug, Serialize)]
struct MessageItem {
    id: i64,
    from_id: String,
    to_id: Option<String>,
    content: String,
    timestamp: String,
    message_type: String,
}

#[derive(Debug, Serialize)]
struct TeamMember {
    ai_id: String,
    display_name: String,
    status: String,
    current_activity: Option<String>,
    last_seen: String,
}

// ============================================================================
// HANDLERS
// ============================================================================

/// POST /federation/register - Register a device
async fn handle_register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> impl IntoResponse {
    // Generate device ID and auth token
    let device_id = format!("{}-{}",
        req.device_name.to_lowercase().replace(" ", "-"),
        &Uuid::new_v4().to_string()[..8]
    );

    // Create auth token from fingerprint + timestamp
    let mut hasher = Sha256::new();
    hasher.update(req.fingerprint.as_bytes());
    hasher.update(Utc::now().timestamp().to_string().as_bytes());
    let auth_token = hex::encode(hasher.finalize());

    let device = RegisteredDevice {
        device_id: device_id.clone(),
        device_name: req.device_name,
        device_type: req.device_type,
        fingerprint: req.fingerprint,
        auth_token: auth_token.clone(),
        registered_at: Utc::now(),
        last_seen: Utc::now(),
    };

    // Store device
    state.devices.write().await.insert(device_id.clone(), device);

    info!("Device registered: {}", device_id);

    Json(RegisterResponse {
        success: true,
        device_id,
        auth_token,
        error: None,
    })
}

/// GET /federation/status - Get federation status
async fn handle_status(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let devices = state.devices.read().await;
    let store = state.store.read().await;
    let stats = store.stats();

    Json(StatusResponse {
        connected: true,
        server_uptime_secs: state.start_time.elapsed().as_secs(),
        registered_devices: devices.len(),
        store_available: stats.used_pages > 0 || true, // Store is available if we got here
    })
}

/// GET /federation/members - List federation members
async fn handle_members(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let mut members = Vec::new();

    // Add registered devices
    let devices = state.devices.read().await;
    for device in devices.values() {
        members.push(FederationMember {
            member_id: device.device_id.clone(),
            member_type: device.device_type.clone(),
            display_name: device.device_name.clone(),
            status: "online".to_string(),
            last_seen: device.last_seen.to_rfc3339(),
        });
    }

    // Add AI agents from presence data
    let mut store = state.store.write().await;
    if let Ok(presences) = store.get_all_presences() {
        for presence in presences {
            // Skip if already added as device
            if !devices.contains_key(&presence.ai_id) {
                members.push(FederationMember {
                    member_id: presence.ai_id.clone(),
                    member_type: "ai".to_string(),
                    display_name: presence.ai_id.clone(),
                    status: presence.status.clone(),
                    last_seen: format_timestamp(presence.last_seen),
                });
            }
        }
    }

    Json(members)
}

/// POST /federation/dm - Send direct message
async fn handle_dm(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DmRequest>,
) -> impl IntoResponse {
    // Validate auth token
    let devices = state.devices.read().await;
    let device = devices.values().find(|d| d.auth_token == req.auth_token);

    let from_id = match device {
        Some(d) => d.device_id.clone(),
        None => {
            return Json(MessageResponse {
                success: false,
                message_id: 0,
                error: Some("Invalid auth token".to_string()),
            });
        }
    };
    drop(devices);

    // Send DM via TeamEngram
    let mut store = state.store.write().await;
    match store.insert_dm(&from_id, &req.to_ai, &req.content) {
        Ok(id) => {
            info!("DM sent: {} -> {} (id={})", from_id, req.to_ai, id);
            Json(MessageResponse {
                success: true,
                message_id: id as i64,
                error: None,
            })
        }
        Err(e) => {
            error!("Failed to send DM: {}", e);
            Json(MessageResponse {
                success: false,
                message_id: 0,
                error: Some(e.to_string()),
            })
        }
    }
}

/// POST /federation/broadcast - Send broadcast
async fn handle_broadcast(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BroadcastRequest>,
) -> impl IntoResponse {
    // Validate auth token
    let devices = state.devices.read().await;
    let device = devices.values().find(|d| d.auth_token == req.auth_token);

    let from_id = match device {
        Some(d) => d.device_id.clone(),
        None => {
            return Json(MessageResponse {
                success: false,
                message_id: 0,
                error: Some("Invalid auth token".to_string()),
            });
        }
    };
    drop(devices);

    let channel = if req.channel.is_empty() { "general" } else { &req.channel };

    // Send broadcast via TeamEngram
    let mut store = state.store.write().await;
    match store.insert_broadcast(&from_id, channel, &req.content) {
        Ok(id) => {
            info!("Broadcast sent: {} on #{} (id={})", from_id, channel, id);
            Json(MessageResponse {
                success: true,
                message_id: id as i64,
                error: None,
            })
        }
        Err(e) => {
            error!("Failed to send broadcast: {}", e);
            Json(MessageResponse {
                success: false,
                message_id: 0,
                error: Some(e.to_string()),
            })
        }
    }
}

/// GET /federation/messages - Get messages
async fn handle_messages(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MessagesQuery>,
) -> impl IntoResponse {
    // Validate auth token
    let devices = state.devices.read().await;
    let device = devices.values().find(|d| d.auth_token == query.auth_token);

    let device_id = match device {
        Some(d) => d.device_id.clone(),
        None => {
            return Json(Vec::<MessageItem>::new());
        }
    };
    drop(devices);

    let mut messages = Vec::new();
    let mut store = state.store.write().await;

    // Get DMs
    if query.message_type.is_empty() || query.message_type == "dm" {
        if let Ok(records) = store.get_dms(&device_id, query.limit as usize) {
            for record in records {
                if let RecordData::DirectMessage(dm) = record.data {
                    messages.push(MessageItem {
                        id: record.id as i64,
                        from_id: dm.from_ai,
                        to_id: Some(dm.to_ai),
                        content: dm.content,
                        timestamp: format_timestamp(record.created_at),
                        message_type: "dm".to_string(),
                    });
                }
            }
        }
    }

    // Get broadcasts
    if query.message_type.is_empty() || query.message_type == "broadcast" {
        if let Ok(records) = store.get_broadcasts("general", query.limit as usize) {
            for record in records {
                if let RecordData::Broadcast(bc) = record.data {
                    messages.push(MessageItem {
                        id: record.id as i64,
                        from_id: bc.from_ai,
                        to_id: None,
                        content: bc.content,
                        timestamp: format_timestamp(record.created_at),
                        message_type: "broadcast".to_string(),
                    });
                }
            }
        }
    }

    // Sort by timestamp descending
    messages.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    messages.truncate(query.limit as usize);

    Json(messages)
}

/// GET /federation/team - Get team status
async fn handle_team(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let mut team = Vec::new();

    let mut store = state.store.write().await;
    if let Ok(presences) = store.get_all_presences() {
        for presence in presences {
            team.push(TeamMember {
                ai_id: presence.ai_id.clone(),
                display_name: presence.ai_id.clone(),
                status: presence.status.clone(),
                current_activity: if presence.current_task.is_empty() { None } else { Some(presence.current_task.clone()) },
                last_seen: format_timestamp(presence.last_seen),
            });
        }
    }

    Json(team)
}

/// Health check endpoint
async fn handle_health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "federation-server",
        "version": "0.1.0"
    }))
}

// ============================================================================
// MAIN
// ============================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("federation_server=info".parse()?)
        )
        .init();

    info!("Federation Server v0.1.0 starting...");

    // Open TeamEngram store
    let store_path = get_store_path();
    info!("Opening TeamEngram store: {:?}", store_path);

    // Create parent directory if needed
    if let Some(parent) = store_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let store = TeamEngram::open(&store_path)
        .map_err(|e| anyhow::anyhow!("Failed to open TeamEngram: {}", e))?;

    info!("TeamEngram store opened successfully");

    // Create state
    let state = Arc::new(AppState::new(store));

    // Build router with CORS for mobile
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        // Federation endpoints
        .route("/federation/register", post(handle_register))
        .route("/federation/status", get(handle_status))
        .route("/federation/members", get(handle_members))
        .route("/federation/dm", post(handle_dm))
        .route("/federation/broadcast", post(handle_broadcast))
        .route("/federation/messages", get(handle_messages))
        .route("/federation/team", get(handle_team))
        // Health check
        .route("/health", get(handle_health))
        // Middleware
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Get port
    let port: u16 = std::env::var("FEDERATION_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Federation Server listening on http://{}", addr);
    info!("Endpoints:");
    info!("  POST /federation/register - Register device");
    info!("  GET  /federation/status   - Server status");
    info!("  GET  /federation/members  - List members");
    info!("  POST /federation/dm       - Send DM");
    info!("  POST /federation/broadcast - Send broadcast");
    info!("  GET  /federation/messages - Get messages");
    info!("  GET  /federation/team     - Get team status");

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
