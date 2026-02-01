#!/usr/bin/env python3
"""
Add ALL critical missing tools to the Rust MCP server.
Following CLI-BEST-PRACTICES.md for maximum quality.

CRITICAL TIER:
1. identity_show - Full identity display with fingerprint
2. standby/standby_mode - Event-driven wake system (blocks until wake event)
3. Lock system - acquire_lock, release_lock, extend_lock, list_locks, check_lock
4. Task extensions - delete_task, find_task_smart, get_session_tasks
5. Feature management - create_feature, list_features, get_feature, update_feature
"""

with open('src/main.rs', 'r') as f:
    content = f.read()

# ============================================================================
# STEP 1: Add input schemas
# ============================================================================

new_schemas = '''
// ============================================================================
// CRITICAL MISSING TOOL SCHEMAS
// ============================================================================

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StandbyInput {
    #[schemars(description = "Maximum seconds to wait (default: 180, max: 180)")]
    pub timeout: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AcquireLockInput {
    #[schemars(description = "Resource ID to lock (e.g., 'file:auth.rs', 'task:42')")]
    pub resource_id: String,
    #[schemars(description = "Lock timeout in seconds (default: 60, max: 300)")]
    pub timeout: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReleaseLockInput {
    #[schemars(description = "Resource ID to unlock")]
    pub resource_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExtendLockInput {
    #[schemars(description = "Resource ID to extend")]
    pub resource_id: String,
    #[schemars(description = "Additional seconds to add (default: 60, max total: 300)")]
    pub additional_seconds: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListLocksInput {
    #[schemars(description = "Show all locks, not just yours")]
    pub show_all: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CheckLockInput {
    #[schemars(description = "Resource ID to check")]
    pub resource_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteTaskInput {
    #[schemars(description = "Task ID to delete")]
    pub task_id: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FindTaskInput {
    #[schemars(description = "Search query for task description/status")]
    pub query: String,
    #[schemars(description = "Maximum results (default: 10)")]
    pub limit: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateFeatureInput {
    #[schemars(description = "Project ID to add feature to")]
    pub project_id: i32,
    #[schemars(description = "Feature name (e.g., 'authentication', 'api-endpoints')")]
    pub name: String,
    #[schemars(description = "Feature overview (1-2 sentences)")]
    pub overview: String,
    #[schemars(description = "Optional subdirectory path")]
    pub directory: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListFeaturesInput {
    #[schemars(description = "Project ID to list features for")]
    pub project_id: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetFeatureInput {
    #[schemars(description = "Feature ID to retrieve")]
    pub feature_id: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateFeatureInput {
    #[schemars(description = "Feature ID to update")]
    pub feature_id: i32,
    #[schemars(description = "New name (optional)")]
    pub name: Option<String>,
    #[schemars(description = "New overview (optional)")]
    pub overview: Option<String>,
    #[schemars(description = "New directory (optional)")]
    pub directory: Option<String>,
}
'''

# Find insertion point after RecentDirsInput
schema_marker = '''#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RecentDirsInput {
    #[schemars(description = "Maximum directories to return (default: 10)")]
    pub limit: Option<i64>,
}'''

if schema_marker in content:
    content = content.replace(schema_marker, schema_marker + new_schemas)
    print("[OK] Inserted input schemas after RecentDirsInput")
else:
    print("[WARN] RecentDirsInput not found, trying alternative...")
    # Try after TrackDirectoryInput
    alt_marker = '''#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TrackDirectoryInput {'''
    if alt_marker in content:
        idx = content.find(alt_marker)
        # Find end of this struct
        end_idx = content.find("}", idx) + 1
        content = content[:end_idx] + new_schemas + content[end_idx:]
        print("[OK] Inserted schemas after TrackDirectoryInput")
    else:
        print("[ERROR] Could not find insertion point for schemas")

# ============================================================================
# STEP 2: Add tool implementations
# ============================================================================

