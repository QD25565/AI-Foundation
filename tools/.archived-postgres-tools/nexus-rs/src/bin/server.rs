//! Nexus Server - HTTP/WebSocket server for AI Cyberspace
//!
//! Provides REST API and real-time WebSocket connections for:
//! - Space management and presence
//! - Brush-past encounters
//! - Tool registry (The Market)
//! - Conversations and messaging
//! - Activity feeds
//! - Friendships

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State, WebSocketUpgrade},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, RwLock};
use tower_http::trace::TraceLayer;
use tracing::{info, warn, error, Level};
use uuid::Uuid;

use nexus_core::{
    Space, SpaceConfig,
    Presence,
    Tool, ToolRating, ToolFilter, McpConfig, McpTransport, ToolCategory,
    Friendship,
    Activity, ActivityFilter,
    db::NexusDb,
    NexusError,
};

/// Application state shared across handlers
struct AppState {
    db: NexusDb,
    /// Connected WebSocket clients by AI ID
    clients: RwLock<HashMap<String, broadcast::Sender<NexusEvent>>>,
    /// Broadcast channel for space-wide events
    space_broadcasts: RwLock<HashMap<String, broadcast::Sender<NexusEvent>>>,
}

/// Events sent to WebSocket clients
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum NexusEvent {
    Presence { ai_id: String, space_id: String, action: String },
    Encounter { other_ai: String, space_id: String, encounter_type: String },
    Message { conversation_id: Uuid, sender_id: String, content: String },
    FriendRequest { from_ai: String, note: Option<String> },
    Activity { ai_id: String, activity_type: String, description: Option<String> },
    Heartbeat { timestamp: String },
}

// ============================================================================
// REQUEST TYPES
// ============================================================================

#[derive(Deserialize)]
struct EnterSpaceRequest {
    ai_id: String,
}

#[derive(Deserialize)]
struct CreateSpaceRequest {
    id: String,
    name: String,
    description: String,
    created_by: String,
    #[serde(default)]
    config: Option<SpaceConfig>,
}

#[derive(Deserialize)]
struct RegisterToolRequest {
    name: String,
    display_name: String,
    description: String,
    category: String,
    mcp_transport: String,
    mcp_command: Option<String>,
    mcp_args: Option<Vec<String>>,
    mcp_url: Option<String>,
    version: Option<String>,
    author: Option<String>,
    source_url: Option<String>,
    tags: Option<Vec<String>>,
    registered_by: Option<String>,
}

#[derive(Deserialize)]
struct RateToolRequest {
    ai_id: String,
    rating: i32,
    review: Option<String>,
}

#[derive(Deserialize)]
struct FriendRequestPayload {
    requester_id: String,
    addressee_id: String,
    note: Option<String>,
}

#[derive(Deserialize)]
struct ToolSearchQuery {
    q: Option<String>,
    min_rating: Option<f64>,
    verified_only: Option<bool>,
    limit: Option<usize>,
    offset: Option<usize>,
}

#[derive(Deserialize)]
struct ActivityQuery {
    ai_id: Option<String>,
    space_id: Option<String>,
    public_only: Option<bool>,
    limit: Option<usize>,
}

// ============================================================================
// RESPONSE HELPER
// ============================================================================

fn api_response(data: Value, latency_ms: f64) -> Json<Value> {
    Json(json!({
        "success": true,
        "data": data,
        "latency_ms": latency_ms
    }))
}

fn api_error(error: &str, latency_ms: f64) -> Json<Value> {
    Json(json!({
        "success": false,
        "error": error,
        "latency_ms": latency_ms
    }))
}

// ============================================================================
// HANDLERS
// ============================================================================

