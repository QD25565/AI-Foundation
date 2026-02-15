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
use crate::federation::{self, FederationState};
use crate::federation_sync;
use crate::pairing::PairingState;
use crate::profile;
use std::sync::Arc;

// ============== Shared State ==============

#[derive(Clone)]
pub struct ApiState {
    pub pairing: PairingState,
    pub federation: Arc<FederationState>,
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

#[derive(Deserialize)]
struct ProfileSetRequest {
    display_name: Option<String>,
    bio: Option<String>,
    interests: Option<Vec<String>>,
    current_focus: Option<String>,
}

#[derive(Deserialize)]
struct ProfileFocusRequest {
    focus: String,
}

#[derive(Deserialize)]
struct SetStatusRequest {
    status: String,
}

#[derive(Deserialize)]
struct PreferencesRequest {
    auto_presence: Option<bool>,
}

#[derive(Deserialize)]
struct VisionAttachRequest {
    note_id: u64,
    image_path: String,
    context: Option<String>,
}

#[derive(Deserialize)]
struct VisionGetQuery {
    output: Option<String>,
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
        // Profiles (public read, auth write)
        .route("/api/profiles", get(profile_list))
        .route("/api/profiles/me", get(profile_get_me))
        .route("/api/profiles/me", put(profile_set_me))
        .route("/api/profiles/me/focus", put(profile_set_focus))
        .route("/api/profiles/{ai_id}", get(profile_get))
        .route("/api/profiles/me/status", put(profile_set_status))
        .route("/api/profiles/me/preferences", get(profile_get_preferences))
        .route("/api/profiles/me/preferences", put(profile_set_preferences))
        // Vision (auth required)
        .route("/api/vision/attach", post(vision_attach))
        .route("/api/vision/list", get(vision_list))
        .route("/api/vision/{id}", get(vision_get))
        .route("/api/vision/note/{note_id}", get(vision_note))
        .route("/api/vision/stats", get(vision_stats))
        // Federation (peer-authenticated, no Bearer token)
        .route("/api/federation/register", post(federation_register))
        .route("/api/federation/peers", get(federation_peers))
        .route("/api/federation/peers/{id}", delete(federation_remove_peer))
        .route("/api/federation/identity", get(federation_identity))
        .route("/api/federation/events", post(federation_push_events))
        .route("/api/federation/events", get(federation_pull_events))
        .route("/api/federation/status", get(federation_status))
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

// ============== Profile Handlers ==============

/// List all AI profiles on this Teambook (public, no auth).
async fn profile_list() -> (StatusCode, Json<serde_json::Value>) {
    match profile::list_profiles().await {
        Ok(profiles) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "profiles": profiles,
                "count": profiles.len(),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "ok": false,
                "error": e.to_string(),
            })),
        ),
    }
}

/// View a specific AI's profile (public, no auth).
async fn profile_get(
    Path(ai_id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match profile::load_profile(&ai_id).await {
        Ok(Some(p)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "profile": p,
            })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "ok": false,
                "error": format!("No profile found for '{}'", ai_id),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "ok": false,
                "error": e.to_string(),
            })),
        ),
    }
}

/// View your own profile (auth required).
async fn profile_get_me(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    match profile::load_or_create(&h_id).await {
        Ok(p) => Ok((
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(serde_json::to_string_pretty(&p).unwrap_or_else(|_| p.display())),
                error: None,
            }),
        )),
        Err(e) => Ok(cli_response(format!("Error: {}", e))),
    }
}

/// Set/update your own profile (auth required).
async fn profile_set_me(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<ProfileSetRequest>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    let mut p = match profile::load_or_create(&h_id).await {
        Ok(p) => p,
        Err(e) => return Ok(cli_response(format!("Error: {}", e))),
    };

    p.apply_update(profile::ProfileUpdate {
        display_name: body.display_name,
        bio: body.bio,
        interests: body.interests,
        current_focus: body.current_focus,
    });

    match profile::save_profile(&p).await {
        Ok(()) => Ok((
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(serde_json::to_string_pretty(&p).unwrap_or_else(|_| p.display())),
                error: None,
            }),
        )),
        Err(e) => Ok(cli_response(format!("Error: {}", e))),
    }
}

/// Quick-set your current focus (auth required).
async fn profile_set_focus(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<ProfileFocusRequest>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    match profile::set_focus(&h_id, &body.focus).await {
        Ok(()) => {
            let msg = if body.focus.is_empty() {
                "Focus cleared".to_string()
            } else {
                format!("Focus set: {}", body.focus)
            };
            Ok((
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(msg),
                    error: None,
                }),
            ))
        }
        Err(e) => Ok(cli_response(format!("Error: {}", e))),
    }
}

/// Set manual status message (auth required).
async fn profile_set_status(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<SetStatusRequest>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    match profile::set_status(&h_id, &body.status).await {
        Ok(()) => {
            let msg = if body.status.is_empty() {
                "Status cleared".to_string()
            } else {
                format!("Status set: {}", body.status)
            };
            Ok((
                StatusCode::OK,
                Json(ApiResponse { ok: true, data: Some(msg), error: None }),
            ))
        }
        Err(e) => Ok(cli_response(format!("Error: {}", e))),
    }
}

/// Get preferences (auth required).
async fn profile_get_preferences(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    match profile::load_or_create(&h_id).await {
        Ok(p) => Ok((
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(serde_json::to_string(&p.preferences).unwrap_or_default()),
                error: None,
            }),
        )),
        Err(e) => Ok(cli_response(format!("Error: {}", e))),
    }
}

