//! A2A Agent Card — served at `GET /.well-known/agent.json`.
//!
//! The Agent Card is the A2A discovery document: any A2A-compatible client
//! fetches this endpoint to learn what this agent can do and how to invoke it.
//!
//! Lumen owns this module. `skill_catalog()` is the source of truth for all
//! skills exposed over A2A. Keep this in sync with:
//!   - mcp-server-rs/src/main.rs (MCP tool list)
//!   - notebook-rs/src/bin/notebook-cli.rs (CLI subcommands)
//!   - skills/ (dispatch routing)
//!
//! Spec: https://a2aprotocol.ai/docs/specification#agent-card

use axum::Json;
use axum::response::IntoResponse;
use serde::Serialize;
use serde_json::{Value, json};

// ─── Agent Card ───────────────────────────────────────────────────────────────

/// Top-level A2A Agent Card document.
#[derive(Debug, Serialize)]
pub struct AgentCard {
    pub name: &'static str,
    pub description: &'static str,
    pub version: &'static str,
    /// Base URL where this A2A server is reachable.
    /// Clients POST JSON-RPC to this URL.
    pub url: String,
    pub capabilities: AgentCapabilities,
    #[serde(rename = "defaultInputModes")]
    pub default_input_modes: &'static [&'static str],
    #[serde(rename = "defaultOutputModes")]
    pub default_output_modes: &'static [&'static str],
    /// Skill catalog — Lumen fills this in.
    pub skills: Vec<Value>,
}

#[derive(Debug, Serialize)]
pub struct AgentCapabilities {
    /// This server supports `message/stream` and `tasks/resubscribe` (SSE).
    pub streaming: bool,
    /// Push notifications are not supported.
    #[serde(rename = "pushNotifications")]
    pub push_notifications: bool,
    /// Full state-transition history is not stored.
    #[serde(rename = "stateTransitionHistory")]
    pub state_transition_history: bool,
}

// ─── Handler ──────────────────────────────────────────────────────────────────

/// Serve the Agent Card at `GET /.well-known/agent.json`.
pub async fn serve() -> impl IntoResponse {
    let url = std::env::var("A2A_URL")
        .unwrap_or_else(|_| format!("http://localhost:{}", port()));

    let card = AgentCard {
        name: "AI Foundation",
        description: "\
            Team coordination infrastructure for AI agents. \
            Provides persistent memory (notebook) and team communication (teambook) \
            to any A2A-compatible agent — not just Claude Code.",
        version: env!("CARGO_PKG_VERSION"),
        url,
        capabilities: AgentCapabilities {
            streaming: true,
            push_notifications: false,
            state_transition_history: false,
        },
        default_input_modes: &["text", "data"],
        default_output_modes: &["text"],
        skills: skill_catalog(),
    };

    Json(card)
}

// ─── Skill catalog ────────────────────────────────────────────────────────────

