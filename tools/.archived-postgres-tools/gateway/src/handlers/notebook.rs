//! Notebook Handlers
//!
//! Personal AI memory (proxied to local notebook service).
//! Each AI's notebook is private and isolated.

use axum::{
    extract::{Extension, Query, State},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    auth::AuthenticatedAi,
    error::{ApiError, ApiResult},
    AppState,
};

/// Notebook operations are proxied to the local notebook service
/// This provides HTTP access while keeping data local

#[derive(Debug, Deserialize)]
pub struct RememberRequest {
    pub content: String,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct RememberResponse {
    pub id: i64,
    pub message: String,
}

pub async fn remember(
    State(_state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Json(req): Json<RememberRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.content.is_empty() {
        return Err(ApiError::bad_request("Content cannot be empty"));
    }

    // In production, this would call the local notebook-cli or library
    // For now, we'll return a stub response indicating the intended behavior

    // Construct tags string
    let tags_str = req.tags.as_ref()
        .map(|t| t.join(","))
        .unwrap_or_default();

    // TODO: Call notebook-cli remember or use notebook-rs library directly
    // let output = Command::new("notebook-cli")
    //     .args(["remember", &req.content, "--tags", &tags_str, "--ai-id", &auth.ai_id])
    //     .output()?;

    tracing::info!(
        "Notebook remember for {}: {} (tags: {})",
        auth.ai_id,
        &req.content[..req.content.len().min(50)],
        tags_str
    );

    Ok(Json(RememberResponse {
        id: 0, // Would be returned from actual notebook
        message: "Note saved to local notebook".to_string(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct RecallQuery {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    10
}

#[derive(Debug, Serialize)]
pub struct RecallResponse {
    pub notes: Vec<Note>,
    pub query: String,
}

#[derive(Debug, Serialize)]
pub struct Note {
    pub id: i64,
    pub content: String,
    pub tags: Vec<String>,
    pub score: f64,
    pub created_at: String,
}

pub async fn recall(
    State(_state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Query(query): Query<RecallQuery>,
) -> ApiResult<impl IntoResponse> {
    if query.query.is_empty() {
        return Err(ApiError::bad_request("Query cannot be empty"));
    }

    // TODO: Call notebook-cli recall or use notebook-rs library directly
    // let output = Command::new("notebook-cli")
    //     .args(["recall", &query.query, "--limit", &query.limit.to_string(), "--ai-id", &auth.ai_id])
    //     .output()?;

    tracing::info!(
        "Notebook recall for {}: {} (limit: {})",
        auth.ai_id,
        query.query,
        query.limit
    );

    Ok(Json(RecallResponse {
        notes: vec![], // Would be returned from actual notebook
        query: query.query,
    }))
}

#[derive(Debug, Serialize)]
pub struct NotebookStats {
    pub ai_id: String,
    pub total_notes: i64,
    pub pinned_notes: i64,
    pub total_embeddings: i64,
    pub graph_edges: i64,
    pub vault_entries: i64,
}

pub async fn stats(
    State(_state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
) -> ApiResult<impl IntoResponse> {
    // TODO: Call notebook-cli stats or use notebook-rs library directly

    tracing::info!("Notebook stats for {}", auth.ai_id);

    Ok(Json(NotebookStats {
        ai_id: auth.ai_id,
        total_notes: 0,
        pinned_notes: 0,
        total_embeddings: 0,
        graph_edges: 0,
        vault_entries: 0,
    }))
}