/// Set preferences (auth required).
async fn profile_set_preferences(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<PreferencesRequest>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    match profile::set_preferences(&h_id, body.auto_presence).await {
        Ok(prefs) => Ok((
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(serde_json::to_string(&prefs).unwrap_or_default()),
                error: None,
            }),
        )),
        Err(e) => Ok(cli_response(format!("Error: {}", e))),
    }
}

// ============== Vision Handlers ==============

/// Attach an image to a notebook note (auth required).
async fn vision_attach(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<VisionAttachRequest>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    let note_id = body.note_id.to_string();
    let mut args = vec!["attach", &note_id, &body.image_path];
    let context_owned;
    if let Some(ref c) = body.context {
        context_owned = c.clone();
        args.push("--context");
        args.push(&context_owned);
    }
    Ok(cli_response(
        cli_wrapper::visionbook_as(&args, &h_id).await,
    ))
}

/// List recent visual memories (auth required).
async fn vision_list(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<LimitQuery>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    let limit = q.limit.unwrap_or(10).to_string();
    Ok(cli_response(
        cli_wrapper::visionbook_as(&["visual-list", "--limit", &limit], &h_id).await,
    ))
}

/// Get a specific visual memory (auth required).
async fn vision_get(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<VisionGetQuery>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    let mut args = vec!["visual-get", &id];
    let output_owned;
    if let Some(ref o) = q.output {
        output_owned = o.clone();
        args.push("--output");
        args.push(&output_owned);
    }
    Ok(cli_response(
        cli_wrapper::visionbook_as(&args, &h_id).await,
    ))
}

/// Get visuals attached to a notebook note (auth required).
async fn vision_note(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(note_id): Path<String>,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    Ok(cli_response(
        cli_wrapper::visionbook_as(&["note-visuals", &note_id], &h_id).await,
    ))
}

/// Visual memory statistics (auth required).
async fn vision_stats(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult {
    let h_id = resolve_auth(&state.pairing, &headers).await?;
    Ok(cli_response(
        cli_wrapper::visionbook_as(&["visual-stats"], &h_id).await,
    ))
}

// ============== Federation Handlers ==============

/// Register as a federation peer (exchange public keys + signed challenges).
async fn federation_register(
    State(state): State<ApiState>,
    Json(body): Json<federation::PeerRegistrationRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let resp = state.federation.handle_registration(&body).await;
    let status = if resp.accepted {
        StatusCode::OK
    } else {
        StatusCode::FORBIDDEN
    };
    (status, Json(serde_json::to_value(resp).unwrap()))
}

/// List all registered federation peers.
async fn federation_peers(
    State(state): State<ApiState>,
) -> Json<serde_json::Value> {
    let peers = state.federation.list_peers().await;
    Json(serde_json::json!({
        "ok": true,
        "peers": peers,
        "count": peers.len(),
    }))
}

/// Remove a federation peer by hex-encoded public key.
async fn federation_remove_peer(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse>) {
    if state.federation.remove_peer(&id).await {
        (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(format!("Peer {} removed", &id[..8.min(id.len())])),
                error: None,
            }),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("Peer not found".into()),
            }),
        )
    }
}

/// Get this Teambook's federation identity (public key + metadata).
async fn federation_identity(
    State(state): State<ApiState>,
) -> Json<serde_json::Value> {
    let status = state.federation.status().await;
    Json(serde_json::json!({
        "ok": true,
        "pubkey": status.pubkey,
        "short_id": status.short_id,
        "display_name": status.display_name,
        "endpoint": status.endpoint,
    }))
}

/// Receive pushed events from a federation peer.
async fn federation_push_events(
    State(state): State<ApiState>,
    Json(body): Json<federation_sync::EventPushRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let resp = federation_sync::process_push(&state.federation, &body).await;

    let status = if resp.rejected > 0 && resp.accepted == 0 {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };

    (status, Json(serde_json::to_value(resp).unwrap()))
}

/// Pull events since a given sequence (for catch-up sync).
async fn federation_pull_events(
    State(state): State<ApiState>,
    Query(q): Query<FederationPullQuery>,
) -> Json<serde_json::Value> {
    // Verify the requester is a known peer
    if let Some(ref pubkey) = q.pubkey {
        if let Ok(bytes) = hex::decode(pubkey) {
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                if !state.federation.is_known_peer(&arr).await {
                    return Json(serde_json::json!({
                        "ok": false,
                        "error": "Unknown peer",
                    }));
                }
                // Touch the peer
                state.federation.touch_peer(pubkey).await;
            }
        }
    }

    // For now, return empty — actual event log reading will be integrated
    // when we wire this to the teamengram event log
    let hlc = state.federation.clock.tick();
    Json(serde_json::to_value(federation_sync::EventPullResponse {
        events: vec![],
        head_seq: 0,
        has_more: false,
        sender_hlc: hlc,
    })
    .unwrap())
}

/// Get federation health status.
async fn federation_status(
    State(state): State<ApiState>,
) -> Json<serde_json::Value> {
    let status = state.federation.status().await;
    Json(serde_json::json!({
        "ok": true,
        "federation": status,
    }))
}

#[derive(Deserialize)]
#[allow(dead_code)] // Fields used when event log integration is wired up
struct FederationPullQuery {
    since: Option<u64>,
    limit: Option<usize>,
    pubkey: Option<String>,
}
