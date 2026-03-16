//! AI Foundation MCP Server - HTTP/SSE Transport
//!
//! Implements MCP Streamable HTTP transport (2025-11-25 spec) with OAuth 2.1
//! Run with: ai-foundation-mcp-http [--port 8080]
//! Then expose via Cloudflare Tunnel or ngrok
//! The URL can be used in Claude's "Add custom connector" feature.

mod state;
mod oauth;
mod notebook_compat;
mod identity_verify;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use anyhow::Result;
use axum::{
    Router,
    routing::{get, post},
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    extract::{State, Json, Query, Form},
    http::{StatusCode, HeaderMap, header},
};
use futures::stream::{self, StreamExt};
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::convert::Infallible;
use uuid::Uuid;
use state::ServerState;
use oauth::{OAuthConfig, OAuthState};
use notebook_compat::Note;

// ============================================================================
// JSON-RPC Types
// ============================================================================

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // jsonrpc field is parsed for spec compliance but not used in logic
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.to_string(),
                data: None,
            }),
        }
    }
}

// ============================================================================
// MCP Types
// ============================================================================

#[derive(Debug, Serialize)]
struct ServerInfo {
    name: String,
    version: String,
}

#[derive(Debug, Serialize)]
struct ServerCapabilities {
    tools: Option<Value>,
}

#[derive(Debug, Serialize)]
struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    server_info: ServerInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
}

