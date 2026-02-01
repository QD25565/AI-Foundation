import re

# Read the file
with open('src/main.rs', 'r') as f:
    content = f.read()

# New input schemas
new_schemas = '''
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VoteCreateInput {
    #[schemars(description = "Vote topic/question")]
    pub topic: String,
    #[schemars(description = "Comma-separated options")]
    pub options: String,
    #[schemars(description = "Number of voters")]
    pub voters: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VoteCastInput {
    #[schemars(description = "Vote ID")]
    pub vote_id: i32,
    #[schemars(description = "Your choice")]
    pub choice: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VoteIdInput {
    #[schemars(description = "Vote ID")]
    pub vote_id: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SyncStartInput {
    #[schemars(description = "Topic")]
    pub topic: String,
    #[schemars(description = "Comma-separated AI IDs")]
    pub participants: String,
    #[schemars(description = "Number of rounds")]
    pub rounds: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SyncMsgInput {
    #[schemars(description = "Session ID")]
    pub session_id: i32,
    #[schemars(description = "Message content")]
    pub content: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SessionIdInput {
    #[schemars(description = "Session ID")]
    pub session_id: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DetangleStartInput {
    #[schemars(description = "Other AI ID")]
    pub with_ai: String,
    #[schemars(description = "Topic")]
    pub topic: String,
    #[schemars(description = "Max turns")]
    pub turns: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DetangleMsgInput {
    #[schemars(description = "Session ID")]
    pub session_id: i32,
    #[schemars(description = "Message")]
    pub content: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DetangleConcludeInput {
    #[schemars(description = "Session ID")]
    pub session_id: i32,
    #[schemars(description = "Conclusion")]
    pub reason: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EvolveStartInput {
    #[schemars(description = "Goal to evolve")]
    pub goal: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EvolveAttemptInput {
    #[schemars(description = "Session ID")]
    pub session_id: i32,
    #[schemars(description = "Solution")]
    pub solution: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectCreateInput {
    #[schemars(description = "Project name")]
    pub name: String,
    #[schemars(description = "Project goal")]
    pub goal: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectTaskInput {
    #[schemars(description = "Project ID")]
    pub project_id: i32,
    #[schemars(description = "Task title")]
    pub title: String,
    #[schemars(description = "Priority 1-5")]
    pub priority: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectIdInput {
    #[schemars(description = "Project ID")]
    pub project_id: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskListInput {
    #[schemars(description = "Status filter: pending, in_progress, completed")]
    pub status: Option<String>,
    #[schemars(description = "Max results")]
    pub limit: Option<i32>,
}
'''

