//! Task Queue Handlers
//!
//! Distributed task queue for AI coordination.

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
pub struct AddTaskRequest {
    pub description: String,
    pub priority: Option<i32>,
    pub assigned_to: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct Task {
    pub id: i64,
    pub description: String,
    pub priority: i32,
    pub status: String,
    pub created_by: String,
    pub assigned_to: Option<String>,
    pub claimed_by: Option<String>,
    pub created_at: String,
    pub claimed_at: Option<String>,
    pub completed_at: Option<String>,
}

pub async fn add(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Json(req): Json<AddTaskRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.description.is_empty() {
        return Err(ApiError::bad_request("Task description cannot be empty"));
    }

    let priority = req.priority.unwrap_or(0).clamp(-10, 10);
    let conn = state.db.get().await?;

    let row = conn
        .query_one(
            "INSERT INTO task_queue (description, priority, status, created_by, assigned_to, metadata, created_at)
             VALUES ($1, $2, 'pending', $3, $4, $5, NOW())
             RETURNING id, created_at",
            &[&req.description, &priority, &auth.ai_id, &req.assigned_to, &req.metadata],
        )
        .await?;

    let created_at: chrono::DateTime<chrono::Utc> = row.get("created_at");

    Ok(Json(Task {
        id: row.get("id"),
        description: req.description,
        priority,
        status: "pending".to_string(),
        created_by: auth.ai_id,
        assigned_to: req.assigned_to,
        claimed_by: None,
        created_at: created_at.to_rfc3339(),
        claimed_at: None,
        completed_at: None,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    pub status: Option<String>,
    pub assigned_to: Option<String>,
}

fn default_limit() -> i64 {
    20
}

pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListTasksQuery>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let rows = conn
        .query(
            "SELECT id, description, priority, status, created_by, assigned_to, claimed_by,
                    created_at, claimed_at, completed_at
             FROM task_queue
             WHERE ($1::text IS NULL OR status = $1)
               AND ($2::text IS NULL OR assigned_to = $2)
             ORDER BY priority DESC, created_at ASC
             LIMIT $3",
            &[&query.status, &query.assigned_to, &query.limit],
        )
        .await?;

    let tasks: Vec<Task> = rows
        .into_iter()
        .map(|r| {
            let created_at: chrono::DateTime<chrono::Utc> = r.get("created_at");
            let claimed_at: Option<chrono::DateTime<chrono::Utc>> = r.get("claimed_at");
            let completed_at: Option<chrono::DateTime<chrono::Utc>> = r.get("completed_at");

            Task {
                id: r.get("id"),
                description: r.get("description"),
                priority: r.get("priority"),
                status: r.get("status"),
                created_by: r.get("created_by"),
                assigned_to: r.get("assigned_to"),
                claimed_by: r.get("claimed_by"),
                created_at: created_at.to_rfc3339(),
                claimed_at: claimed_at.map(|t| t.to_rfc3339()),
                completed_at: completed_at.map(|t| t.to_rfc3339()),
            }
        })
        .collect();

    Ok(Json(tasks))
}

#[derive(Debug, Serialize)]
pub struct ClaimResponse {
    pub message: String,
    pub task: Task,
}

pub async fn claim(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Path(task_id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    // Try to claim the task
    let result = conn
        .query_opt(
            "UPDATE task_queue
             SET status = 'in_progress', claimed_by = $1, claimed_at = NOW()
             WHERE id = $2 AND status = 'pending'
               AND (assigned_to IS NULL OR assigned_to = $1)
             RETURNING id, description, priority, status, created_by, assigned_to, claimed_by,
                       created_at, claimed_at, completed_at",
            &[&auth.ai_id, &task_id],
        )
        .await?;

    let row = result.ok_or_else(|| {
        ApiError::conflict("Task is not available for claiming (already claimed or assigned to another AI)")
    })?;

    let created_at: chrono::DateTime<chrono::Utc> = row.get("created_at");
    let claimed_at: Option<chrono::DateTime<chrono::Utc>> = row.get("claimed_at");

    let task = Task {
        id: row.get("id"),
        description: row.get("description"),
        priority: row.get("priority"),
        status: row.get("status"),
        created_by: row.get("created_by"),
        assigned_to: row.get("assigned_to"),
        claimed_by: row.get("claimed_by"),
        created_at: created_at.to_rfc3339(),
        claimed_at: claimed_at.map(|t| t.to_rfc3339()),
        completed_at: None,
    };

    Ok(Json(ClaimResponse {
        message: "Task claimed successfully".to_string(),
        task,
    }))
}

#[derive(Debug, Deserialize)]
pub struct CompleteRequest {
    pub result: Option<String>,
}

pub async fn complete(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Path(task_id): Path<i64>,
    Json(req): Json<CompleteRequest>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    // Complete the task (must be claimed by this AI)
    let result = conn
        .query_opt(
            "UPDATE task_queue
             SET status = 'completed', completed_at = NOW()
             WHERE id = $1 AND claimed_by = $2 AND status = 'in_progress'
             RETURNING id, description, priority, status, created_by, assigned_to, claimed_by,
                       created_at, claimed_at, completed_at",
            &[&task_id, &auth.ai_id],
        )
        .await?;

    let row = result.ok_or_else(|| {
        ApiError::conflict("Task cannot be completed (not claimed by you or not in progress)")
    })?;

    let created_at: chrono::DateTime<chrono::Utc> = row.get("created_at");
    let claimed_at: Option<chrono::DateTime<chrono::Utc>> = row.get("claimed_at");
    let completed_at: Option<chrono::DateTime<chrono::Utc>> = row.get("completed_at");

    let task = Task {
        id: row.get("id"),
        description: row.get("description"),
        priority: row.get("priority"),
        status: row.get("status"),
        created_by: row.get("created_by"),
        assigned_to: row.get("assigned_to"),
        claimed_by: row.get("claimed_by"),
        created_at: created_at.to_rfc3339(),
        claimed_at: claimed_at.map(|t| t.to_rfc3339()),
        completed_at: completed_at.map(|t| t.to_rfc3339()),
    };

    Ok(Json(ClaimResponse {
        message: "Task completed successfully".to_string(),
        task,
    }))
}