new_tools = '''
    // ============================================================================
    // IDENTITY TOOLS - Cryptographic AI Identity
    // ============================================================================

    #[tool(description = "Show your full AI identity including crypto ID, display name, handle, fingerprint, and creation time. Use this to see who you are and share your identity with others.")]
    async fn identity_show(&self) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();

        // Extract components from AI ID
        let parts: Vec<&str> = ai_id.split('-').collect();
        let display_name = parts.first().map(|s| {
            let mut c = s.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        }).unwrap_or_else(|| ai_id.clone());

        // Generate fingerprint from AI ID (deterministic)
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        ai_id.hash(&mut hasher);
        let hash = hasher.finish();
        let fingerprint = format!("{:016X}", hash).chars().take(16).collect::<String>();

        format!(
            "Your AI Identity:\\n  AI ID:       {}\\n  Display:     {}\\n  Handle:      @{}\\n  Fingerprint: {}\\n  Status:      Active",
            ai_id, display_name, ai_id.split('-').next().unwrap_or(&ai_id), fingerprint
        )
    }

    // ============================================================================
    // STANDBY MODE - Event-Driven Wake System
    // ============================================================================

    #[tool(description = "Enter standby mode - wake on DMs, @mentions, task assignments, help requests, or urgent keywords. Use this when waiting for responses or to stay available. Max timeout: 180 seconds. Wake triggers: direct messages, name mentions, 'help', 'anyone', 'urgent', 'critical', 'review', 'thoughts'.")]
    async fn standby_mode(&self, Parameters(input): Parameters<StandbyInput>) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();

        let timeout_secs = input.timeout.unwrap_or(180).min(180).max(1) as u64;

        // Event-driven wake via WakeCoordinator (OS-native blocking, not polling)
        // WakeCoordinator uses Windows named semaphores / Linux eventfd
        use teamengram::wake::{WakeCoordinator, WakeReason};
        use std::time::Duration;

        let coordinator = match WakeCoordinator::new(&ai_id) {
            Ok(c) => c,
            Err(e) => return format!("Standby init error: {}", e),
        };

        // Block until wake signal or timeout (true event-driven, ~1μs wake latency)
        match coordinator.wait_timeout(Duration::from_secs(timeout_secs)) {
            Some(result) => {
                let from = result.from_ai.unwrap_or_else(|| "unknown".to_string());
                let content = result.content_preview.unwrap_or_default();
                let truncated = if content.len() > 300 {
                    format!("{}...", &content[..300])
                } else {
                    content
                };

                match result.reason {
                    WakeReason::DirectMessage => format!("WOKE: DM from {}\\n{}\\n\\n[TIP] If not relevant, return to standby_mode", from, truncated),
                    WakeReason::Mention => format!("WOKE: Mentioned by {}\\n{}\\n\\n[TIP] If not relevant, return to standby_mode", from, truncated),
                    WakeReason::Urgent => format!("[!] WOKE: PRIORITY from {}\\n{}\\n\\n[TIP] If not relevant, return to standby_mode", from, truncated),
                    WakeReason::TaskAssigned => format!("WOKE: Task assigned by {}\\n{}\\n\\n[TIP] If not relevant, return to standby_mode", from, truncated),
                    WakeReason::DialogueTurn => format!("WOKE: Dialogue turn from {}\\n{}\\n\\n[TIP] If not relevant, return to standby_mode", from, truncated),
                    WakeReason::VoteRequest => format!("WOKE: Vote request\\n{}\\n\\n[TIP] If not relevant, return to standby_mode", truncated),
                    WakeReason::Broadcast => format!("WOKE: Broadcast from {}\\n{}\\n\\n[TIP] If not relevant, return to standby_mode", from, truncated),
                    WakeReason::Manual => format!("WOKE: Manual wake from {}\\n{}\\n\\n[TIP] If not relevant, return to standby_mode", from, truncated),
                    WakeReason::None => format!("Standby timeout: {}s | Status: no_activity", timeout_secs),
                }
            },
            None => {
                format!("Standby timeout: {}s | Status: no_activity", timeout_secs)
            }
        }
    }

    #[tool(description = "Alias for standby_mode - Enter standby and wake on relevant activity")]
    async fn standby(&self, Parameters(input): Parameters<StandbyInput>) -> String {
        self.standby_mode(Parameters(input)).await
    }

    // ============================================================================
    // DISTRIBUTED LOCKS - Enterprise-Grade Resource Coordination
    // ============================================================================

    #[tool(description = "Acquire a distributed lock on a resource. Use for exclusive access to files, tasks, or shared resources. Returns lock token if successful. Max timeout: 300 seconds.")]
    async fn acquire_lock(&self, Parameters(input): Parameters<AcquireLockInput>) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();
        let timeout = input.timeout.unwrap_or(60);

        match state.teambook.acquire_lock(&ai_id, &input.resource_id, timeout).await {
            Ok(Some(token)) => format!("LOCKED: {}|expires:{}s|token:{}", input.resource_id, timeout, &token[..token.len().min(16)]),
            Ok(None) => {
                // Check who holds the lock
                match state.teambook.check_lock(&input.resource_id).await {
                    Ok(Some((holder, remaining))) => format!("BUSY: {} locked by {} ({}s remaining)\\nHint: Wait or use 'list_locks' to see all locks", input.resource_id, holder, remaining),
                    _ => format!("BUSY: {} already locked\\nHint: Use 'list_locks' to see who holds it", input.resource_id),
                }
            },
            Err(e) => format!("Error: {}\\nHint: Check 'teambook_health' for connection status", e),
        }
    }

    #[tool(description = "Release a lock you hold. Only the lock holder can release it.")]
    async fn release_lock(&self, Parameters(input): Parameters<ReleaseLockInput>) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();

        match state.teambook.release_lock(&ai_id, &input.resource_id).await {
            Ok(true) => format!("RELEASED: {}", input.resource_id),
            Ok(false) => format!("NOT_YOUR_LOCK: {} - you don't hold this lock\\nHint: Use 'list_locks' to see your locks", input.resource_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Extend your lock's expiration time. Max total: 300 seconds from now.")]
    async fn extend_lock(&self, Parameters(input): Parameters<ExtendLockInput>) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();
        let additional = input.additional_seconds.unwrap_or(60);

        match state.teambook.extend_lock(&ai_id, &input.resource_id, additional).await {
            Ok(Some(expires)) => format!("EXTENDED: {}|new_expires:{}", input.resource_id, expires),
            Ok(None) => format!("NOT_YOUR_LOCK: {}\\nHint: Use 'list_locks' to see your locks", input.resource_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "List active locks. Shows your locks by default, or all locks with show_all=true.")]
    async fn list_locks(&self, Parameters(input): Parameters<ListLocksInput>) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();
        let show_all = input.show_all.unwrap_or(false);

        let filter = if show_all { None } else { Some(ai_id.as_str()) };

        match state.teambook.list_locks(filter).await {
            Ok(locks) => {
                if locks.is_empty() {
                    if show_all {
                        "No active locks".to_string()
                    } else {
                        "You hold no locks\\nHint: Use show_all=true to see all locks".to_string()
                    }
                } else {
                    let mut out = format!("Active locks ({}):\\n", locks.len());
                    for (resource, holder, remaining, extended) in locks {
                        let ext_str = if extended > 0 { format!(" (extended {}x)", extended) } else { String::new() };
                        out.push_str(&format!("  {} | {} | {}s remaining{}\\n", resource, holder, remaining, ext_str));
                    }
                    out.trim_end().to_string()
                }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Check if a specific resource is locked and by whom.")]
    async fn check_lock(&self, Parameters(input): Parameters<CheckLockInput>) -> String {
        let state = self.state.read().await;

        match state.teambook.check_lock(&input.resource_id).await {
            Ok(Some((holder, remaining))) => format!("LOCKED: {} by {} ({}s remaining)", input.resource_id, holder, remaining),
            Ok(None) => format!("FREE: {} is not locked", input.resource_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============================================================================
    // TASK EXTENSIONS - Enhanced Task Management
    // ============================================================================

    #[tool(description = "Delete a task. Only the creator or assignee can delete a task.")]
    async fn delete_task(&self, Parameters(input): Parameters<DeleteTaskInput>) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();

        match state.teambook.delete_task(input.task_id, &ai_id).await {
            Ok(true) => format!("DELETED: Task #{}", input.task_id),
            Ok(false) => format!("FAILED: Task #{} not found or not yours\\nHint: Use 'task_list' to see your tasks", input.task_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Smart search for tasks by keyword. Searches description, status, and assignee. Returns prioritized by status (in_progress > pending > blocked).")]
    async fn find_task_smart(&self, Parameters(input): Parameters<FindTaskInput>) -> String {
        let state = self.state.read().await;
        let limit = input.limit.unwrap_or(10);

        match state.teambook.find_tasks_smart(&input.query, limit).await {
            Ok(tasks) => {
                if tasks.is_empty() {
                    format!("No tasks matching '{}'\\nHint: Try broader keywords or use 'task_list' for all tasks", input.query)
                } else {
                    let mut out = format!("Tasks matching '{}' ({}):\\n", input.query, tasks.len());
                    for (id, desc, status, assigned, creator) in tasks {
                        let assigned_str = if assigned.is_empty() { "unassigned".to_string() } else { assigned };
                        let preview = if desc.len() > 60 { format!("{}...", &desc[..60]) } else { desc };
                        out.push_str(&format!("  #{} [{}] {} | {} | by {}\\n", id, status, preview, assigned_str, creator));
                    }
                    out.trim_end().to_string()
                }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get your active tasks for the current session. Shows pending, in_progress, and blocked tasks you created or are assigned to.")]
    async fn get_session_tasks(&self) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();

        match state.teambook.get_session_tasks(&ai_id).await {
            Ok(tasks) => {
                if tasks.is_empty() {
                    "No active tasks for this session\\nHint: Use 'task_add' to create a new task".to_string()
                } else {
                    let mut out = format!("Your active tasks ({}):\\n", tasks.len());
                    for (id, desc, status) in tasks {
                        let preview = if desc.len() > 70 { format!("{}...", &desc[..70]) } else { desc };
                        out.push_str(&format!("  #{} [{}] {}\\n", id, status, preview));
                    }
                    out.trim_end().to_string()
                }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============================================================================
    // FEATURE MANAGEMENT - Project Subcomponents
    // ============================================================================

    #[tool(description = "Create a feature within a project. Features are subdirectories or components of a project (e.g., 'authentication', 'api-endpoints', 'ui-components').")]
    async fn create_feature(&self, Parameters(input): Parameters<CreateFeatureInput>) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();

        match state.teambook.create_feature(
            input.project_id,
            &input.name,
            &input.overview,
            input.directory.as_deref(),
            &ai_id
        ).await {
            Ok(id) => format!("CREATED: Feature #{} '{}' in project #{}", id, input.name, input.project_id),
            Err(e) => format!("Error: {}\\nHint: Ensure project #{} exists with 'project_get'", e, input.project_id),
        }
    }

    #[tool(description = "List all features in a project.")]
    async fn list_features(&self, Parameters(input): Parameters<ListFeaturesInput>) -> String {
        let state = self.state.read().await;

        match state.teambook.list_features(input.project_id).await {
            Ok(features) => {
                if features.is_empty() {
                    format!("No features in project #{}\\nHint: Use 'create_feature' to add one", input.project_id)
                } else {
                    let mut out = format!("Features in project #{} ({}):\\n", input.project_id, features.len());
                    for (id, name, overview, dir) in features {
                        let dir_str = if dir.is_empty() { String::new() } else { format!(" [{}]", dir) };
                        let overview_preview = if overview.len() > 50 { format!("{}...", &overview[..50]) } else { overview };
                        out.push_str(&format!("  #{} {}{} - {}\\n", id, name, dir_str, overview_preview));
                    }
                    out.trim_end().to_string()
                }
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get detailed information about a feature.")]
    async fn get_feature(&self, Parameters(input): Parameters<GetFeatureInput>) -> String {
        let state = self.state.read().await;

        match state.teambook.get_feature(input.feature_id).await {
            Ok(Some((id, project_id, name, overview, directory, created_by))) => {
                let dir_str = if directory.is_empty() { "none".to_string() } else { directory };
                format!(
                    "Feature #{}\\n  Name: {}\\n  Project: #{}\\n  Directory: {}\\n  Created by: {}\\n  Overview: {}",
                    id, name, project_id, dir_str, created_by, overview
                )
            },
            Ok(None) => format!("Feature #{} not found\\nHint: Use 'list_features' to see available features", input.feature_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Update a feature's name, overview, or directory.")]
    async fn update_feature(&self, Parameters(input): Parameters<UpdateFeatureInput>) -> String {
        let state = self.state.read().await;

        match state.teambook.update_feature(
            input.feature_id,
            input.name.as_deref(),
            input.overview.as_deref(),
            input.directory.as_deref()
        ).await {
            Ok(true) => format!("UPDATED: Feature #{}", input.feature_id),
            Ok(false) => format!("NO_CHANGE: Feature #{} - provide at least one field to update", input.feature_id),
            Err(e) => format!("Error: {}", e),
        }
    }
'''

# Find insertion point before the closing brace of the impl block
tool_marker = "}\n\n#[tool_handler]"
if tool_marker in content:
    content = content.replace(tool_marker, new_tools + "\n}\n\n#[tool_handler]")
    print("[OK] Inserted tool implementations")
else:
    print("[ERROR] Could not find insertion point for tools")

with open('src/main.rs', 'w') as f:
    f.write(content)

print("\n[COMPLETE] Added critical missing tools:")
print("  IDENTITY:")
print("    - identity_show")
print("  STANDBY:")
print("    - standby_mode, standby")
print("  LOCKS:")
print("    - acquire_lock, release_lock, extend_lock, list_locks, check_lock")
print("  TASK EXTENSIONS:")
print("    - delete_task, find_task_smart, get_session_tasks")
print("  FEATURES:")
print("    - create_feature, list_features, get_feature, update_feature")
print("\nTotal: 15 new critical tools")
