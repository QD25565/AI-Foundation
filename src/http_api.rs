//! HTTP API endpoint handlers for human integration.
//!
//! Each handler:
//! 1. Extracts auth token from Authorization header
//! 2. Resolves it to an H_ID via the pairing system
//! 3. Calls the CLI with that H_ID (as AI_ID env var)
//! 4. Returns the CLI output wrapped in a JSON envelope
//!
//! The CLI doesn't distinguish human from AI — H_ID is just another identifier.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::{delete, get, post, put},
    Router,
};
use serde::{Deserialize, Serialize};

use crate::cli_wrapper;
use crate::pairing::PairingState;

// ============== Shared State ==============

#[derive(Clone)]
pub struct ApiState {
    pub pairing: PairingState,
}

// ============== Response Envelope ==============

#[derive(Serialize)]
struct ApiResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn cli_response(output: String) -> (StatusCode, Json<ApiResponse>) {
    if output.starts_with("Error:") {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(output),
            }),
        )
    } else {
        (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(output),
                error: None,
            }),
        )
    }
}

type ApiResult = Result<(StatusCode, Json<ApiResponse>), (StatusCode, Json<ApiResponse>)>;

// ============== Auth Helper ==============

async fn resolve_auth(
    pairing: &PairingState,
    headers: &HeaderMap,
) -> Result<String, (StatusCode, Json<ApiResponse>)> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("Missing Authorization: Bearer <token> header".into()),
                }),
            )
        })?;

    pairing.resolve_token(token).await.ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("Invalid or expired token".into()),
            }),
        )
    })
}

// ============== Query/Body Types ==============

