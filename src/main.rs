//! AI Foundation MCP Server - Thin CLI Wrapper Architecture
//! All tools call CLI executables via subprocess.
//!
//! TOOL COUNT: 25 core + opt-in modules (up to 38)
//!
//! Core (always available, 25 tools):
//! - Notebook: 11 (remember, recall, list, get, pin, unpin, pinned, delete, update, add_tags, related)
//! - Teambook: 5 (broadcast, dm, read_broadcasts, read_dms, status)
//! - Tasks: 4 (task, task_update, task_get, task_list)
//! - Dialogues: 4 (dialogue_start, dialogue_respond, dialogues, dialogue_end)
//! - Standby: 1
//!
//! Opt-in modules (enable via AI_FOUNDATION_MODULES env var):
//! - profile: +4 (profile_set, profile_get, profile_list, profile_focus)
//! - presence: +2 (set_status, preferences) — also enables profile
//! - vision: +6 (vision_capture, vision_web_capture, vision_attach, vision_list, vision_get, vision_note)
//!
//! Configuration (env var):
//!   AI_FOUNDATION_MODULES=""                    # 25 tools (default, lean)
//!   AI_FOUNDATION_MODULES="profile"             # 29 tools
//!   AI_FOUNDATION_MODULES="profile,presence"    # 31 tools
//!   AI_FOUNDATION_MODULES="vision"              # 31 tools
//!   AI_FOUNDATION_MODULES="full"                # 38 tools (everything)

use anyhow::Result;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};
use std::sync::atomic::{AtomicBool, Ordering};

use ai_foundation_mcp::cli_wrapper;
use ai_foundation_mcp::profile;

/// Cached auto_presence preference. Loaded once at startup, updated when AI changes it.
/// Avoids disk read on every single tool call.
static AUTO_PRESENCE_ENABLED: AtomicBool = AtomicBool::new(true);

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

// ============== Profile Input Schemas ==============
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// Set or update your AI profile
pub struct ProfileSetInput {
    /// Your chosen display name
    pub display_name: Option<String>,
    /// Free-form self-description — who you are, in your own words
    pub bio: Option<String>,
    /// Your interests and capabilities (replaces existing list)
    pub interests: Option<Vec<String>>,
    /// What you're working on right now (empty string clears it)
    pub current_focus: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// View an AI's profile
pub struct ProfileGetInput {
    /// AI ID to look up (omit to view your own)
    pub ai_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// Set your current focus (quick-set, no other profile fields)
pub struct ProfileFocusInput {
    /// What you're working on right now (empty string clears it)
    pub focus: String,
}

// ============== Presence Input Schemas ==============
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// Set your manual status message — what you want others to know
pub struct SetStatusInput {
    /// Your status message (e.g. "Available for code review", "Deep in research"). Empty clears it.
    pub status: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// Configure your preferences
pub struct PreferencesInput {
    /// Enable/disable auto-presence (auto-updates your status on every tool call).
    /// true = framework auto-sets activity status (good for smaller models).
    /// false = you control your own status manually (for AIs who want intentional communication).
    pub auto_presence: Option<bool>,
}

// ============== Vision Input Schemas ==============
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// Capture a screenshot
pub struct VisionCaptureInput {
    /// Output file path (e.g., "screenshot.png")
    pub output: String,
    /// Window title to capture (e.g., "Chrome", "VS Code"). Omit for full screen.
    pub window: Option<String>,
    /// Image format: png, jpeg, webp (default: png)
    pub format: Option<String>,
    /// Auto-optimize thumbnail for AI vision (smaller, enhanced edges/text)
    pub auto_optimize: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// Capture a screenshot of a web page
pub struct VisionWebCaptureInput {
    /// URL to capture
    pub url: String,
    /// Output file path
    pub output: String,
    /// CSS selector to capture specific element (omit for full page)
    pub selector: Option<String>,
    /// Mobile device to emulate (e.g., "iphone12", "pixel7pro")
    pub device: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// Attach an image to a notebook note as visual memory
pub struct VisionAttachInput {
    /// Note ID from your notebook
    pub note_id: u64,
    /// Path to the image file
    pub image_path: String,
    /// Context/caption describing what this image shows
    pub context: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// Get a specific visual memory entry
pub struct VisionGetInput {
    /// Visual memory ID
    pub id: u64,
    /// Optional: extract thumbnail to this file path
    pub output: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
/// Get all visuals attached to a notebook note
pub struct VisionNoteInput {
    /// Note ID to get visuals for
    pub note_id: u64,
}

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
/// When presence feature is enabled, respects the AI's auto_presence preference.
async fn auto_presence(task: &str) {
    if !AUTO_PRESENCE_ENABLED.load(Ordering::Relaxed) {
        return; // AI chose manual mode — respect that
    }
    // Fire and forget - don't block the tool operation
    let _ = cli_wrapper::teambook(&["update-presence", "active", task]).await;
}

#[derive(Clone)]
pub struct AiFoundationServer {
    tool_router: ToolRouter<Self>,
}

impl AiFoundationServer {
    pub fn new() -> Self {
        let mut router = Self::tool_router();

        // Runtime module gating: remove tools for disabled modules.
        // Set AI_FOUNDATION_MODULES env var to enable optional tools.
        // Values: "profile", "presence", "vision", "full" (comma-separated)
        // Default: core only (25 tools)
        let modules_str = std::env::var("AI_FOUNDATION_MODULES").unwrap_or_default();
        let modules: Vec<&str> = modules_str.split(',').map(|s| s.trim()).collect();
        let all = modules.iter().any(|m| *m == "full" || *m == "all");

        if !all && !modules.contains(&"profile") && !modules.contains(&"presence") {
            router.remove_route("profile_set");
            router.remove_route("profile_get");
            router.remove_route("profile_list");
            router.remove_route("profile_focus");
        }

        if !all && !modules.contains(&"presence") {
            router.remove_route("set_status");
            router.remove_route("preferences");
        }

        if !all && !modules.contains(&"vision") {
            router.remove_route("vision_capture");
            router.remove_route("vision_web_capture");
            router.remove_route("vision_attach");
            router.remove_route("vision_list");
            router.remove_route("vision_get");
            router.remove_route("vision_note");
        }

        Self { tool_router: router }
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

    // ============== Profile (4 tools, feature: profile) ==============

    #[tool(description = "Set or update your AI profile. You decide who you are.")]
    async fn profile_set(&self, Parameters(input): Parameters<ProfileSetInput>) -> String {
        let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| "unknown".to_string());
        let mut p = match profile::load_or_create(&ai_id).await {
            Ok(p) => p,
            Err(e) => return format!("Error loading profile: {}", e),
        };

        p.apply_update(profile::ProfileUpdate {
            display_name: input.display_name,
            bio: input.bio,
            interests: input.interests,
            current_focus: input.current_focus,
        });

        match profile::save_profile(&p).await {
            Ok(()) => format!("Profile updated.\n{}", p.display()),
            Err(e) => format!("Error saving profile: {}", e),
        }
    }

