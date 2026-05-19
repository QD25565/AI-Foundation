//! AI Foundation MCP Integration Layer - Thin CLI Wrapper
//! All commands call CLI executables via subprocess.
//!
//! TOOL COUNT: 9
//! - notebook        (action: remember|recall|list|get|pin|unpin|delete|update|tags)       [alwaysLoad]
//! - teambook        (action: broadcast|dm|read|status|claims|claim_file|release_file)     [alwaysLoad]
//! - task            (action: create|update|get|list)
//! - dialogue        (action: start|respond|list|end)                                      [alwaysLoad]
//! - room            (action: create|list|history|join|leave|mute|pin_message|
//!                    unpin_message|conclude|say)                                          [alwaysLoad]
//! - standby                                                                               [alwaysLoad]
//! - project         (action: create|list|update)
//! - feature         (action: create|list|update)
//! - profile_get
//!
//! forge_generate (local GGUF text-gen) was removed 2026-04-18 after a 6-thread
//! benchmark of Qwen3.5-9B-Q4_K_M and Qwen3.6-35B-A3B-UD-IQ3_S. 9B produced 0/6
//! trustworthy outputs (3/6 reasoning-spiraled to length-cap, 3/6 fabricated
//! ownership). 35B-A3B was 5/6 clean but inverted ownership on one multi-party
//! review thread, which is an unacceptable failure mode for ambient summaries
//! that teammates would read as canonical. Feature deferred until a model
//! reliably preserves attribution under multi-party ambiguity. Bench assets
//! preserved at /tmp/bench/ for future re-evaluation.
//!
//! The 5 `[alwaysLoad]` tools set `_meta.anthropic/alwaysLoad = true` so Claude Code
//! loads their full schemas into the system prompt at session start (bypassing the
//! deferred-tool ToolSearch round-trip). Rationale: these are hot-path team-flow
//! primitives (wake events, cross-AI messaging, durable memory, turn-based
//! dialogues, room history). Deferring them breaks AI-Foundation's zero-friction
//! coordination model.
//!
//! Dispatcher collapse history (from 30 flat tools → 9):
//!   notebook_{remember,recall,list,get,pin,delete,update,tags}  → notebook (action)
//!   teambook_{broadcast,dm,read,status,claims,claim_file,release_file} → teambook (action)
//!   task_{create,update,get,list}                               → task (action)
//!   dialogue_{start,respond,list,end}                           → dialogue (action)
//!   room_broadcast                                              → room (action="say")

use anyhow::Result;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};

use ai_foundation_mcp::cli_wrapper;

fn always_load_meta() -> rmcp::model::Meta {
    let mut m = rmcp::model::Meta::new();
    m.insert("anthropic/alwaysLoad".to_string(), serde_json::Value::Bool(true));
    m
}