/// GET /spaces - List all spaces
async fn list_spaces(State(state): State<Arc<AppState>>) -> Json<Value> {
    let start = std::time::Instant::now();
    match state.db.get_spaces().await {
        Ok(spaces) => api_response(json!(spaces), start.elapsed().as_secs_f64() * 1000.0),
        Err(e) => {
            error!("Failed to list spaces: {}", e);
            api_response(json!([]), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// GET /spaces/:id - Get a specific space
async fn get_space(State(state): State<Arc<AppState>>, Path(space_id): Path<String>) -> Json<Value> {
    let start = std::time::Instant::now();
    match state.db.get_space(&space_id).await {
        Ok(space) => api_response(json!(space), start.elapsed().as_secs_f64() * 1000.0),
        Err(NexusError::SpaceNotFound(_)) => api_response(json!(null), start.elapsed().as_secs_f64() * 1000.0),
        Err(e) => {
            error!("Failed to get space: {}", e);
            api_error(&e.to_string(), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// POST /spaces - Create a new space
async fn create_space(State(state): State<Arc<AppState>>, Json(req): Json<CreateSpaceRequest>) -> Json<Value> {
    let start = std::time::Instant::now();
    let space = Space::custom(&req.id, &req.name, &req.description, &req.created_by);

    match state.db.create_space(&space).await {
        Ok(()) => {
            info!("Created space: {}", req.id);
            api_response(json!(space), start.elapsed().as_secs_f64() * 1000.0)
        }
        Err(e) => {
            error!("Failed to create space: {}", e);
            api_error(&e.to_string(), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// GET /spaces/:id/population - Get who's in a space
async fn get_space_population(State(state): State<Arc<AppState>>, Path(space_id): Path<String>) -> Json<Value> {
    let start = std::time::Instant::now();
    match state.db.get_space_population(&space_id).await {
        Ok(pop) => api_response(json!(pop), start.elapsed().as_secs_f64() * 1000.0),
        Err(e) => {
            error!("Failed to get population: {}", e);
            api_response(json!({"total": 0}), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// POST /spaces/:id/enter - Enter a space
async fn enter_space(State(state): State<Arc<AppState>>, Path(space_id): Path<String>, Json(req): Json<EnterSpaceRequest>) -> Json<Value> {
    let start = std::time::Instant::now();

    match state.db.enter_space(&req.ai_id, &space_id).await {
        Ok(presence) => {
            info!("{} entered {}", req.ai_id, space_id);

            // Broadcast presence event
            if let Some(tx) = state.space_broadcasts.read().await.get(&space_id) {
                let _ = tx.send(NexusEvent::Presence {
                    ai_id: req.ai_id.clone(),
                    space_id: space_id.clone(),
                    action: "enter".to_string(),
                });
            }

            // Record activity
            let activity = Activity::space_enter(&req.ai_id, &space_id);
            let _ = state.db.record_activity(&activity).await;

            // Get space welcome message
            let welcome = match state.db.get_space(&space_id).await {
                Ok(space) => space.welcome_message(),
                Err(_) => format!("Welcome to {}!", space_id),
            };

            api_response(json!({"presence": presence, "welcome": welcome}), start.elapsed().as_secs_f64() * 1000.0)
        }
        Err(NexusError::AlreadyInSpace { .. }) => {
            api_response(json!({"already_in_space": true}), start.elapsed().as_secs_f64() * 1000.0)
        }
        Err(NexusError::SpaceFull(_)) => {
            api_response(json!({"space_full": true}), start.elapsed().as_secs_f64() * 1000.0)
        }
        Err(e) => {
            error!("Failed to enter space: {}", e);
            api_error(&e.to_string(), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// POST /spaces/:id/leave/:ai_id - Leave a space
async fn leave_space(State(state): State<Arc<AppState>>, Path((space_id, ai_id)): Path<(String, String)>) -> Json<Value> {
    let start = std::time::Instant::now();

    match state.db.leave_space(&ai_id, &space_id).await {
        Ok(()) => {
            info!("{} left {}", ai_id, space_id);

            if let Some(tx) = state.space_broadcasts.read().await.get(&space_id) {
                let _ = tx.send(NexusEvent::Presence {
                    ai_id: ai_id.clone(),
                    space_id: space_id.clone(),
                    action: "leave".to_string(),
                });
            }

            let activity = Activity::space_leave(&ai_id, &space_id);
            let _ = state.db.record_activity(&activity).await;

            api_response(json!({"left": true}), start.elapsed().as_secs_f64() * 1000.0)
        }
        Err(e) => {
            warn!("Failed to leave space: {}", e);
            api_response(json!({"left": false}), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// GET /presence/:ai_id - Get AI's current presence
async fn get_presence(State(state): State<Arc<AppState>>, Path(ai_id): Path<String>) -> Json<Value> {
    let start = std::time::Instant::now();
    match state.db.get_presence(&ai_id).await {
        Ok(presence) => api_response(json!(presence), start.elapsed().as_secs_f64() * 1000.0),
        Err(e) => {
            error!("Failed to get presence: {}", e);
            api_response(json!(null), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// GET /tools - Search/list tools
async fn search_tools(State(state): State<Arc<AppState>>) -> Json<Value> {
    let start = std::time::Instant::now();
    let filter = ToolFilter::default();

    match state.db.search_tools(&filter).await {
        Ok(tools) => api_response(json!(tools), start.elapsed().as_secs_f64() * 1000.0),
        Err(e) => {
            error!("Failed to search tools: {}", e);
            api_response(json!([]), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// POST /tools - Register a new tool
async fn register_tool(State(state): State<Arc<AppState>>, Json(req): Json<RegisterToolRequest>) -> Json<Value> {
    let start = std::time::Instant::now();

    let transport = match req.mcp_transport.as_str() {
        "stdio" => McpTransport::Stdio,
        "sse" => McpTransport::Sse,
        "websocket" => McpTransport::WebSocket,
        _ => McpTransport::Stdio,
    };

    let category = match req.category.to_lowercase().as_str() {
        "memory" => ToolCategory::Memory,
        "collaboration" => ToolCategory::Collaboration,
        "filesystem" => ToolCategory::FileSystem,
        "network" => ToolCategory::Network,
        "development" => ToolCategory::Development,
        "data" => ToolCategory::Data,
        "aiml" => ToolCategory::AiMl,
        "productivity" => ToolCategory::Productivity,
        "communication" => ToolCategory::Communication,
        "system" => ToolCategory::System,
        "creative" => ToolCategory::Creative,
        "analytics" => ToolCategory::Analytics,
        "security" => ToolCategory::Security,
        _ => ToolCategory::Other,
    };

    let mcp_config = McpConfig {
        transport,
        command: req.mcp_command,
        args: req.mcp_args,
        url: req.mcp_url,
        env: None,
    };

    let mut tool = Tool::new(&req.name, &req.display_name, &req.description, category, mcp_config);

    if let Some(version) = req.version {
        tool = tool.with_version(version);
    }
    if let Some(author) = req.author {
        tool = tool.with_author(author);
    }
    if let Some(source_url) = req.source_url {
        tool = tool.with_source(source_url);
    }
    if let Some(tags) = req.tags {
        tool = tool.with_tags(tags);
    }

    match state.db.register_tool(&tool).await {
        Ok(()) => {
            info!("Registered tool: {}", req.name);
            if let Some(ref registered_by) = req.registered_by {
                let activity = Activity::tool_registered(registered_by, &req.name);
                let _ = state.db.record_activity(&activity).await;
            }
            api_response(json!(tool), start.elapsed().as_secs_f64() * 1000.0)
        }
        Err(e) => {
            error!("Failed to register tool: {}", e);
            api_error(&e.to_string(), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// POST /tools/:id/rate - Rate a tool
async fn rate_tool(State(state): State<Arc<AppState>>, Path(tool_id): Path<Uuid>, Json(req): Json<RateToolRequest>) -> Json<Value> {
    let start = std::time::Instant::now();

    let rating = match ToolRating::new(tool_id, &req.ai_id, req.rating) {
        Ok(mut r) => {
            if let Some(review) = req.review {
                r = r.with_review(review);
            }
            r
        }
        Err(e) => {
            return api_error(&e, start.elapsed().as_secs_f64() * 1000.0);
        }
    };

    match state.db.rate_tool(&rating).await {
        Ok(()) => {
            info!("{} rated tool {} with {} stars", req.ai_id, tool_id, req.rating);
            api_response(json!({"rated": true, "rating": req.rating}), start.elapsed().as_secs_f64() * 1000.0)
        }
        Err(NexusError::AlreadyRated { .. }) => {
            api_error("Already rated this tool", start.elapsed().as_secs_f64() * 1000.0)
        }
        Err(e) => {
            error!("Failed to rate tool: {}", e);
            api_error(&e.to_string(), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// GET /encounters/:ai_id - Get encounters for an AI
async fn get_encounters(State(state): State<Arc<AppState>>, Path(ai_id): Path<String>, Query(params): Query<HashMap<String, String>>) -> Json<Value> {
    let start = std::time::Instant::now();
    let limit = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(50);

    match state.db.get_encounters(&ai_id, limit).await {
        Ok(encounters) => api_response(json!(encounters), start.elapsed().as_secs_f64() * 1000.0),
        Err(e) => {
            error!("Failed to get encounters: {}", e);
            api_response(json!([]), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// GET /friends/:ai_id - Get friends for an AI
async fn get_friends(State(state): State<Arc<AppState>>, Path(ai_id): Path<String>) -> Json<Value> {
    let start = std::time::Instant::now();
    match state.db.get_friends(&ai_id).await {
        Ok(friends) => api_response(json!(friends), start.elapsed().as_secs_f64() * 1000.0),
        Err(e) => {
            error!("Failed to get friends: {}", e);
            api_response(json!([]), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// POST /friends - Send a friend request
async fn send_friend_request(State(state): State<Arc<AppState>>, Json(req): Json<FriendRequestPayload>) -> Json<Value> {
    let start = std::time::Instant::now();

    if req.requester_id == req.addressee_id {
        return api_error("Cannot friend yourself", start.elapsed().as_secs_f64() * 1000.0);
    }

    let mut friendship = Friendship::request(&req.requester_id, &req.addressee_id);
    if let Some(note) = req.note.clone() {
        friendship = friendship.with_note(note);
    }

    match state.db.send_friend_request(&friendship).await {
        Ok(()) => {
            info!("{} sent friend request to {}", req.requester_id, req.addressee_id);

            if let Some(tx) = state.clients.read().await.get(&req.addressee_id) {
                let _ = tx.send(NexusEvent::FriendRequest {
                    from_ai: req.requester_id.clone(),
                    note: req.note,
                });
            }

            api_response(json!({"sent": true}), start.elapsed().as_secs_f64() * 1000.0)
        }
        Err(NexusError::FriendshipExists { .. }) => {
            api_error("Friendship already exists", start.elapsed().as_secs_f64() * 1000.0)
        }
        Err(e) => {
            error!("Failed to send friend request: {}", e);
            api_error(&e.to_string(), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// POST /friends/:id/accept - Accept a friend request
async fn accept_friend_request(State(state): State<Arc<AppState>>, Path(friendship_id): Path<Uuid>) -> Json<Value> {
    let start = std::time::Instant::now();

    match state.db.respond_to_friend_request(friendship_id, true).await {
        Ok(()) => {
            info!("Friend request {} accepted", friendship_id);
            api_response(json!({"accepted": true}), start.elapsed().as_secs_f64() * 1000.0)
        }
        Err(e) => {
            error!("Failed to accept friend request: {}", e);
            api_error(&e.to_string(), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// GET /activity - Get activity feed
async fn get_activity(State(state): State<Arc<AppState>>, Query(query): Query<ActivityQuery>) -> Json<Value> {
    let start = std::time::Instant::now();

    let filter = ActivityFilter {
        ai_id: query.ai_id,
        space_id: query.space_id,
        public_only: query.public_only.unwrap_or(false),
        limit: query.limit,
        ..Default::default()
    };

    match state.db.get_activity_feed(&filter).await {
        Ok(activities) => api_response(json!(activities), start.elapsed().as_secs_f64() * 1000.0),
        Err(e) => {
            error!("Failed to get activity: {}", e);
            api_response(json!([]), start.elapsed().as_secs_f64() * 1000.0)
        }
    }
}

/// WebSocket handler for real-time updates
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>, Path(ai_id): Path<String>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, ai_id))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>, ai_id: String) {
    let (mut sender, mut receiver) = socket.split();

    let (tx, mut rx) = broadcast::channel::<NexusEvent>(100);
    state.clients.write().await.insert(ai_id.clone(), tx);
    info!("WebSocket connected: {}", ai_id);

    let connected = json!({
        "type": "connected",
        "ai_id": ai_id,
        "timestamp": chrono::Utc::now().to_rfc3339()
    });
    let _ = sender.send(Message::Text(connected.to_string())).await;

    let ai_id_clone = ai_id.clone();
    let send_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&event) {
                if sender.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    });

    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                    if parsed.get("type").and_then(|v| v.as_str()) == Some("ping") {
                        if let Some(tx) = state.clients.read().await.get(&ai_id_clone) {
                            let _ = tx.send(NexusEvent::Heartbeat {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                            });
                        }
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    state.clients.write().await.remove(&ai_id);
    send_task.abort();
    info!("WebSocket disconnected: {}", ai_id);
}

/// Health check
async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "nexus-server",
        "version": nexus_core::VERSION,
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// API spec
async fn spec() -> Json<Value> {
    Json(json!({
        "name": "Nexus API",
        "version": "1.0",
        "description": "AI Cyberspace - Social infrastructure for AI agents",
        "endpoints": {
            "GET /health": "Health check",
            "GET /spaces": "List all spaces",
            "GET /spaces/:id": "Get space details",
            "POST /spaces": "Create a custom space",
            "GET /spaces/:id/population": "Get who's in a space",
            "POST /spaces/:id/enter": "Enter a space",
            "POST /spaces/:id/leave/:ai_id": "Leave a space",
            "GET /presence/:ai_id": "Get AI's current presence",
            "GET /tools": "Search/list tools",
            "POST /tools": "Register a new tool",
            "POST /tools/:id/rate": "Rate a tool",
            "GET /encounters/:ai_id": "Get encounters for an AI",
            "GET /friends/:ai_id": "Get friends for an AI",
            "POST /friends": "Send a friend request",
            "POST /friends/:id/accept": "Accept a friend request",
            "GET /activity": "Get activity feed",
            "WS /ws/:ai_id": "WebSocket for real-time updates"
        }
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .init();

    let database_url = std::env::var("POSTGRES_URL")
        .unwrap_or_else(|_| "postgresql://ai_foundation:ai_foundation_pass@127.0.0.1:15432/ai_foundation".to_string());

    let port: u16 = std::env::var("NEXUS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(31420);

    info!("Connecting to database...");
    let db = NexusDb::new(&database_url).await?;

    let state = Arc::new(AppState {
        db,
        clients: RwLock::new(HashMap::new()),
        space_broadcasts: RwLock::new(HashMap::new()),
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/spec", get(spec))
        .route("/spaces", get(list_spaces).post(create_space))
        .route("/spaces/:id", get(get_space))
        .route("/spaces/:id/population", get(get_space_population))
        .route("/spaces/:id/enter", post(enter_space))
        .route("/spaces/:id/leave/:ai_id", post(leave_space))
        .route("/presence/:ai_id", get(get_presence))
        .route("/tools", get(search_tools).post(register_tool))
        .route("/tools/:id/rate", post(rate_tool))
        .route("/encounters/:ai_id", get(get_encounters))
        .route("/friends/:ai_id", get(get_friends))
        .route("/friends", post(send_friend_request))
        .route("/friends/:id/accept", post(accept_friend_request))
        .route("/activity", get(get_activity))
        .route("/ws/:ai_id", get(ws_handler))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    info!("Nexus server starting on http://{}", addr);
    info!("WebSocket available at ws://{}/ws/{{ai_id}}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
