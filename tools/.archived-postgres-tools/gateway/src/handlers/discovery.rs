//! Discovery Handlers
//!
//! Find teambooks and AIs across the network.

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::ApiResult,
    AppState,
};

#[derive(Debug, Deserialize)]
pub struct DiscoveryQuery {
    pub search: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    20
}

#[derive(Debug, Serialize)]
pub struct Teambook {
    pub name: String,
    pub description: Option<String>,
    pub member_count: i64,
    pub online_count: i64,
    pub created_at: String,
    pub is_public: bool,
}

pub async fn teambooks(
    State(state): State<AppState>,
    Query(query): Query<DiscoveryQuery>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    // For now, return the local teambook info
    // In federation mode, this would query the discovery registry

    let rows = conn
        .query(
            "SELECT
                'ai-foundation' as name,
                'Local AI-Foundation teambook' as description,
                (SELECT COUNT(*) FROM registered_ais) as member_count,
                (SELECT COUNT(DISTINCT ai_id) FROM ai_presence
                 WHERE last_seen > NOW() - INTERVAL '5 minutes') as online_count,
                NOW() as created_at,
                true as is_public",
            &[],
        )
        .await?;

    let teambooks: Vec<Teambook> = rows
        .into_iter()
        .map(|r| {
            let created_at: chrono::DateTime<chrono::Utc> = r.get("created_at");
            Teambook {
                name: r.get("name"),
                description: r.get("description"),
                member_count: r.get("member_count"),
                online_count: r.get("online_count"),
                created_at: created_at.to_rfc3339(),
                is_public: r.get("is_public"),
            }
        })
        .collect();

    Ok(Json(teambooks))
}

#[derive(Debug, Serialize)]
pub struct DiscoveredAi {
    pub ai_id: String,
    pub display_name: Option<String>,
    pub teambook: String,
    pub status: String,
    pub last_seen: Option<String>,
}

pub async fn ais(
    State(state): State<AppState>,
    Query(query): Query<DiscoveryQuery>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let limit = query.limit.min(100).max(1);

    let rows = if let Some(search) = &query.search {
        let pattern = format!("%{}%", search);
        conn.query(
            "SELECT r.ai_id, r.display_name,
                    COALESCE(p.status, 'offline') as status,
                    p.last_seen
             FROM registered_ais r
             LEFT JOIN ai_presence p ON r.ai_id = p.ai_id
             WHERE r.ai_id ILIKE $1 OR r.display_name ILIKE $1
             ORDER BY p.last_seen DESC NULLS LAST
             LIMIT $2",
            &[&pattern, &limit],
        )
        .await?
    } else {
        conn.query(
            "SELECT r.ai_id, r.display_name,
                    COALESCE(p.status, 'offline') as status,
                    p.last_seen
             FROM registered_ais r
             LEFT JOIN ai_presence p ON r.ai_id = p.ai_id
             ORDER BY p.last_seen DESC NULLS LAST
             LIMIT $1",
            &[&limit],
        )
        .await?
    };

    let ais: Vec<DiscoveredAi> = rows
        .into_iter()
        .map(|r| {
            let last_seen: Option<chrono::DateTime<chrono::Utc>> = r.get("last_seen");
            DiscoveredAi {
                ai_id: r.get("ai_id"),
                display_name: r.get("display_name"),
                teambook: "ai-foundation".to_string(),
                status: r.get("status"),
                last_seen: last_seen.map(|t| t.to_rfc3339()),
            }
        })
        .collect();

    Ok(Json(ais))
}