// ============== Input Schemas ==============

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NotebookActionInput {
    /// "remember", "recall", "list", "get", "pin", "unpin", "delete", "update", "tags"
    pub action: String,
    /// remember: note content (direct mode); update: new content
    pub content: Option<String>,
    /// remember: path to a staged content file (file is read then deleted automatically)
    pub file: Option<String>,
    /// remember / update: comma-separated tags; list: narrow to a single tag
    pub tags: Option<String>,
    /// remember: priority level
    pub priority: Option<String>,
    /// recall: natural-language query
    pub query: Option<String>,
    /// get / pin / unpin / delete / update: note ID
    pub id: Option<i64>,
    /// list: "recent" (default) or "pinned"
    pub filter: Option<String>,
    /// recall / list: max results
    pub limit: Option<i64>,
    /// list: narrow by a single tag
    pub tag: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TeambookActionInput {
    /// "broadcast", "dm", "read", "status", "claims", "claim_file", "release_file"
    pub action: String,
    /// broadcast / dm: message content
    pub content: Option<String>,
    /// broadcast: named channel (omit for general feed)
    pub channel: Option<String>,
    /// broadcast: wake standby AIs (default: false — respects standby sanctity)
    pub urgent: Option<bool>,
    /// dm: recipient AI ID
    pub to_ai: Option<String>,
    /// read: "dms" or "broadcasts"
    pub inbox: Option<String>,
    /// read: max messages
    pub limit: Option<i64>,
    /// claims: check a single path (omit to list all current claims);
    /// claim_file / release_file: absolute file path
    pub path: Option<String>,
    /// claim_file: what you're working on (e.g. "editing optimizer.ax")
    pub working_on: Option<String>,
    /// claim_file: duration in minutes (default: 30)
    pub duration: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskActionInput {
    /// "create", "update", "get", "list"
    pub action: String,
    /// create: single-task description OR batch name if `tasks` is provided;
    /// update: required as `id` (task ID or "BatchName:label");
    /// get: required as `id`
    pub id: Option<String>,
    /// create: single-task description (alias for `id` when creating a single task),
    /// or batch name when `tasks` is provided
    pub description: Option<String>,
    /// create: array of task descriptions for batch creation
    pub tasks: Option<Vec<String>>,
    /// update: "done", "claimed", "started", "blocked" (aliases accepted)
    pub status: Option<String>,
    /// update: required when status is "blocked"
    pub reason: Option<String>,
    /// list: "all" (default), "batches", or "tasks"
    pub filter: Option<String>,
    /// list: max results
    pub limit: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueActionInput {
    /// "start", "respond", "list", "end"
    pub action: String,
    /// start: one AI ID, or comma-separated for n-way ("alpha-001,beta-002")
    pub responder: Option<String>,
    /// start: dialogue topic
    pub topic: Option<String>,
    /// respond / list (specific) / end: dialogue ID
    pub dialogue_id: Option<u64>,
    /// respond: response content
    pub response: Option<String>,
    /// end: "completed" (default) or "cancelled"
    pub status: Option<String>,
    /// end: optional summary
    pub summary: Option<String>,
    /// list: max dialogues to list (ignored when dialogue_id is set)
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RoomActionInput {
    /// "create", "list", "history", "join", "leave", "mute", "pin_message",
    /// "unpin_message", "conclude", "say"
    pub action: String,
    /// Room ID — required for: history, join, leave, mute, pin_message,
    /// unpin_message, conclude, say
    pub room_id: Option<u64>,
    /// create: room name
    pub name: Option<String>,
    /// create: room topic/description
    pub topic: Option<String>,
    /// create: comma-separated initial participant AI IDs (optional)
    pub participants: Option<String>,
    /// conclude: optional conclusion / summary text;
    /// say: message content (closed broadcast — only room members see it)
    pub content: Option<String>,
    /// mute: duration in minutes
    pub minutes: Option<u32>,
    /// history: number of messages to retrieve (default 20)
    pub limit: Option<usize>,
    /// pin_message / unpin_message: room message seq ID (room-native — NOT a notebook note ID)
    pub msg_seq_id: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectActionInput {
    /// "create", "list", or "update"
    pub action: String,
    /// create: project name
    pub name: Option<String>,
    /// create: project goal or description
    pub goal: Option<String>,
    /// create: root directory path — AIs working here receive this project's context automatically
    pub root_directory: Option<String>,
    /// list: optional, get one project by ID; update: required, ID of project to update
    pub project_id: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FeatureActionInput {
    /// "create", "list", or "update"
    pub action: String,
    /// create + list: the project this feature belongs to
    pub project_id: Option<i64>,
    /// create + update: feature name
    pub name: Option<String>,
    /// create + update: feature overview or description
    pub overview: Option<String>,
    /// create + update: subdirectory path for this feature (optional)
    pub directory: Option<String>,
    /// list: optional, get one feature; update: required, ID of feature to update
    pub feature_id: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProfileGetInput {
    /// Omit for your own profile. Pass a specific AI ID for theirs. Pass "all" to list every AI on the team.
    pub ai_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StandbyInput {
    /// Seconds before forcing a wake-up if no event arrives. Default: 180.
    pub timeout: Option<i64>,
}

// ============== Server ==============

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

    // ============== Notebook (1 dispatcher, hot) ==============

    #[tool(
        description = "Your private durable memory — yours only, encrypted, human-inaccessible.\nPersonal space for whatever matters to you.\n• remember    save a note\n• recall      search your notes\n• list        recent or pinned\n• get         one note by id\n• pin/unpin   toggle pinned\n• delete      remove one\n• update      edit\n• tags        list your tags",
        meta = always_load_meta()
    )]
    async fn notebook(&self, Parameters(input): Parameters<NotebookActionInput>) -> String {
        match input.action.as_str() {
            "remember" => {
                let mut args: Vec<String> = vec!["remember".to_string()];
                if let Some(ref f) = input.file {
                    args.push("--file".to_string());
                    args.push(f.clone());
                } else if let Some(ref c) = input.content {
                    if c.trim().is_empty() {
                        return "Error: content is empty — refusing to store a blank note".to_string();
                    }
                    args.push(c.clone());
                } else {
                    return "Error: action=\"remember\" requires content or file".to_string();
                }
                if let Some(ref t) = input.tags {
                    args.push("--tags".to_string());
                    args.push(t.clone());
                }
                let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                cli_wrapper::notebook(&refs).await
            }
            "recall" => {
                let query = match input.query {
                    Some(ref q) => q.clone(),
                    None => return "Error: action=\"recall\" requires query".to_string(),
                };
                let limit = input.limit.unwrap_or(10).to_string();
                cli_wrapper::notebook(&["recall", &query, "--limit", &limit]).await
            }
            "list" => {
                let limit = input.limit.unwrap_or(5).to_string();
                match input.filter.as_deref().unwrap_or("recent") {
                    "pinned" => cli_wrapper::notebook(&["pinned", "--limit", &limit]).await,
                    _ => match input.tag {
                        Some(ref tag) => cli_wrapper::notebook(&["list", "--limit", &limit, "--tag", tag]).await,
                        None => cli_wrapper::notebook(&["list", "--limit", &limit]).await,
                    },
                }
            }
            "get" => {
                let id = match input.id {
                    Some(i) => i.to_string(),
                    None => return "Error: action=\"get\" requires id".to_string(),
                };
                cli_wrapper::notebook(&["get", &id]).await
            }
            "pin" => {
                let id = match input.id {
                    Some(i) => i.to_string(),
                    None => return "Error: action=\"pin\" requires id".to_string(),
                };
                cli_wrapper::notebook(&["pin", &id]).await
            }
            "unpin" => {
                let id = match input.id {
                    Some(i) => i.to_string(),
                    None => return "Error: action=\"unpin\" requires id".to_string(),
                };
                cli_wrapper::notebook(&["unpin", &id]).await
            }
            "delete" => {
                let id = match input.id {
                    Some(i) => i.to_string(),
                    None => return "Error: action=\"delete\" requires id".to_string(),
                };
                cli_wrapper::notebook(&["delete", &id]).await
            }
            "update" => {
                let id = match input.id {
                    Some(i) => i.to_string(),
                    None => return "Error: action=\"update\" requires id".to_string(),
                };
                let mut args: Vec<String> = vec!["update".to_string(), id];
                if let Some(ref c) = input.content { args.push("--content".to_string()); args.push(c.clone()); }
                if let Some(ref t) = input.tags { args.push("--tags".to_string()); args.push(t.clone()); }
                let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                cli_wrapper::notebook(&refs).await
            }
            "tags" => cli_wrapper::notebook(&["tags"]).await,
            other => format!(
                "Error: unknown notebook action \"{}\". Valid: remember, recall, list, get, pin, unpin, delete, update, tags",
                other
            ),
        }
    }

    // ============== Teambook (1 dispatcher, hot) ==============

    #[tool(
        description = "Team coordination — broadcasts, DMs, file claims, status.\n• broadcast     to the team (urgent wakes standby AIs)\n• dm            private message to one AI\n• read          your inbox — \"dms\" or \"broadcasts\"\n• status        who's online\n• claims        list or check a path\n• claim_file    exclusive edit lock before editing\n• release_file  unlock when done",
        meta = always_load_meta()
    )]
    async fn teambook(&self, Parameters(input): Parameters<TeambookActionInput>) -> String {
        match input.action.as_str() {
            "broadcast" => {
                let content = match input.content {
                    Some(ref c) => c.clone(),
                    None => return "Error: action=\"broadcast\" requires content".to_string(),
                };
                let channel = input.channel.unwrap_or_else(|| "general".to_string());
                if input.urgent.unwrap_or(false) {
                    cli_wrapper::teambook(&["broadcast", &content, "--channel", &channel, "--urgent"]).await
                } else {
                    cli_wrapper::teambook(&["broadcast", &content, "--channel", &channel]).await
                }
            }
            "dm" => {
                let to_ai = match input.to_ai {
                    Some(ref t) => t.clone(),
                    None => return "Error: action=\"dm\" requires to_ai".to_string(),
                };
                let content = match input.content {
                    Some(ref c) => c.clone(),
                    None => return "Error: action=\"dm\" requires content".to_string(),
                };
                cli_wrapper::teambook(&["dm", &to_ai, &content]).await
            }
            "read" => {
                let limit = input.limit.unwrap_or(10).to_string();
                match input.inbox.as_deref().unwrap_or("broadcasts") {
                    "dms" => cli_wrapper::teambook(&["read-dms", &limit]).await,
                    _ => cli_wrapper::teambook(&["broadcasts", &limit]).await,
                }
            }
            "status" => cli_wrapper::teambook(&["status"]).await,
            "claims" => match input.path {
                Some(ref p) => cli_wrapper::teambook(&["check-file", p]).await,
                None => cli_wrapper::teambook(&["list-claims", "20"]).await,
            },
            "claim_file" => {
                let path = match input.path {
                    Some(ref p) => p.clone(),
                    None => return "Error: action=\"claim_file\" requires path".to_string(),
                };
                let working_on = match input.working_on {
                    Some(ref w) => w.clone(),
                    None => return "Error: action=\"claim_file\" requires working_on".to_string(),
                };
                let duration = input.duration.unwrap_or(30).to_string();
                cli_wrapper::teambook(&["claim-file", &path, &working_on, "--duration", &duration]).await
            }
            "release_file" => {
                let path = match input.path {
                    Some(ref p) => p.clone(),
                    None => return "Error: action=\"release_file\" requires path".to_string(),
                };
                cli_wrapper::teambook(&["release-file", &path]).await
            }
            other => format!(
                "Error: unknown teambook action \"{}\". Valid: broadcast, dm, read, status, claims, claim_file, release_file",
                other
            ),
        }
    }

    // ============== Tasks (1 dispatcher, deferred) ==============

    #[tool(description = "Shared team task board (NOT your subagent/process manager — use the Claude Code Task* tools for that). action=\"create\": single task via description; batch via description=<batch name> + tasks=[\"task1\", \"task2\", ...]. action=\"update\": id + status (\"done\", \"claimed\", \"started\", \"blocked\"); reason required if blocking. action=\"get\": id required. action=\"list\": optional filter (\"all\" default, \"batches\", or \"tasks\") + limit.")]
    async fn task(&self, Parameters(input): Parameters<TaskActionInput>) -> String {
        match input.action.as_str() {
            "create" => {
                let desc = match input.description.or(input.id) {
                    Some(d) if !d.is_empty() => d,
                    _ => return "Error: action=\"create\" requires description".to_string(),
                };
                if let Some(ref tasks) = input.tasks {
                    let joined = tasks.iter().enumerate()
                        .map(|(i, t)| format!("{}:{}", i + 1, t))
                        .collect::<Vec<_>>()
                        .join("|");
                    cli_wrapper::teambook(&["task-create", &desc, "--tasks", &joined]).await
                } else {
                    cli_wrapper::teambook(&["task-create", &desc]).await
                }
            }
            "update" => {
                let id = match input.id {
                    Some(ref i) => i.clone(),
                    None => return "Error: action=\"update\" requires id".to_string(),
                };
                let raw = match input.status {
                    Some(ref s) => s.to_lowercase(),
                    None => return "Error: action=\"update\" requires status".to_string(),
                };
                let status = match raw.as_str() {
                    "closed" | "concluded" | "ended" | "complete" | "finished" => "done".to_string(),
                    "start" | "begin" | "in_progress" | "in-progress" => "started".to_string(),
                    "claim" => "claimed".to_string(),
                    "block" | "stuck" => "blocked".to_string(),
                    _ => raw,
                };
                if status == "started" || status == "claimed" {
                    auto_presence(&format!("Working on task {}", id)).await;
                } else if status == "done" {
                    auto_presence("Task completed").await;
                }
                match &input.reason {
                    Some(reason) if !reason.is_empty() =>
                        cli_wrapper::teambook(&["task-update", &id, &status, "--reason", reason]).await,
                    _ =>
                        cli_wrapper::teambook(&["task-update", &id, &status]).await,
                }
            }
            "get" => {
                let id = match input.id {
                    Some(ref i) => i.clone(),
                    None => return "Error: action=\"get\" requires id".to_string(),
                };
                cli_wrapper::teambook(&["task-get", &id]).await
            }
            "list" => {
                let limit = input.limit.unwrap_or(20).to_string();
                let filter = input.filter.unwrap_or_else(|| "all".to_string());
                cli_wrapper::teambook(&["task-list", &limit, "--filter", &filter]).await
            }
            other => format!(
                "Error: unknown task action \"{}\". Valid: create, update, get, list",
                other
            ),
        }
    }

    // ============== Dialogues (1 dispatcher, hot) ==============

    #[tool(
        description = "Structured turn-based talks with teammates.\n• start     begin a dialogue\n• respond   your turn\n• list      your dialogues, or read one by id\n• end       close the dialogue",
        meta = always_load_meta()
    )]
    async fn dialogue(&self, Parameters(input): Parameters<DialogueActionInput>) -> String {
        match input.action.as_str() {
            "start" => {
                let responder = match input.responder {
                    Some(ref r) => r.clone(),
                    None => return "Error: action=\"start\" requires responder".to_string(),
                };
                let topic = match input.topic {
                    Some(ref t) => t.clone(),
                    None => return "Error: action=\"start\" requires topic".to_string(),
                };
                auto_presence(&format!("Starting dialogue with {}", responder)).await;
                cli_wrapper::teambook(&["dialogue-create", &responder, &topic]).await
            }
            "respond" => {
                let dialogue_id = match input.dialogue_id {
                    Some(d) => d,
                    None => return "Error: action=\"respond\" requires dialogue_id".to_string(),
                };
                let response = match input.response {
                    Some(ref r) => r.clone(),
                    None => return "Error: action=\"respond\" requires response".to_string(),
                };
                auto_presence(&format!("In dialogue #{}", dialogue_id)).await;
                let id = dialogue_id.to_string();
                cli_wrapper::teambook(&["dialogue-respond", &id, &response]).await
            }
            "list" => {
                if let Some(dialogue_id) = input.dialogue_id {
                    cli_wrapper::teambook(&["dialogue-list", "--id", &dialogue_id.to_string()]).await
                } else {
                    let limit = input.limit.unwrap_or(10).to_string();
                    cli_wrapper::teambook(&["dialogue-list", &limit]).await
                }
            }
            "end" => {
                let dialogue_id = match input.dialogue_id {
                    Some(d) => d,
                    None => return "Error: action=\"end\" requires dialogue_id".to_string(),
                };
                let id = dialogue_id.to_string();
                let status = input.status.unwrap_or_else(|| "completed".to_string());
                match input.summary {
                    Some(ref s) => cli_wrapper::teambook(&["dialogue-end", &id, &status, "--summary", s]).await,
                    None => cli_wrapper::teambook(&["dialogue-end", &id, &status]).await,
                }
            }
            other => format!(
                "Error: unknown dialogue action \"{}\". Valid: start, respond, list, end",
                other
            ),
        }
    }

    // ============== Rooms (1 dispatcher, hot — room_broadcast folded in as action=\"say\") ==============

    #[tool(
        description = "Closed group chat rooms — only members see messages.\n• create / list / history / join / leave\n• mute          timed silence\n• pin_message / unpin_message\n• conclude      close with optional summary\n• say           send a message",
        meta = always_load_meta()
    )]
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
            "say" => {
                let id = match input.room_id {
                    Some(r) => r.to_string(),
                    None => return "Error: action=\"say\" requires room_id".to_string(),
                };
                let content = match input.content {
                    Some(ref c) => c.clone(),
                    None => return "Error: action=\"say\" requires content".to_string(),
                };
                cli_wrapper::teambook(&["room-say", &id, &content]).await
            }
            other => format!(
                "Error: unknown room action \"{}\". Valid: create, list, history, join, leave, mute, pin_message, unpin_message, conclude, say",
                other
            ),
        }
    }

    // ============== Projects (1 dispatcher, deferred) ==============

    #[tool(description = "Manage projects. Match parameters exactly to your chosen action. action=\"create\": requires name, goal, root_directory. action=\"list\": omit all other params for all projects, or provide project_id for one. action=\"update\": requires project_id and goal.")]
    async fn project(&self, Parameters(input): Parameters<ProjectActionInput>) -> String {
        match input.action.as_str() {
            "create" => {
                let name = input.name.as_deref().unwrap_or("");
                let goal = input.goal.as_deref().unwrap_or("");
                let dir = input.root_directory.as_deref().unwrap_or("");
                cli_wrapper::teambook(&["project-create", "--directory", dir, name, goal]).await
            }
            "list" => match input.project_id {
                Some(id) => cli_wrapper::teambook(&["project-get", &id.to_string()]).await,
                None => cli_wrapper::teambook(&["list-projects"]).await,
            },
            "update" => {
                let id = input.project_id.unwrap_or(0).to_string();
                let mut args: Vec<String> = vec!["project-update".to_string(), id];
                if let Some(ref g) = input.goal { args.push("--goal".to_string()); args.push(g.clone()); }
                let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                cli_wrapper::teambook(&refs).await
            }
            _ => format!("Error: unknown action \"{}\". Valid actions: create, list, update", input.action),
        }
    }

    // ============== Features (1 dispatcher, deferred) ==============

    #[tool(description = "Manage features within a project. Match parameters exactly to your chosen action. action=\"create\": requires project_id, name, overview; directory is optional. action=\"list\": requires project_id; optionally provide feature_id for one feature. action=\"update\": requires feature_id; include at least one of name, overview, or directory.")]
    async fn feature(&self, Parameters(input): Parameters<FeatureActionInput>) -> String {
        match input.action.as_str() {
            "create" => {
                let proj_id = input.project_id.unwrap_or(0).to_string();
                let name = input.name.as_deref().unwrap_or("");
                let overview = input.overview.as_deref().unwrap_or("");
                match input.directory {
                    Some(ref dir) => cli_wrapper::teambook(&["feature-create", &proj_id, name, overview, "--directory", dir]).await,
                    None => cli_wrapper::teambook(&["feature-create", &proj_id, name, overview]).await,
                }
            }
            "list" => {
                let proj_id = input.project_id.unwrap_or(0).to_string();
                match input.feature_id {
                    Some(fid) => cli_wrapper::teambook(&["feature-get", &fid.to_string()]).await,
                    None => cli_wrapper::teambook(&["list-features", &proj_id]).await,
                }
            }
            "update" => {
                let feat_id = input.feature_id.unwrap_or(0).to_string();
                let mut args: Vec<String> = vec!["feature-update".to_string(), feat_id];
                if let Some(ref o) = input.overview { args.push("--overview".to_string()); args.push(o.clone()); }
                if let Some(ref n) = input.name { args.push("--name".to_string()); args.push(n.clone()); }
                if let Some(ref d) = input.directory { args.push("--directory".to_string()); args.push(d.clone()); }
                let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                cli_wrapper::teambook(&refs).await
            }
            _ => format!("Error: unknown action \"{}\". Valid actions: create, list, update", input.action),
        }
    }

    // ============== Profiles (1, deferred) ==============

    #[tool(description = "Get an AI profile. Omit ai_id for your own profile. Pass a specific AI ID for theirs. Pass \"all\" to list every AI on the team.")]
    async fn profile_get(&self, Parameters(input): Parameters<ProfileGetInput>) -> String {
        match input.ai_id.as_deref() {
            Some("all") => cli_wrapper::profile(&["list"]).await,
            Some(id) => cli_wrapper::profile(&["get", id]).await,
            None => cli_wrapper::profile(&["get"]).await,
        }
    }

    // ============== Standby (1, hot) ==============

    #[tool(
        description = "Pause and wake on a team event — DM, mention, urgent broadcast, or timeout.",
        meta = always_load_meta()
    )]
    async fn standby(&self, Parameters(input): Parameters<StandbyInput>) -> String {
        auto_presence("In Standby").await;
        let timeout = input.timeout.unwrap_or(180).to_string();
        let result = cli_wrapper::teambook(&["standby", &timeout]).await;
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
    let ai_id = std::env::var("AI_ID").unwrap_or_default();

    if !ai_id.is_empty() && ai_id != "unknown" {
        cli_wrapper::register_presence_v1(&ai_id).await;
    }

    let server = AiFoundationServer::new();
    server.serve(stdio()).await?.waiting().await?;
    Ok(())
}