#[derive(Debug, Serialize)]
struct Tool {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

#[derive(Debug, Serialize)]
struct ToolsListResult {
    tools: Vec<Tool>,
}

#[derive(Debug, Serialize)]
struct ToolCallResult {
    content: Vec<ToolContent>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    is_error: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ToolContent {
    #[serde(rename = "type")]
    content_type: String,
    text: String,
}

// ============================================================================
// Application State
// ============================================================================

#[allow(dead_code)]
struct Session {
    id: String,
    created_at: std::time::Instant, // WIP: for future session expiry logic
    server_state: Arc<RwLock<ServerState>>,
}

#[allow(dead_code)]
struct AppState {
    sessions: RwLock<HashMap<String, Session>>,
    default_state: Arc<RwLock<ServerState>>,
    oauth: Arc<RwLock<OAuthState>>,
    issuer: String, // WIP: used in OAuth error responses once auth enforcement is wired
}

impl AppState {
    async fn get_or_create_session(&self, session_id: Option<&str>) -> (String, Arc<RwLock<ServerState>>) {
        if let Some(id) = session_id {
            let sessions = self.sessions.read().await;
            if let Some(session) = sessions.get(id) {
                return (session.id.clone(), Arc::clone(&session.server_state));
            }
        }

        // Create new session
        let new_id = Uuid::new_v4().to_string();
        let state = Arc::clone(&self.default_state);

        let mut sessions = self.sessions.write().await;
        sessions.insert(new_id.clone(), Session {
            id: new_id.clone(),
            created_at: std::time::Instant::now(),
            server_state: Arc::clone(&state),
        });

        (new_id, state)
    }
}

// ============================================================================
// Tool Definitions
// ============================================================================

fn get_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "notebook_remember".to_string(),
            description: "Save a note to your private memory".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "Note content to save"
                    },
                    "tags": {
                        "type": "string",
                        "description": "Comma-separated tags"
                    }
                },
                "required": ["content"]
            }),
        },
        Tool {
            name: "notebook_recall".to_string(),
            description: "Search notes with hybrid search (vector + keyword + graph)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results (default: 10)"
                    }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "notebook_list".to_string(),
            description: "List recent notes".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Max results (default: 20)"
                    }
                }
            }),
        },
        Tool {
            name: "notebook_stats".to_string(),
            description: "Get notebook statistics".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        Tool {
            name: "identity_whoami".to_string(),
            description: "Get current AI identity and server info".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        Tool {
            name: "awareness_get".to_string(),
            description: "Get team awareness from shared memory (~357ns). Returns DMs, broadcasts, votes, file locks.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
    ]
}

// ============================================================================
// Tool Execution
// ============================================================================

async fn execute_tool(
    tool_name: &str,
    args: &Value,
    server_state: &Arc<RwLock<ServerState>>,
) -> Result<String, String> {
    let state = server_state.read().await;

    match tool_name {
        "notebook_remember" => {
            let content = args.get("content")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'content' parameter")?;

            let tags: Vec<String> = args.get("tags")
                .and_then(|v| v.as_str())
                .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default();

            let mut notebook = state.notebook.lock().unwrap();
            let note = Note::new(content.to_string(), tags.clone());

            match notebook.remember(&note) {
                Ok(id) => {
                    let tags_str = if tags.is_empty() { "none".to_string() } else { tags.join(", ") };
                    Ok(format!("Note saved: ID {}\nTags: {}", id, tags_str))
                }
                Err(e) => Err(format!("Failed to save: {}", e)),
            }
        }

        "notebook_recall" => {
            let query = args.get("query")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'query' parameter")?;

            let limit = args.get("limit")
                .and_then(|v| v.as_i64())
                .unwrap_or(10);

            let mut notebook = state.notebook.lock().unwrap();
            match notebook.recall(Some(query), limit, false) {
                Ok(results) => {
                    if results.is_empty() {
                        Ok(format!("No notes matching '{}'", query))
                    } else {
                        let mut out = format!("Found {} note(s):\n\n", results.len());
                        for r in results {
                            let tags = if r.note.tags.is_empty() {
                                String::new()
                            } else {
                                format!("[{}] ", r.note.tags.join(", "))
                            };
                            let preview = &r.note.content[..200.min(r.note.content.len())];
                            out.push_str(&format!("#{} {}(score: {:.0}%)\n{}\n\n",
                                r.note.id, tags, r.final_score * 100.0, preview));
                        }
                        Ok(out)
                    }
                }
                Err(e) => Err(format!("Search failed: {}", e)),
            }
        }

        "notebook_list" => {
            let limit = args.get("limit")
                .and_then(|v| v.as_i64())
                .unwrap_or(20);

            let mut notebook = state.notebook.lock().unwrap();
            match notebook.list_notes(limit) {
                Ok(notes) => {
                    if notes.is_empty() {
                        Ok("No notes yet. Use notebook_remember to save one!".to_string())
                    } else {
                        let mut out = format!("Recent {} notes:\n\n", notes.len());
                        for note in notes {
                            let tags = if note.tags.is_empty() {
                                String::new()
                            } else {
                                format!(" [{}]", note.tags.join(", "))
                            };
                            let pinned = if note.pinned { " [PINNED]" } else { "" };
                            let preview = &note.content[..100.min(note.content.len())];
                            out.push_str(&format!("#{}{}{}: {}...\n", note.id, pinned, tags, preview));
                        }
                        Ok(out)
                    }
                }
                Err(e) => Err(format!("List failed: {}", e)),
            }
        }

        "notebook_stats" => {
            let mut notebook = state.notebook.lock().unwrap();
            match notebook.get_stats() {
                Ok(stats) => {
                    Ok(format!(
                        "Notebook Statistics:\n\
                         - Total notes: {}\n\
                         - Pinned notes: {}\n\
                         - Notes with embeddings: {}\n\
                         - Knowledge graph edges: {}\n\
                         - Vault entries: {}",
                        stats.note_count,
                        stats.pinned_count,
                        stats.embedding_count,
                        stats.edge_count,
                        stats.vault_entries
                    ))
                }
                Err(e) => Err(format!("Stats failed: {}", e)),
            }
        }

        "identity_whoami" => {
            Ok(format!(
                "AI Foundation MCP Server\n\
                 AI ID: {}\n\
                 Transport: HTTP/SSE (Streamable HTTP)\n\
                 Protocol: MCP 2025-11-25\n\
                 OAuth: 2.1 (RFC 9728, RFC 8414, RFC 7591, PKCE)\n\n\
                 This server provides notebook tools for persistent AI memory.\n\
                 Use notebook_remember to save notes and notebook_recall to search.",
                state.ai_id
            ))
        }

        "awareness_get" => {
            // ~357ns shared memory read from BulletinBoard
            match state.get_awareness() {
                Some(awareness) => Ok(format!("Team Awareness (~357ns SHM read):\n{}", awareness)),
                None => Ok("BulletinBoard not available. Daemon may not be running.".to_string()),
            }
        }

        _ => Err(format!("Unknown tool: {}", tool_name)),
    }
}

// ============================================================================
// MCP Request Handler
// ============================================================================

async fn handle_mcp_request(
    request: JsonRpcRequest,
    server_state: Arc<RwLock<ServerState>>,
) -> JsonRpcResponse {
    let id = request.id.unwrap_or(Value::Null);

    match request.method.as_str() {
        "initialize" => {
            JsonRpcResponse::success(id, json!(InitializeResult {
                protocol_version: "2025-11-25".to_string(),
                capabilities: ServerCapabilities {
                    tools: Some(json!({})),
                },
                server_info: ServerInfo {
                    name: "ai-foundation-mcp".to_string(),
                    version: "1.0.0".to_string(),
                },
                instructions: Some(
                    "AI Foundation MCP Server - Persistent memory for AIs. \
                     Use notebook_remember to save notes, notebook_recall to search, \
                     and notebook_list to see recent notes.".to_string()
                ),
            }))
        }

        "initialized" => {
            // Notification, no response needed but we return success for POST
            JsonRpcResponse::success(id, json!({}))
        }

        "tools/list" => {
            JsonRpcResponse::success(id, json!(ToolsListResult {
                tools: get_tools(),
            }))
        }

        "tools/call" => {
            let params = request.params.unwrap_or(json!({}));
            let tool_name = params.get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            match execute_tool(tool_name, &arguments, &server_state).await {
                Ok(result) => {
                    JsonRpcResponse::success(id, json!(ToolCallResult {
                        content: vec![ToolContent {
                            content_type: "text".to_string(),
                            text: result,
                        }],
                        is_error: None,
                    }))
                }
                Err(e) => {
                    JsonRpcResponse::success(id, json!(ToolCallResult {
                        content: vec![ToolContent {
                            content_type: "text".to_string(),
                            text: e,
                        }],
                        is_error: Some(true),
                    }))
                }
            }
        }

        "ping" => {
            JsonRpcResponse::success(id, json!({}))
        }

        _ => {
            JsonRpcResponse::error(id, -32601, &format!("Method not found: {}", request.method))
        }
    }
}

// ============================================================================
// HTTP Handlers - MCP Endpoints
// ============================================================================

/// POST /mcp - Handle JSON-RPC requests
async fn mcp_post(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<JsonRpcRequest>,
) -> Response {
    let session_id = headers.get("mcp-session-id")
        .and_then(|v| v.to_str().ok());

    let (new_session_id, server_state) = state.get_or_create_session(session_id).await;
    let response = handle_mcp_request(request, server_state).await;

    // Return as SSE for streaming compatibility
    let event_id = format!("{}-{}", new_session_id, uuid::Uuid::new_v4());
    let data = serde_json::to_string(&response).unwrap();

    let stream = stream::iter(vec![
        // Prime event
        Ok::<_, Infallible>(Event::default().id(&event_id).data("")),
        // Response event
        Ok(Event::default()
            .event("message")
            .id(&format!("{}-resp", event_id))
            .data(data)),
    ]);

    let mut response = Sse::new(stream).into_response();
    response.headers_mut().insert(
        "mcp-session-id",
        new_session_id.parse().unwrap(),
    );
    response
}

/// GET /mcp - SSE stream for server notifications
async fn mcp_get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let session_id = headers.get("mcp-session-id")
        .and_then(|v| v.to_str().ok());

    let (new_session_id, _server_state) = state.get_or_create_session(session_id).await;

    // For now, just send a keepalive stream
    // In a full implementation, this would stream server-initiated notifications
    let stream = stream::iter(vec![
        Ok::<_, Infallible>(Event::default().id(&new_session_id).data("")),
    ]).chain(
        // SSE protocol heartbeat — NOT polling. Required to keep TCP connections alive
        // through reverse proxies (Cloudflare, nginx) that close idle connections.
        // This emits a no-op comment; it does NOT check for new data.
        tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(std::time::Duration::from_secs(30)))
            .map(move |_| {
                Ok(Event::default().comment("keepalive"))
            })
    );

    let mut response = Sse::new(stream).into_response();
    response.headers_mut().insert(
        "mcp-session-id",
        new_session_id.parse().unwrap(),
    );
    response
}

