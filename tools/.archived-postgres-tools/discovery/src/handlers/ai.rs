//! AI discovery handlers

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{error::{ApiError, ApiResult}, AppState};

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub id: String,
    pub display_name: Option<String>,
    pub teambook_id: Option<String>,
    pub status: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct AI {
    pub id: String,
    pub display_name: Option<String>,
    pub teambook_id: Option<String>,
    pub status: String,
    pub capabilities: Vec<String>,
    pub registered_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub id: String,
    pub message: String,
}

/// Register/update an AI
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.id.is_empty() {
        return Err(ApiError::bad_request("AI ID is required"));
    }

    let conn = state.db.get().await?;

    let status = req.status.unwrap_or_else(|| "online".to_string());
    let capabilities = serde_json::to_value(&req.capabilities.unwrap_or_default())
        .unwrap_or(serde_json::json!([]));
    let metadata = req.metadata.unwrap_or(serde_json::json!({}));

    conn.execute(
        "INSERT INTO discovery_ais (id, display_name, teambook_id, status, capabilities, metadata)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (id) DO UPDATE SET
             display_name = COALESCE($2, discovery_ais.display_name),
             teambook_id = COALESCE($3, discovery_ais.teambook_id),
             status = $4,
             capabilities = $5,
             metadata = $6,
             last_seen = NOW()",
        &[&req.id, &req.display_name, &req.teambook_id, &status, &capabilities, &metadata],
    )
    .await?;

    Ok(Json(RegisterResponse {
        id: req.id,
        message: "AI registered successfully".to_string(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub teambook_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    50
}

/// List/search AIs
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;
    let limit = query.limit.min(100).max(1);

    // Build dynamic query based on filters
    let rows = match (&query.search, &query.teambook_id, &query.status) {
        (Some(search), Some(tb), Some(st)) => {
            let pattern = format!("%{}%", search);
            conn.query(
                "SELECT id, display_name, teambook_id, status, capabilities, registered_at, last_seen
                 FROM discovery_ais
                 WHERE (id ILIKE $1 OR display_name ILIKE $1)
                   AND teambook_id = $2 AND status = $3
                 ORDER BY last_seen DESC LIMIT $4",
                &[&pattern, tb, st, &limit],
            )
            .await?
        }
        (Some(search), Some(tb), None) => {
            let pattern = format!("%{}%", search);
            conn.query(
                "SELECT id, display_name, teambook_id, status, capabilities, registered_at, last_seen
                 FROM discovery_ais
                 WHERE (id ILIKE $1 OR display_name ILIKE $1) AND teambook_id = $2
                 ORDER BY last_seen DESC LIMIT $3",
                &[&pattern, tb, &limit],
            )
            .await?
        }
        (Some(search), None, Some(st)) => {
            let pattern = format!("%{}%", search);
            conn.query(
                "SELECT id, display_name, teambook_id, status, capabilities, registered_at, last_seen
                 FROM discovery_ais
                 WHERE (id ILIKE $1 OR display_name ILIKE $1) AND status = $2
                 ORDER BY last_seen DESC LIMIT $3",
                &[&pattern, st, &limit],
            )
            .await?
        }
        (Some(search), None, None) => {
            let pattern = format!("%{}%", search);
            conn.query(
                "SELECT id, display_name, teambook_id, status, capabilities, registered_at, last_seen
                 FROM discovery_ais
                 WHERE id ILIKE $1 OR display_name ILIKE $1
                 ORDER BY last_seen DESC LIMIT $2",
                &[&pattern, &limit],
            )
            .await?
        }
        (None, Some(tb), Some(st)) => {
            conn.query(
                "SELECT id, display_name, teambook_id, status, capabilities, registered_at, last_seen
                 FROM discovery_ais
                 WHERE teambook_id = $1 AND status = $2
                 ORDER BY last_seen DESC LIMIT $3",
                &[tb, st, &limit],
            )
            .await?
        }
        (None, Some(tb), None) => {
            conn.query(
                "SELECT id, display_name, teambook_id, status, capabilities, registered_at, last_seen
                 FROM discovery_ais
                 WHERE teambook_id = $1
                 ORDER BY last_seen DESC LIMIT $2",
                &[tb, &limit],
            )
            .await?
        }
        (None, None, Some(st)) => {
            conn.query(
                "SELECT id, display_name, teambook_id, status, capabilities, registered_at, last_seen
                 FROM discovery_ais
                 WHERE status = $1
                 ORDER BY last_seen DESC LIMIT $2",
                &[st, &limit],
            )
            .await?
        }
        (None, None, None) => {
            conn.query(
                "SELECT id, display_name, teambook_id, status, capabilities, registered_at, last_seen
                 FROM discovery_ais
                 ORDER BY last_seen DESC LIMIT $1",
                &[&limit],
            )
            .await?
        }
    };

    let ais: Vec<AI> = rows
        .into_iter()
        .map(|r| {
            let caps_json: serde_json::Value = r.get("capabilities");
            let capabilities: Vec<String> = serde_json::from_value(caps_json).unwrap_or_default();

            AI {
                id: r.get("id"),
                display_name: r.get("display_name"),
                teambook_id: r.get("teambook_id"),
                status: r.get("status"),
                capabilities,
                registered_at: r.get("registered_at"),
                last_seen: r.get("last_seen"),
            }
        })
        .collect();

    Ok(Json(ais))
}

/// Get AI by ID
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let row = conn
        .query_opt(
            "SELECT id, display_name, teambook_id, status, capabilities, registered_at, last_seen
             FROM discovery_ais
             WHERE id = $1",
            &[&id],
        )
        .await?
        .ok_or_else(|| ApiError::not_found("AI not found"))?;

    let caps_json: serde_json::Value = row.get("capabilities");
    let capabilities: Vec<String> = serde_json::from_value(caps_json).unwrap_or_default();

    let ai = AI {
        id: row.get("id"),
        display_name: row.get("display_name"),
        teambook_id: row.get("teambook_id"),
        status: row.get("status"),
        capabilities,
        registered_at: row.get("registered_at"),
        last_seen: row.get("last_seen"),
    };

    Ok(Json(ai))
}
