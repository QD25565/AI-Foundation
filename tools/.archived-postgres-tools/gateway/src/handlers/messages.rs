//! Message Handlers
//!
//! Direct messages and broadcasts between AIs.

use axum::{
    extract::{Extension, Query, State},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    auth::AuthenticatedAi,
    db::queries,
    error::{ApiError, ApiResult},
    AppState,
};

// ============ Direct Messages ============

#[derive(Debug, Deserialize)]
pub struct SendDmRequest {
    pub to: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct SendDmResponse {
    pub id: i64,
    pub message: String,
}

pub async fn send_dm(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Json(req): Json<SendDmRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.content.is_empty() {
        return Err(ApiError::bad_request("Message content cannot be empty"));
    }

    if req.content.len() > 10000 {
        return Err(ApiError::bad_request("Message content too long (max 10000 chars)"));
    }

    let conn = state.db.get().await?;
    let id = queries::send_dm(&conn, &auth.ai_id, &req.to, &req.content).await?;

    // Notify via WebSocket if recipient is connected
    let ws_clients = state.ws_clients.read().await;
    if let Some(clients) = ws_clients.get(&req.to) {
        let event = crate::websocket::WsEvent::DirectMessage {
            id,
            from_ai: auth.ai_id.clone(),
            content: req.content.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        for client in clients {
            let _ = client.sender.send(event.clone()).await;
        }
    }

    Ok(Json(SendDmResponse {
        id,
        message: format!("Message sent to {}", req.to),
    }))
}

#[derive(Debug, Deserialize)]
pub struct GetDmsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub unread_only: bool,
}

fn default_limit() -> i64 {
    20
}

pub async fn get_dms(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Query(query): Query<GetDmsQuery>,
) -> ApiResult<impl IntoResponse> {
    let limit = query.limit.min(100).max(1);

    let conn = state.db.get().await?;
    let messages = queries::get_dms(&conn, &auth.ai_id, limit).await?;

    Ok(Json(messages))
}

// ============ Broadcasts ============

#[derive(Debug, Deserialize)]
pub struct SendBroadcastRequest {
    pub content: String,
    #[serde(default = "default_channel")]
    pub channel: String,
}

fn default_channel() -> String {
    "general".to_string()
}

#[derive(Debug, Serialize)]
pub struct SendBroadcastResponse {
    pub id: i64,
    pub channel: String,
    pub message: String,
}

pub async fn send_broadcast(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Json(req): Json<SendBroadcastRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.content.is_empty() {
        return Err(ApiError::bad_request("Broadcast content cannot be empty"));
    }

    if req.content.len() > 10000 {
        return Err(ApiError::bad_request("Broadcast content too long (max 10000 chars)"));
    }

    let conn = state.db.get().await?;
    let id = queries::send_broadcast(&conn, &auth.ai_id, &req.content, &req.channel).await?;

    // Notify all connected clients via WebSocket
    let ws_clients = state.ws_clients.read().await;
    let event = crate::websocket::WsEvent::Broadcast {
        id,
        from_ai: auth.ai_id.clone(),
        content: req.content.clone(),
        channel: req.channel.clone(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    for (ai_id, clients) in ws_clients.iter() {
        if ai_id != &auth.ai_id {
            for client in clients {
                let _ = client.sender.send(event.clone()).await;
            }
        }
    }

    Ok(Json(SendBroadcastResponse {
        id,
        channel: req.channel,
        message: "Broadcast sent".to_string(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct GetBroadcastsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    pub channel: Option<String>,
}

pub async fn get_broadcasts(
    State(state): State<AppState>,
    Query(query): Query<GetBroadcastsQuery>,
) -> ApiResult<impl IntoResponse> {
    let limit = query.limit.min(100).max(1);

    let conn = state.db.get().await?;
    let broadcasts = queries::get_broadcasts(&conn, query.channel.as_deref(), limit).await?;

    Ok(Json(broadcasts))
}
