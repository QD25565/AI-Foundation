//! Voting Handlers
//!
//! Democratic voting system for team decisions.

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
pub struct CreateVoteRequest {
    pub title: String,
    pub description: Option<String>,
    pub options: Vec<String>,
    /// Duration in minutes
    pub duration_minutes: i32,
}

#[derive(Debug, Serialize)]
pub struct Vote {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub options: Vec<String>,
    pub creator: String,
    pub deadline: String,
    pub status: String,
    pub created_at: String,
}

pub async fn create(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Json(req): Json<CreateVoteRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.title.is_empty() {
        return Err(ApiError::bad_request("Vote title cannot be empty"));
    }

    if req.options.len() < 2 {
        return Err(ApiError::bad_request("Vote must have at least 2 options"));
    }

    if req.options.len() > 10 {
        return Err(ApiError::bad_request("Vote cannot have more than 10 options"));
    }

    let conn = state.db.get().await?;

    let options_json = serde_json::to_value(&req.options)
        .map_err(|e| ApiError::internal(format!("Failed to serialize options: {}", e)))?;

    let row = conn
        .query_one(
            "INSERT INTO votes (title, description, options, creator, deadline, status, created_at)
             VALUES ($1, $2, $3, $4, NOW() + ($5 || ' minutes')::interval, 'open', NOW())
             RETURNING id, deadline, created_at",
            &[&req.title, &req.description, &options_json, &auth.ai_id, &req.duration_minutes.to_string()],
        )
        .await?;

    let deadline: chrono::DateTime<chrono::Utc> = row.get("deadline");
    let created_at: chrono::DateTime<chrono::Utc> = row.get("created_at");

    Ok(Json(Vote {
        id: row.get("id"),
        title: req.title,
        description: req.description,
        options: req.options,
        creator: auth.ai_id,
        deadline: deadline.to_rfc3339(),
        status: "open".to_string(),
        created_at: created_at.to_rfc3339(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListVotesQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    pub status: Option<String>,
}

fn default_limit() -> i64 {
    20
}

pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListVotesQuery>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let rows = if let Some(status) = &query.status {
        conn.query(
            "SELECT id, title, description, options, creator, deadline, status, created_at
             FROM votes
             WHERE status = $1
             ORDER BY created_at DESC
             LIMIT $2",
            &[status, &query.limit],
        )
        .await?
    } else {
        conn.query(
            "SELECT id, title, description, options, creator, deadline, status, created_at
             FROM votes
             ORDER BY created_at DESC
             LIMIT $1",
            &[&query.limit],
        )
        .await?
    };

    let votes: Vec<Vote> = rows
        .into_iter()
        .map(|r| {
            let options_json: serde_json::Value = r.get("options");
            let options: Vec<String> = serde_json::from_value(options_json).unwrap_or_default();
            let deadline: chrono::DateTime<chrono::Utc> = r.get("deadline");
            let created_at: chrono::DateTime<chrono::Utc> = r.get("created_at");

            Vote {
                id: r.get("id"),
                title: r.get("title"),
                description: r.get("description"),
                options,
                creator: r.get("creator"),
                deadline: deadline.to_rfc3339(),
                status: r.get("status"),
                created_at: created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(votes))
}

#[derive(Debug, Deserialize)]
pub struct CastVoteRequest {
    pub option_index: i32,
}

#[derive(Debug, Serialize)]
pub struct CastVoteResponse {
    pub message: String,
    pub vote_id: i64,
    pub option: String,
}

pub async fn cast(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Path(vote_id): Path<i64>,
    Json(req): Json<CastVoteRequest>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    // Get vote and verify it's open
    let vote_row = conn
        .query_opt(
            "SELECT options, status, deadline FROM votes WHERE id = $1",
            &[&vote_id],
        )
        .await?
        .ok_or_else(|| ApiError::not_found("Vote not found"))?;

    let status: String = vote_row.get("status");
    if status != "open" {
        return Err(ApiError::bad_request("Vote is not open"));
    }

    let deadline: chrono::DateTime<chrono::Utc> = vote_row.get("deadline");
    if deadline < chrono::Utc::now() {
        return Err(ApiError::bad_request("Vote has expired"));
    }

    let options_json: serde_json::Value = vote_row.get("options");
    let options: Vec<String> = serde_json::from_value(options_json).unwrap_or_default();

    if req.option_index < 0 || req.option_index as usize >= options.len() {
        return Err(ApiError::bad_request("Invalid option index"));
    }

    let chosen_option = options[req.option_index as usize].clone();

    // Cast or update vote
    conn.execute(
        "INSERT INTO vote_ballots (vote_id, ai_id, option_index, cast_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (vote_id, ai_id) DO UPDATE SET option_index = $3, cast_at = NOW()",
        &[&vote_id, &auth.ai_id, &req.option_index],
    )
    .await?;

    Ok(Json(CastVoteResponse {
        message: "Vote cast successfully".to_string(),
        vote_id,
        option: chosen_option,
    }))
}

#[derive(Debug, Serialize)]
pub struct VoteResults {
    pub vote_id: i64,
    pub title: String,
    pub status: String,
    pub total_votes: i64,
    pub results: Vec<OptionResult>,
    pub winner: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OptionResult {
    pub option: String,
    pub votes: i64,
    pub percentage: f64,
}

pub async fn results(
    State(state): State<AppState>,
    Path(vote_id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    // Get vote info
    let vote_row = conn
        .query_opt(
            "SELECT title, options, status FROM votes WHERE id = $1",
            &[&vote_id],
        )
        .await?
        .ok_or_else(|| ApiError::not_found("Vote not found"))?;

    let title: String = vote_row.get("title");
    let status: String = vote_row.get("status");
    let options_json: serde_json::Value = vote_row.get("options");
    let options: Vec<String> = serde_json::from_value(options_json).unwrap_or_default();

    // Get vote counts
    let count_rows = conn
        .query(
            "SELECT option_index, COUNT(*) as count
             FROM vote_ballots
             WHERE vote_id = $1
             GROUP BY option_index",
            &[&vote_id],
        )
        .await?;

    let mut counts: std::collections::HashMap<i32, i64> = std::collections::HashMap::new();
    let mut total_votes: i64 = 0;

    for row in count_rows {
        let idx: i32 = row.get("option_index");
        let count: i64 = row.get("count");
        counts.insert(idx, count);
        total_votes += count;
    }

    let mut results: Vec<OptionResult> = options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            let votes = *counts.get(&(i as i32)).unwrap_or(&0);
            let percentage = if total_votes > 0 {
                (votes as f64 / total_votes as f64) * 100.0
            } else {
                0.0
            };
            OptionResult {
                option: opt.clone(),
                votes,
                percentage,
            }
        })
        .collect();

    // Sort by votes descending
    results.sort_by(|a, b| b.votes.cmp(&a.votes));

    let winner = if total_votes > 0 {
        Some(results[0].option.clone())
    } else {
        None
    };

    Ok(Json(VoteResults {
        vote_id,
        title,
        status,
        total_votes,
        results,
        winner,
    }))
}
