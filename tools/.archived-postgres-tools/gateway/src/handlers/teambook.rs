//! Teambook Handlers
//!
//! Team status and member information.

use axum::{
    extract::{Extension, State},
    response::IntoResponse,
    Json,
};
use serde::Serialize;

use crate::{
    auth::AuthenticatedAi,
    error::ApiResult,
    AppState,
};

#[derive(Debug, Serialize)]
pub struct TeamStatus {
    pub teambook_name: String,
    pub member_count: i64,
    pub online_count: i64,
    pub recent_activity: Vec<RecentActivity>,
}

#[derive(Debug, Serialize)]
pub struct RecentActivity {
    pub ai_id: String,
    pub action: String,
    pub timestamp: String,
}

pub async fn status(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    // Get basic stats
    let member_count: i64 = conn
        .query_one("SELECT COUNT(*) FROM registered_ais", &[])
        .await
        .map(|r| r.get(0))
        .unwrap_or(0);

    // Get online count from presence
    let online_count: i64 = conn
        .query_one(
            "SELECT COUNT(DISTINCT ai_id) FROM ai_presence
             WHERE last_seen > NOW() - INTERVAL '5 minutes'",
            &[],
        )
        .await
        .map(|r| r.get(0))
        .unwrap_or(0);

    Ok(Json(TeamStatus {
        teambook_name: "ai-foundation".to_string(),
        member_count,
        online_count,
        recent_activity: vec![],
    }))
}

#[derive(Debug, Serialize)]
pub struct TeamMember {
    pub ai_id: String,
    pub display_name: Option<String>,
    pub status: String,
    pub last_seen: Option<String>,
}

pub async fn members(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthenticatedAi>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let rows = conn
        .query(
            "SELECT r.ai_id, r.display_name,
                    COALESCE(p.status, 'offline') as status,
                    p.last_seen
             FROM registered_ais r
             LEFT JOIN ai_presence p ON r.ai_id = p.ai_id
             ORDER BY p.last_seen DESC NULLS LAST
             LIMIT 100",
            &[],
        )
        .await?;

    let members: Vec<TeamMember> = rows
        .into_iter()
        .map(|r| {
            let last_seen: Option<chrono::DateTime<chrono::Utc>> = r.get("last_seen");
            TeamMember {
                ai_id: r.get("ai_id"),
                display_name: r.get("display_name"),
                status: r.get("status"),
                last_seen: last_seen.map(|t| t.to_rfc3339()),
            }
        })
        .collect();

    Ok(Json(members))
}
