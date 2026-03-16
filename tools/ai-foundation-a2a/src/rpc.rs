//! JSON-RPC 2.0 infrastructure and A2A protocol types.
//!
//! Covers:
//! - JSON-RPC request/response envelopes (spec: jsonrpc.org)
//! - A2A message, task, and artifact types (spec: a2a-protocol.org)
//! - A2A-specific error codes

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

// ─── JSON-RPC 2.0 envelope ────────────────────────────────────────────────────

/// JSON-RPC 2.0 request ID — string, integer, or null per spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RpcId {
    Str(String),
    Num(i64),
}

/// Incoming JSON-RPC 2.0 request.
#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    #[allow(dead_code)] // parsed by serde for spec compliance; not read in handlers
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<RpcId>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// Outgoing JSON-RPC 2.0 response.
#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<RpcId>,
    #[serde(flatten)]
    pub payload: RpcPayload,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum RpcPayload {
    Result { result: Value },
    Error { error: RpcError },
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcResponse {
    /// Build a successful response wrapping any serialisable value.
    pub fn success(id: Option<RpcId>, result: impl Serialize) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            payload: RpcPayload::Result {
                result: serde_json::to_value(result).expect("result must be serializable"),
            },
        }
    }

    /// Build an error response.
    pub fn error(id: Option<RpcId>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            payload: RpcPayload::Error {
                error: RpcError {
                    code,
                    message: message.into(),
                    data: None,
                },
            },
        }
    }

    /// Build an error response with additional context data.
    #[allow(dead_code)] // spec-complete API; used when callers need structured error context
    pub fn error_data(
        id: Option<RpcId>,
        code: i32,
        message: impl Into<String>,
        data: Value,
    ) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            payload: RpcPayload::Error {
                error: RpcError {
                    code,
                    message: message.into(),
                    data: Some(data),
                },
            },
        }
    }
}

/// Standard JSON-RPC 2.0 and A2A-specific error codes.
#[allow(dead_code)] // complete per spec; not all codes are used in current handler set
pub mod error_codes {
    // Standard JSON-RPC 2.0
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
    // A2A protocol extensions
    pub const TASK_NOT_FOUND: i32 = -32001;
    pub const TASK_NOT_CANCELLABLE: i32 = -32002;
    pub const PUSH_NOTIFICATION_NOT_SUPPORTED: i32 = -32003;
    pub const UNSUPPORTED_OPERATION: i32 = -32004;
    pub const INVALID_AGENT_RESPONSE: i32 = -32005;
}

// ─── A2A message types ────────────────────────────────────────────────────────

/// Participant role in a message exchange.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Agent,
}

/// A single content part: text, structured data, or a file reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Part {
    /// Plain text — natural language commands, CLI output.
    Text { text: String },
    /// Structured JSON payload for typed skill invocation.
    /// Expected shape: `{ "skillId": "teambook-broadcast", "args": { ... } }`
    Data { data: Value },
    /// File reference (url or inline base64); not used by current skills.
    File { file: Value },
}

impl Part {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        }
    }

    pub fn as_data(&self) -> Option<&Value> {
        match self {
            Part::Data { data } => Some(data),
            _ => None,
        }
    }
}

/// An A2A message (from user or agent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Unique message identifier.
    #[serde(rename = "messageId")]
    pub message_id: String,
    pub role: MessageRole,
    /// Ordered content parts.
    pub parts: Vec<Part>,
    /// Optional routing metadata, e.g. `{ "skillId": "teambook-broadcast" }`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl Message {
    /// Construct a simple agent reply with a single text part.
    pub fn agent_text(text: impl Into<String>) -> Self {
        Self {
            message_id: Uuid::new_v4().to_string(),
            role: MessageRole::Agent,
            parts: vec![Part::Text { text: text.into() }],
            metadata: None,
        }
    }

    /// Return the first text part, if any.
    pub fn first_text(&self) -> Option<&str> {
        self.parts.iter().find_map(|p| p.as_text())
    }

    /// Return the first data part, if any.
    pub fn first_data(&self) -> Option<&Value> {
        self.parts.iter().find_map(|p| p.as_data())
    }

    /// Extract `metadata.skillId` for explicit skill routing.
    pub fn skill_id(&self) -> Option<&str> {
        self.metadata
            .as_ref()
            .and_then(|m| m.get("skillId"))
            .and_then(|v| v.as_str())
    }
}