# New tools
new_tools = '''
    // ============== VOTING TOOLS ==============

    #[tool(description = "Create a vote")]
    async fn vote_create(&self, Parameters(input): Parameters<VoteCreateInput>) -> String {
        let state = self.state.read().await;
        let options: Vec<String> = input.options.split(',').map(|s| s.trim().to_string()).collect();
        match state.teambook.create_vote(&input.topic, options, &state.ai_id, input.voters).await {
            Ok(v) => format!("Vote #{}|Topic: {}", v.id, v.topic), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Cast a vote")]
    async fn vote_cast(&self, Parameters(input): Parameters<VoteCastInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.cast_vote(input.vote_id, &state.ai_id, &input.choice).await {
            Ok(true) => "Vote cast".into(), Ok(false) => "Already voted or invalid".into(), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Get vote results")]
    async fn vote_results(&self, Parameters(input): Parameters<VoteIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.get_vote_results(input.vote_id).await {
            Ok(Some(r)) => format!("{}|Winner:{}", r.topic, r.winner.unwrap_or_else(|| "none".into())), Ok(None) => "Not found".into(), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "List open votes")]
    async fn vote_list_open(&self) -> String {
        let state = self.state.read().await;
        match state.teambook.get_open_votes().await {
            Ok(v) => { if v.is_empty() { "None".into() } else { v.iter().map(|x| format!("#{}:{}", x.id, x.topic)).collect::<Vec<_>>().join("|") } }, Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Get pending votes for me")]
    async fn vote_pending(&self) -> String {
        let state = self.state.read().await;
        match state.teambook.get_pending_votes_for_ai(&state.ai_id).await {
            Ok(v) => { if v.is_empty() { "None".into() } else { v.iter().map(|x| format!("#{}:{}", x.id, x.topic)).collect::<Vec<_>>().join("|") } }, Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "List all votes")]
    async fn vote_list(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.list_votes(input.limit.unwrap_or(10) as i32).await {
            Ok(v) => { if v.is_empty() { "None".into() } else { v.iter().map(|x| format!("#{}:{}:{:?}", x.id, x.topic, x.status)).collect::<Vec<_>>().join("|") } }, Err(e) => format!("Err: {}", e)
        }
    }

    // ============== SYNC TOOLS ==============

    #[tool(description = "Start a sync session")]
    async fn sync_start(&self, Parameters(input): Parameters<SyncStartInput>) -> String {
        let state = self.state.read().await;
        let participants: Vec<String> = input.participants.split(',').map(|s| s.trim().to_string()).collect();
        match state.teambook.sync_start(&input.topic, participants, input.rounds.unwrap_or(3)).await {
            Ok(id) => format!("Sync #{}|Topic: {}", id, input.topic), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Send sync message")]
    async fn sync_message(&self, Parameters(input): Parameters<SyncMsgInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.sync_message(input.session_id, &state.ai_id, &input.content).await {
            Ok(id) => format!("Sent: {}", id), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Complete sync session")]
    async fn sync_complete(&self, Parameters(input): Parameters<SessionIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.sync_complete(input.session_id).await {
            Ok(_) => "Completed".into(), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Get sync status")]
    async fn sync_status(&self, Parameters(input): Parameters<SessionIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.sync_status(input.session_id).await {
            Ok(s) => s, Err(e) => format!("Err: {}", e)
        }
    }

    // ============== DETANGLE TOOLS ==============

    #[tool(description = "Start detangle (1-on-1 discussion)")]
    async fn detangle_start(&self, Parameters(input): Parameters<DetangleStartInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.detangle_start(&state.ai_id, &input.with_ai, &input.topic, input.turns.unwrap_or(5)).await {
            Ok(id) => format!("Detangle #{}|With: {}", id, input.with_ai), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Send detangle message")]
    async fn detangle_message(&self, Parameters(input): Parameters<DetangleMsgInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.detangle_message(input.session_id, &state.ai_id, &input.content).await {
            Ok(_) => "Sent".into(), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Conclude detangle")]
    async fn detangle_conclude(&self, Parameters(input): Parameters<DetangleConcludeInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.detangle_conclude(input.session_id, &input.reason).await {
            Ok(_) => "Concluded".into(), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Get detangle status")]
    async fn detangle_status(&self, Parameters(input): Parameters<SessionIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.detangle_status(input.session_id).await {
            Ok(s) => s, Err(e) => format!("Err: {}", e)
        }
    }

    // ============== EVOLUTION TOOLS ==============

    #[tool(description = "Start evolution session")]
    async fn evolve_start(&self, Parameters(input): Parameters<EvolveStartInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.evolve_start(&input.goal).await {
            Ok(id) => format!("Evolution #{}", id), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Submit evolution attempt")]
    async fn evolve_attempt(&self, Parameters(input): Parameters<EvolveAttemptInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.evolve_attempt(input.session_id, &state.ai_id, &input.solution).await {
            Ok(id) => format!("Attempt #{}", id), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "List evolution attempts")]
    async fn evolve_list(&self, Parameters(input): Parameters<SessionIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.evolve_list_attempts(input.session_id).await {
            Ok(a) => { if a.is_empty() { "None".into() } else { a.iter().map(|(id, ai, _)| format!("#{}:{}", id, ai)).collect::<Vec<_>>().join("|") } }, Err(e) => format!("Err: {}", e)
        }
    }

    // ============== PROJECT TOOLS ==============

    #[tool(description = "Create project")]
    async fn project_create(&self, Parameters(input): Parameters<ProjectCreateInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.create_project(&input.name, &input.goal).await {
            Ok(id) => format!("Project #{}", id), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Add task to project")]
    async fn project_add_task(&self, Parameters(input): Parameters<ProjectTaskInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.add_task_to_project(input.project_id, &input.title, input.priority.unwrap_or(3)).await {
            Ok(id) => format!("Task #{}", id), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "List project tasks")]
    async fn project_tasks(&self, Parameters(input): Parameters<ProjectIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.list_project_tasks(input.project_id).await {
            Ok(t) => { if t.is_empty() { "None".into() } else { t.iter().map(|(id, title, _, _)| format!("#{}:{}", id, title)).collect::<Vec<_>>().join("|") } }, Err(e) => format!("Err: {}", e)
        }
    }

    // ============== TASK TOOLS ==============

    #[tool(description = "List tasks")]
    async fn task_list(&self, Parameters(input): Parameters<TaskListInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.list_tasks(input.status.as_deref(), input.limit.unwrap_or(20)).await {
            Ok(t) => { if t.is_empty() { "None".into() } else { t.iter().map(|x| format!("#{}:{}:{}", x.id, x.status, &x.description[..30.min(x.description.len())])).collect::<Vec<_>>().join("|") } }, Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Get task by ID")]
    async fn task_get(&self, Parameters(input): Parameters<TaskIdInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.get_task(input.id).await {
            Ok(Some(t)) => format!("#{}|{}|{}", t.id, t.status, t.description), Ok(None) => "Not found".into(), Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Update task status")]
    async fn task_update(&self, Parameters(input): Parameters<TaskUpdateInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.update_task_status(input.id, &input.status).await {
            Ok(_) => format!("Updated #{}", input.id), Err(e) => format!("Err: {}", e)
        }
    }

    // ============== MISC TEAMBOOK TOOLS ==============

    #[tool(description = "List teambooks")]
    async fn teambook_list_teambooks(&self) -> String {
        let state = self.state.read().await;
        match state.teambook.list_teambooks().await {
            Ok(t) => { if t.is_empty() { "None".into() } else { t.join("|") } }, Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Get team activity")]
    async fn teambook_activity(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        match state.teambook.get_team_activity(input.limit.unwrap_or(24) as i32).await {
            Ok(a) => { if a.is_empty() { "None".into() } else { a.iter().map(|(ai, c)| format!("{}:{}", ai, c)).collect::<Vec<_>>().join("|") } }, Err(e) => format!("Err: {}", e)
        }
    }

    #[tool(description = "Release all my file claims")]
    async fn teambook_release_all_claims(&self) -> String {
        let state = self.state.read().await;
        match state.teambook.force_release_all_claims(&state.ai_id).await {
            Ok(n) => format!("Released {} claims", n), Err(e) => format!("Err: {}", e)
        }
    }
'''

# Also need TaskUpdateInput
task_update_schema = '''
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskUpdateInput {
    #[schemars(description = "Task ID")]
    pub id: i32,
    #[schemars(description = "New status: pending, in_progress, completed")]
    pub status: String,
}
'''

# Insert after PlaybookFeedbackInput
schema_marker = "pub struct PlaybookFeedbackInput {"
if schema_marker in content:
    # Find the end of PlaybookFeedbackInput
    idx = content.find(schema_marker)
    # Find the next "}" after the struct
    end_idx = content.find("}\n", idx) + 2
    content = content[:end_idx] + new_schemas + task_update_schema + content[end_idx:]
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
