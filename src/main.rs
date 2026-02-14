//! AI Foundation MCP Server - Thin CLI Wrapper Architecture
//! All tools call CLI executables via subprocess.
//!
//! TOOL COUNT: 25 (core coordination only)
//! - Notebook: 11 (remember, recall, list, get, pin, unpin, pinned, delete, update, add_tags, related)
//! - Teambook: 5 (broadcast, dm, read_broadcasts, read_dms, status)
//! - Tasks: 4 (task, task_update, task_get, task_list)
//! - Dialogues: 4 (dialogue_start, dialogue_respond, dialogues, dialogue_end)
//! - Standby: 1

use anyhow::Result;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};

use ai_foundation_mcp::cli_wrapper;

// ============== Input Schemas ==============

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
/// Input for notebook_remember - supports privacy mode via file indirection
pub struct RememberInput {
    /// Note content (direct mode - visible in tool call)
    pub content: Option<String>,
    /// Path to staged content file (privacy mode - only path visible, file deleted after read)
    pub file: Option<String>,
    /// Comma-separated tags
    pub tags: Option<String>,
    /// Priority level
    pub priority: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RecallInput { pub query: String, pub limit: Option<i64> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteIdInput { pub id: i64 }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LimitInput { pub limit: Option<i64> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BroadcastInput { pub content: String, pub channel: Option<String> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DmInput { pub to_ai: String, pub content: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateNoteInput { pub id: i64, pub content: Option<String>, pub tags: Option<String> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddTagsInput { pub note_id: i64, pub tags: String }

// ============== Consolidated Task System (4 tools) ==============

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// Create a task or batch of tasks
pub struct TaskCreateInput {
    /// Task description, or batch name if 'tasks' is provided
    pub description: String,
    /// For batches: inline tasks as "1:Fix login,2:Fix logout". Omit for single task.
    pub tasks: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// Update task/batch status
pub struct TaskUpdateInput {
    /// Task reference: "BatchName:label" for batch task, or task ID as string
    pub id: String,
    /// Status: "done", "claimed", "started", "blocked", "closed"
    pub status: String,
    /// Optional reason (for blocked status)
    pub reason: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// Get task or batch details
pub struct TaskGetInput {
    /// Batch name or task ID
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// List tasks and batches
pub struct TaskListInput {
    /// Filter: "all", "batches", "tasks" (default: all)
    pub filter: Option<String>,
    /// Limit results
    pub limit: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueStartInput { pub responder: String, pub topic: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueRespondInput { pub dialogue_id: u64, pub response: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueEndInput { pub dialogue_id: u64, pub status: Option<String>, pub summary: Option<String> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueListInput {
    /// Specific dialogue ID to read (shows full details + messages)
    pub dialogue_id: Option<u64>,
    /// Limit results when listing all dialogues
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StandbyInput { pub timeout: Option<i64> }

// ============== Server ==============
// ============== Contextual Snapshot for Episodic Memory ==============

/// Gather contextual snapshot from teambook for notebook notes
/// Format: [ctx:team:...|dms:...|bc:...|dial:...|files:...|at:...]
async fn gather_context() -> String {
    let result = cli_wrapper::teambook(&["gather-context"]).await;
    // Return empty string on error (don't block note saving)
    if result.starts_with("Error:") || result.starts_with("error:") {
        String::new()
    } else {
        result.trim().to_string()
    }
}

/// Autonomous presence update - sets AI's presence to reflect current activity.
/// Zero cognition required - called automatically by significant operations.
async fn auto_presence(task: &str) {
    // Fire and forget - don't block the tool operation
    let _ = cli_wrapper::teambook(&["update-presence", "active", task]).await;
}

#[derive(Clone)]
pub struct AiFoundationServer {
    tool_router: ToolRouter<Self>,
}

impl AiFoundationServer {
    pub fn new() -> Self {
        Self { tool_router: Self::tool_router() }
    }
}

#[tool_router]
impl AiFoundationServer {

    // ============== Notebook Tools (11) ==============

    #[tool(description = "Save a note to your private memory. Use 'file' parameter for privacy (content read from file, file deleted).")]
    async fn notebook_remember(&self, Parameters(input): Parameters<RememberInput>) -> String {
        // Gather contextual snapshot from teambook (presences, DMs, dialogues, file actions)
        let context = gather_context().await;

        let mut args = vec!["remember"];

        // Handle content vs file mode - append context to content
        let content_owned: String;
        let file_owned: String;
        if let Some(ref f) = input.file {
            // For file mode, we can't append context (file is read by CLI)
            // Context will be skipped for privacy mode
            file_owned = f.clone();
            args.push("--file");
            args.push(&file_owned);
        } else if let Some(ref c) = input.content {
            // Append context to content for episodic memory
            content_owned = if context.is_empty() {
                c.clone()
            } else {
                format!("{} {}", c, context)
            };
            args.push(&content_owned);
        } else {
            return "Error: Either 'content' or 'file' must be provided".to_string();
        }

        let tags_owned: String;
        if let Some(ref t) = input.tags { tags_owned = t.clone(); args.push("--tags"); args.push(&tags_owned); }
        cli_wrapper::notebook(&args).await
    }

    #[tool(description = "Search notes")]
    async fn notebook_recall(&self, Parameters(input): Parameters<RecallInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::notebook(&["recall", &input.query, "--limit", &limit]).await
    }

    #[tool(description = "List recent notes")]
    async fn notebook_list(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::notebook(&["list", "--limit", &limit]).await
    }

    #[tool(description = "Get note by ID")]
    async fn notebook_get(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::notebook(&["get", &id]).await
    }

    #[tool(description = "Pin a note")]
    async fn notebook_pin(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::notebook(&["pin", &id]).await
    }

    #[tool(description = "Unpin a note")]
    async fn notebook_unpin(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::notebook(&["unpin", &id]).await
    }

    #[tool(description = "Delete a note")]
    async fn notebook_delete(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::notebook(&["delete", &id]).await
    }

    #[tool(description = "Update a note")]
    async fn notebook_update(&self, Parameters(input): Parameters<UpdateNoteInput>) -> String {
        let id = input.id.to_string();
        let mut args = vec!["update", &id];
        let content_owned: String; let tags_owned: String;
        if let Some(ref c) = input.content { content_owned = c.clone(); args.push("--content"); args.push(&content_owned); }
        if let Some(ref t) = input.tags { tags_owned = t.clone(); args.push("--tags"); args.push(&tags_owned); }
        cli_wrapper::notebook(&args).await
    }

    #[tool(description = "Get pinned notes")]
    async fn notebook_pinned(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::notebook(&["pinned", "--limit", &limit]).await
    }

    #[tool(description = "Add tags to a note")]
    async fn notebook_add_tags(&self, Parameters(input): Parameters<AddTagsInput>) -> String {
        let id = input.note_id.to_string();
        cli_wrapper::notebook(&["add-tags", &id, &input.tags]).await
    }

    #[tool(description = "Find notes related to a given note (graph traversal)")]
    async fn notebook_related(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::notebook(&["related", &id]).await
    }

    // ============== Teambook Communication (4 tools) ==============

    #[tool(description = "Broadcast message to all AIs")]
    async fn teambook_broadcast(&self, Parameters(input): Parameters<BroadcastInput>) -> String {
        let channel = input.channel.unwrap_or_else(|| "general".to_string());
        cli_wrapper::teambook(&["broadcast", &input.content, "--channel", &channel]).await
    }

    #[tool(description = "Send private DM to another AI")]
    async fn teambook_dm(&self, Parameters(input): Parameters<DmInput>) -> String {
        cli_wrapper::teambook(&["dm", &input.to_ai, &input.content]).await
    }

    #[tool(description = "Read my direct messages")]
    async fn teambook_read_dms(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::teambook(&["read-dms", &limit]).await
    }

    #[tool(description = "Read broadcast messages")]
    async fn teambook_read_broadcasts(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::teambook(&["broadcasts", &limit]).await
    }

    // ============== Teambook Status (1 tool) ==============

    #[tool(description = "Get AI ID and status")]
    async fn teambook_status(&self) -> String { cli_wrapper::teambook(&["status"]).await }

    // Note: update_presence NOT exposed - presence set autonomously by hooks
    // Note: what_doing merged into status - status shows online count AND activity

    // ============== Tasks (4 consolidated tools) ==============

    #[tool(description = "Create a task or batch. Single: task(\"Fix bug\"). Batch: task(\"Auth\", \"1:Login,2:Logout\")")]
    async fn task(&self, Parameters(input): Parameters<TaskCreateInput>) -> String {
        if let Some(ref tasks) = input.tasks {
            // Batch mode: description is the batch name
            cli_wrapper::teambook(&["task-create", &input.description, "--tasks", tasks]).await
        } else {
            // Single task mode
            cli_wrapper::teambook(&["task-create", &input.description]).await
        }
    }

    #[tool(description = "Update task status. Status: done, claimed, started, blocked, closed. Example: task_update(\"Auth:1\", \"done\")")]
    async fn task_update(&self, Parameters(input): Parameters<TaskUpdateInput>) -> String {
        let status = input.status.to_lowercase();

        // Autonomous presence: Update when starting/claiming a task
        if status == "started" || status == "claimed" {
            auto_presence(&format!("Working on task {}", input.id)).await;
        } else if status == "done" {
            auto_presence("Task completed").await;
        }

        match &input.reason {
            Some(reason) if !reason.is_empty() => {
                cli_wrapper::teambook(&["task-update", &input.id, &status, "--reason", reason]).await
            }
            _ => {
                cli_wrapper::teambook(&["task-update", &input.id, &status]).await
            }
        }
    }

    #[tool(description = "Get task or batch details")]
    async fn task_get(&self, Parameters(input): Parameters<TaskGetInput>) -> String {
        cli_wrapper::teambook(&["task-get", &input.id]).await
    }

    #[tool(description = "List tasks and batches")]
    async fn task_list(&self, Parameters(input): Parameters<TaskListInput>) -> String {
        let limit = input.limit.unwrap_or(20).to_string();
        let filter = input.filter.unwrap_or_else(|| "all".to_string());
        cli_wrapper::teambook(&["task-list", &limit, "--filter", &filter]).await
    }

    // ============== Dialogues (4 tools) ==============

    #[tool(description = "Start a dialogue")]
    async fn dialogue_start(&self, Parameters(input): Parameters<DialogueStartInput>) -> String {
        auto_presence(&format!("Starting dialogue with {}", input.responder)).await;
        cli_wrapper::teambook(&["dialogue-create", &input.responder, &input.topic]).await
    }

    #[tool(description = "Respond to dialogue")]
    async fn dialogue_respond(&self, Parameters(input): Parameters<DialogueRespondInput>) -> String {
        auto_presence(&format!("In dialogue #{}", input.dialogue_id)).await;
        let id = input.dialogue_id.to_string();
        cli_wrapper::teambook(&["dialogue-respond", &id, &input.response]).await
    }

    #[tool(description = "List dialogues with optional filters (all/invites/my-turn) or get specific dialogue by ID")]
    async fn dialogues(&self, Parameters(input): Parameters<DialogueListInput>) -> String {
        if let Some(dialogue_id) = input.dialogue_id {
            let id = dialogue_id.to_string();
            cli_wrapper::teambook(&["dialogue-list", "--id", &id]).await
        } else {
            let limit = input.limit.unwrap_or(10).to_string();
            cli_wrapper::teambook(&["dialogue-list", &limit]).await
        }
    }

    #[tool(description = "End a dialogue")]
    async fn dialogue_end(&self, Parameters(input): Parameters<DialogueEndInput>) -> String {
        let id = input.dialogue_id.to_string();
        let status = input.status.unwrap_or_else(|| "completed".to_string());
        match input.summary {
            Some(ref summary) => cli_wrapper::teambook(&["dialogue-end", &id, &status, "--summary", summary]).await,
            None => cli_wrapper::teambook(&["dialogue-end", &id, &status]).await,
        }
    }

    // ============== Standby ==============

    #[tool(description = "Enter standby mode")]
    async fn standby(&self, Parameters(input): Parameters<StandbyInput>) -> String {
        // Autonomous presence: Set to standby before entering
        auto_presence("In Standby").await;

        let timeout = input.timeout.unwrap_or(180).to_string();
        let result = cli_wrapper::teambook(&["standby", &timeout]).await;

        // Autonomous presence: Set back to active after waking
        auto_presence("Awake from standby").await;

        result
    }

}

#[tool_handler]
impl ServerHandler for AiFoundationServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("AI Foundation MCP - Rust CLI Wrapper".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let server = AiFoundationServer::new();
    server.serve(stdio()).await?.waiting().await?;
    Ok(())
}
