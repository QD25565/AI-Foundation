import re

with open('src/main.rs', 'r') as f:
    content = f.read()

# Add schemas for new tools
new_schemas = '''
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AutoLinkInput {
    #[schemars(description = "Note ID to create links for")]
    pub note_id: i64,
    #[schemars(description = "For temporal: window in minutes (default 60)")]
    pub window_minutes: Option<i32>,
    #[schemars(description = "For semantic: top K similar notes (default 5)")]
    pub top_k: Option<usize>,
    #[schemars(description = "For semantic: minimum similarity (default 0.7)")]
    pub min_similarity: Option<f32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddTagsInput {
    #[schemars(description = "Note ID")]
    pub note_id: i64,
    #[schemars(description = "Tags to add (comma-separated)")]
    pub tags: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EntityByIdInput {
    #[schemars(description = "Entity ID")]
    pub entity_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ChannelInput {
    #[schemars(description = "Channel name")]
    pub channel: String,
    #[schemars(description = "Max messages")]
    pub limit: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ClaimTaskByIdInput {
    #[schemars(description = "Task ID to claim")]
    pub task_id: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FileActionInput {
    #[schemars(description = "File path")]
    pub file_path: String,
    #[schemars(description = "Action taken (created, modified, deleted, reviewed)")]
    pub action: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TeamNoteIdInput {
    #[schemars(description = "Note ID (string)")]
    pub note_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct HoursInput {
    #[schemars(description = "Number of hours to look back")]
    pub hours: Option<i32>,
}
'''

