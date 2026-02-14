//! AI Foundation HTTP API Server
//!
//! Serves REST endpoints for mobile and desktop human clients.
//! Uses the same CLI subprocess pattern as the MCP server —
//! CLIs are the source of truth, this is just another thin wrapper.
//!
//! Usage:
//!   AI_FOUNDATION_HTTP_PORT=8080 ai-foundation-http
//!
//! Endpoints:
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

use ai_foundation_mcp::{http_api, pairing::PairingState, sse};
use anyhow::Result;
use std::net::SocketAddr;
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

    let pairing = PairingState::new();
    let state = http_api::ApiState { pairing };

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

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
