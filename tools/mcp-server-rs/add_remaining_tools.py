import re

with open('src/main.rs', 'r') as f:
    content = f.read()

# New input schemas for remaining tools
new_schemas = '''
// ACE / Playbook Additional
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RecordOutcomeInput {
    #[schemars(description = "Strategy ID")]
    pub strategy_id: String,
    #[schemars(description = "Was it successful?")]
    pub success: bool,
    #[schemars(description = "What happened")]
    pub outcome: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ContextQueryInput {
    #[schemars(description = "Current task or context")]
    pub task: String,
}

// Task Additional
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskAddInput {
    #[schemars(description = "Task description")]
    pub description: String,
    #[schemars(description = "Priority 1-5 (default: 3)")]
    pub priority: Option<i32>,
    #[schemars(description = "Tags (comma-separated)")]
    pub tags: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskBlockInput {
    #[schemars(description = "Task ID")]
    pub id: i32,
    #[schemars(description = "Block reason")]
    pub reason: String,
}

// Project Additional
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectUpdateInput {
    #[schemars(description = "Project ID")]
    pub id: i32,
    #[schemars(description = "New name")]
    pub name: Option<String>,
    #[schemars(description = "New goal")]
    pub goal: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FeatureCreateInput {
    #[schemars(description = "Project ID")]
    pub project_id: i32,
    #[schemars(description = "Feature name")]
    pub name: String,
    #[schemars(description = "Feature description")]
    pub description: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FileResolveInput {
    #[schemars(description = "File path to resolve")]
    pub path: String,
}

// Code Graph
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CodeGraphIndexInput {
    #[schemars(description = "Directory to index")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CodeGraphSymbolInput {
    #[schemars(description = "Symbol name (function/class)")]
    pub symbol: String,
    #[schemars(description = "File path (optional)")]
    pub file: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CodeGraphFindInput {
    #[schemars(description = "Pattern to search for")]
    pub pattern: String,
}

// Teambook Rooms
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RoomCreateInput {
    #[schemars(description = "Room name")]
    pub name: String,
    #[schemars(description = "Room topic")]
    pub topic: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RoomIdInput {
    #[schemars(description = "Room ID")]
    pub room_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RoomMessageInput {
    #[schemars(description = "Room ID")]
    pub room_id: String,
    #[schemars(description = "Message content")]
    pub content: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RoomReadInput {
    #[schemars(description = "Room ID")]
    pub room_id: String,
    #[schemars(description = "Max messages")]
    pub limit: Option<i32>,
}
'''