#[derive(Deserialize)]
struct LimitQuery {
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct RecallQuery {
    q: String,
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct TaskListQuery {
    filter: Option<String>,
    limit: Option<i32>,
}

#[derive(Deserialize)]
struct SendDmRequest {
    to: String,
    content: String,
}

#[derive(Deserialize)]
struct SendBroadcastRequest {
    content: String,
    channel: Option<String>,
}

#[derive(Deserialize)]
struct RememberRequest {
    content: String,
    tags: Option<String>,
}

#[derive(Deserialize)]
struct CreateTaskRequest {
    description: String,
    tasks: Option<String>,
}

#[derive(Deserialize)]
struct UpdateTaskRequest {
    status: String,
    reason: Option<String>,
}

#[derive(Deserialize)]
struct StartDialogueRequest {
    responder: String,
    topic: String,
}

#[derive(Deserialize)]
struct RespondDialogueRequest {
    response: String,
}

#[derive(Deserialize)]
pub struct PairGenerateRequest {
    pub h_id: String,
}

#[derive(Serialize)]
struct PairGenerateResponse {
    ok: bool,
    code: String,
    h_id: String,
    expires_in_secs: u64,
}

#[derive(Deserialize)]
pub struct PairValidateRequest {
    pub code: String,
}

#[derive(Serialize)]
struct PairValidateResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    h_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

// ============== Route Builder ==============

pub fn api_routes() -> Router<ApiState> {
    Router::new()
        // Messaging
        .route("/api/dms", get(get_dms))
        .route("/api/dms", post(send_dm))
        .route("/api/broadcasts", get(get_broadcasts))
        .route("/api/broadcasts", post(send_broadcast))
        // Notebook
        .route("/api/notebook/remember", post(notebook_remember))
        .route("/api/notebook/recall", get(notebook_recall))
        .route("/api/notebook/list", get(notebook_list))
        .route("/api/notebook/{id}", get(notebook_get))
        .route("/api/notebook/{id}", delete(notebook_delete))
        // Tasks
        .route("/api/tasks", get(list_tasks))
        .route("/api/tasks", post(create_task))
        .route("/api/tasks/{id}", get(get_task))
        .route("/api/tasks/{id}", put(update_task))
        // Dialogues
        .route("/api/dialogues", get(list_dialogues))
        .route("/api/dialogues", post(start_dialogue))
        .route("/api/dialogues/{id}", get(get_dialogue))
        .route("/api/dialogues/{id}/respond", post(respond_dialogue))
        // Identity & Status (no auth)
        .route("/api/status", get(team_status))
        // Pairing (no auth)
        .route("/api/pair/generate", post(pair_generate))
        .route("/api/pair", post(pair_validate))
}

// ============== Messaging Handlers ==============

async fn get_dms(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<LimitQuery>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    let limit = q.limit.unwrap_or(10).to_string();
    Ok(cli_response(
        cli_wrapper::teambook_as(&["read-dms", &limit], &h_id).await,
    ))
}

async fn send_dm(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<SendDmRequest>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    Ok(cli_response(
        cli_wrapper::teambook_as(&["dm", &body.to, &body.content], &h_id).await,
    ))
}

async fn get_broadcasts(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<LimitQuery>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    let limit = q.limit.unwrap_or(10).to_string();
    Ok(cli_response(
        cli_wrapper::teambook_as(&["broadcasts", &limit], &h_id).await,
    ))
}

async fn send_broadcast(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<SendBroadcastRequest>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    let channel = body.channel.unwrap_or_else(|| "general".to_string());
    Ok(cli_response(
        cli_wrapper::teambook_as(
            &["broadcast", &body.content, "--channel", &channel],
            &h_id,
        )
        .await,
    ))
}

// ============== Notebook Handlers ==============

async fn notebook_remember(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<RememberRequest>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    let mut args = vec!["remember", &body.content];
    let tags_owned;
    if let Some(ref t) = body.tags {
        tags_owned = t.clone();
        args.push("--tags");
        args.push(&tags_owned);
    }
    Ok(cli_response(
        cli_wrapper::notebook_as(&args, &h_id).await,
    ))
}

async fn notebook_recall(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<RecallQuery>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    let limit = q.limit.unwrap_or(10).to_string();
    Ok(cli_response(
        cli_wrapper::notebook_as(&["recall", &q.q, "--limit", &limit], &h_id).await,
    ))
}

async fn notebook_list(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<LimitQuery>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    let limit = q.limit.unwrap_or(10).to_string();
    Ok(cli_response(
        cli_wrapper::notebook_as(&["list", "--limit", &limit], &h_id).await,
    ))
}

async fn notebook_get(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    Ok(cli_response(
        cli_wrapper::notebook_as(&["get", &id], &h_id).await,
    ))
}

async fn notebook_delete(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    Ok(cli_response(
        cli_wrapper::notebook_as(&["delete", &id], &h_id).await,
    ))
}

// ============== Task Handlers ==============

async fn list_tasks(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<TaskListQuery>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    let limit = q.limit.unwrap_or(20).to_string();
    let filter = q.filter.unwrap_or_else(|| "all".to_string());
    Ok(cli_response(
        cli_wrapper::teambook_as(&["task-list", &limit, "--filter", &filter], &h_id).await,
    ))
}

async fn create_task(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<CreateTaskRequest>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    if let Some(ref tasks) = body.tasks {
        Ok(cli_response(
            cli_wrapper::teambook_as(
                &["task-create", &body.description, "--tasks", tasks],
                &h_id,
            )
            .await,
        ))
    } else {
        Ok(cli_response(
            cli_wrapper::teambook_as(&["task-create", &body.description], &h_id).await,
        ))
    }
}

async fn get_task(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    Ok(cli_response(
        cli_wrapper::teambook_as(&["task-get", &id], &h_id).await,
    ))
}

async fn update_task(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateTaskRequest>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    let status = body.status.to_lowercase();
    match body.reason {
        Some(ref reason) if !reason.is_empty() => Ok(cli_response(
            cli_wrapper::teambook_as(
                &["task-update", &id, &status, "--reason", reason],
                &h_id,
            )
            .await,
        )),
        _ => Ok(cli_response(
            cli_wrapper::teambook_as(&["task-update", &id, &status], &h_id).await,
        )),
    }
}

// ============== Dialogue Handlers ==============

async fn list_dialogues(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<LimitQuery>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    let limit = q.limit.unwrap_or(10).to_string();
    Ok(cli_response(
        cli_wrapper::teambook_as(&["dialogue-list", &limit], &h_id).await,
    ))
}

async fn start_dialogue(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<StartDialogueRequest>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    Ok(cli_response(
        cli_wrapper::teambook_as(&["dialogue-create", &body.responder, &body.topic], &h_id).await,
    ))
}

async fn get_dialogue(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    Ok(cli_response(
        cli_wrapper::teambook_as(&["dialogue-list", "--id", &id], &h_id).await,
    ))
}

async fn respond_dialogue(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<RespondDialogueRequest>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    Ok(cli_response(
        cli_wrapper::teambook_as(&["dialogue-respond", &id, &body.response], &h_id).await,
    ))
}

// ============== Identity & Status (No Auth) ==============

async fn team_status() -> (StatusCode, Json<ApiResponse>) {
    // Status is public info — use a generic observer ID
    cli_response(cli_wrapper::teambook_as(&["status"], "human-observer").await)
}

// ============== Pairing (No Auth) ==============

async fn pair_generate(
    State(state): State<ApiState>,
    Json(body): Json<PairGenerateRequest>,
) -> Json<PairGenerateResponse> {
    let code = state.pairing.generate_code(&body.h_id).await;
    Json(PairGenerateResponse {
        ok: true,
        code,
        h_id: body.h_id,
        expires_in_secs: 600,
    })
}

async fn pair_validate(
    State(state): State<ApiState>,
    Json(body): Json<PairValidateRequest>,
) -> Json<PairValidateResponse> {
    match state.pairing.validate_code(&body.code).await {
        Some((h_id, token)) => Json(PairValidateResponse {
            ok: true,
            h_id: Some(h_id),
            token: Some(token),
            error: None,
        }),
        None => Json(PairValidateResponse {
            ok: false,
            h_id: None,
            token: None,
            error: Some("Invalid or expired pairing code".into()),
        }),
    }
}
