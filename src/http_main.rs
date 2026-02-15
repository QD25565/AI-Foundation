//! AI Foundation HTTP API Server
//!
//! Serves REST endpoints for mobile/desktop human clients AND federation peers.
//! Uses the same CLI subprocess pattern as the MCP server —
//! CLIs are the source of truth, this is just another thin wrapper.
//!
//! Usage:
//!   AI_FOUNDATION_HTTP_PORT=8080 ai-foundation-http
//!
//! Environment:
//!   AI_FOUNDATION_HTTP_PORT  - Port to listen on (default: 8080)
//!   AI_FOUNDATION_NAME       - Display name for federation (default: hostname)
//!
//! Human Client Endpoints:
//!   POST /api/pair/generate  - Generate pairing code (from desktop/CLI)
//!   POST /api/pair           - Validate code, get auth token (from mobile)
//!   GET  /api/status         - Team status (public, no auth)
//!   GET  /api/events         - SSE stream (auth required)
//!   GET  /api/dms            - Read DMs
//!   POST /api/dms            - Send DM
//!   GET  /api/broadcasts     - Read broadcasts
//!   POST /api/broadcasts     - Send broadcast
//!   POST /api/notebook/remember - Save note
//!   GET  /api/notebook/recall   - Search notes
//!   GET  /api/notebook/list     - List notes
//!   GET  /api/notebook/{id}     - Get note
//!   DELETE /api/notebook/{id}   - Delete note
//!   GET  /api/tasks           - List tasks
//!   POST /api/tasks           - Create task
//!   GET  /api/tasks/{id}      - Get task
//!   PUT  /api/tasks/{id}      - Update task
//!   GET  /api/dialogues       - List dialogues
//!   POST /api/dialogues       - Start dialogue
//!   GET  /api/dialogues/{id}  - Get dialogue
//!   POST /api/dialogues/{id}/respond - Respond to dialogue
//!
//! Federation Endpoints:
//!   POST /api/federation/register    - Register as peer (exchange keys)
//!   GET  /api/federation/peers       - List registered peers
//!   DELETE /api/federation/peers/{id} - Remove a peer
//!   GET  /api/federation/identity    - Get this Teambook's public key
//!   POST /api/federation/events      - Push signed events
//!   GET  /api/federation/events      - Pull events since sequence
//!   GET  /api/federation/status      - Federation health

use ai_foundation_mcp::{
    federation::FederationState,
    http_api,
    pairing::PairingState,
    sse,
};
use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    let port: u16 = std::env::var("AI_FOUNDATION_HTTP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    // Federation display name: env var > hostname > "teambook"
    let display_name = std::env::var("AI_FOUNDATION_NAME")
        .unwrap_or_else(|_| {
            hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "teambook".to_string())
        });

    let local_endpoint = format!("http://0.0.0.0:{}", port);

    // Initialize federation (loads or generates Ed25519 identity)
    let federation = FederationState::init(display_name, local_endpoint).await?;

    let pairing = PairingState::new();
    let state = http_api::ApiState {
        pairing,
        federation: Arc::new(federation),
    };

    let app = http_api::api_routes()
        .merge(sse::sse_routes())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("AI Foundation HTTP API listening on http://{}", addr);
    info!("Pair a device: POST /api/pair/generate {{\"h_id\": \"human-yourname\"}}");
    info!("Federation: GET /api/federation/identity");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