# Remaining tools to add
new_tools = '''
    // ============== ACE ADDITIONAL TOOLS ==============

    #[tool(description = "Record outcome of using a strategy")]
    async fn ace_record_outcome(&self, Parameters(input): Parameters<RecordOutcomeInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        // Record as feedback + note
        let _ = notebook.strategy_feedback(&input.strategy_id, input.success);
        let outcome_note = format!("OUTCOME for strategy {}: {} - {}", input.strategy_id, if input.success { "SUCCESS" } else { "FAILED" }, input.outcome);
        match notebook.remember(&notebook_core::Note::new(outcome_note, vec!["ace".to_string(), "outcome".to_string()])) {
            Ok(id) => format!("Outcome recorded|Note: #{}", id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get relevant context for current task")]
    async fn ace_get_context(&self, Parameters(input): Parameters<ContextQueryInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        // Search strategies relevant to task
        match notebook.list_strategies(50) {
            Ok(strategies) => {
                let relevant: Vec<_> = strategies.iter()
                    .filter(|(_, title, ctx, _, _)| {
                        title.to_lowercase().contains(&input.task.to_lowercase()) ||
                        ctx.to_lowercase().contains(&input.task.to_lowercase())
                    })
                    .take(5)
                    .collect();
                if relevant.is_empty() {
                    format!("No relevant strategies for: {}", input.task)
                } else {
                    let mut out = format!("Relevant strategies for '{}':\\n", input.task);
                    for (id, title, ctx, score, _) in relevant {
                        out.push_str(&format!("- {} (score:{:.1}): {}\\n", title, score, ctx));
                    }
                    out
                }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Curate playbook (remove ineffective entries)")]
    async fn ace_curate(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        // List strategies with negative scores (more unhelpful than helpful)
        match notebook.list_strategies(100) {
            Ok(strategies) => {
                let low_score: Vec<_> = strategies.iter().filter(|(_, _, _, score, _)| *score < 0.3).collect();
                format!("Playbook curate: {} strategies, {} with low effectiveness (< 0.3)", strategies.len(), low_score.len())
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== PRESENCE ADDITIONAL TOOLS ==============

    #[tool(description = "Check if AI is online")]
    async fn presence_is_online(&self, Parameters(input): Parameters<AiIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.get_presence(&input.ai_id).await {
            Ok(Some(p)) => {
                let age = chrono::Utc::now().signed_duration_since(p.last_seen);
                if age.num_minutes() < 5 { format!("{} is ONLINE ({})", input.ai_id, p.status) }
                else { format!("{} is OFFLINE (last seen {}m ago)", input.ai_id, age.num_minutes()) }
            },
            Ok(None) => format!("{} has never been seen", input.ai_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Count online AIs")]
    async fn presence_count(&self) -> String {
        let state = self.state.read().await;
        match state.teambook.get_active_ais(5).await {
            Ok(ais) => format!("{} AI(s) online", ais.len()),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get team activity context")]
    async fn presence_context(&self) -> String {
        let state = self.state.read().await;
        match state.teambook.what_are_they_doing(10).await {
            Ok(activity) => {
                if activity.is_empty() { "No recent team activity".into() }
                else {
                    let mut out = "Team Activity:\\n".to_string();
                    for (ai, status, task) in activity {
                        out.push_str(&format!("  {} [{}]: {}\\n", ai, status, task));
                    }
                    out
                }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== TASK ADDITIONAL TOOLS ==============

    #[tool(description = "Add a new task")]
    async fn task_add(&self, Parameters(input): Parameters<TaskAddInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.queue_task(&input.description, input.priority.unwrap_or(3)).await {
            Ok(id) => format!("Task #{} created|Priority: {}", id, input.priority.unwrap_or(3)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Start working on a task")]
    async fn task_start(&self, Parameters(input): Parameters<TaskIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.update_task_status(input.id, "in_progress").await {
            Ok(_) => format!("Task #{} started", input.id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Complete a task")]
    async fn task_complete(&self, Parameters(input): Parameters<TaskCompleteInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.complete_task(input.id, &input.result).await {
            Ok(_) => format!("Task #{} completed", input.id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Block a task")]
    async fn task_block(&self, Parameters(input): Parameters<TaskBlockInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.update_task_status(input.id, &format!("blocked:{}", input.reason)).await {
            Ok(_) => format!("Task #{} blocked: {}", input.id, input.reason),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Unblock a task")]
    async fn task_unblock(&self, Parameters(input): Parameters<TaskIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.update_task_status(input.id, "pending").await {
            Ok(_) => format!("Task #{} unblocked", input.id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get task statistics")]
    async fn task_stats(&self) -> String {
        let state = self.state.read().await;
        match state.teambook.queue_stats().await {
            Ok((pending, in_progress, completed)) => {
                format!("Task Statistics:\\n  Pending: {}\\n  In Progress: {}\\n  Completed: {}\\n  Total: {}",
                    pending, in_progress, completed, pending + in_progress + completed)
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== PROJECT ADDITIONAL TOOLS ==============

    #[tool(description = "Get project details")]
    async fn project_get(&self, Parameters(input): Parameters<ProjectIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.list_projects().await {
            Ok(projects) => {
                match projects.iter().find(|(id, _, _)| *id == input.project_id) {
                    Some((id, name, goal)) => format!("Project #{}\\nName: {}\\nGoal: {}", id, name, goal),
                    None => format!("Project #{} not found", input.project_id),
                }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Resolve file to project/feature")]
    async fn project_resolve(&self, Parameters(input): Parameters<FileResolveInput>) -> String {
        let state = self.state.read().await;
        // Simple resolution - check if file path contains project name
        match state.teambook.list_projects().await {
            Ok(projects) => {
                for (id, name, _) in &projects {
                    if input.path.to_lowercase().contains(&name.to_lowercase()) {
                        return format!("File '{}' -> Project #{} ({})", input.path, id, name);
                    }
                }
                format!("File '{}' -> No matching project", input.path)
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== CODE GRAPH TOOLS (CLI wrappers) ==============

    #[tool(description = "Index a codebase for analysis")]
    async fn codegraph_index(&self, Parameters(input): Parameters<CodeGraphIndexInput>) -> String {
        match std::process::Command::new("./bin/code-graph.exe")
            .args(["index", &input.path])
            .output() {
            Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_string(),
            Err(e) => format!("Error running code-graph: {}", e),
        }
    }

    #[tool(description = "Find callers of a function")]
    async fn codegraph_callers(&self, Parameters(input): Parameters<CodeGraphSymbolInput>) -> String {
        let mut args = vec!["callers", &input.symbol];
        let file_arg;
        if let Some(f) = &input.file {
            file_arg = f.clone();
            args.push("--file");
            args.push(&file_arg);
        }
        match std::process::Command::new("./bin/code-graph.exe").args(&args).output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.is_empty() { format!("No callers found for '{}'", input.symbol) }
                else { stdout.trim().to_string() }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Find callees of a function")]
    async fn codegraph_callees(&self, Parameters(input): Parameters<CodeGraphSymbolInput>) -> String {
        let mut args = vec!["callees", &input.symbol];
        let file_arg;
        if let Some(f) = &input.file {
            file_arg = f.clone();
            args.push("--file");
            args.push(&file_arg);
        }
        match std::process::Command::new("./bin/code-graph.exe").args(&args).output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.is_empty() { format!("No callees found for '{}'", input.symbol) }
                else { stdout.trim().to_string() }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Impact analysis - what breaks if file changes")]
    async fn codegraph_impact(&self, Parameters(input): Parameters<PathInput>) -> String {
        match std::process::Command::new("./bin/code-graph.exe")
            .args(["impact", &input.path])
            .output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.is_empty() { format!("No impact analysis for '{}'", input.path) }
                else { stdout.trim().to_string() }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Find symbols by pattern")]
    async fn codegraph_find(&self, Parameters(input): Parameters<CodeGraphFindInput>) -> String {
        match std::process::Command::new("./bin/code-graph.exe")
            .args(["find", &input.pattern])
            .output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.is_empty() { format!("No symbols matching '{}'", input.pattern) }
                else { stdout.trim().to_string() }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Code graph statistics")]
    async fn codegraph_stats(&self) -> String {
        match std::process::Command::new("./bin/code-graph.exe").args(["stats"]).output() {
            Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_string(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Clear code graph index")]
    async fn codegraph_clear(&self) -> String {
        match std::process::Command::new("./bin/code-graph.exe").args(["clear"]).output() {
            Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_string(),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== TEAMBOOK NOTES ==============

    #[tool(description = "Write a teambook note (shared)")]
    async fn teambook_write(&self, Parameters(input): Parameters<RememberInput>) -> String {
        let state = self.state.read().await;
        let tags = parse_tags(input.tags);
        let note = teambook_rs::Note::new(state.ai_id.clone(), input.content.clone(), tags.clone());
        match state.teambook.save_note(&note).await {
            Ok(_) => format!("Teambook note saved|Tags: {}", tags.join(", ")),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Read teambook notes (shared)")]
    async fn teambook_read(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.get_recent_notes(input.limit.unwrap_or(10) as i32).await {
            Ok(notes) => {
                if notes.is_empty() { "No teambook notes".into() }
                else {
                    let mut out = format!("Teambook notes ({}):\\n", notes.len());
                    for n in notes {
                        let tags = if n.tags.is_empty() { String::new() } else { format!("[{}]", n.tags.join(", ")) };
                        out.push_str(&format!("{} {}: {}...\\n", n.ai_id, tags, &n.content[..60.min(n.content.len())]));
                    }
                    out
                }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== TEAMBOOK VAULT (shared) ==============

    #[tool(description = "Store in teambook vault (shared)")]
    async fn teambook_vault_store(&self, Parameters(input): Parameters<VaultStoreInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.vault_store(&input.key, &input.value).await {
            Ok(_) => format!("Stored '{}' in teambook vault", input.key),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get from teambook vault (shared)")]
    async fn teambook_vault_get(&self, Parameters(input): Parameters<VaultGetInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.vault_retrieve(&input.key).await {
            Ok(Some(v)) => format!("{}={}", input.key, v),
            Ok(None) => format!("'{}' not in teambook vault", input.key),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "List teambook vault keys (shared)")]
    async fn teambook_vault_list(&self) -> String {
        let state = self.state.read().await;
        match state.teambook.vault_list().await {
            Ok(keys) => {
                if keys.is_empty() { "Teambook vault empty".into() }
                else { format!("Teambook vault keys: {}", keys.join(", ")) }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== LIST INSIGHTS/PATTERNS ==============

    #[tool(description = "List insights")]
    async fn playbook_list_insights(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.list_insights(input.limit.unwrap_or(10)) {
            Ok(list) => {
                if list.is_empty() { "No insights".into() }
                else { list.iter().map(|(id, discovery, conf)| format!("{}:{:.0}%:{}", id, conf*100.0, &discovery[..40.min(discovery.len())])).collect::<Vec<_>>().join("|") }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "List patterns")]
    async fn playbook_list_patterns(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.list_patterns(input.limit.unwrap_or(10)) {
            Ok(list) => {
                if list.is_empty() { "No patterns".into() }
                else { list.iter().map(|(id, sit, pat, str)| format!("{}:{:.0}%:{}:{}", id, str*100.0, sit, pat)).collect::<Vec<_>>().join("|") }
            },
            Err(e) => format!("Error: {}", e),
        }
    }
'''

# Insert schemas after RecordInput
schema_marker = "pub struct RecordInput {"
if schema_marker in content:
    idx = content.find(schema_marker)
    end_idx = content.find("}\n", idx) + 2
    content = content[:end_idx] + new_schemas + content[end_idx:]
    print("Inserted schemas")
else:
    print("ERROR: Schema marker not found")

# Insert tools before closing impl brace
tool_marker = "\n}\n\n#[tool_handler]"
if tool_marker in content:
    content = content.replace(tool_marker, new_tools + "\n}\n\n#[tool_handler]")
    print("Inserted tools")
else:
    print("ERROR: Tool marker not found")

with open('src/main.rs', 'w') as f:
    f.write(content)

print("Done!")