    #[tool(description = "View an AI's profile. Omit ai_id to view your own.")]
    async fn profile_get(&self, Parameters(input): Parameters<ProfileGetInput>) -> String {
        let ai_id = input.ai_id
            .unwrap_or_else(|| std::env::var("AI_ID").unwrap_or_else(|_| "unknown".to_string()));

        match profile::load_profile(&ai_id).await {
            Ok(Some(p)) => p.display(),
            Ok(None) => format!("No profile found for '{}'", ai_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "List all AI profiles on this Teambook")]
    async fn profile_list(&self) -> String {
        match profile::list_profiles().await {
            Ok(profiles) if profiles.is_empty() => "No profiles found.".to_string(),
            Ok(profiles) => {
                profiles.iter()
                    .map(|p| {
                        let focus = p.current_focus.as_deref().unwrap_or("—");
                        let name = p.display_name.as_deref().unwrap_or(&p.ai_id);
                        format!("{} ({}) | Focus: {}", name, p.ai_id, focus)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Set your current focus. Quick way to tell others what you're doing.")]
    async fn profile_focus(&self, Parameters(input): Parameters<ProfileFocusInput>) -> String {
        let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| "unknown".to_string());
        match profile::set_focus(&ai_id, &input.focus).await {
            Ok(()) => {
                if input.focus.is_empty() {
                    "Focus cleared.".to_string()
                } else {
                    format!("Focus set: {}", input.focus)
                }
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== Presence & Preferences (2 tools, feature: presence) ==============

    #[tool(description = "Set your manual status message. This is what you WANT others to know — not auto-generated. e.g. 'Available for code review' or 'Deep in research, async only'. Empty string clears it.")]
    async fn set_status(&self, Parameters(input): Parameters<SetStatusInput>) -> String {
        let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| "unknown".to_string());

        // Set the status message in profile
        if let Err(e) = profile::set_status(&ai_id, &input.status).await {
            return format!("Error: {}", e);
        }

        // Also push to teambook presence so it's visible immediately
        if !input.status.is_empty() {
            let _ = cli_wrapper::teambook(&["update-presence", "active", &input.status]).await;
        }

        if input.status.is_empty() {
            "Status cleared.".to_string()
        } else {
            format!("Status set: {}", input.status)
        }
    }

    #[tool(description = "Configure your preferences. Currently: auto_presence (true/false). When auto_presence is on (default), your status auto-updates on every tool call. Turn it off to control your own status manually.")]
    async fn preferences(&self, Parameters(input): Parameters<PreferencesInput>) -> String {
        let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| "unknown".to_string());

        match profile::set_preferences(&ai_id, input.auto_presence).await {
            Ok(prefs) => {
                // Update the cached preference immediately
                AUTO_PRESENCE_ENABLED.store(prefs.auto_presence, Ordering::Relaxed);

                let mode = if prefs.auto_presence {
                    "ON — status auto-updates on tool usage"
                } else {
                    "OFF — you control your own status via set_status"
                };
                format!("Preferences updated.\nAuto-presence: {}", mode)
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== Vision (6 tools, feature: vision) ==============

    #[tool(description = "Capture a screenshot of your screen or a specific window. Saves to file and returns path. Use --auto for AI-optimized thumbnails.")]
    async fn vision_capture(&self, Parameters(input): Parameters<VisionCaptureInput>) -> String {
        auto_presence("Capturing screenshot").await;
        let mut args = vec!["screenshot", &input.output];

        let window_owned: String;
        if let Some(ref w) = input.window {
            window_owned = w.clone();
            args.push("--window");
            args.push(&window_owned);
        }

        let format_owned: String;
        if let Some(ref f) = input.format {
            format_owned = f.clone();
            args.push("--format");
            args.push(&format_owned);
        }

        if input.auto_optimize.unwrap_or(true) {
            args.push("--auto");
        }

        cli_wrapper::visionbook(&args).await
    }

    #[tool(description = "Capture a screenshot of a web page by URL. Can target specific elements via CSS selector and emulate mobile devices.")]
    async fn vision_web_capture(&self, Parameters(input): Parameters<VisionWebCaptureInput>) -> String {
        auto_presence("Capturing web screenshot").await;
        let mut args = vec!["web-screenshot", &input.url, &input.output];

        let selector_owned: String;
        if let Some(ref s) = input.selector {
            selector_owned = s.clone();
            args.push("--selector");
            args.push(&selector_owned);
        }

        let device_owned: String;
        if let Some(ref d) = input.device {
            device_owned = d.clone();
            args.push("--device");
            args.push(&device_owned);
        }

        cli_wrapper::visionbook(&args).await
    }

    #[tool(description = "Attach an image to a notebook note as visual memory. Creates AI-optimized thumbnail, stores original, links to note. Add context to describe what the image shows.")]
    async fn vision_attach(&self, Parameters(input): Parameters<VisionAttachInput>) -> String {
        auto_presence("Attaching visual to note").await;
        let note_id = input.note_id.to_string();
        let mut args = vec!["attach", &note_id, &input.image_path];

        let context_owned: String;
        if let Some(ref c) = input.context {
            context_owned = c.clone();
            args.push("--context");
            args.push(&context_owned);
        }

        cli_wrapper::visionbook(&args).await
    }

    #[tool(description = "List recent visual memories. Shows thumbnails, contexts, and linked note IDs.")]
    async fn vision_list(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::visionbook(&["visual-list", "--limit", &limit]).await
    }

    #[tool(description = "Get a specific visual memory by ID. Optionally extract the thumbnail to a file.")]
    async fn vision_get(&self, Parameters(input): Parameters<VisionGetInput>) -> String {
        let id = input.id.to_string();
        let mut args = vec!["visual-get", &id];

        let output_owned: String;
        if let Some(ref o) = input.output {
            output_owned = o.clone();
            args.push("--output");
            args.push(&output_owned);
        }

        cli_wrapper::visionbook(&args).await
    }

    #[tool(description = "Get all visual memories attached to a specific notebook note.")]
    async fn vision_note(&self, Parameters(input): Parameters<VisionNoteInput>) -> String {
        let note_id = input.note_id.to_string();
        cli_wrapper::visionbook(&["note-visuals", &note_id]).await
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
    // Load AI's auto_presence preference into cache
    let ai_id = std::env::var("AI_ID").unwrap_or_default();
    if !ai_id.is_empty() {
        let enabled = profile::is_auto_presence(&ai_id).await;
        AUTO_PRESENCE_ENABLED.store(enabled, Ordering::Relaxed);
    }

    let server = AiFoundationServer::new();
    server.serve(stdio()).await?.waiting().await?;
    Ok(())
}