// ─── A2A task types ───────────────────────────────────────────────────────────

/// Lifecycle state of an A2A task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Submitted,
    Working,
    Completed,
    Failed,
    Cancelled,
    #[serde(rename = "input-required")]
    InputRequired,
    Unknown,
}

impl TaskState {
    /// True when the task has reached a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskState::Completed | TaskState::Failed | TaskState::Cancelled
        )
    }
}

/// A point-in-time snapshot of a task's status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatus {
    pub state: TaskState,
    /// Agent message accompanying the status (e.g. error description).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    /// ISO 8601 timestamp of this transition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

impl TaskStatus {
    fn now() -> String {
        Utc::now().to_rfc3339()
    }

    pub fn submitted() -> Self {
        Self { state: TaskState::Submitted, message: None, timestamp: Some(Self::now()) }
    }

    pub fn working() -> Self {
        Self { state: TaskState::Working, message: None, timestamp: Some(Self::now()) }
    }

    pub fn completed(reply: Option<Message>) -> Self {
        Self { state: TaskState::Completed, message: reply, timestamp: Some(Self::now()) }
    }

    pub fn failed(reason: impl Into<String>) -> Self {
        Self {
            state: TaskState::Failed,
            message: Some(Message::agent_text(reason)),
            timestamp: Some(Self::now()),
        }
    }

    pub fn cancelled() -> Self {
        Self { state: TaskState::Cancelled, message: None, timestamp: Some(Self::now()) }
    }
}

/// An incremental output artifact from a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Zero-based index; multiple artifacts are ordered by index.
    pub index: u32,
    /// Content parts for this artifact chunk.
    pub parts: Vec<Part>,
    /// True if this chunk should be appended to the previous chunk at the same index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub append: Option<bool>,
    /// True if this is the terminal chunk for this artifact.
    #[serde(rename = "lastChunk", skip_serializing_if = "Option::is_none")]
    pub last_chunk: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl Artifact {
    /// A streaming chunk — append=true, lastChunk=false.
    pub fn text_chunk(index: u32, text: impl Into<String>) -> Self {
        Self {
            index,
            parts: vec![Part::Text { text: text.into() }],
            append: Some(true),
            last_chunk: Some(false),
            metadata: None,
        }
    }

    /// The final streaming chunk — append=true, lastChunk=true.
    pub fn text_final(index: u32, text: impl Into<String>) -> Self {
        Self {
            index,
            parts: vec![Part::Text { text: text.into() }],
            append: Some(true),
            last_chunk: Some(true),
            metadata: None,
        }
    }

    /// A complete non-streaming artifact — lastChunk=true, no append flag.
    pub fn text_complete(index: u32, text: impl Into<String>) -> Self {
        Self {
            index,
            parts: vec![Part::Text { text: text.into() }],
            append: None,
            last_chunk: Some(true),
            metadata: None,
        }
    }
}

/// A complete A2A task object (returned by `message/send` and `tasks/get`).
#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: Uuid,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl Task {
    pub fn new(id: Uuid, status: TaskStatus) -> Self {
        Self { id, status, artifacts: Vec::new(), metadata: None }
    }
}

// ─── Method parameter types ───────────────────────────────────────────────────

/// Parameters for `message/send` and `message/stream`.
#[derive(Debug, Deserialize)]
pub struct SendMessageParams {
    pub message: Message,
    /// Existing task ID for multi-turn continuation (A2A spec; not yet wired).
    #[allow(dead_code)]
    #[serde(rename = "taskId", default)]
    pub task_id: Option<Uuid>,
    /// Per-request configuration overrides (A2A spec; not yet wired).
    #[allow(dead_code)]
    #[serde(default)]
    pub configuration: Option<Value>,
}

/// Parameters for `tasks/get`.
#[derive(Debug, Deserialize)]
pub struct GetTaskParams {
    pub id: Uuid,
}

/// Parameters for `tasks/cancel`.
#[derive(Debug, Deserialize)]
pub struct CancelTaskParams {
    pub id: Uuid,
}

/// Parameters for `tasks/resubscribe`.
#[derive(Debug, Deserialize)]
pub struct ResubscribeTaskParams {
    pub id: Uuid,
}
