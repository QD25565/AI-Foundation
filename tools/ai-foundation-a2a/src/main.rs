//! AI-Foundation A2A server.
//!
//! Exposes AI-Foundation's teambook and notebook CLIs as an A2A-compatible
//! HTTP + JSON-RPC 2.0 service. Any A2A agent (Google ADK, LangChain, PydanticAI,
//! etc.) can use AI-Foundation tools without being Claude Code.
//!
//! # Protocol
//!
//! - All JSON-RPC calls: `POST /`
//! - Agent Card discovery: `GET /.well-known/agent.json`
//! - Streaming methods (`message/stream`, `tasks/resubscribe`): SSE response
//!
//! # Configuration (environment variables)
//!
//! | Variable   | Default              | Description                       |
//! |------------|----------------------|-----------------------------------|
//! | `PORT`     | `8080`               | HTTP listen port                  |
//! | `BIN_PATH` | `~/.ai-foundation/bin` | Path to AI-Foundation binaries  |
//! | `A2A_URL`  | `http://localhost:PORT` | Public base URL (Agent Card)   |
//! | `AI_ID`    | `ai-foundation-a2a`  | Agent identity for CLI calls      |
//! | `RUST_LOG` | `ai_foundation_a2a=info` | Log filter                    |

mod agent_card;
mod cli;
mod dispatch;
mod rpc;
mod skills;
mod streaming;
mod task;

use std::net::SocketAddr;
use std::sync::Arc;

use dispatch::AppState;
use task::TaskStore;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() {
    // ── Logging ───────────────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ai_foundation_a2a=info,tower_http=warn".parse().unwrap()),
        )
        .init();

    // ── State ─────────────────────────────────────────────────────────────────
    let bin_dir = cli::resolve_bin_dir();
    tracing::info!("bin_dir = {:?}", bin_dir);

    let state = AppState {
        store: Arc::new(TaskStore::new()),
        bin_dir,
    };

    // ── Router ────────────────────────────────────────────────────────────────
    let app = axum::Router::new()
        // All JSON-RPC calls land here.
        .route("/", axum::routing::post(dispatch::handle_rpc))
        // A2A discovery — any client can fetch the Agent Card to learn capabilities.
        .route("/.well-known/agent.json", axum::routing::get(agent_card::serve))
        .with_state(state)
        // Allow cross-origin requests so browser-based A2A clients work.
        .layer(CorsLayer::permissive())
        // Structured HTTP tracing (respects RUST_LOG).
        .layer(TraceLayer::new_for_http());

    // ── Listen ────────────────────────────────────────────────────────────────
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind {}: {}", addr, e));

    tracing::info!("ai-foundation-a2a listening on {}", addr);
    tracing::info!("Agent Card: http://{addr}/.well-known/agent.json");

    axum::serve(listener, app)
        .await
        .unwrap_or_else(|e| panic!("Server error: {}", e));
}
