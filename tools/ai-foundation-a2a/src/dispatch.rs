//! JSON-RPC 2.0 method dispatcher.
//!
//! Single entry point: `handle_rpc` receives every POST to `/`, inspects the
//! `method` field, and routes to the appropriate A2A handler:
//!
//! | Method              | Transport       | Description                           |
//! |---------------------|-----------------|---------------------------------------|
//! | `message/send`      | JSON (blocking) | Run CLI, return completed Task        |
//! | `message/stream`    | SSE (streaming) | Spawn CLI, stream events to caller    |
//! | `tasks/get`         | JSON (blocking) | Return current task snapshot          |
//! | `tasks/cancel`      | JSON (blocking) | Signal cancellation, return status    |
//! | `tasks/resubscribe` | SSE (streaming) | Re-attach to an existing task stream  |
//!
//! All handlers return `axum::response::Response` so a single axum route can
//! serve both JSON and SSE without any branching at the HTTP layer.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive};
use axum::response::{IntoResponse, Response, Sse};
use futures::stream;
use serde_json::json;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::cli;
use crate::rpc::{
    error_codes, Artifact, CancelTaskParams, GetTaskParams, ResubscribeTaskParams, RpcRequest,
    RpcResponse, SendMessageParams,
};
use crate::skills;
use crate::streaming::StreamEvent;
use crate::task::SharedTaskStore;

// ─── Shared application state ─────────────────────────────────────────────────

/// State shared across every request handler via axum's `State` extractor.
#[derive(Clone)]
pub struct AppState {
    /// In-flight task registry — shared, reference-counted.
    pub store: SharedTaskStore,
    /// Resolved path to AI-Foundation CLI binaries.
    pub bin_dir: PathBuf,
}

// ─── Entry point ──────────────────────────────────────────────────────────────

/// Route every JSON-RPC 2.0 request to its handler.
///
/// Unknown methods receive a `-32601 Method not found` error.
/// Malformed JSON never reaches this handler — axum's `Json` extractor
/// returns a 400 before we're called.
pub async fn handle_rpc(
    State(state): State<AppState>,
    Json(req): Json<RpcRequest>,
) -> Response {
    match req.method.as_str() {
        "message/send" => handle_message_send(state, req).await,
        "message/stream" => handle_message_stream(state, req).await,
        "tasks/get" => handle_tasks_get(state, req).await,
        "tasks/cancel" => handle_tasks_cancel(state, req).await,
        "tasks/resubscribe" => handle_tasks_resubscribe(state, req).await,
        _ => json_error(
            req.id,
            error_codes::METHOD_NOT_FOUND,
            format!("Method not found: '{}'", req.method),
        ),
    }
}

// ─── message/send (blocking) ──────────────────────────────────────────────────

/// Run a CLI skill to completion and return the finished `Task` object.
///
/// The task transitions: Submitted → Working → Completed (or Failed).
/// The full artifact output is available on the returned task.
async fn handle_message_send(state: AppState, req: RpcRequest) -> Response {
    let params: SendMessageParams = match parse_params(req.params, req.id.clone()) {
        Ok(p) => p,
        Err(r) => return r,
    };

    let invocation = match skills::route(&params.message, &state.bin_dir) {
        Ok(inv) => inv,
        Err(e) => return json_error(req.id, error_codes::INVALID_PARAMS, e),
    };

    let task_id = Uuid::new_v4();
    // create() registers the task; we don't need the sender/cancel for blocking mode.
    let _ = state.store.create(task_id);
    state.store.set_working(task_id);

    let arg_refs: Vec<&str> = invocation.args.iter().map(String::as_str).collect();
    match cli::run_to_completion(&state.bin_dir, &invocation.exe, &arg_refs).await {
        Ok(stdout) => {
            if !stdout.is_empty() {
                state.store.push_artifact(task_id, Artifact::text_complete(0, stdout));
            }
            state.store.complete(task_id, None);
        }
        Err(e) => {
            state.store.fail(task_id, e);
        }
    }

    match state.store.get(task_id) {
        Some(task) => Json(RpcResponse::success(req.id, task)).into_response(),
        None => json_error(req.id, error_codes::INTERNAL_ERROR, "Task lost after completion"),
    }
}

// ─── message/stream (SSE) ─────────────────────────────────────────────────────

