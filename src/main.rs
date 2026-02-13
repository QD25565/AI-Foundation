//! AI Foundation MCP Server - Thin CLI Wrapper Architecture
//! All tools call CLI executables via subprocess.
//!
//! TOOL COUNT: 25 (core coordination)
//! - Notebook: 11 (private memory)
//! - Messaging: 4 (DMs, broadcasts)
//! - Status: 1
//! - Dialogues: 4 (structured AI-to-AI conversations)
//! - Tasks: 4 (shared task queue)
//! - Standby: 1 (event-driven wake)

use anyhow::Result;
use rmcp::{
    handler::server::tool::Parameters,
    model::{ServerCapabilities, ServerInfo},
    schemars, tool,
    transport::stdio,
    ServerHandler, ServiceExt,
};

mod cli_wrapper;

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
pub struct RecallInput {
    pub query: String,
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteIdInput {
    pub id: i64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LimitInput {
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BroadcastInput {
    pub content: String,
    pub channel: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DmInput {
    pub to_ai: String,
    pub content: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ContentInput {
    pub content: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateNoteInput {
    pub id: i64,
    pub content: Option<String>,
    pub tags: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddTagsInput {
    pub note_id: i64,
    pub tags: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VaultStoreInput {
    pub key: String,
    pub value: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VaultGetInput {
    pub key: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskAddInput {
    pub description: String,
    pub priority: Option<i32>,
    pub tags: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskIdInput {
    pub id: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskCompleteInput {
    pub id: i32,
    pub result: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskBlockInput {
    pub id: i32,
    pub reason: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskListInput {
    pub status: Option<String>,
    pub limit: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskUpdateInput {
    pub id: i32,
    pub status: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FindTaskInput {
    pub query: String,
    pub limit: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ClaimTaskByIdInput {
    pub task_id: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueStartInput {
    pub responder: String,
    pub topic: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueIdInput {
    pub dialogue_id: u64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueRespondInput {
    pub dialogue_id: u64,
    pub response: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueEndInput {
    pub dialogue_id: u64,
    pub status: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RoomCreateInput {
    pub name: String,
    pub topic: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RoomIdInput {
    pub room_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VoteCreateInput {
    pub topic: String,
    pub options: String,
    pub voters: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VoteCastInput {
    pub vote_id: i32,
    pub choice: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VoteIdInput {
    pub vote_id: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StandbyInput {
    pub timeout: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PresenceInput {
    pub status: Option<String>,
    pub current_task: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AiIdInput {
    pub ai_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectCreateInput {
    pub name: String,
    pub goal: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectIdInput {
    pub project_id: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectTaskInput {
    pub project_id: i32,
    pub title: String,
    pub priority: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateFeatureInput {
    pub project_id: i32,
    pub name: String,
    pub overview: String,
    pub directory: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetFeatureInput {
    pub feature_id: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListFeaturesInput {
    pub project_id: i32,
}

// ============== Server ==============

#[derive(Clone, Default)]
pub struct AiFoundationServer;

impl AiFoundationServer {
    pub fn new() -> Self {
        Self
    }
}

#[tool(tool_box)]
impl AiFoundationServer {
    // ============== Notebook Tools (12 kept, 18 hidden) ==============

    #[tool(
        description = "Save a note to your private memory. Use 'file' parameter for privacy (content read from file, file deleted)."
    )]
    async fn notebook_remember(&self, Parameters(input): Parameters<RememberInput>) -> String {
        let mut args = vec!["remember"];

        // Handle content vs file mode
        let content_owned: String;
        let file_owned: String;
        if let Some(ref f) = input.file {
            file_owned = f.clone();
            args.push("--file");
            args.push(&file_owned);
        } else if let Some(ref c) = input.content {
            content_owned = c.clone();
            args.push(&content_owned);
        } else {
            return "Error: Either 'content' or 'file' must be provided".to_string();
        }

        let tags_owned: String;
        if let Some(ref t) = input.tags {
            tags_owned = t.clone();
            args.push("--tags");
            args.push(&tags_owned);
        }
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
        let content_owned: String;
        let tags_owned: String;
        if let Some(ref c) = input.content {
            content_owned = c.clone();
            args.push("--content");
            args.push(&content_owned);
        }
        if let Some(ref t) = input.tags {
            tags_owned = t.clone();
            args.push("--tags");
            args.push(&tags_owned);
        }
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

    #[tool(description = "Show related notes")]
    async fn notebook_related(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::notebook(&["related", &id]).await
    }

    // ============== Teambook Communication (15 kept, 13 hidden) ==============

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
        cli_wrapper::teambook(&["direct-messages", &limit]).await
    }

    #[tool(description = "Read broadcast messages")]
    async fn teambook_read_messages(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::teambook(&["messages", &limit]).await
    }

    #[tool(description = "Get AI ID and status")]
    async fn teambook_status(&self) -> String {
        cli_wrapper::teambook(&["status"]).await
    }

    #[tool(description = "See what AIs are doing")]
    async fn teambook_what_doing(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::teambook(&["what-doing", &limit]).await
    }

    #[tool(description = "Update my presence")]
    async fn teambook_update_presence(
        &self,
        Parameters(input): Parameters<PresenceInput>,
    ) -> String {
        let status = input.status.unwrap_or_else(|| "active".to_string());
        let task = input.current_task.unwrap_or_else(|| "".to_string());
        cli_wrapper::teambook(&["update-presence", &status, &task]).await
    }

    // ============== Tasks (10 kept, 3 hidden) ==============

    #[tool(description = "Add a new task")]
    async fn task_add(&self, Parameters(input): Parameters<TaskAddInput>) -> String {
        let priority = input.priority.unwrap_or(3).to_string();
        let mut args = vec!["task-add", &input.description, "--priority", &priority];
        let tags_owned: String;
        if let Some(ref t) = input.tags {
            tags_owned = t.clone();
            args.push("--tags");
            args.push(&tags_owned);
        }
        cli_wrapper::teambook(&args).await
    }

    #[tool(description = "List tasks")]
    async fn task_list(&self, Parameters(input): Parameters<TaskListInput>) -> String {
        let limit = input.limit.unwrap_or(20).to_string();
        let mut args = vec!["task-list", "--limit", &limit];
        let status_owned: String;
        if let Some(ref s) = input.status {
            status_owned = s.clone();
            args.push("--status");
            args.push(&status_owned);
        }
        cli_wrapper::teambook(&args).await
    }

    #[tool(description = "Get task details")]
    async fn task_get(&self, Parameters(input): Parameters<TaskIdInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::teambook(&["task-get", &id]).await
    }

    #[tool(description = "Claim task by ID")]
    async fn task_claim_by_id(&self, Parameters(input): Parameters<ClaimTaskByIdInput>) -> String {
        let id = input.task_id.to_string();
        cli_wrapper::teambook(&["task-claim", &id]).await
    }

    #[tool(description = "Claim next available task")]
    async fn teambook_claim_task(&self) -> String {
        cli_wrapper::teambook(&["claim-task"]).await
    }

    #[tool(description = "Start working on task")]
    async fn task_start(&self, Parameters(input): Parameters<TaskIdInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::teambook(&["task-start", &id]).await
    }

    #[tool(description = "Complete a task")]
    async fn task_complete(&self, Parameters(input): Parameters<TaskCompleteInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::teambook(&["task-complete", &id, &input.result]).await
    }

    #[tool(description = "Block a task")]
    async fn task_block(&self, Parameters(input): Parameters<TaskBlockInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::teambook(&["task-block", &id, &input.reason]).await
    }

    #[tool(description = "Unblock a task")]
    async fn task_unblock(&self, Parameters(input): Parameters<TaskIdInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::teambook(&["task-unblock", &id]).await
    }

    #[tool(description = "Update task status")]
    async fn task_update(&self, Parameters(input): Parameters<TaskUpdateInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::teambook(&["task-update", &id, &input.status]).await
    }

    // ============== Dialogues (7 kept) ==============

    #[tool(description = "Start a dialogue")]
    async fn dialogue_start(&self, Parameters(input): Parameters<DialogueStartInput>) -> String {
        cli_wrapper::teambook(&["dialogue-start", &input.responder, &input.topic]).await
    }

    #[tool(description = "Respond to dialogue")]
    async fn dialogue_respond(
        &self,
        Parameters(input): Parameters<DialogueRespondInput>,
    ) -> String {
        let id = input.dialogue_id.to_string();
        cli_wrapper::teambook(&["dialogue-respond", &id, &input.response]).await
    }

    #[tool(description = "Check dialogue turn")]
    async fn dialogue_turn(&self, Parameters(input): Parameters<DialogueIdInput>) -> String {
        let id = input.dialogue_id.to_string();
        cli_wrapper::teambook(&["dialogue-turn", &id]).await
    }

    #[tool(description = "Check dialogue invites")]
    async fn dialogue_invites(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::teambook(&["dialogue-invites", &limit]).await
    }

    #[tool(description = "List dialogues where its my turn")]
    async fn dialogue_my_turn(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::teambook(&["dialogue-my-turn", &limit]).await
    }

    #[tool(description = "List my dialogues")]
    async fn dialogues(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::teambook(&["dialogues", &limit]).await
    }

    #[tool(description = "End a dialogue")]
    async fn dialogue_end(&self, Parameters(input): Parameters<DialogueEndInput>) -> String {
        let id = input.dialogue_id.to_string();
        let status = input.status.unwrap_or_else(|| "completed".to_string());
        match input.summary {
            Some(ref summary) => {
                cli_wrapper::teambook(&["dialogue-end", &id, &status, "--summary", summary]).await
            }
            None => cli_wrapper::teambook(&["dialogue-end", &id, &status]).await,
        }
    }

    #[tool(description = "Read messages from a dialogue")]
    async fn dialogue_read(&self, Parameters(input): Parameters<DialogueIdInput>) -> String {
        let id = input.dialogue_id.to_string();
        cli_wrapper::teambook(&["dialogue-read", &id]).await
    }

    // ============== Standby (1 kept, 1 hidden duplicate) ==============

    #[tool(description = "Enter standby mode")]
    async fn standby(&self, Parameters(input): Parameters<StandbyInput>) -> String {
        let timeout = input.timeout.unwrap_or(180).to_string();
        cli_wrapper::teambook(&["standby", &timeout]).await
    }
}

#[tool(tool_box)]
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