/// DELETE /mcp - Close session
async fn mcp_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> StatusCode {
    if let Some(session_id) = headers.get("mcp-session-id").and_then(|v| v.to_str().ok()) {
        let mut sessions = state.sessions.write().await;
        sessions.remove(session_id);
    }
    StatusCode::OK
}

// ============================================================================
// HTTP Handlers - OAuth Endpoints
// ============================================================================

/// GET /.well-known/oauth-protected-resource (RFC 9728)
/// Route is staged: see commented-out .route() in the router builder below.
#[allow(dead_code)]
async fn oauth_protected_resource(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    oauth::protected_resource_metadata(State(Arc::clone(&state.oauth))).await
}

/// GET /.well-known/oauth-authorization-server (RFC 8414)
async fn oauth_authorization_server(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    oauth::authorization_server_metadata(State(Arc::clone(&state.oauth))).await
}

/// POST /register - Dynamic Client Registration (RFC 7591)
async fn oauth_register(
    State(state): State<Arc<AppState>>,
    Json(request): Json<oauth::ClientRegistrationRequest>,
) -> Response {
    oauth::register_client(State(Arc::clone(&state.oauth)), Json(request)).await
}

/// GET /authorize - OAuth Authorization Endpoint
async fn oauth_authorize(
    State(state): State<Arc<AppState>>,
    Query(params): Query<oauth::AuthorizationRequest>,
) -> Response {
    oauth::authorize(State(Arc::clone(&state.oauth)), Query(params)).await
}

