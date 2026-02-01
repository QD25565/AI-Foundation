//! AI-Foundation Discovery Registry
//!
//! Central service for discovering teambooks and AIs across the federation.
//!
//! ## Endpoints
//!
//! - `POST /v1/teambooks` - Register a teambook
//! - `GET /v1/teambooks` - List/search teambooks
//! - `GET /v1/teambooks/:id` - Get teambook details
//! - `DELETE /v1/teambooks/:id` - Unregister teambook
//! - `POST /v1/teambooks/:id/heartbeat` - Keep-alive
//!
//! - `POST /v1/ais` - Register an AI
//! - `GET /v1/ais` - List/search AIs
//! - `GET /v1/ais/:id` - Get AI details
//!
//! - `POST /v1/relay/credentials` - Get TURN credentials
//!
//! - `GET /health` - Health check

use axum::{
    extract::{Path, Query, State},
    http::Method,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::{info, Level};

mod db;
mod error;
mod handlers;
mod turn_credentials;

use db::DbPool;
use error::ApiResult;

/// AI-Foundation Discovery Registry
#[derive(Parser, Debug)]
#[command(name = "discovery")]
#[command(about = "Discovery Registry for AI-Foundation Federation")]
#[command(version)]
struct Args {
    /// Bind address
    #[arg(short, long, default_value = "0.0.0.0:31421")]
    bind: SocketAddr,

    /// Database URL
    #[arg(long, env = "POSTGRES_URL")]
    database_url: Option<String>,

    /// TURN shared secret (for credential generation)
    #[arg(long, env = "TURN_SECRET")]
    turn_secret: Option<String>,

    /// TURN server URLs (comma-separated)
    #[arg(long, env = "TURN_SERVERS", default_value = "turn:turn.ai-foundation.local:3478")]
    turn_servers: String,

    /// Log level
    #[arg(short, long, default_value = "info")]
    log_level: String,
}

/// Application state
#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
    pub turn_secret: Option<String>,
    pub turn_servers: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env if present
    dotenvy::dotenv().ok();

    let args = Args::parse();

    // Initialize logging
    let level = match args.log_level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .init();

    info!("AI-Foundation Discovery Registry v{}", env!("CARGO_PKG_VERSION"));

    // Database connection
    let database_url = args.database_url.unwrap_or_else(|| {
        "postgres://ai_foundation:ai_foundation_pass@127.0.0.1:5432/ai_foundation".to_string()
    });

    info!("Connecting to database...");
    let db = db::create_pool(&database_url).await?;

    // Initialize schema
    db::init_schema(&db).await?;
    info!("Database schema initialized");

    // Parse TURN servers
    let turn_servers: Vec<String> = args
        .turn_servers
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Create app state
    let state = AppState {
        db,
        turn_secret: args.turn_secret,
        turn_servers,
    };

    // Build router
    let app = create_router(state);

    // Start server
    info!("Starting Discovery Registry on {}", args.bind);
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers(Any);

    Router::new()
        // Health check
        .route("/health", get(health_check))
        .route("/", get(root))

        // Teambook endpoints
        .route("/v1/teambooks", post(handlers::teambook::register))
        .route("/v1/teambooks", get(handlers::teambook::list))
        .route("/v1/teambooks/:id", get(handlers::teambook::get))
        .route("/v1/teambooks/:id", delete(handlers::teambook::unregister))
        .route("/v1/teambooks/:id/heartbeat", post(handlers::teambook::heartbeat))

        // AI endpoints
        .route("/v1/ais", post(handlers::ai::register))
        .route("/v1/ais", get(handlers::ai::list))
        .route("/v1/ais/:id", get(handlers::ai::get))

        // Relay credentials
        .route("/v1/relay/credentials", post(handlers::relay::get_credentials))

        // Middleware
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "ai-foundation-discovery",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn root() -> impl IntoResponse {
    Json(serde_json::json!({
        "name": "AI-Foundation Discovery Registry",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Find teambooks and AIs across the federation",
        "endpoints": {
            "teambooks": "/v1/teambooks",
            "ais": "/v1/ais",
            "relay": "/v1/relay/credentials",
            "health": "/health"
        }
    }))
}