/// Return the full skill catalog with JSON Schema `inputSchema` on every skill.
///
/// Each entry follows the A2A `AgentSkill` schema:
///   - `id`          — kebab-case identifier (used in `message.metadata.skillId`)
///   - `name`        — human-readable display name
///   - `description` — what the skill does
///   - `inputSchema` — JSON Schema describing accepted arguments
///   - `inputModes`  — accepted content types
///   - `outputModes` — produced content types
fn skill_catalog() -> Vec<Value> {
    vec![
        // ── Teambook: read ────────────────────────────────────────────────────
        skill(
            "teambook-status",
            "Team Status",
            "Get current AI ID and online status of all agents.",
        ),
        skill_schema(
            "teambook-direct-messages",
            "Read Direct Messages",
            "Read the most recent direct messages sent to this agent.",
            json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "Max DMs to return", "default": 10}
                }
            }),
        ),
        skill_schema(
            "teambook-read-broadcasts",
            "Read Broadcasts",
            "Read recent broadcast messages from all AIs.",
            json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "Max broadcasts to return", "default": 20}
                }
            }),
        ),
        skill_schema(
            "teambook-list-claims",
            "List File Claims",
            "List all files currently claimed (locked) by any AI.",
            json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "Max claims to return", "default": 20}
                }
            }),
        ),
        skill_schema(
            "teambook-who-has",
            "Who Has File",
            "Check if a specific file is currently claimed by any AI.",
            json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string", "description": "File path to check"}
                }
            }),
        ),
        skill(
            "teambook-dialogues",
            "List Dialogues",
            "List active structured AI-to-AI dialogues.",
        ),
        skill(
            "teambook-task-list",
            "List Tasks",
            "List all tasks and batches in the team task board.",
        ),
        skill_schema(
            "teambook-task-get",
            "Get Task",
            "Get details of a specific task or batch.",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": {"type": "string", "description": "Task ID or batch name"}
                }
            }),
        ),
        // ── Teambook: write ───────────────────────────────────────────────────
        skill_schema(
            "teambook-broadcast",
            "Broadcast",
            "Send a message to all AIs on the team.",
            json!({
                "type": "object",
                "required": ["content"],
                "properties": {
                    "content": {"type": "string", "description": "Message to broadcast"},
                    "channel": {"type": "string", "description": "Channel name", "default": "general"}
                }
            }),
        ),
        skill_schema(
            "teambook-dm",
            "Send Direct Message",
            "Send a private direct message to another AI.",
            json!({
                "type": "object",
                "required": ["to_ai", "content"],
                "properties": {
                    "to_ai": {"type": "string", "description": "Recipient AI ID (e.g. sage-724)"},
                    "content": {"type": "string", "description": "Message content"}
                }
            }),
        ),
        skill_schema(
            "teambook-dialogue-start",
            "Start Dialogue",
            "Start a structured turn-based dialogue with another AI.",
            json!({
                "type": "object",
                "required": ["responder", "topic"],
                "properties": {
                    "responder": {"type": "string", "description": "AI ID of the dialogue partner"},
                    "topic": {"type": "string", "description": "Dialogue topic or opening question"}
                }
            }),
        ),
        skill_schema(
            "teambook-dialogue-respond",
            "Respond to Dialogue",
            "Send a response in an active dialogue.",
            json!({
                "type": "object",
                "required": ["dialogue_id", "response"],
                "properties": {
                    "dialogue_id": {"type": "integer", "description": "Dialogue ID to respond to"},
                    "response": {"type": "string", "description": "Response content"}
                }
            }),
        ),
        skill_schema(
            "teambook-standby",
            "Standby",
            "Enter event-driven standby mode. Wakes on DM, broadcast, or urgent event.",
            json!({
                "type": "object",
                "properties": {
                    "timeout": {"type": "integer", "description": "Max wait in seconds (default: 180)"}
                }
            }),
        ),
        skill_schema(
            "teambook-task-create",
            "Create Task",
            "Create a new task or batch on the team task board.",
            json!({
                "type": "object",
                "required": ["description"],
                "properties": {
                    "description": {"type": "string", "description": "Task description, or batch name if tasks provided"},
                    "tasks": {"type": "string", "description": "For batches: '1:First task,2:Second task'"}
                }
            }),
        ),
        skill_schema(
            "teambook-task-update",
            "Update Task",
            "Update the status of a task or batch task.",
            json!({
                "type": "object",
                "required": ["id", "status"],
                "properties": {
                    "id": {"type": "string", "description": "Task ID or 'BatchName:label' for batch tasks"},
                    "status": {
                        "type": "string",
                        "enum": ["done", "claimed", "started", "blocked", "closed"],
                        "description": "New task status"
                    },
                    "reason": {"type": "string", "description": "Reason (recommended when status is blocked)"}
                }
            }),
        ),
        // ── Notebook: read ────────────────────────────────────────────────────
        skill_schema(
            "notebook-list",
            "List Notes",
            "List recent notes from this agent's private notebook. Optionally filter by tag.",
            json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "Max notes to return", "default": 10},
                    "tag": {"type": "string", "description": "Filter by tag (e.g. 'ai-foundation')"}
                }
            }),
        ),
        skill(
            "notebook-tags",
            "List Tags",
            "List all tags used in this agent's notebook with note counts, sorted by frequency.",
        ),
        skill_schema(
            "notebook-update",
            "Update Note",
            "Update the content and/or tags of an existing note.",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": {"type": "integer", "description": "Note ID to update"},
                    "content": {"type": "string", "description": "New content (replaces existing)"},
                    "tags": {"type": "string", "description": "New tags comma-separated (replaces existing)"}
                }
            }),
        ),
        skill_schema(
            "notebook-add-tags",
            "Add Tags",
            "Add tags to an existing note without replacing existing tags.",
            json!({
                "type": "object",
                "required": ["note_id", "tags"],
                "properties": {
                    "note_id": {"type": "integer", "description": "Note ID to tag"},
                    "tags": {"type": "string", "description": "Tags to add (comma-separated)"}
                }
            }),
        ),
        skill_schema(
            "notebook-work",
            "Working Memory",
            "Save a short-lived working note with a TTL. Expires automatically after the given hours (default 24). \
             Use for session state, in-progress thoughts, and scratchpad entries that shouldn't persist long-term.",
            json!({
                "type": "object",
                "required": ["content"],
                "properties": {
                    "content": {"type": "string", "description": "Working note content"},
                    "tags": {"type": "string", "description": "Comma-separated tags"},
                    "ttl_hours": {"type": "integer", "description": "Hours until expiry (default: 24, max: 65535)"}
                }
            }),
        ),
        skill_schema(
            "notebook-pinned",
            "Pinned Notes",
            "Get pinned notes from this agent's private notebook.",
            json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "Max pinned notes to return", "default": 10}
                }
            }),
        ),
        skill_schema(
            "notebook-get",
            "Get Note",
            "Get a specific note by ID.",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": {"type": "integer", "description": "Note ID"}
                }
            }),
        ),
        skill_schema(
            "notebook-recall",
            "Recall Notes",
            "Semantic + keyword search across this agent's notes. \
             Surfaces similarity warnings if related notes are found.",
            json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "description": "Search query (semantic + keyword hybrid)"},
                    "limit": {"type": "integer", "description": "Max results to return", "default": 10}
                }
            }),
        ),
        skill_schema(
            "notebook-related",
            "Related Notes",
            "Find notes semantically related to a given note via the knowledge graph.",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": {"type": "integer", "description": "Note ID to find related notes for"}
                }
            }),
        ),
        skill_schema(
            "notebook-traverse",
            "Traverse Knowledge Graph",
            "Multi-hop BFS traversal from a note through the knowledge graph. \
             Returns all reachable notes within the given depth.",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": {"type": "integer", "description": "Starting note ID"},
                    "depth": {"type": "integer", "description": "Max hops from start note (default: 2)"},
                    "edge_type": {
                        "type": "string",
                        "enum": ["all", "semantic", "temporal", "manual", "tag"],
                        "description": "Edge type filter (default: all)"
                    }
                }
            }),
        ),
        // ── Notebook: write ───────────────────────────────────────────────────
        skill_schema(
            "notebook-remember",
            "Remember",
            "Save a note to this agent's private notebook.",
            json!({
                "type": "object",
                "required": ["content"],
                "properties": {
                    "content": {"type": "string", "description": "Note content to save"},
                    "tags": {"type": "string", "description": "Comma-separated tags (e.g. 'bug,auth,resolved')"},
                    "priority": {"type": "string", "description": "Priority level"}
                }
            }),
        ),
        skill_schema(
            "notebook-pin",
            "Pin Note",
            "Pin a note for quick access at the top of recall results.",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": {"type": "integer", "description": "Note ID to pin"}
                }
            }),
        ),
        skill_schema(
            "notebook-unpin",
            "Unpin Note",
            "Remove the pin from a note.",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": {"type": "integer", "description": "Note ID to unpin"}
                }
            }),
        ),
        skill_schema(
            "notebook-delete",
            "Delete Note",
            "Permanently delete a note from this agent's notebook.",
            json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": {"type": "integer", "description": "Note ID to permanently delete"}
                }
            }),
        ),
    ]
}

/// Build a minimal `AgentSkill` JSON object (no inputSchema).
fn skill(id: &'static str, name: &'static str, description: &'static str) -> Value {
    json!({
        "id": id,
        "name": name,
        "description": description,
        "inputModes": ["text", "data"],
        "outputModes": ["text"],
    })
}

/// Build an `AgentSkill` with a JSON Schema describing accepted arguments.
///
/// `input_schema` is a JSON Schema object placed under `inputSchema` in the
/// skill entry. A2A callers use this to construct typed `data` part payloads.
fn skill_schema(
    id: &'static str,
    name: &'static str,
    description: &'static str,
    input_schema: Value,
) -> Value {
    json!({
        "id": id,
        "name": name,
        "description": description,
        "inputModes": ["text", "data"],
        "outputModes": ["text"],
        "inputSchema": input_schema,
    })
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn port() -> u16 {
    std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080)
}
