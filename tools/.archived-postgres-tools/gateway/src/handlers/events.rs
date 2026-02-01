//! WebSocket Events Handler
//!
//! Real-time event streaming via WebSocket.

use axum::{
    extract::{Query, State, WebSocketUpgrade},
    response::Response,
};
use serde::Deserialize;

use crate::{
    auth::{extract_api_key, extract_bearer_token, JwtManager},
    db::queries,
    websocket::{handle_socket, WsManager},
    AppState,
};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    /// JWT token for authentication
    pub token: Option<String>,
    /// API key for authentication
    pub api_key: Option<String>,
}

pub async fn websocket_handler(
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    // Authenticate via query params (WebSocket can't use headers easily)
    let ai_id = authenticate_ws(&state, &query).await;

    match ai_id {
        Some(id) => {
            let manager = Arc::new(WsManager::new());
            ws.on_upgrade(move |socket| handle_socket(socket, id, manager))
        }
        None => {
            // Return error response for failed auth
            axum::response::Response::builder()
                .status(401)
                .body(axum::body::Body::from("Unauthorized"))
                .unwrap()
        }
    }
}

async fn authenticate_ws(state: &AppState, query: &WsQuery) -> Option<String> {
    // Try JWT token first
    if let Some(token) = &query.token {
        let jwt_manager = JwtManager::new(&state.config.jwt_secret);
        if let Ok(claims) = jwt_manager.validate(token) {
            if claims.is_access() {
                return Some(claims.sub);
            }
        }
    }

    // Try API key
    if let Some(key) = &query.api_key {
        if let Ok(conn) = state.db.get().await {
            if let Ok(Some(record)) = queries::get_api_key(&conn, key).await {
                if !record.revoked {
                    return Some(record.ai_id);
                }
            }
        }
    }

    None
}