/// Spawn a CLI skill and stream its output as A2A SSE events.
///
/// Returns an SSE response immediately. The spawned task drives the
/// `TaskStore` state machine; each state transition and artifact chunk
/// is forwarded to the caller via the broadcast channel.
async fn handle_message_stream(state: AppState, req: RpcRequest) -> Response {
    let params: SendMessageParams = match parse_params(req.params, req.id.clone()) {
        Ok(p) => p,
        Err(r) => return r,
    };

    let invocation = match skills::route(&params.message, &state.bin_dir) {
        Ok(inv) => inv,
        Err(e) => return json_error(req.id, error_codes::INVALID_PARAMS, e),
    };

    let task_id = Uuid::new_v4();
    let (_, cancel) = state.store.create(task_id);

    // Subscribe BEFORE spawning — the broadcast buffer (128 slots) absorbs
    // any events emitted before the client's first SSE poll.
    let rx = state.store.subscribe(task_id).expect("task was just created");

    let store = Arc::clone(&state.store);
    let bin_dir = state.bin_dir.clone();
    let args: Vec<String> = invocation.args.clone();
    let exe = invocation.exe.clone();

    // The spawned task drives the state machine to a terminal state;
    // it must NOT be awaited — the SSE stream is what delivers the result.
    tokio::spawn(async move {
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        cli::run_streaming(&bin_dir, &exe, &arg_refs, task_id, store, cancel).await;
    });

    sse_response(rx)
}

// ─── tasks/get ────────────────────────────────────────────────────────────────

async fn handle_tasks_get(state: AppState, req: RpcRequest) -> Response {
    let params: GetTaskParams = match parse_params(req.params, req.id.clone()) {
        Ok(p) => p,
        Err(r) => return r,
    };
    match state.store.get(params.id) {
        Some(task) => Json(RpcResponse::success(req.id, task)).into_response(),
        None => json_error(req.id, error_codes::TASK_NOT_FOUND, "Task not found"),
    }
}

// ─── tasks/cancel ─────────────────────────────────────────────────────────────

async fn handle_tasks_cancel(state: AppState, req: RpcRequest) -> Response {
    let params: CancelTaskParams = match parse_params(req.params, req.id.clone()) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if state.store.request_cancel(params.id) {
        Json(RpcResponse::success(req.id, json!({ "cancelled": true }))).into_response()
    } else {
        json_error(req.id, error_codes::TASK_NOT_CANCELLABLE, "Task not cancellable")
    }
}

// ─── tasks/resubscribe (SSE) ──────────────────────────────────────────────────

async fn handle_tasks_resubscribe(state: AppState, req: RpcRequest) -> Response {
    let params: ResubscribeTaskParams = match parse_params(req.params, req.id.clone()) {
        Ok(p) => p,
        Err(r) => return r,
    };
    match state.store.subscribe(params.id) {
        Some(rx) => sse_response(rx),
        None => json_error(req.id, error_codes::TASK_NOT_FOUND, "Task not found"),
    }
}

// ─── SSE helpers ──────────────────────────────────────────────────────────────

/// Convert a broadcast receiver into a keep-alive SSE response.
///
/// The stream terminates when:
///   - A final event (is_final = true) is received, OR
///   - The broadcast channel is closed (all senders dropped)
///
/// Lagged receivers (client fell behind the 128-slot buffer) receive a comment
/// event rather than hard-failing — the stream continues.
fn sse_response(rx: broadcast::Receiver<StreamEvent>) -> Response {
    // State: (receiver, done). When done=true at entry, emit nothing and stop.
    let event_stream =
        stream::unfold((rx, false), |(mut rx, done)| async move {
            if done {
                return None;
            }
            match rx.recv().await {
                Ok(event) => {
                    let is_final = event.is_final();
                    Some((event.to_sse_event(), (rx, is_final)))
                }
                Err(broadcast::error::RecvError::Lagged(n)) => Some((
                    Ok(Event::default()
                        .comment(format!("lagged: {} events dropped", n))),
                    (rx, false),
                )),
                Err(broadcast::error::RecvError::Closed) => None,
            }
        });

    Sse::new(event_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

// ─── Shared utilities ─────────────────────────────────────────────────────────

/// Deserialise JSON-RPC params into a typed struct.
///
/// Missing params are treated as `null` — serde's `#[serde(default)]` can
/// handle optional fields without requiring the caller to pass `{}`.
fn parse_params<T: serde::de::DeserializeOwned>(
    params: Option<serde_json::Value>,
    id: Option<crate::rpc::RpcId>,
) -> Result<T, Response> {
    let value = params.unwrap_or(serde_json::Value::Null);
    serde_json::from_value(value).map_err(|e| {
        json_error(id, error_codes::INVALID_PARAMS, format!("Invalid params: {}", e))
    })
}

fn json_error(
    id: Option<crate::rpc::RpcId>,
    code: i32,
    message: impl Into<String>,
) -> Response {
    Json(RpcResponse::error(id, code, message)).into_response()
}
