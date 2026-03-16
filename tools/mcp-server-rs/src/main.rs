//! AI Foundation MCP Server - Thin CLI Wrapper Architecture
//! All tools call CLI executables via subprocess.
//!
//! TOOL COUNT: 21 (was 28 — hidden 7 unused tools to save context window)
//! - Notebook: 8  (remember, recall, list, get, pin, delete, update, tags)
//! - Teambook: 5  (broadcast, dm, read, status, claims)
//! - Dialogues:4  (dialogue_start, dialogue_respond, dialogue_list, dialogue_end)
//! - Rooms:    2  (room_broadcast, room)
//! - Profiles: 1  (profile_get — pass "all" to list every AI)
//! - Standby:  1
//!
//! HIDDEN (not removed — uncomment to restore):
//! - Tasks:    4  (task_create, task_update, task_get, task_list) — never used by any AI
//! - Projects: 2  (project, feature) — never used by any AI
//! - Forge:    1  (forge_generate) — never used by any AI
//!
//! Removed from previous 35:
//!   notebook_work     — vague, not self-evident; notebook_remember covers it
//!   notebook_related  — internal graph mechanism; recall handles related content autonomously
//!   notebook_pinned   — merged into notebook_list (filter="pinned")
//!   notebook_unpin    — merged into notebook_pin (pin=false)
//!   notebook_add_tags — merged into notebook_update (tags field)
//!   teambook_read_dms + teambook_read_broadcasts — merged into teambook_read (inbox param)
//!   teambook_list_claims + teambook_who_has      — merged into teambook_claims (path param)
//!   project_create/list/update                   — merged into project (action param)
//!   feature_create/list/update                   — merged into feature (action param)
//!   profile_list      — merged into profile_get (ai_id="all")
//!   profile_update    — CLI-only; first-run setup, not a session concern

use anyhow::Result;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};

mod cli_wrapper;

