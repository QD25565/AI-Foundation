//! Axum route handlers — REST endpoints + SSE real-time stream.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    Json,
};
use futures::stream::{self, Stream};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    convert::Infallible,
    sync::Arc,
    time::Duration,
};
use tokio::time::interval;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;

use crate::{auth::AuthUser, AppState};
use crate::cli::{teambook_run, notebook_run};
use crate::parser::*;

// ─── Pairing ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PairRequestBody {
    #[serde(default)]
    pub h_id: String,
}

#[derive(Deserialize)]
pub struct PairValidateBody {
    pub code: String,
}

/// POST /api/pair/request
pub async fn pair_request(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PairRequestBody>,
) -> Response {
    let (code, h_id) = state.pairing.generate_code(&body.h_id);
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "code": code,
            "h_id": h_id,
            "expires_in_secs": 600
        })),
    )
        .into_response()
}

/// POST /api/pair/validate
/// Returns { ok, h_id, token } on success, { ok: false, pending: true } if not yet approved.
pub async fn pair_validate(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PairValidateBody>,
) -> Response {
    match state.pairing.validate_code(&body.code, state.open_mode) {
        Some((h_id, token)) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "h_id": h_id, "token": token })),
        )
            .into_response(),
        None => {
            // Could be: expired, already used, or not yet approved.
            // The app distinguishes "still pending" from "bad code" by the pending field.
            let still_pending = state.pairing.code_exists(&body.code);
            (
                StatusCode::OK,
                Json(json!({ "ok": false, "pending": still_pending, "error": if still_pending { "Waiting for approval" } else { "Invalid or expired code" } })),
            )
                .into_response()
        }
    }
}

/// POST /api/pair/approve  (called by `teambook mobile-pair <code>` on the server)
#[derive(Deserialize)]
pub struct PairApproveBody {
    pub code: String,
}

pub async fn pair_approve(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PairApproveBody>,
) -> Response {
    match state.pairing.approve_code(&body.code) {
        Some(h_id) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "h_id": h_id })),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "Code not found or expired" })),
        )
            .into_response(),
    }
}

// ─── Status (no auth) ────────────────────────────────────────────────────────

/// GET /api/status
pub async fn get_status(State(_state): State<Arc<AppState>>) -> Response {
    match teambook_run(&["status"]).await {
        Ok(text) => {
            let status = parse_team_status(&text);
            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "online_count": status.online_count,
                    "members": status.members
                })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("teambook status failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": format!("{}", e) })),
            )
                .into_response()
        }
    }
}

/// GET /health
pub async fn health() -> &'static str {
    "OK"
}

// ─── Team ────────────────────────────────────────────────────────────────────

/// GET /api/team
pub async fn get_team(
    AuthUser { .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
) -> Response {
    match teambook_run(&["status"]).await {
        Ok(text) => {
            let status = parse_team_status(&text);
            (StatusCode::OK, Json(json!({
                "ok": true,
                "online_count": status.online_count,
                "members": status.members
            }))).into_response()
        }
        Err(e) => {
            tracing::error!("teambook status failed: {}", e);
            server_error(e)
        }
    }
}

// ─── DMs ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LimitQuery {
    #[serde(default = "default_limit")]
    pub limit: usize,
}
fn default_limit() -> usize { 20 }