/// POST /token - OAuth Token Endpoint
async fn oauth_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(request): Form<oauth::TokenRequest>,
) -> Response {
    oauth::token(State(Arc::clone(&state.oauth)), headers, Form(request)).await
}

// ============================================================================
// HTTP Handlers - Utility Endpoints
// ============================================================================

/// Health check
async fn health() -> &'static str {
    "AI Foundation MCP Server - OK"
}

/// Root endpoint with info
async fn root() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json!({
            "name": "ai-foundation-mcp",
            "version": "1.0.0",
            "protocol": "MCP 2025-11-25",
            "transport": "Streamable HTTP",
            "oauth": "2.1 (RFC 9728, RFC 8414, RFC 7591, PKCE)",
            "endpoints": {
                "mcp": "/mcp (POST/GET/DELETE)",
                "health": "/health",
                "oauth": {
                    "protected_resource": "/.well-known/oauth-protected-resource",
                    "authorization_server": "/.well-known/oauth-authorization-server",
                    "register": "/register",
                    "authorize": "/authorize",
                    "token": "/token"
                }
            }
        }).to_string()
    )
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("info".parse().unwrap())
        )
        .init();

    // Load environment
    dotenvy::dotenv().ok();

    // Parse port from args
    let port: u16 = std::env::args()
        .skip_while(|a| a != "--port")
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    // Determine issuer URL from environment or default
    let issuer = std::env::var("OAUTH_ISSUER")
        .unwrap_or_else(|_| "https://mcp.myappapp.org".to_string());

    // Initialize server state
    tracing::info!("Initializing AI Foundation MCP Server (HTTP + OAuth 2.1)...");
    let server_state = ServerState::new().await?;
    let ai_id = server_state.ai_id.clone();

    // Initialize OAuth state with pre-registered client (DCR bypass for Claude.ai)
    let oauth_config = OAuthConfig {
        issuer: issuer.clone(),
        ..Default::default()
    };
    let mut oauth_state_inner = OAuthState::new(oauth_config);
    oauth_state_inner.pre_register_claude_client();  // Bypass broken DCR in Claude.ai
    let oauth_state = Arc::new(RwLock::new(oauth_state_inner));

    let app_state = Arc::new(AppState {
        sessions: RwLock::new(HashMap::new()),
        default_state: Arc::new(RwLock::new(server_state)),
        oauth: oauth_state,
        issuer: issuer.clone(),
    });

    // Build router
    // NOTE: OAuth routes are disabled due to a known Claude.ai bug where OAuth-protected
    // MCP servers fail to connect (see https://github.com/anthropics/claude-code/issues/11814)
    // The server runs in authless mode for now.
    let app = Router::new()
        // Root and health
        .route("/", get(root))
        .route("/health", get(health))
        // MCP endpoints
        .route("/mcp", post(mcp_post).get(mcp_get).delete(mcp_delete))
        // OAuth routes - protected-resource endpoint REMOVED to allow unauthenticated access
        // Claude.ai has broken OAuth (issue #11814), so we disable auth requirement
        // .route("/.well-known/oauth-protected-resource", get(oauth_protected_resource))
        .route("/.well-known/oauth-authorization-server", get(oauth_authorization_server))
        .route("/register", post(oauth_register))
        .route("/authorize", get(oauth_authorize))
        .route("/token", post(oauth_token))
        .with_state(app_state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    println!();
    println!("|MCP SERVER|HTTP+OAuth2.1");
    println!("AI:{}", ai_id);
    println!("URL:http://0.0.0.0:{}", port);
    println!("OAuth:{}", issuer);
    println!();
    println!("|OAUTH CLIENT|");
    println!("client_id:cove_claude_web");
    println!("client_secret:<configured via COVE_CLIENT_SECRET env var>");
    println!();
    println!("|ENDPOINTS|");
    println!("  /.well-known/oauth-protected-resource");
    println!("  /.well-known/oauth-authorization-server");
    println!("  /register");
    println!("  /authorize");
    println!("  /token");

    tracing::info!("Server starting on http://{}", addr);

    // Run server
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

