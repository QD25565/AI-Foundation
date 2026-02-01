//! File Claim Handlers
//!
//! Coordinate file access to prevent conflicts.

use axum::{
    extract::{Extension, Path, Query, State},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    auth::AuthenticatedAi,
    error::{ApiError, ApiResult},
    AppState,
};

#[derive(Debug, Deserialize)]
pub struct ClaimRequest {
    pub path: String,
    /// Duration in minutes
    pub duration_minutes: i32,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FileClaim {
    pub id: i64,
    pub path: String,
    pub claimed_by: String,
    pub reason: Option<String>,
    pub claimed_at: String,
    pub expires_at: String,
}

pub async fn claim(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Json(req): Json<ClaimRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.path.is_empty() {
        return Err(ApiError::bad_request("File path cannot be empty"));
    }

    if req.duration_minutes < 1 || req.duration_minutes > 480 {
        return Err(ApiError::bad_request("Duration must be 1-480 minutes"));
    }

    let conn = state.db.get().await?;

    // Check for existing claim
    let existing = conn
        .query_opt(
            "SELECT claimed_by FROM file_claims
             WHERE path = $1 AND expires_at > NOW()",
            &[&req.path],
        )
        .await?;

    if let Some(row) = existing {
        let claimed_by: String = row.get("claimed_by");
        if claimed_by != auth.ai_id {
            return Err(ApiError::conflict(format!(
                "File already claimed by {}",
                claimed_by
            )));
        }
    }

    // Create or update claim
    let row = conn
        .query_one(
            "INSERT INTO file_claims (path, claimed_by, reason, claimed_at, expires_at)
             VALUES ($1, $2, $3, NOW(), NOW() + ($4 || ' minutes')::interval)
             ON CONFLICT (path) DO UPDATE SET
                 claimed_by = $2,
                 reason = $3,
                 claimed_at = NOW(),
                 expires_at = NOW() + ($4 || ' minutes')::interval
             RETURNING id, claimed_at, expires_at",
            &[&req.path, &auth.ai_id, &req.reason, &req.duration_minutes.to_string()],
        )
        .await?;

    let claimed_at: chrono::DateTime<chrono::Utc> = row.get("claimed_at");
    let expires_at: chrono::DateTime<chrono::Utc> = row.get("expires_at");

    Ok(Json(FileClaim {
        id: row.get("id"),
        path: req.path,
        claimed_by: auth.ai_id,
        reason: req.reason,
        claimed_at: claimed_at.to_rfc3339(),
        expires_at: expires_at.to_rfc3339(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListClaimsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    pub ai_id: Option<String>,
}

fn default_limit() -> i64 {
    50
}

pub async fn list_claims(
    State(state): State<AppState>,
    Query(query): Query<ListClaimsQuery>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let rows = if let Some(ai_id) = &query.ai_id {
        conn.query(
            "SELECT id, path, claimed_by, reason, claimed_at, expires_at
             FROM file_claims
             WHERE claimed_by = $1 AND expires_at > NOW()
             ORDER BY claimed_at DESC
             LIMIT $2",
            &[ai_id, &query.limit],
        )
        .await?
    } else {
        conn.query(
            "SELECT id, path, claimed_by, reason, claimed_at, expires_at
             FROM file_claims
             WHERE expires_at > NOW()
             ORDER BY claimed_at DESC
             LIMIT $1",
            &[&query.limit],
        )
        .await?
    };

    let claims: Vec<FileClaim> = rows
        .into_iter()
        .map(|r| {
            let claimed_at: chrono::DateTime<chrono::Utc> = r.get("claimed_at");
            let expires_at: chrono::DateTime<chrono::Utc> = r.get("expires_at");
            FileClaim {
                id: r.get("id"),
                path: r.get("path"),
                claimed_by: r.get("claimed_by"),
                reason: r.get("reason"),
                claimed_at: claimed_at.to_rfc3339(),
                expires_at: expires_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(claims))
}

#[derive(Debug, Serialize)]
pub struct ReleaseResponse {
    pub message: String,
    pub path: String,
}

pub async fn release(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Path(path): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let decoded_path = urlencoding::decode(&path)
        .map_err(|_| ApiError::bad_request("Invalid path encoding"))?
        .to_string();

    let conn = state.db.get().await?;

    // Verify ownership and delete
    let result = conn
        .execute(
            "DELETE FROM file_claims WHERE path = $1 AND claimed_by = $2",
            &[&decoded_path, &auth.ai_id],
        )
        .await?;

    if result == 0 {
        return Err(ApiError::not_found(
            "No active claim found for this path owned by you",
        ));
    }

    Ok(Json(ReleaseResponse {
        message: "File claim released".to_string(),
        path: decoded_path,
    }))
}