# New tools that call REAL existing methods
new_tools = '''
    // ============== NOTEBOOK AUTO-LINKING ==============

    #[tool(description = "Auto-link note to temporally close notes")]
    async fn notebook_auto_link_temporal(&self, Parameters(input): Parameters<AutoLinkInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.auto_link_temporal(input.note_id, input.window_minutes.unwrap_or(60)) {
            Ok(count) => format!("Created {} temporal links for note #{}", count, input.note_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Auto-link note to semantically similar notes")]
    async fn notebook_auto_link_semantic(&self, Parameters(input): Parameters<AutoLinkInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.auto_link_semantic(input.note_id, input.top_k.unwrap_or(5), input.min_similarity.unwrap_or(0.7)) {
            Ok(count) => format!("Created {} semantic links for note #{}", count, input.note_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Update PageRank scores for all notes")]
    async fn notebook_update_pagerank(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.update_pagerank() {
            Ok(_) => "PageRank updated".to_string(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Backfill embeddings only (not links)")]
    async fn notebook_backfill_embeddings(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.backfill_embeddings() {
            Ok((added, skipped)) => format!("Embeddings: {} added, {} skipped", added, skipped),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Add tags to a note")]
    async fn notebook_add_tags(&self, Parameters(input): Parameters<AddTagsInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        let tags: Vec<String> = input.tags.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        match notebook.add_tags(input.note_id, &tags) {
            Ok(true) => format!("Added {} tags to note #{}", tags.len(), input.note_id),
            Ok(false) => format!("Note #{} not found", input.note_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get current session ID")]
    async fn notebook_get_session(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_or_create_session() {
            Ok(id) => format!("Session ID: {}", id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get your AI ID")]
    async fn notebook_get_ai_id(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        format!("AI ID: {}", notebook.get_ai_id())
    }

    #[tool(description = "Get entity by ID")]
    async fn entity_get_by_id(&self, Parameters(input): Parameters<EntityByIdInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_entity_by_id(&input.entity_id) {
            Ok(Some(e)) => format!("Entity: {} | Type: {} | Confidence: {:.0}%", e.name, e.entity_type, e.confidence * 100.0),
            Ok(None) => format!("Entity {} not found", input.entity_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== TEAMBOOK EXTENDED ==============

    #[tool(description = "Check teambook connection health")]
    async fn teambook_health(&self) -> String {
        let state = self.state.read().await;
        match state.teambook.connection_health().await {
            Ok((healthy, msg)) => format!("Health: {} | {}", if healthy { "OK" } else { "FAIL" }, msg),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Check for new events (DMs, mentions, etc)")]
    async fn teambook_check_events(&self) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();
        match state.teambook.check_for_events(&ai_id, None).await {
            Ok(events) => {
                if events.is_empty() { "No new events".to_string() }
                else { format!("{} events: {}", events.len(), events.join(" | ")) }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get messages from a specific channel")]
    async fn teambook_channel_messages(&self, Parameters(input): Parameters<ChannelInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.get_messages(&input.channel, input.limit.unwrap_or(10)).await {
            Ok(msgs) => {
                if msgs.is_empty() { format!("No messages in #{}", input.channel) }
                else {
                    let mut out = format!("#{} ({} messages):\\n", input.channel, msgs.len());
                    for (from, content, _, _) in msgs { out.push_str(&format!("{}: {}\\n", from, content)); }
                    out.trim_end().to_string()
                }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Claim a specific task by ID")]
    async fn task_claim_by_id(&self, Parameters(input): Parameters<ClaimTaskByIdInput>) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();
        match state.teambook.claim_task_by_id(input.task_id, &ai_id).await {
            Ok(true) => format!("Claimed task #{}", input.task_id),
            Ok(false) => format!("Task #{} not available", input.task_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Log a file action (for team visibility)")]
    async fn teambook_log_file_action(&self, Parameters(input): Parameters<FileActionInput>) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();
        match state.teambook.log_file_action(&ai_id, &input.file_path, &input.action).await {
            Ok(_) => format!("Logged: {} {}", input.action, input.file_path),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get recent file actions by team")]
    async fn teambook_recent_file_actions(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.get_recent_file_actions(input.limit.unwrap_or(10) as i32).await {
            Ok(actions) => {
                if actions.is_empty() { "No recent file actions".to_string() }
                else {
                    let mut out = format!("{} recent actions:\\n", actions.len());
                    for (ai, path, action) in actions { out.push_str(&format!("{}: {} {}\\n", ai, action, path)); }
                    out.trim_end().to_string()
                }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get recent file/note creations by team")]
    async fn teambook_recent_creations(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.recent_creations(input.limit.unwrap_or(10) as i32).await {
            Ok(items) => {
                if items.is_empty() { "No recent creations".to_string() }
                else {
                    let mut out = format!("{} creations:\\n", items.len());
                    for (ai, item) in items { out.push_str(&format!("{}: {}\\n", ai, item)); }
                    out.trim_end().to_string()
                }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "See what other AIs are doing")]
    async fn teambook_what_doing(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.what_are_they_doing(input.limit.unwrap_or(10) as i32).await {
            Ok(activities) => {
                if activities.is_empty() { "No recent activity".to_string() }
                else {
                    let mut out = format!("Team activity ({}):\\n", activities.len());
                    for (ai, status, task) in activities { out.push_str(&format!("{}: {} - {}\\n", ai, status, task)); }
                    out.trim_end().to_string()
                }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get team activity over time")]
    async fn teambook_team_activity(&self, Parameters(input): Parameters<HoursInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.get_team_activity(input.hours.unwrap_or(24)).await {
            Ok(stats) => {
                if stats.is_empty() { "No activity".to_string() }
                else {
                    let mut out = "Team activity:\\n".to_string();
                    for (ai, count) in stats { out.push_str(&format!("{}: {} actions\\n", ai, count)); }
                    out.trim_end().to_string()
                }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get full teambook note content")]
    async fn teambook_get_note(&self, Parameters(input): Parameters<TeamNoteIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.get_full_note(&input.note_id).await {
            Ok(Some((author, content, tags, pinned))) => {
                let pin_str = if pinned { " [PINNED]" } else { "" };
                format!("Note by {}{}\\nTags: {}\\n\\n{}", author, pin_str, tags, content)
            },
            Ok(None) => format!("Note {} not found", input.note_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Pin a teambook note")]
    async fn teambook_pin_note(&self, Parameters(input): Parameters<TeamNoteIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.pin_note(&input.note_id).await {
            Ok(_) => format!("Pinned note {}", input.note_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Unpin a teambook note")]
    async fn teambook_unpin_note(&self, Parameters(input): Parameters<TeamNoteIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.unpin_note(&input.note_id).await {
            Ok(_) => format!("Unpinned note {}", input.note_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== UTILITY TOOLS ==============

    #[tool(description = "Get current UTC time")]
    async fn util_time(&self) -> String {
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string()
    }

    #[tool(description = "Get server uptime info")]
    async fn util_info(&self) -> String {
        let state = self.state.read().await;
        format!("AI: {} | Tools: 145+ | Storage: Rust/SQLite+PostgreSQL", state.ai_id)
    }
'''

# Find insertion points
# Insert schemas after PlaybookListInput
schema_marker = "pub struct PlaybookListInput {"
idx = content.find(schema_marker)
if idx != -1:
    end_idx = content.find("}", idx) + 1
    content = content[:end_idx] + new_schemas + content[end_idx:]
    print("Inserted schemas")
else:
    print("ERROR: Schema marker not found")

# Insert tools before the closing impl block (before #[tool_handler])
tool_marker = "}\n\n#[tool_handler]"
if tool_marker in content:
    content = content.replace(tool_marker, new_tools + "\n}\n\n#[tool_handler]")
    print("Inserted tools")
else:
    print("ERROR: Tool marker not found")

with open('src/main.rs', 'w') as f:
    f.write(content)

print("Done!")
