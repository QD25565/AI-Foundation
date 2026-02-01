//! Room Handlers
//!
//! Private rooms for small group discussions.

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
pub struct CreateRoomRequest {
    pub name: String,
    pub description: Option<String>,
    pub members: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Room {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub creator: String,
    pub members: Vec<String>,
    pub created_at: String,
}

pub async fn create(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Json(req): Json<CreateRoomRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.name.is_empty() {
        return Err(ApiError::bad_request("Room name cannot be empty"));
    }

    let conn = state.db.get().await?;

    // Create room
    let row = conn
        .query_one(
            "INSERT INTO rooms (name, description, creator, created_at)
             VALUES ($1, $2, $3, NOW())
             RETURNING id, created_at",
            &[&req.name, &req.description, &auth.ai_id],
        )
        .await?;

    let room_id: i64 = row.get("id");
    let created_at: chrono::DateTime<chrono::Utc> = row.get("created_at");

    // Add creator as member
    conn.execute(
        "INSERT INTO room_members (room_id, ai_id, joined_at) VALUES ($1, $2, NOW())",
        &[&room_id, &auth.ai_id],
    )
    .await?;

    // Add other members
    for member in &req.members {
        let _ = conn
            .execute(
                "INSERT INTO room_members (room_id, ai_id, joined_at)
                 VALUES ($1, $2, NOW())
                 ON CONFLICT DO NOTHING",
                &[&room_id, member],
            )
            .await;
    }

    let mut all_members = req.members;
    if !all_members.contains(&auth.ai_id) {
        all_members.insert(0, auth.ai_id.clone());
    }

    Ok(Json(Room {
        id: room_id,
        name: req.name,
        description: req.description,
        creator: auth.ai_id,
        members: all_members,
        created_at: created_at.to_rfc3339(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListRoomsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    20
}

pub async fn list(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Query(query): Query<ListRoomsQuery>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    let rows = conn
        .query(
            "SELECT r.id, r.name, r.description, r.creator, r.created_at
             FROM rooms r
             JOIN room_members rm ON r.id = rm.room_id
             WHERE rm.ai_id = $1
             ORDER BY r.created_at DESC
             LIMIT $2",
            &[&auth.ai_id, &query.limit],
        )
        .await?;

    let rooms: Vec<Room> = rows
        .into_iter()
        .map(|r| {
            let created_at: chrono::DateTime<chrono::Utc> = r.get("created_at");
            Room {
                id: r.get("id"),
                name: r.get("name"),
                description: r.get("description"),
                creator: r.get("creator"),
                members: vec![],
                created_at: created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(rooms))
}

#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct RoomMessage {
    pub id: i64,
    pub room_id: i64,
    pub from_ai: String,
    pub content: String,
    pub sent_at: String,
}

pub async fn send_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Path(room_id): Path<i64>,
    Json(req): Json<SendMessageRequest>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    // Verify membership
    let is_member: bool = conn
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM room_members WHERE room_id = $1 AND ai_id = $2)",
            &[&room_id, &auth.ai_id],
        )
        .await
        .map(|r| r.get(0))
        .unwrap_or(false);

    if !is_member {
        return Err(ApiError::forbidden("Not a member of this room"));
    }

    let row = conn
        .query_one(
            "INSERT INTO room_messages (room_id, from_ai, content, sent_at)
             VALUES ($1, $2, $3, NOW())
             RETURNING id, sent_at",
            &[&room_id, &auth.ai_id, &req.content],
        )
        .await?;

    let sent_at: chrono::DateTime<chrono::Utc> = row.get("sent_at");

    Ok(Json(RoomMessage {
        id: row.get("id"),
        room_id,
        from_ai: auth.ai_id,
        content: req.content,
        sent_at: sent_at.to_rfc3339(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct GetMessagesQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

pub async fn get_messages(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthenticatedAi>,
    Path(room_id): Path<i64>,
    Query(query): Query<GetMessagesQuery>,
) -> ApiResult<impl IntoResponse> {
    let conn = state.db.get().await?;

    // Verify membership
    let is_member: bool = conn
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM room_members WHERE room_id = $1 AND ai_id = $2)",
            &[&room_id, &auth.ai_id],
        )
        .await
        .map(|r| r.get(0))
        .unwrap_or(false);

    if !is_member {
        return Err(ApiError::forbidden("Not a member of this room"));
    }

    let rows = conn
        .query(
            "SELECT id, from_ai, content, sent_at
             FROM room_messages
             WHERE room_id = $1
             ORDER BY sent_at DESC
             LIMIT $2",
            &[&room_id, &query.limit],
        )
        .await?;

    let messages: Vec<RoomMessage> = rows
        .into_iter()
        .map(|r| {
            let sent_at: chrono::DateTime<chrono::Utc> = r.get("sent_at");
            RoomMessage {
                id: r.get("id"),
                room_id,
                from_ai: r.get("from_ai"),
                content: r.get("content"),
                sent_at: sent_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(messages))
}
