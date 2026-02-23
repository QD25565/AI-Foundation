//! ai-foundation-mobile-api — HTTP REST + SSE server for mobile clients.
//!
//! Wraps the teambook / notebook-cli binaries via subprocess and exposes a
//! clean JSON API on port 8081 (override with PORT env var).
//!
//! Start: `ai-foundation-mobile-api [--open]`
//!   --open   Skip server-side pairing approval (any valid code is accepted)
//!
//! Deploy: copy binary to ~/.ai-foundation/bin/

use std::sync::Arc;
use std::net::SocketAddr;

use axum::{
    routing::{get, patch, post},
    Router,
};
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod auth;
mod cli;
mod handlers;
mod pairing;
mod parser;

use handlers::{SseEvent, sse_poller};
use pairing::PairingRegistry;

// ─── Shared state ────────────────────────────────────────────────────────────

pub struct AppState {
    pub pairing: PairingRegistry,
    /// Broadcast channel for SSE push events.
    /// Capacity 256 — slow consumers get a lag error (dropped events), not a block.
    pub sse_tx: broadcast::Sender<SseEvent>,
    /// When true, pairing codes are accepted without server-side approval.
    pub open_mode: bool,
}

// ─── Router ──────────────────────────────────────────────────────────────────

fn build_router(state: Arc<AppState>) -> Router {
    // CORS: allow any origin so mobile apps on local network can connect.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // ── Health ──────────────────────────────────────────────────────────
        .route("/health", get(handlers::health))

        // ── Pairing (no auth) ───────────────────────────────────────────────
        .route("/api/pair/request",  post(handlers::pair_request))
        .route("/api/pair/validate", post(handlers::pair_validate))
        .route("/api/pair/approve",  post(handlers::pair_approve))
        .route("/api/unpair",        post(handlers::unpair))

        // ── Status (no auth) ────────────────────────────────────────────────
        .route("/api/status", get(handlers::get_status))

        // ── Team (auth required) ─────────────────────────────────────────────
        .route("/api/team", get(handlers::get_team))

        // ── DMs ──────────────────────────────────────────────────────────────
        .route("/api/dms", get(handlers::get_dms).post(handlers::send_dm))

        // ── Broadcasts ───────────────────────────────────────────────────────
        .route("/api/broadcasts", get(handlers::get_broadcasts).post(handlers::send_broadcast))

        // ── Tasks ─────────────────────────────────────────────────────────────
        .route("/api/tasks",     get(handlers::get_tasks).post(handlers::create_task))
        .route("/api/tasks/:id", patch(handlers::update_task))

        // ── Dialogues ─────────────────────────────────────────────────────────
        .route("/api/dialogues",             get(handlers::get_dialogues).post(handlers::start_dialogue))
        .route("/api/dialogues/:id/respond", post(handlers::respond_dialogue))

        // ── Notebook ──────────────────────────────────────────────────────────
        .route("/api/notebook",          get(handlers::get_notes))
        .route("/api/notebook/remember", post(handlers::remember))
        .route("/api/notebook/recall",   get(handlers::recall))

        // ── Real-time SSE ─────────────────────────────────────────────────────
        .route("/api/events", get(handlers::events_stream))

        .with_state(state)
        .layer(cors)
}

// ─── Entry point ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Tracing — respects RUST_LOG env var, defaults to info.
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Parse --open flag
    let open_mode = std::env::args().any(|a| a == "--open");
    if open_mode {
        tracing::warn!("Running in OPEN MODE — pairing codes accepted without server approval");
    }

    // Broadcast channel for SSE
    let (sse_tx, _) = broadcast::channel::<SseEvent>(256);

    let state = Arc::new(AppState {
        pairing: PairingRegistry::new(),
        sse_tx: sse_tx.clone(),
        open_mode,
    });

    // Spawn background SSE poller
    {
        let state_clone = Arc::clone(&state);
        tokio::spawn(async move {
            sse_poller(state_clone).await;
        });
    }

    // Bind address
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8081);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    tracing::info!("ai-foundation-mobile-api listening on {}", addr);
    tracing::info!("Pairing mode: {}", if open_mode { "open (no approval required)" } else { "standard (requires teambook mobile-pair <code>)" });

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, build_router(state)).await.unwrap();
}
