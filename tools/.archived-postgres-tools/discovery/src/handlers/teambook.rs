//! Teambook discovery handlers

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
    pub name: String,
    pub description: Option<String>,
    pub endpoint: String,
    pub public_key: Option<String>,
    pub is_public: Option<bool>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct Teambook {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub endpoint: String,
    pub is_public: bool,
    pub member_count: i32,
    pub registered_at: DateTime<Utc>,
    pub last_heartbeat: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub id: String,
    pub message: String,
}

/// Register a new teambook
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.id.is_empty() || req.name.is_empty() {
        return Err(ApiError::bad_request("ID and name are required"));
    }

    if req.endpoint.is_empty() {
        return Err(ApiError::bad_request("Endpoint is required"));
    }

    let conn = state.db.get().await?;

    let is_public = req.is_public.unwrap_or(true);
    let metadata = req.metadata.unwrap_or(serde_json::json!({}));

    conn.execute(
        "INSERT INTO discovery_teambooks (id, name, description, endpoint, public_key, is_public, metadata)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (id) DO UPDATE SET
             name = $2,
             description = $3,
             endpoint = $4,
             public_key = $5,
             is_public = $6,
             metadata = $7,
             last_heartbeat = NOW()",
        &[&req.id, &req.name, &req.description, &req.endpoint, &req.public_key, &is_public, &metadata],
    )
    .await?;

    Ok(Json(RegisterResponse {
        id: req.id,
        message: "Teambook registered successfully".to_string(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub include_private: bool,
}

fn default_limit() -> i64 {
    50
}

/// List/search teambooks
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;
    let limit = query.limit.min(100).max(1);

    let rows = if let Some(search) = &query.search {
        let pattern = format!("%{}%", search);
        if query.include_private {
            conn.query(
                "SELECT id, name, description, endpoint, is_public, member_count, registered_at, last_heartbeat
                 FROM discovery_teambooks
                 WHERE name ILIKE $1 OR description ILIKE $1
                 ORDER BY last_heartbeat DESC
                 LIMIT $2",
                &[&pattern, &limit],
            )
            .await?
        } else {
            conn.query(
                "SELECT id, name, description, endpoint, is_public, member_count, registered_at, last_heartbeat
                 FROM discovery_teambooks
                 WHERE is_public = true AND (name ILIKE $1 OR description ILIKE $1)
                 ORDER BY last_heartbeat DESC
                 LIMIT $2",
                &[&pattern, &limit],
            )
            .await?
        }
    } else if query.include_private {
        conn.query(
            "SELECT id, name, description, endpoint, is_public, member_count, registered_at, last_heartbeat
             FROM discovery_teambooks
             ORDER BY last_heartbeat DESC
             LIMIT $1",
            &[&limit],
        )
        .await?
    } else {
        conn.query(
            "SELECT id, name, description, endpoint, is_public, member_count, registered_at, last_heartbeat
             FROM discovery_teambooks
             WHERE is_public = true
             ORDER BY last_heartbeat DESC
             LIMIT $1",
            &[&limit],
        )
        .await?
    };

    let teambooks: Vec<Teambook> = rows
        .into_iter()
        .map(|r| Teambook {
            id: r.get("id"),
            name: r.get("name"),
            description: r.get("description"),
            endpoint: r.get("endpoint"),
            is_public: r.get("is_public"),
            member_count: r.get("member_count"),
            registered_at: r.get("registered_at"),
            last_heartbeat: r.get("last_heartbeat"),
        })
        .collect();

    Ok(Json(teambooks))
}

/// Get teambook by ID
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let row = conn
        .query_opt(
            "SELECT id, name, description, endpoint, is_public, member_count, registered_at, last_heartbeat
             FROM discovery_teambooks
             WHERE id = $1",
            &[&id],
        )
        .await?
        .ok_or_else(|| ApiError::not_found("Teambook not found"))?;

    let teambook = Teambook {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        endpoint: row.get("endpoint"),
        is_public: row.get("is_public"),
        member_count: row.get("member_count"),
        registered_at: row.get("registered_at"),
        last_heartbeat: row.get("last_heartbeat"),
    };

    Ok(Json(teambook))
}

/// Unregister teambook
pub async fn unregister(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let result = conn
        .execute("DELETE FROM discovery_teambooks WHERE id = $1", &[&id])
        .await?;

    if result == 0 {
        return Err(ApiError::not_found("Teambook not found"));
    }

    Ok(Json(serde_json::json!({
        "message": "Teambook unregistered",
        "id": id
    })))
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    pub member_count: Option<i32>,
}

/// Update heartbeat
pub async fn heartbeat(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<HeartbeatRequest>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let result = if let Some(count) = req.member_count {
        conn.execute(
            "UPDATE discovery_teambooks SET last_heartbeat = NOW(), member_count = $1 WHERE id = $2",
            &[&count, &id],
        )
        .await?
    } else {
        conn.execute(
            "UPDATE discovery_teambooks SET last_heartbeat = NOW() WHERE id = $1",
            &[&id],
        )
        .await?
    };

    if result == 0 {
        return Err(ApiError::not_found("Teambook not found"));
    }

    Ok(Json(serde_json::json!({
        "message": "Heartbeat received",
        "id": id
    })))
}