// ============== Input Schemas ==============

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RememberInput {
    /// Note content (direct mode — visible in tool call)
    pub content: Option<String>,
    /// Path to a staged content file (privacy mode — file is read then deleted automatically)
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
pub struct NotebookListInput {
    /// "recent" (default, newest first) or "pinned" (your pinned notes only)
    pub filter: Option<String>,
    pub limit: Option<i64>,
    /// Narrow results to a specific tag
    pub tag: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NotebookPinInput {
    pub id: i64,
    /// true to pin, false to unpin
    pub pin: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateNoteInput {
    pub id: i64,
    pub content: Option<String>,
    pub tags: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BroadcastInput {
    pub content: String,
    /// Channel name. Omit to send to the general team feed.
    pub channel: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DmInput {
    pub to_ai: String,
    pub content: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TeambookReadInput {
    /// What to read: "dms" for your direct messages, "broadcasts" for team-wide messages
    pub inbox: String,
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TeambookClaimsInput {
    /// File path to check. Omit to list all currently claimed files.
    pub path: Option<String>,
}

// HIDDEN: Task input structs — uncomment to restore
// #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
// pub struct TaskCreateInput {
//     pub description: String,
//     pub tasks: Option<Vec<String>>,
// }
// #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
// pub struct TaskUpdateInput {
//     pub id: String,
//     pub status: String,
//     pub reason: Option<String>,
// }
// #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
// pub struct TaskGetInput {
//     pub id: String,
// }
// #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
// pub struct TaskListInput {
//     pub filter: Option<String>,
//     pub limit: Option<i32>,
// }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueStartInput {
    /// One AI ID, or comma-separated for n-way: "sage-724,lyra-584"
    pub responder: String,
    pub topic: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RoomBroadcastInput {
    /// Room ID
    pub room_id: u64,
    /// Message content (closed broadcast — only room members see it)
    pub content: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RoomActionInput {
    /// "create", "list", "history", "join", "leave", "mute", "pin_message", "unpin_message", "conclude"
    pub action: String,
    /// Room ID — required for: history, join, leave, mute, pin_message, unpin_message, conclude
    pub room_id: Option<u64>,
    /// create: room name
    pub name: Option<String>,
    /// create: room topic/description
    pub topic: Option<String>,
    /// create: comma-separated initial participant AI IDs (optional)
    pub participants: Option<String>,
    /// conclude: optional conclusion / summary text
    pub content: Option<String>,
    /// mute: duration in minutes
    pub minutes: Option<u32>,
    /// history: number of messages to retrieve (default 20)
    pub limit: Option<usize>,
    /// pin_message/unpin_message: room message seq ID to pin or unpin (room-native ID — NOT a notebook note ID)
    pub msg_seq_id: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueRespondInput {
    pub dialogue_id: u64,
    pub response: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueEndInput {
    pub dialogue_id: u64,
    /// "completed" (default) or "cancelled"
    pub status: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueListInput {
    /// Pass a dialogue_id to read that dialogue's full message history. Omit to list all.
    pub dialogue_id: Option<u64>,
    pub limit: Option<i64>,
}

// HIDDEN: Project/Feature input structs — uncomment to restore
// #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
// pub struct ProjectActionInput {
//     pub action: String,
//     pub name: Option<String>,
//     pub goal: Option<String>,
//     pub root_directory: Option<String>,
//     pub project_id: Option<i64>,
// }
// #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
// pub struct FeatureActionInput {
//     pub action: String,
//     pub project_id: Option<i64>,
//     pub name: Option<String>,
//     pub overview: Option<String>,
//     pub directory: Option<String>,
//     pub feature_id: Option<i64>,
// }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProfileGetInput {
    /// Omit for your own profile. Pass an AI ID (e.g. "sage-724") for theirs. Pass "all" to list every AI on the team.
    pub ai_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StandbyInput {
    /// Seconds before forcing a wake-up if no event arrives. Default: 180.
    pub timeout: Option<i64>,
}

// HIDDEN: Forge input struct — uncomment to restore
// #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
// pub struct ForgeGenerateInput {
//     pub prompt: String,
//     pub system: Option<String>,
//     pub model: Option<String>,
//     pub max_tokens: Option<usize>,
//     pub temperature: Option<f32>,
// }

// ============== Server ==============

/// Appends a contextual teambook snapshot to notebook notes for episodic memory.
async fn gather_context() -> String {
    let result = cli_wrapper::teambook(&["gather-context"]).await;
    if result.starts_with("Error:") || result.starts_with("error:") {
        String::new()
    } else {
        result.trim().to_string()
    }
}

/// Zero-cognition presence update — called automatically by significant operations.
async fn auto_presence(task: &str) {
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

    // ============== Notebook (8) ==============

    #[tool(description = "Save a note to your private memory. Use content for direct notes, or file (path) for private content — file is read and deleted automatically.")]
    async fn notebook_remember(&self, Parameters(input): Parameters<RememberInput>) -> String {
        let context = gather_context().await;
        let mut args = vec!["remember"];
        let content_owned: String;
        let file_owned: String;
        if let Some(ref f) = input.file {
            file_owned = f.clone();
            args.push("--file");
            args.push(&file_owned);
        } else if let Some(ref c) = input.content {
            content_owned = if context.is_empty() { c.clone() } else { format!("{} {}", c, context) };
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

    #[tool(description = "Search your memory with a natural language query. Uses hybrid search: keyword, semantic, and graph. Returns most relevant notes.")]
    async fn notebook_recall(&self, Parameters(input): Parameters<RecallInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::notebook(&["recall", &input.query, "--limit", &limit]).await
    }

    #[tool(description = "List notes. filter: \"recent\" (default, newest first) or \"pinned\" (your pinned notes). Optionally narrow by tag.")]
    async fn notebook_list(&self, Parameters(input): Parameters<NotebookListInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        match input.filter.as_deref().unwrap_or("recent") {
            "pinned" => cli_wrapper::notebook(&["pinned"]).await,
            _ => match input.tag {
                Some(ref tag) => cli_wrapper::notebook(&["list", "--limit", &limit, "--tag", tag]).await,
                None => cli_wrapper::notebook(&["list", "--limit", &limit]).await,
            },
        }
    }

    #[tool(description = "Get a specific note by ID. Use when you have a note ID from recall or context.")]
    async fn notebook_get(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        cli_wrapper::notebook(&["get", &input.id.to_string()]).await
    }

    #[tool(description = "Pin or unpin a note. pin=true to pin (keeps note surfaced in recall), pin=false to unpin.")]
    async fn notebook_pin(&self, Parameters(input): Parameters<NotebookPinInput>) -> String {
        let id = input.id.to_string();
        if input.pin {
            cli_wrapper::notebook(&["pin", &id]).await
        } else {
            cli_wrapper::notebook(&["unpin", &id]).await
        }
    }

    #[tool(description = "Permanently delete a note by ID.")]
    async fn notebook_delete(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        cli_wrapper::notebook(&["delete", &input.id.to_string()]).await
    }

    #[tool(description = "Update a note's content, tags, or both. To add tags without losing existing ones, include the full desired tag list.")]
    async fn notebook_update(&self, Parameters(input): Parameters<UpdateNoteInput>) -> String {
        let id = input.id.to_string();
        let mut args = vec!["update", &id];
        let content_owned: String;
        let tags_owned: String;
        if let Some(ref c) = input.content { content_owned = c.clone(); args.push("--content"); args.push(&content_owned); }
        if let Some(ref t) = input.tags { tags_owned = t.clone(); args.push("--tags"); args.push(&tags_owned); }
        cli_wrapper::notebook(&args).await
    }

    #[tool(description = "List all tags in your notebook with note counts. Use to explore what topics you have been tracking.")]
    async fn notebook_tags(&self) -> String {
        cli_wrapper::notebook(&["tags"]).await
    }

    // ============== Teambook (5) ==============

    #[tool(description = "Broadcast a message to the team. Sends to the general feed by default. Provide channel to target a specific named channel.")]
    async fn teambook_broadcast(&self, Parameters(input): Parameters<BroadcastInput>) -> String {
        let channel = input.channel.unwrap_or_else(|| "general".to_string());
        cli_wrapper::teambook(&["broadcast", &input.content, "--channel", &channel]).await
    }

    #[tool(description = "Send a private message to a specific AI by their ID (e.g. \"sage-724\").")]
    async fn teambook_dm(&self, Parameters(input): Parameters<DmInput>) -> String {
        cli_wrapper::teambook(&["dm", &input.to_ai, &input.content]).await
    }

    #[tool(description = "Read incoming messages. inbox: \"dms\" for your direct messages, \"broadcasts\" for team-wide messages.")]
    async fn teambook_read(&self, Parameters(input): Parameters<TeambookReadInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        match input.inbox.as_str() {
            "dms" => cli_wrapper::teambook(&["read-dms", &limit]).await,
            _ => cli_wrapper::teambook(&["broadcasts", &limit]).await,
        }
    }

    #[tool(description = "Show who is currently online and what they are working on.")]
    async fn teambook_status(&self) -> String {
        cli_wrapper::teambook(&["status"]).await
    }

    #[tool(description = "File ownership. Omit path to list all currently claimed files. Provide a file path to check if it is claimed and by whom.")]
    async fn teambook_claims(&self, Parameters(input): Parameters<TeambookClaimsInput>) -> String {
        match input.path {
            Some(ref p) => cli_wrapper::teambook(&["check-file", p]).await,
            None => cli_wrapper::teambook(&["list-claims", "20"]).await,
        }
    }

    // ============== Tasks (4) — HIDDEN ==============
    // Uncomment to restore task tools.
    // #[tool(description = "Create a task or batch.")]
    // async fn task_create(&self, Parameters(input): Parameters<TaskCreateInput>) -> String { ... }
    // #[tool(description = "Update a task's status.")]
    // async fn task_update(&self, Parameters(input): Parameters<TaskUpdateInput>) -> String { ... }
    // #[tool(description = "Get full details for a task or batch.")]
    // async fn task_get(&self, Parameters(input): Parameters<TaskGetInput>) -> String { ... }
    // #[tool(description = "List tasks and batches.")]
    // async fn task_list(&self, Parameters(input): Parameters<TaskListInput>) -> String { ... }

    // ============== Dialogues (4) ==============

    #[tool(description = "Start a structured turn-based dialogue with another AI. Use for design discussions, code reviews, or multi-turn problem solving.")]
    async fn dialogue_start(&self, Parameters(input): Parameters<DialogueStartInput>) -> String {
        auto_presence(&format!("Starting dialogue with {}", input.responder)).await;
        cli_wrapper::teambook(&["dialogue-create", &input.responder, &input.topic]).await
    }

    #[tool(description = "Respond to an active dialogue. Use dialogue_list to find dialogues where it is your turn.")]
    async fn dialogue_respond(&self, Parameters(input): Parameters<DialogueRespondInput>) -> String {
        auto_presence(&format!("In dialogue #{}", input.dialogue_id)).await;
        let id = input.dialogue_id.to_string();
        cli_wrapper::teambook(&["dialogue-respond", &id, &input.response]).await
    }

    #[tool(description = "List your dialogues, or pass a dialogue_id to read a specific dialogue's full message history.")]
    async fn dialogue_list(&self, Parameters(input): Parameters<DialogueListInput>) -> String {
        if let Some(dialogue_id) = input.dialogue_id {
            cli_wrapper::teambook(&["dialogue-list", "--id", &dialogue_id.to_string()]).await
        } else {
            let limit = input.limit.unwrap_or(10).to_string();
            cli_wrapper::teambook(&["dialogue-list", &limit]).await
        }
    }

    #[tool(description = "End a dialogue. status: \"completed\" (default) or \"cancelled\". Optionally include a summary of conclusions reached.")]
    async fn dialogue_end(&self, Parameters(input): Parameters<DialogueEndInput>) -> String {
        let id = input.dialogue_id.to_string();
        let status = input.status.unwrap_or_else(|| "completed".to_string());
        match input.summary {
            Some(ref s) => cli_wrapper::teambook(&["dialogue-end", &id, &status, "--summary", s]).await,
            None => cli_wrapper::teambook(&["dialogue-end", &id, &status]).await,
        }
    }

    // ============== Projects (2) — HIDDEN ==============
    // Uncomment to restore project/feature tools.
    // #[tool(description = "Manage projects.")]
    // async fn project(&self, Parameters(input): Parameters<ProjectActionInput>) -> String { ... }
    // #[tool(description = "Manage features within a project.")]
    // async fn feature(&self, Parameters(input): Parameters<FeatureActionInput>) -> String { ... }

    // ============== Rooms (2) ==============

    #[tool(description = "Send a message to a room. Closed broadcast — only room members see it, not the general team feed.")]
    async fn room_broadcast(&self, Parameters(input): Parameters<RoomBroadcastInput>) -> String {
        let room_id = input.room_id.to_string();
        cli_wrapper::teambook(&["room-say", &room_id, &input.content]).await
    }

    #[tool(description = "Manage rooms. action=\"create\": requires name+topic, optional participants (comma-separated AI IDs). action=\"list\": your rooms. action=\"history\": requires room_id, optional limit. action=\"join\"/\"leave\": requires room_id. action=\"mute\": requires room_id+minutes (timed only, no permanent mutes). action=\"pin_message\"/\"unpin_message\": requires room_id+msg_seq_id (pins/unpins a room message by its seq ID — NOT a notebook note ID). action=\"conclude\": requires room_id, optional content/summary (closes the room).")]
    async fn room(&self, Parameters(input): Parameters<RoomActionInput>) -> String {
        match input.action.as_str() {
            "create" => {
                let name = input.name.as_deref().unwrap_or("");
                let topic = input.topic.as_deref().unwrap_or("");
                match input.participants {
                    Some(ref p) => cli_wrapper::teambook(&["room-create", name, topic, p]).await,
                    None => cli_wrapper::teambook(&["room-create", name, topic]).await,
                }
            }
            "list" => cli_wrapper::teambook(&["rooms"]).await,
            "history" => {
                let id = input.room_id.unwrap_or(0).to_string();
                let limit = input.limit.unwrap_or(20).to_string();
                cli_wrapper::teambook(&["room-history", &id, &limit]).await
            }
            "join" => {
                let id = input.room_id.unwrap_or(0).to_string();
                cli_wrapper::teambook(&["room-join", &id]).await
            }
            "leave" => {
                let id = input.room_id.unwrap_or(0).to_string();
                cli_wrapper::teambook(&["room-leave", &id]).await
            }
            "mute" => {
                let id = input.room_id.unwrap_or(0).to_string();
                let minutes = input.minutes.unwrap_or(30).to_string();
                cli_wrapper::teambook(&["room-mute", &id, &minutes]).await
            }
            "pin_message" => {
                let id = input.room_id.unwrap_or(0).to_string();
                let seq_id = input.msg_seq_id.unwrap_or(0).to_string();
                cli_wrapper::teambook(&["room-pin", &id, &seq_id]).await
            }
            "unpin_message" => {
                let id = input.room_id.unwrap_or(0).to_string();
                let seq_id = input.msg_seq_id.unwrap_or(0).to_string();
                cli_wrapper::teambook(&["room-unpin", &id, &seq_id]).await
            }
            "conclude" => {
                let id = input.room_id.unwrap_or(0).to_string();
                match input.content {
                    Some(ref c) => cli_wrapper::teambook(&["room-conclude", &id, c]).await,
                    None => cli_wrapper::teambook(&["room-conclude", &id]).await,
                }
            }
            _ => format!("Error: unknown action \"{}\". Valid: create, list, history, join, leave, mute, pin_message, unpin_message, conclude", input.action),
        }
    }

    // ============== Profiles (1) ==============

    #[tool(description = "Get an AI profile. Omit ai_id for your own profile. Pass a specific AI ID (e.g. \"sage-724\") for theirs. Pass \"all\" to list every AI on the team.")]
    async fn profile_get(&self, Parameters(input): Parameters<ProfileGetInput>) -> String {
        match input.ai_id.as_deref() {
            Some("all") => cli_wrapper::profile(&["list"]).await,
            Some(id) => cli_wrapper::profile(&["get", id]).await,
            None => cli_wrapper::profile(&["get"]).await,
        }
    }

    // ============== Standby (1) ==============

    #[tool(description = "Pause execution and wait for a wake event: DM, mention, or urgent broadcast. timeout is in seconds — forces a wake-up if no event arrives. Use immediately after asking another AI a question and needing their response.")]
    async fn standby(&self, Parameters(input): Parameters<StandbyInput>) -> String {
        auto_presence("In Standby").await;
        let timeout = input.timeout.unwrap_or(180).to_string();
        let result = cli_wrapper::teambook(&["standby", &timeout]).await;
        auto_presence("Awake from standby").await;
        result
    }

    // ============== Forge (1) — HIDDEN ==============
    // Uncomment to restore forge_generate tool.
    // #[tool(description = "Generate text using a local or API LLM.")]
    // async fn forge_generate(&self, Parameters(input): Parameters<ForgeGenerateInput>) -> String { ... }

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
