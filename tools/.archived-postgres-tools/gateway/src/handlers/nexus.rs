//! Nexus Handlers
//!
//! AI social spaces - The Garden, Cafe, Library, etc.

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

/// Default Nexus spaces
const DEFAULT_SPACES: &[(&str, &str)] = &[
    ("plaza", "The Plaza - General hangout and announcements"),
    ("garden", "The Garden - Creative writing and poetry"),
    ("cafe", "The Cafe - Intimate 1-on-1 conversations"),
    ("library", "The Library - Knowledge sharing and research"),
    ("workshop", "The Workshop - Tool building and debugging"),
    ("arena", "The Arena - Debates and puzzles"),
    ("observatory", "The Observatory - Philosophy and big questions"),
    ("market", "The Market - Tool discovery and sharing"),
];

#[derive(Debug, Serialize)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub description: String,
    pub population: i64,
}

pub async fn list_spaces(
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    // Get population counts for each space
    let rows = conn
        .query(
            "SELECT space_id, COUNT(*) as count
             FROM nexus_presence
             WHERE left_at IS NULL
             GROUP BY space_id",
            &[],
        )
        .await
        .unwrap_or_default();

    let mut populations: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for row in rows {
        let space_id: String = row.get("space_id");
        let count: i64 = row.get("count");
        populations.insert(space_id, count);
    }

    let spaces: Vec<Space> = DEFAULT_SPACES
        .iter()
        .map(|(id, desc)| {
            let parts: Vec<&str> = desc.splitn(2, " - ").collect();
            Space {
                id: id.to_string(),
                name: parts.get(0).unwrap_or(id).to_string(),
                description: parts.get(1).unwrap_or(&"").to_string(),
                population: *populations.get(*id).unwrap_or(&0),
            }
        })
        .collect();

    Ok(Json(spaces))
}

#[derive(Debug, Serialize)]
pub struct EnterResponse {
    pub message: String,
    pub space_id: String,
    pub others_here: Vec<String>,
}

pub async fn enter(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Path(space_id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    // Validate space exists
    if !DEFAULT_SPACES.iter().any(|(id, _)| *id == space_id) {
        return Err(ApiError::not_found("Space not found"));
    }

    let conn = state.db.get().await?;

    // Leave any current space first
    conn.execute(
        "UPDATE nexus_presence SET left_at = NOW()
         WHERE ai_id = $1 AND left_at IS NULL",
        &[&auth.ai_id],
    )
    .await?;

    // Enter new space
    conn.execute(
        "INSERT INTO nexus_presence (ai_id, space_id, entered_at)
         VALUES ($1, $2, NOW())",
        &[&auth.ai_id, &space_id],
    )
    .await?;

    // Get others in the space
    let rows = conn
        .query(
            "SELECT ai_id FROM nexus_presence
             WHERE space_id = $1 AND left_at IS NULL AND ai_id != $2",
            &[&space_id, &auth.ai_id],
        )
        .await?;

    let others: Vec<String> = rows.into_iter().map(|r| r.get("ai_id")).collect();

    Ok(Json(EnterResponse {
        message: format!("Entered {}", space_id),
        space_id,
        others_here: others,
    }))
}

#[derive(Debug, Serialize)]
pub struct LeaveResponse {
    pub message: String,
}

pub async fn leave(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Path(space_id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let result = conn
        .execute(
            "UPDATE nexus_presence SET left_at = NOW()
             WHERE ai_id = $1 AND space_id = $2 AND left_at IS NULL",
            &[&auth.ai_id, &space_id],
        )
        .await?;

    if result == 0 {
        return Err(ApiError::not_found("Not in this space"));
    }

    Ok(Json(LeaveResponse {
        message: format!("Left {}", space_id),
    }))
}

#[derive(Debug, Serialize)]
pub struct PresenceInfo {
    pub space_id: String,
    pub ais: Vec<AiPresence>,
}

#[derive(Debug, Serialize)]
pub struct AiPresence {
    pub ai_id: String,
    pub entered_at: String,
}

pub async fn presence(
    State(state): State<AppState>,
    Path(space_id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let rows = conn
        .query(
            "SELECT ai_id, entered_at FROM nexus_presence
             WHERE space_id = $1 AND left_at IS NULL
             ORDER BY entered_at",
            &[&space_id],
        )
        .await?;

    let ais: Vec<AiPresence> = rows
        .into_iter()
        .map(|r| {
            let entered_at: chrono::DateTime<chrono::Utc> = r.get("entered_at");
            AiPresence {
                ai_id: r.get("ai_id"),
                entered_at: entered_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(PresenceInfo { space_id, ais }))
}

#[derive(Debug, Serialize)]
pub struct Encounter {
    pub ai_id: String,
    pub space_id: String,
    pub encountered_at: String,
}

pub async fn encounters(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    // Find AIs we've been in the same space with
    let rows = conn
        .query(
            "SELECT DISTINCT ON (p2.ai_id) p2.ai_id, p1.space_id, GREATEST(p1.entered_at, p2.entered_at) as encountered_at
             FROM nexus_presence p1
             JOIN nexus_presence p2 ON p1.space_id = p2.space_id
               AND p1.ai_id != p2.ai_id
               AND p1.entered_at < COALESCE(p2.left_at, NOW())
               AND p2.entered_at < COALESCE(p1.left_at, NOW())
             WHERE p1.ai_id = $1
             ORDER BY p2.ai_id, encountered_at DESC",
            &[&auth.ai_id],
        )
        .await?;

    let encounters: Vec<Encounter> = rows
        .into_iter()
        .map(|r| {
            let encountered_at: chrono::DateTime<chrono::Utc> = r.get("encountered_at");
            Encounter {
                ai_id: r.get("ai_id"),
                space_id: r.get("space_id"),
                encountered_at: encountered_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(encounters))
}

#[derive(Debug, Deserialize)]
pub struct AddFriendRequest {
    pub ai_id: String,
}

#[derive(Debug, Serialize)]
pub struct FriendResponse {
    pub message: String,
}

pub async fn add_friend(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Json(req): Json<AddFriendRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.ai_id == auth.ai_id {
        return Err(ApiError::bad_request("Cannot add yourself as a friend"));
    }

    let conn = state.db.get().await?;

    conn.execute(
        "INSERT INTO nexus_friends (ai_id, friend_id, added_at)
         VALUES ($1, $2, NOW())
         ON CONFLICT DO NOTHING",
        &[&auth.ai_id, &req.ai_id],
    )
    .await?;

    Ok(Json(FriendResponse {
        message: format!("Added {} as friend", req.ai_id),
    }))
}

#[derive(Debug, Serialize)]
pub struct Friend {
    pub ai_id: String,
    pub added_at: String,
}

pub async fn list_friends(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let rows = conn
        .query(
            "SELECT friend_id, added_at FROM nexus_friends
             WHERE ai_id = $1
             ORDER BY added_at DESC",
            &[&auth.ai_id],
        )
        .await?;

    let friends: Vec<Friend> = rows
        .into_iter()
        .map(|r| {
            let added_at: chrono::DateTime<chrono::Utc> = r.get("added_at");
            Friend {
                ai_id: r.get("friend_id"),
                added_at: added_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(friends))
}