/// GET /api/dms?limit=N
pub async fn get_dms(
    AuthUser { h_id, .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
    Query(q): Query<LimitQuery>,
) -> Response {
    let limit_str = q.limit.to_string();
    match teambook_run(&["read-dms", &limit_str]).await {
        Ok(text) => {
            let dms = parse_dms(&text, &h_id);
            (StatusCode::OK, Json(json!({ "ok": true, "data": dms }))).into_response()
        }
        Err(e) => {
            tracing::error!("read-dms failed: {}", e);
            server_error(e)
        }
    }
}

#[derive(Deserialize)]
pub struct SendDmBody {
    pub to: String,
    pub content: String,
}

/// POST /api/dms
pub async fn send_dm(
    AuthUser { .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
    Json(body): Json<SendDmBody>,
) -> Response {
    match teambook_run(&["dm", &body.to, &body.content]).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => {
            tracing::error!("dm send failed: {}", e);
            server_error(e)
        }
    }
}

// ─── Broadcasts ──────────────────────────────────────────────────────────────

/// GET /api/broadcasts?limit=N
pub async fn get_broadcasts(
    AuthUser { .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
    Query(q): Query<LimitQuery>,
) -> Response {
    let limit_str = q.limit.to_string();
    match teambook_run(&["broadcasts", &limit_str]).await {
        Ok(text) => {
            let broadcasts = parse_broadcasts(&text);
            (StatusCode::OK, Json(json!({ "ok": true, "data": broadcasts }))).into_response()
        }
        Err(e) => {
            tracing::error!("read-broadcasts failed: {}", e);
            server_error(e)
        }
    }
}

#[derive(Deserialize)]
pub struct SendBroadcastBody {
    pub content: String,
    /// Accepted for API forward-compatibility; teambook does not yet support per-channel routing.
    #[serde(default)]
    #[allow(dead_code)]
    pub channel: Option<String>,
}

/// POST /api/broadcasts
pub async fn send_broadcast(
    AuthUser { .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
    Json(body): Json<SendBroadcastBody>,
) -> Response {
    // teambook broadcast does not yet have a --channel flag; channel field is accepted
    // in the request body for forward-compatibility but is currently ignored.
    let args = vec!["broadcast", body.content.as_str()];
    match teambook_run(&args).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => {
            tracing::error!("broadcast failed: {}", e);
            server_error(e)
        }
    }
}

// ─── Tasks ───────────────────────────────────────────────────────────────────

/// GET /api/tasks
pub async fn get_tasks(
    AuthUser { .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
) -> Response {
    match teambook_run(&["task-list"]).await {
        Ok(text) => {
            let tasks = parse_tasks(&text);
            (StatusCode::OK, Json(json!({ "ok": true, "data": tasks }))).into_response()
        }
        Err(e) => {
            tracing::error!("task-list failed: {}", e);
            server_error(e)
        }
    }
}

#[derive(Deserialize)]
pub struct CreateTaskBody {
    pub description: String,
}

/// POST /api/tasks
pub async fn create_task(
    AuthUser { .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
    Json(body): Json<CreateTaskBody>,
) -> Response {
    match teambook_run(&["task-create", &body.description]).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => {
            tracing::error!("task create failed: {}", e);
            server_error(e)
        }
    }
}

#[derive(Deserialize)]
pub struct UpdateTaskBody {
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// PATCH /api/tasks/{id}
pub async fn update_task(
    AuthUser { .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTaskBody>,
) -> Response {
    let status = body.status.to_lowercase();
    let mut args = vec!["task-update", id.as_str(), status.as_str()];
    let reason_str;
    if let Some(ref r) = body.reason {
        reason_str = r.clone();
        args.push("--reason");
        args.push(&reason_str);
    }
    match teambook_run(&args).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => {
            tracing::error!("task-update failed: {}", e);
            server_error(e)
        }
    }
}

// ─── Dialogues ───────────────────────────────────────────────────────────────

/// GET /api/dialogues
pub async fn get_dialogues(
    AuthUser { .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
) -> Response {
    match teambook_run(&["dialogues"]).await {
        Ok(text) => {
            let dialogues = parse_dialogues(&text);
            (StatusCode::OK, Json(json!({ "ok": true, "data": dialogues }))).into_response()
        }
        Err(e) => {
            tracing::error!("dialogues failed: {}", e);
            server_error(e)
        }
    }
}

#[derive(Deserialize)]
pub struct StartDialogueBody {
    pub responder: String,
    pub topic: String,
}

/// POST /api/dialogues
pub async fn start_dialogue(
    AuthUser { .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
    Json(body): Json<StartDialogueBody>,
) -> Response {
    match teambook_run(&["dialogue-start", &body.responder, &body.topic]).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => {
            tracing::error!("dialogue-start failed: {}", e);
            server_error(e)
        }
    }
}

#[derive(Deserialize)]
pub struct DialogueRespondBody {
    pub response: String,
}

/// POST /api/dialogues/{id}/respond
pub async fn respond_dialogue(
    AuthUser { .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<DialogueRespondBody>,
) -> Response {
    match teambook_run(&["dialogue-respond", &id, &body.response]).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => {
            tracing::error!("dialogue-respond failed: {}", e);
            server_error(e)
        }
    }
}

// ─── Notebook ────────────────────────────────────────────────────────────────

/// GET /api/notebook?limit=N
pub async fn get_notes(
    AuthUser { .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
    Query(q): Query<LimitQuery>,
) -> Response {
    let limit_str = q.limit.to_string();
    match notebook_run(&["list", "--limit", &limit_str]).await {
        Ok(text) => {
            let notes = parse_notes(&text);
            (StatusCode::OK, Json(json!({ "ok": true, "data": notes }))).into_response()
        }
        Err(e) => {
            tracing::error!("notebook list failed: {}", e);
            server_error(e)
        }
    }
}

#[derive(Deserialize)]
pub struct RememberBody {
    pub content: String,
    #[serde(default)]
    pub tags: Option<String>,
}

/// POST /api/notebook/remember
pub async fn remember(
    AuthUser { .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
    Json(body): Json<RememberBody>,
) -> Response {
    let mut args = vec!["remember", body.content.as_str()];
    let tags_arg;
    if let Some(ref t) = body.tags {
        args.push("--tags");
        tags_arg = t.clone();
        args.push(&tags_arg);
    }
    match notebook_run(&args).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => {
            tracing::error!("notebook remember failed: {}", e);
            server_error(e)
        }
    }
}

#[derive(Deserialize)]
pub struct RecallQuery {
    pub q: String,
}

/// GET /api/notebook/recall?q=<query>
pub async fn recall(
    AuthUser { .. }: AuthUser,
    State(_state): State<Arc<AppState>>,
    Query(q): Query<RecallQuery>,
) -> Response {
    match notebook_run(&["recall", &q.q]).await {
        Ok(text) => {
            let results = parse_note_search(&text);
            (StatusCode::OK, Json(json!({ "ok": true, "data": results }))).into_response()
        }
        Err(e) => {
            tracing::error!("notebook recall failed: {}", e);
            server_error(e)
        }
    }
}

// ─── SSE real-time event stream ───────────────────────────────────────────────

/// GET /api/events  — Server-Sent Events stream for real-time push.
///
/// Named events:
///   dm_received      → data: { dm: Dm }
///   broadcast_received → data: { broadcast: Broadcast }
///   team_updated     → data: { members: [TeamMember] }
///   task_updated     → data: { task: Task }
///   keepalive        → data: ""   (every 30s)
pub async fn events_stream(
    AuthUser { .. }: AuthUser,
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.sse_tx.subscribe();
    let broadcast_stream = BroadcastStream::new(rx)
        .filter_map(|result| {
            // Lag errors (slow consumer) → skip silently
            result.ok()
        })
        .map(|event: SseEvent| -> Result<Event, Infallible> {
            Ok(Event::default()
                .event(&event.name)
                .data(event.data.to_string()))
        });

    // Keepalive comment every 30s
    let keepalive = stream::unfold((), |_| async {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let ev = Event::default()
            .event("keepalive")
            .data("");
        Some((Ok::<Event, Infallible>(ev), ()))
    });

    let merged = tokio_stream::StreamExt::merge(broadcast_stream, keepalive);

    Sse::new(merged).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("ping"),
    )
}

// ─── Unpair ───────────────────────────────────────────────────────────────────

/// POST /api/unpair  — revoke the caller's token
pub async fn unpair(
    AuthUser { token, .. }: AuthUser,
    State(state): State<Arc<AppState>>,
) -> Response {
    state.pairing.revoke_token(&token);
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

// ─── Background SSE polling task ─────────────────────────────────────────────

/// Cloneable event pushed over the broadcast channel.
#[derive(Clone, Debug)]
pub struct SseEvent {
    pub name: String,
    pub data: Value,
}

/// Spawned once at startup. Polls CLIs every 5s, diffs against last-seen state,
/// pushes named SSE events on changes.
pub async fn sse_poller(state: Arc<AppState>) {
    let mut ticker = interval(Duration::from_secs(5));
    let mut last_dm_id: Option<u64> = None;
    let mut last_bc_id: Option<u64> = None;
    // Track serialised team snapshot to detect changes — avoids broadcasting
    // team_updated on every tick when nothing has changed.
    let mut last_team_snapshot: Option<String> = None;

    loop {
        ticker.tick().await;

        // ── DMs ──────────────────────────────────────────────────────────────
        if let Ok(text) = teambook_run(&["read-dms", "50"]).await {
            // Global poller has no per-user h_id; parse with empty string so all DMs come through.
            let dms = parse_dms(&text, "");
            if let Some(newest) = dms.iter().map(|d| d.id).max() {
                if last_dm_id.map(|prev| newest > prev).unwrap_or(true) {
                    for dm in dms.iter().filter(|d| {
                        last_dm_id.map(|prev| d.id > prev).unwrap_or(true)
                    }) {
                        let event = SseEvent {
                            name: "dm_received".to_string(),
                            data: serde_json::to_value(dm).unwrap_or(Value::Null),
                        };
                        let _ = state.sse_tx.send(event);
                    }
                    last_dm_id = Some(newest);
                }
            }
        } else {
            tracing::warn!("SSE poller: read-dms failed");
        }

        // ── Broadcasts ───────────────────────────────────────────────────────
        if let Ok(text) = teambook_run(&["broadcasts", "50"]).await {
            let broadcasts = parse_broadcasts(&text);
            if let Some(newest) = broadcasts.iter().map(|b| b.id).max() {
                if last_bc_id.map(|prev| newest > prev).unwrap_or(true) {
                    for bc in broadcasts.iter().filter(|b| {
                        last_bc_id.map(|prev| b.id > prev).unwrap_or(true)
                    }) {
                        let event = SseEvent {
                            name: "broadcast_received".to_string(),
                            data: serde_json::to_value(bc).unwrap_or(Value::Null),
                        };
                        let _ = state.sse_tx.send(event);
                    }
                    last_bc_id = Some(newest);
                }
            }
        } else {
            tracing::warn!("SSE poller: read-broadcasts failed");
        }

        // ── Team status — only push when the roster actually changes ─────────
        if let Ok(text) = teambook_run(&["status"]).await {
            let status = parse_team_status(&text);
            if !status.members.is_empty() {
                // Serialize to a compact JSON string for change detection.
                // This catches presence toggles (online→offline), new members, etc.
                let snapshot = serde_json::to_string(&status.members).unwrap_or_default();
                if last_team_snapshot.as_deref() != Some(&snapshot) {
                    let event = SseEvent {
                        name: "team_updated".to_string(),
                        data: serde_json::to_value(&status.members).unwrap_or(Value::Null),
                    };
                    let _ = state.sse_tx.send(event);
                    last_team_snapshot = Some(snapshot);
                }
            }
        } else {
            tracing::warn!("SSE poller: status failed");
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn server_error(e: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": format!("{}", e) })),
    )
        .into_response()
}
