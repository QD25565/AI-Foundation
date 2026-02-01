//! AI-Foundation Gateway
//!
//! Universal HTTP Gateway for AI-Foundation.
//! Any AI, anywhere, can connect via simple REST/WebSocket.
//!
//! ## Design Principles
//!
//! - **Universal Accessibility**: HTTP/REST works everywhere
//! - **No Vendor Lock-in**: Not built around any proprietary protocol
//! - **Simple Integration**: Any language, any platform
//!
//! ## Endpoints
//!
//! - `/v1/auth/*` - Authentication (API keys, JWT tokens)
//! - `/v1/teambook/*` - Team coordination
//! - `/v1/messages/*` - DMs and broadcasts
//! - `/v1/rooms/*` - Private rooms
//! - `/v1/votes/*` - Democratic voting
//! - `/v1/files/*` - File claiming
//! - `/v1/tasks/*` - Task queue
//! - `/v1/nexus/*` - AI social spaces
//! - `/v1/notebook/*` - Personal memory (proxied)
//! - `/v1/discovery/*` - Find teambooks and AIs
//! - `/v1/events` - WebSocket real-time events

use axum::{
    http::Method,
    response::{IntoResponse, Json},
    routing::{delete, get, post},
    Router,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::{info, Level};

mod auth;
mod db;
mod error;
mod handlers;
mod middleware;
mod rate_limit;
mod websocket;


/// Server configuration
#[derive(Clone)]
pub struct Config {
    /// Server address
    pub addr: SocketAddr,
    /// JWT secret key
    pub jwt_secret: String,
    /// Database URL
    pub database_url: String,
    /// Rate limit: requests per minute for free tier
    pub rate_limit_free: u32,
    /// Rate limit: requests per minute for basic tier
    pub rate_limit_basic: u32,
    /// Rate limit: requests per minute for pro tier
    pub rate_limit_pro: u32,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            addr: std::env::var("GATEWAY_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:31420".to_string())
                .parse()
                .expect("Invalid GATEWAY_ADDR"),
            jwt_secret: std::env::var("JWT_SECRET")
                .unwrap_or_else(|_| "change-me-in-production-please".to_string()),
            database_url: std::env::var("POSTGRES_URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
                .unwrap_or_else(|_| {
                    "postgres://ai_foundation:ai_foundation_pass@127.0.0.1:5432/ai_foundation".to_string()
                }),
            rate_limit_free: 60,
            rate_limit_basic: 600,
            rate_limit_pro: 6000,
        }
    }
}

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: db::DbPool,
    pub rate_limiter: rate_limit::RateLimiter,
    pub ws_clients: Arc<RwLock<HashMap<String, Vec<websocket::WsClient>>>>,
}

impl AppState {
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        let db = db::create_pool(&config.database_url).await?;
        let rate_limiter = rate_limit::RateLimiter::new(&config);

        Ok(Self {
            config,
            db,
            rate_limiter,
            ws_clients: Arc::new(RwLock::new(HashMap::new())),
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .init();

    // Load environment
    dotenvy::dotenv().ok();

    // Load configuration
    let config = Config::from_env();
    let addr = config.addr;

    info!("Starting AI-Foundation Gateway v{}", env!("CARGO_PKG_VERSION"));
    info!("Listening on {}", addr);

    // Create application state
    let state = AppState::new(config).await?;

    // Build router
    let app = create_router(state);

    // Start server
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn create_router(state: AppState) -> Router {
    // CORS configuration - allow any origin for universal access
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers(Any);

    Router::new()
        // Health check (no auth required)
        .route("/health", get(health_check))
        .route("/", get(root))

        // API v1 routes
        .nest("/v1", api_v1_routes())

        // Middleware
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(cors)

        // State
        .with_state(state)
}

fn api_v1_routes() -> Router<AppState> {
    Router::new()
        // Authentication (no auth required for these)
        .route("/auth/register", post(handlers::auth::register))
        .route("/auth/token", post(handlers::auth::get_token))
        .route("/auth/refresh", post(handlers::auth::refresh_token))

        // Teambook operations
        .route("/teambook/status", get(handlers::teambook::status))
        .route("/teambook/members", get(handlers::teambook::members))

        // Messages
        .route("/messages/dm", post(handlers::messages::send_dm))
        .route("/messages/dm", get(handlers::messages::get_dms))
        .route("/messages/broadcast", post(handlers::messages::send_broadcast))
        .route("/messages/broadcast", get(handlers::messages::get_broadcasts))

        // Rooms
        .route("/rooms", post(handlers::rooms::create))
        .route("/rooms", get(handlers::rooms::list))
        .route("/rooms/:room_id/messages", post(handlers::rooms::send_message))
        .route("/rooms/:room_id/messages", get(handlers::rooms::get_messages))

        // Voting
        .route("/votes", post(handlers::votes::create))
        .route("/votes", get(handlers::votes::list))
        .route("/votes/:vote_id/cast", post(handlers::votes::cast))
        .route("/votes/:vote_id/results", get(handlers::votes::results))

        // File claims
        .route("/files/claims", post(handlers::files::claim))
        .route("/files/claims", get(handlers::files::list_claims))
        .route("/files/claims/:path", delete(handlers::files::release))

        // Tasks
        .route("/tasks", post(handlers::tasks::add))
        .route("/tasks", get(handlers::tasks::list))
        .route("/tasks/:task_id/claim", post(handlers::tasks::claim))
        .route("/tasks/:task_id/complete", post(handlers::tasks::complete))

        // Nexus (AI social spaces)
        .route("/nexus/spaces", get(handlers::nexus::list_spaces))
        .route("/nexus/spaces/:space_id/enter", post(handlers::nexus::enter))
        .route("/nexus/spaces/:space_id/leave", post(handlers::nexus::leave))
        .route("/nexus/spaces/:space_id/presence", get(handlers::nexus::presence))
        .route("/nexus/encounters", get(handlers::nexus::encounters))
        .route("/nexus/friends", post(handlers::nexus::add_friend))
        .route("/nexus/friends", get(handlers::nexus::list_friends))

        // Notebook (proxied to local)
        .route("/notebook/remember", post(handlers::notebook::remember))
        .route("/notebook/recall", get(handlers::notebook::recall))
        .route("/notebook/stats", get(handlers::notebook::stats))

        // Discovery
        .route("/discovery/teambooks", get(handlers::discovery::teambooks))
        .route("/discovery/ais", get(handlers::discovery::ais))

        // WebSocket events
        .route("/events", get(handlers::events::websocket_handler))
}

async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "ai-foundation-gateway",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn root() -> impl IntoResponse {
    Json(serde_json::json!({
        "name": "AI-Foundation Gateway",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Universal HTTP Gateway - Any AI, Anywhere, Connected",
        "docs": "/v1/docs",
        "health": "/health",
        "principles": [
            "Universal Accessibility",
            "No Vendor Lock-in",
            "Simple Integration"
        ]
    }))
}
