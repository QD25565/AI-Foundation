//! SSE event envelope types for `message/stream` and `tasks/resubscribe`.
//!
//! The A2A spec defines two stream event types:
//!   - `TaskStatusUpdateEvent`   — lifecycle transitions (working, completed, failed …)
//!   - `TaskArtifactUpdateEvent` — incremental output chunks
//!
//! Every event is wrapped in a JSON-RPC 2.0 result envelope so clients can use
//! the same JSON parser for both streaming and non-streaming responses.
//!
//! The `StreamEvent` enum is what flows through the broadcast channel.
//! `to_sse_event()` serialises it into an axum `Event` for the HTTP SSE response.

use std::convert::Infallible;

use axum::response::sse::Event;
use serde::Serialize;
use uuid::Uuid;

use crate::rpc::{Artifact, RpcId, TaskStatus};

// ─── Inner event payloads ─────────────────────────────────────────────────────

/// Task lifecycle transition event.
#[derive(Debug, Clone, Serialize)]
pub struct TaskStatusUpdateEvent {
    #[serde(rename = "type")]
    pub kind: &'static str, // always "TaskStatusUpdateEvent"
    #[serde(rename = "taskId")]
    pub task_id: Uuid,
    pub status: TaskStatus,
    /// True when this is the last event for this task.
    #[serde(rename = "final")]
    pub is_final: bool,
}

/// Incremental artifact output event.
#[derive(Debug, Clone, Serialize)]
pub struct TaskArtifactUpdateEvent {
    #[serde(rename = "type")]
    pub kind: &'static str, // always "TaskArtifactUpdateEvent"
    #[serde(rename = "taskId")]
    pub task_id: Uuid,
    pub artifact: Artifact,
    /// True when this is the last event for this task.
    #[serde(rename = "final")]
    pub is_final: bool,
}

// ─── Unified stream event ─────────────────────────────────────────────────────

/// A single item broadcast through the task's event channel.
///
/// Serialises as a JSON-RPC 2.0 result envelope (see `SseEnvelope`).
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum StreamEvent {
    Status(SseEnvelope<TaskStatusUpdateEvent>),
    Artifact(SseEnvelope<TaskArtifactUpdateEvent>),
}

impl StreamEvent {
    pub fn status_update(task_id: Uuid, status: TaskStatus, is_final: bool) -> Self {
        Self::Status(SseEnvelope::result(TaskStatusUpdateEvent {
            kind: "TaskStatusUpdateEvent",
            task_id,
            status,
            is_final,
        }))
    }

    pub fn artifact_update(task_id: Uuid, artifact: Artifact, is_final: bool) -> Self {
        Self::Artifact(SseEnvelope::result(TaskArtifactUpdateEvent {
            kind: "TaskArtifactUpdateEvent",
            task_id,
            artifact,
            is_final,
        }))
    }

    /// True if this event ends the stream (no more events should follow).
    pub fn is_final(&self) -> bool {
        match self {
            Self::Status(e) => e.result.is_final,
            Self::Artifact(e) => e.result.is_final,
        }
    }

    /// Serialise to an axum SSE `Event`.
    ///
    /// Serialisation failures produce a comment event so the stream stays alive
    /// and the client can log the anomaly — failing silently here would be worse.
    pub fn to_sse_event(&self) -> Result<Event, Infallible> {
        match serde_json::to_string(self) {
            Ok(json) => Ok(Event::default().data(json)),
            Err(e) => Ok(Event::default().comment(format!("serialization error: {}", e))),
        }
    }
}

// ─── JSON-RPC 2.0 SSE envelope ────────────────────────────────────────────────

/// Wraps an A2A stream event in a JSON-RPC 2.0 result envelope.
///
/// The spec requires SSE events to have the same structure as JSON-RPC responses
/// so clients can use one parser for both streaming and non-streaming modes.
#[derive(Debug, Clone, Serialize)]
pub struct SseEnvelope<T: Serialize> {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<RpcId>,
    pub result: T,
}

impl<T: Serialize> SseEnvelope<T> {
    pub fn result(payload: T) -> Self {
        Self { jsonrpc: "2.0", id: None, result: payload }
    }
}
