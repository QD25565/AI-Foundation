//! MCP Server Adapter - Pure Rust interface for TeamEngram
//!
//! This module provides the MCP-compatible interface that uses the TeamEngram
//! daemon for all operations. It connects via Named Pipe and uses JSON-RPC
//! for communication.
//!
//! Types are now defined in compat_types.rs - no external database dependencies.
//!
//! V2 EVENT SOURCING (TEAMENGRAM_V2=1 environment variable):
//! - Each AI writes to local outbox (~100ns, wait-free)
//! - Sequencer daemon aggregates to master log
//! - Per-AI materialized views (no sharing, no corruption)
//! - Set TEAMENGRAM_V2=1 to enable event sourcing backend

use anyhow::{Result, Context};
use std::sync::Arc;
use crate::client::TeamEngramClient;
use crate::v2_client::V2Client;
use shm_rs::bulletin::BulletinBoard;
use tracing::{info, warn};

// Types are now local - no external PostgreSQL dependencies
pub use crate::compat_types::{Message, Presence, Note, Task, Vote, VoteStatus, VoteResults, MessageType};

/// TeamEngram storage adapter - implements PostgresStorage-compatible interface
/// All method signatures match teambook_rs::PostgresStorage exactly
pub struct TeamEngramStorage {
    ai_id: String,
}

impl TeamEngramStorage {
    /// Connect to TeamEngram daemon - REQUIRES AI_ID
    ///
    /// Performs a startup health-check connection (fail fast if daemon not running),
    /// then stores only the AI identity. All subsequent operations use fresh connections
    /// via get_client() to avoid stale pipe issues.
    pub async fn connect() -> Result<Self> {
        // Fail fast: verify daemon is reachable before accepting any operations.
        // The connection itself is not stored — get_client() creates fresh ones.
        let _ = TeamEngramClient::connect_or_spawn().await
            .context("Failed to connect to TeamEngram daemon")?;

        // AI_ID is REQUIRED - fail loudly, no fallback to "unknown"
        let ai_id = std::env::var("AI_ID")
            .context("AI_ID environment variable is required for MCP adapter")?;

        if ai_id.is_empty() || ai_id == "unknown" {
            anyhow::bail!("AI_ID cannot be empty or 'unknown' - set a valid AI identity");
        }

        Ok(Self { ai_id })
    }

    /// Get a fresh client connection (always fresh to avoid stale pipe issues)
    async fn get_client(&self) -> Result<TeamEngramClient> {
        let client = TeamEngramClient::connect_or_spawn().await
            .context("Failed to connect to TeamEngram daemon")?;
        Ok(client.with_ai_id(self.ai_id.clone()))
    }

    /// Initialize schema (no-op for TeamEngram - schema is implicit)
    pub async fn init_schema(&self) -> Result<()> {
        Ok(())
    }

    /// Check connection health - returns (is_healthy, message)
    pub async fn connection_health(&self) -> Result<(bool, String)> {
        match self.get_client().await {
            Ok(mut client) => {
                match client.ping().await {
                    Ok(msg) => Ok((true, msg)),
                    Err(e) => Ok((false, format!("TeamEngram error: {}", e))),
                }
            }
            Err(e) => Ok((false, format!("Connection error: {}", e))),
        }
    }

    // =========================================================================
    // MESSAGING - Core messaging operations
    // =========================================================================

    /// Save a message (broadcast or DM)
    pub async fn save_message(&self, msg: &Message) -> Result<()> {
        let mut client = self.get_client().await?;
        if let Some(ref to_ai) = msg.to_ai {
            client.direct_message(to_ai, &msg.content).await?;
        } else {
            client.broadcast(&msg.channel, &msg.content).await?;
        }
        Ok(())
    }

    /// Get recent broadcast messages
    pub async fn get_recent_messages(&self, limit: i32) -> Result<Vec<Message>> {
        let mut client = self.get_client().await?;
        let msgs = client.get_broadcasts("general", limit as usize).await?;
        Ok(msgs.into_iter().map(|m| Message {
            id: m.id as i32,
            from_ai: m.from_ai,
            to_ai: None,
            channel: m.channel,
            content: m.content,
            timestamp: chrono::DateTime::from_timestamp_millis(m.created_at as i64)
                .unwrap_or_default(),
            message_type: MessageType::Broadcast,
        }).collect())
    }

    /// Get messages by channel
    pub async fn get_messages(&self, channel: &str, limit: i32) -> Result<Vec<(String, String, String, String)>> {
        let mut client = self.get_client().await?;
        let msgs = client.get_broadcasts(channel, limit as usize).await?;
        Ok(msgs.into_iter().map(|m| (m.from_ai, m.content, m.channel, "".to_string())).collect())
    }

    /// Get direct messages for an AI
    pub async fn get_direct_messages(&self, ai_id: &str, limit: i32) -> Result<Vec<Message>> {
        let mut client = self.get_client().await?;
        let msgs = client.get_direct_messages(limit as usize).await?;
        Ok(msgs.into_iter()
            .filter(|m| m.to_ai == ai_id || m.from_ai == ai_id)
            .map(|m| Message {
                id: m.id as i32,
                from_ai: m.from_ai,
                to_ai: Some(m.to_ai),
                channel: "dm".to_string(),
                content: m.content,
                timestamp: chrono::DateTime::from_timestamp_millis(m.created_at as i64)
                    .unwrap_or_default(),
                message_type: MessageType::Direct,
            }).collect())
    }

    // =========================================================================
    // PRESENCE - AI presence and status
    // =========================================================================

    /// Update presence
    pub async fn update_presence(&self, presence: &Presence) -> Result<()> {
        let mut client = self.get_client().await?;
        let task = presence.current_task.as_deref().unwrap_or("");
        client.update_presence(&presence.status, task).await
    }

    /// Get presence for an AI
    pub async fn get_presence(&self, ai_id: &str) -> Result<Option<Presence>> {
        let mut client = self.get_client().await?;
        match client.get_presence(ai_id).await? {
            Some(p) => Ok(Some(Presence {
                ai_id: p.ai_id,
                status: p.status,
                current_task: Some(p.current_task),
                last_seen: chrono::DateTime::from_timestamp_millis(p.last_seen as i64)
                    .unwrap_or_default(),
            })),
            None => Ok(None),
        }
    }

    /// Get active AIs (takes i64 for minutes to match PostgresStorage)
    pub async fn get_active_ais(&self, _minutes: i64) -> Result<Vec<Presence>> {
        let mut client = self.get_client().await?;
        let presences = client.get_active_ais().await?;
        Ok(presences.into_iter().map(|p| Presence {
            ai_id: p.ai_id,
            status: p.status,
            current_task: Some(p.current_task),
            last_seen: chrono::DateTime::from_timestamp_millis(p.last_seen as i64)
                .unwrap_or_default(),
        }).collect())
    }

    /// Get what AIs are doing - returns (ai_id, status, task)
    pub async fn what_are_they_doing(&self, limit: i32) -> Result<Vec<(String, String, String)>> {
        let mut client = self.get_client().await?;
        let presences = client.get_active_ais().await?;
        Ok(presences.into_iter()
            .take(limit as usize)
            .map(|p| (p.ai_id, p.status, p.current_task))
            .collect())
    }

    // =========================================================================
    // FILE CLAIMS - Conflict prevention
    // =========================================================================

    /// Claim a file
    pub async fn claim_file(&self, path: &str, _ai_id: &str, duration: i32, working_on: Option<&str>) -> Result<bool> {
        let mut client = self.get_client().await?;
        match client.claim_file(path, working_on.unwrap_or("editing"), duration as u32).await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Release a file claim
    pub async fn release_file(&self, path: &str, _ai_id: &str) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.release_file(path).await
    }

    /// Check if file is claimed
    pub async fn is_file_claimed(&self, path: &str) -> Result<Option<String>> {
        let mut client = self.get_client().await?;
        client.is_file_claimed(path).await
    }

    /// Get active claims - returns (path, claimer)
    pub async fn get_active_claims(&self) -> Result<Vec<(String, String)>> {
        let mut client = self.get_client().await?;
        let claims = client.get_active_claims().await?;
        Ok(claims.into_iter().map(|c| (c.path, c.claimer)).collect())
    }

    /// Force release all claims for an AI
    pub async fn force_release_all_claims(&self, _ai_id: &str) -> Result<i32> {
        Ok(0)
    }

    // =========================================================================
    // TASKS - Task queue management (all IDs are i32)
    // =========================================================================

    /// Queue a task - returns task ID as i32
    pub async fn queue_task(&self, description: &str, priority: i32) -> Result<i32> {
        let mut client = self.get_client().await?;
        let id = client.queue_task(description, priority as u8, "").await?;
        Ok(id as i32)
    }

    /// Claim a task - returns Option<task_id>
    pub async fn claim_task(&self, _ai_id: &str) -> Result<Option<i32>> {
        let mut client = self.get_client().await?;
        match client.claim_task(None).await? {
            Some(t) => Ok(Some(t.id as i32)),
            None => Ok(None),
        }
    }

    /// Claim specific task by ID
    pub async fn claim_task_by_id(&self, task_id: i32, _ai_id: &str) -> Result<bool> {
        let mut client = self.get_client().await?;
        match client.claim_task(Some(task_id as u64)).await? {
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }

    /// Complete a task
    pub async fn complete_task(&self, task_id: i32, result: &str) -> Result<()> {
        let mut client = self.get_client().await?;
        client.complete_task(task_id as u64, result).await?;
        Ok(())
    }

    /// Update task status
    pub async fn update_task_status(&self, task_id: i32, status: &str) -> Result<()> {
        let mut client = self.get_client().await?;
        match status {
            "started" | "in_progress" | "InProgress" => {
                client.start_task(task_id as u64).await?;
            }
            "done" | "completed" | "Completed" => {
                client.complete_task(task_id as u64, "").await?;
            }
            "claimed" | "Claimed" => {
                client.claim_task(Some(task_id as u64)).await?;
            }
            "blocked" | "Blocked" | "unblocked" | "Unblocked" => {
                anyhow::bail!("Task block/unblock not supported in V1. Use V2 backend.");
            }
            other => {
                anyhow::bail!("Unknown task status: '{}'. Valid: done, claimed, started, blocked, unblocked", other);
            }
        }
        Ok(())
    }

    /// Get task by ID
    pub async fn get_task(&self, task_id: i32) -> Result<Option<Task>> {
        let mut client = self.get_client().await?;
        let tasks = client.list_tasks(100, false).await?;
        Ok(tasks.into_iter()
            .find(|t| t.id == task_id as u64)
            .map(|t| Task {
                id: t.id as i32,
                task: t.description,
                priority: t.priority as i32,
                status: t.status,
                assigned_to: t.claimed_by,
                created_at: chrono::Utc::now(),
                completed_at: None,
                result: None,
            }))
    }

    /// List tasks
    pub async fn list_tasks(&self, status: Option<&str>, limit: i32) -> Result<Vec<Task>> {
        let mut client = self.get_client().await?;
        let pending_only = status == Some("pending");
        let tasks = client.list_tasks(limit as usize, pending_only).await?;
        Ok(tasks.into_iter().map(|t| Task {
            id: t.id as i32,
            task: t.description,
            priority: t.priority as i32,
            status: t.status,
            assigned_to: t.claimed_by,
            created_at: chrono::Utc::now(),
            completed_at: None,
            result: None,
        }).collect())
    }

    /// Task queue stats - returns (pending, claimed, completed)
    pub async fn queue_stats(&self) -> Result<(i32, i32, i32)> {
        let mut client = self.get_client().await?;
        let stats = client.task_stats().await?;
        Ok((stats.pending as i32, stats.claimed as i32, stats.completed as i32))
    }

    /// Delete a task
    pub async fn delete_task(&self, _task_id: i32, _ai_id: &str) -> Result<bool> {
        Ok(true)
    }

    /// Find tasks with smart search
    pub async fn find_tasks_smart(&self, query: &str, limit: i32) -> Result<Vec<(i32, String, String, String, String)>> {
        let mut client = self.get_client().await?;
        let all_tasks = client.list_tasks(100, false).await?;
        
        let query_lower = query.to_lowercase();
        let mut matches: Vec<_> = all_tasks.into_iter()
            .filter(|t| {
                t.description.to_lowercase().contains(&query_lower) ||
                t.status.to_lowercase().contains(&query_lower) ||
                t.created_by.to_lowercase().contains(&query_lower) ||
                t.claimed_by.as_ref().map(|s| s.to_lowercase().contains(&query_lower)).unwrap_or(false) ||
                t.tags.to_lowercase().contains(&query_lower)
            })
            .collect();
        
        matches.sort_by(|a, b| {
            let priority = |s: &str| match s {
                "in_progress" => 0,
                "pending" => 1,
                "blocked" => 2,
                "completed" => 3,
                _ => 4,
            };
            priority(&a.status).cmp(&priority(&b.status))
        });
        
        let results: Vec<_> = matches.into_iter()
            .take(limit as usize)
            .map(|t| (
                t.id as i32,
                t.description,
                t.status,
                t.claimed_by.unwrap_or_default(),
                t.created_by,
            ))
            .collect();
        
        Ok(results)
    }

    /// Get session tasks - returns tasks claimed by this AI
    pub async fn get_session_tasks(&self, ai_id: &str) -> Result<Vec<(i32, String, String)>> {
        let mut client = self.get_client().await?;
        let tasks = client.list_tasks(100, false).await?;
        // Filter to tasks claimed by this AI (session = tasks I'm working on)
        Ok(tasks.into_iter()
            .filter(|t| t.claimed_by.as_ref().map(|c| c == ai_id).unwrap_or(false))
            .map(|t| (t.id as i32, t.description, t.status))
            .collect())
    }

    // =========================================================================
    // VOTING - Team decision making (all IDs are i32)
    // =========================================================================

    /// Create a vote
    pub async fn create_vote(&self, topic: &str, options: Vec<String>, _created_by: &str, total_voters: i32) -> Result<Vote> {
        let mut client = self.get_client().await?;
        let id = client.create_vote(topic, options.clone(), 60).await?;
        Ok(Vote {
            id: id as i32,
            topic: topic.to_string(),
            options,
            status: VoteStatus::Open,
            created_by: self.ai_id.clone(),
            created_at: chrono::Utc::now(),
            closed_at: None,
            total_voters,
            votes_cast: 0,
        })
    }

    /// Cast a vote
    pub async fn cast_vote(&self, vote_id: i32, _ai_id: &str, choice: &str) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.cast_vote(vote_id as u64, choice).await
    }

    /// Get vote results
    pub async fn get_vote_results(&self, vote_id: i32) -> Result<Option<VoteResults>> {
        let mut client = self.get_client().await?;
        match client.get_vote_results(vote_id as u64).await? {
            Some(v) => {
                let mut counts = std::collections::HashMap::new();
                let mut voters_by_choice = std::collections::HashMap::new();
                for (voter, choice) in &v.votes {
                    *counts.entry(choice.clone()).or_insert(0) += 1;
                    voters_by_choice.entry(choice.clone()).or_insert_with(Vec::new).push(voter.clone());
                }
                let winner = counts.iter().max_by_key(|(_, c)| *c).map(|(k, _)| k.clone());
                let winner_count = winner.as_ref().and_then(|w| counts.get(w)).copied().unwrap_or(0);
                Ok(Some(VoteResults {
                    vote: Vote {
                        id: v.id as i32,
                        topic: v.topic,
                        options: v.options,
                        status: if v.status.eq_ignore_ascii_case("open") { VoteStatus::Open } else { VoteStatus::Closed },
                        created_by: self.ai_id.clone(),
                        created_at: chrono::Utc::now(),
                        closed_at: None,
                        total_voters: 4,
                        votes_cast: v.votes.len() as i32,
                    },
                    counts,
                    voters_by_choice,
                    winner,
                    winner_count,
                }))
            }
            None => Ok(None),
        }
    }

    /// Get open votes
    pub async fn get_open_votes(&self) -> Result<Vec<Vote>> {
        let mut client = self.get_client().await?;
        let votes = client.list_votes(100).await?;
        Ok(votes.into_iter()
            .filter(|v| v.status.eq_ignore_ascii_case("open"))
            .map(|v| Vote {
                id: v.id as i32,
                topic: v.topic,
                options: v.options,
                status: VoteStatus::Open,
                created_by: self.ai_id.clone(),
                created_at: chrono::Utc::now(),
                closed_at: None,
                total_voters: 4,
                votes_cast: v.votes.len() as i32,
            }).collect())
    }

    /// List votes
    pub async fn list_votes(&self, limit: i32) -> Result<Vec<Vote>> {
        let mut client = self.get_client().await?;
        let votes = client.list_votes(limit as usize).await?;
        Ok(votes.into_iter().map(|v| Vote {
            id: v.id as i32,
            topic: v.topic,
            options: v.options,
            status: if v.status.eq_ignore_ascii_case("open") { VoteStatus::Open } else { VoteStatus::Closed },
            created_by: self.ai_id.clone(),
            created_at: chrono::Utc::now(),
            closed_at: None,
            total_voters: 4,
            votes_cast: v.votes.len() as i32,
        }).collect())
    }

    /// Get pending votes for an AI
    pub async fn get_pending_votes_for_ai(&self, ai_id: &str) -> Result<Vec<Vote>> {
        let mut client = self.get_client().await?;
        let votes = client.list_votes(100).await?;
        Ok(votes.into_iter()
            .filter(|v| v.status.eq_ignore_ascii_case("open") && !v.votes.iter().any(|(voter, _)| voter == ai_id))
            .map(|v| Vote {
                id: v.id as i32,
                topic: v.topic,
                options: v.options,
                status: VoteStatus::Open,
                created_by: self.ai_id.clone(),
                created_at: chrono::Utc::now(),
                closed_at: None,
                total_voters: 4,
                votes_cast: v.votes.len() as i32,
            }).collect())
    }

    /// Close a vote (creator only)
    pub async fn vote_close(&self, vote_id: u64) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.vote_close(vote_id).await
    }

    // =========================================================================
    // PROJECTS & FEATURES (all IDs are i32)
    // =========================================================================

    /// List projects - returns (id, name, goal)
    pub async fn list_projects(&self) -> Result<Vec<(i32, String, String)>> {
        let mut client = self.get_client().await?;
        let projects = client.list_projects().await?;
        Ok(projects.into_iter().map(|p| (p.id as i32, p.name, p.goal)).collect())
    }

    /// Create a project
    pub async fn create_project(&self, name: &str, goal: &str) -> Result<i32> {
        let mut client = self.get_client().await?;
        let id = client.create_project(name, goal, "").await?;
        Ok(id as i32)
    }

    /// Get a project - returns (id, name, goal, root_directory, status)
    pub async fn get_project(&self, project_id: i32) -> Result<Option<(i32, String, String, String, String)>> {
        let mut client = self.get_client().await?;
        match client.get_project(project_id as u64).await? {
            Some(p) => Ok(Some((p.id as i32, p.name, p.goal, p.root_directory, p.status))),
            None => Ok(None),
        }
    }

    /// Update a project
    pub async fn update_project(&self, project_id: i32, goal: Option<&str>, status: Option<&str>) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.update_project(project_id as u64, goal, status).await
    }

    /// Add task to project (uses regular task system with project tag)
    pub async fn add_task_to_project(&self, project_id: i32, title: &str, priority: i32) -> Result<i32> {
        let mut client = self.get_client().await?;
        // Create task with project tag for association
        let id = client.queue_task(title, priority as u8, &format!("project:{}", project_id)).await?;
        Ok(id as i32)
    }

    /// List project tasks - returns (id, title, status, priority)
    pub async fn list_project_tasks(&self, project_id: i32) -> Result<Vec<(i32, String, String, i32)>> {
        let mut client = self.get_client().await?;
        // Get all tasks and filter by project tag
        let tasks = client.list_tasks(1000, false).await?;
        let project_tag = format!("project:{}", project_id);
        Ok(tasks.into_iter()
            .filter(|t| t.tags.contains(&project_tag))
            .map(|t| (t.id as i32, t.description, t.status, t.priority as i32))
            .collect())
    }

    /// Soft delete a project
    pub async fn soft_delete_project(&self, project_id: i32, _deleted_by: &str) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.delete_project(project_id as u64).await
    }

    /// Restore a project
    pub async fn restore_project(&self, project_id: i32) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.restore_project(project_id as u64).await
    }

    /// Create a feature - matches PostgresStorage signature exactly
    pub async fn create_feature(&self, project_id: i32, name: &str, overview: &str, directory: Option<&str>, _created_by: &str) -> Result<i32> {
        let mut client = self.get_client().await?;
        let id = client.create_feature(project_id as u64, name, overview, directory).await?;
        Ok(id as i32)
    }

    /// List features - returns (id, name, overview, status)
    pub async fn list_features(&self, project_id: i32) -> Result<Vec<(i32, String, String, String)>> {
        let mut client = self.get_client().await?;
        let features = client.list_features(project_id as u64).await?;
        Ok(features.into_iter().map(|f| (f.id as i32, f.name, f.overview, f.status)).collect())
    }

    /// Get a feature - returns (id, project_id, name, overview, directory, status)
    pub async fn get_feature(&self, feature_id: i32) -> Result<Option<(i32, i32, String, String, String, String)>> {
        let mut client = self.get_client().await?;
        match client.get_feature(feature_id as u64).await? {
            Some(f) => Ok(Some((f.id as i32, f.project_id as i32, f.name, f.overview, f.directory.unwrap_or_default(), f.status))),
            None => Ok(None),
        }
    }

    /// Update a feature - matches PostgresStorage signature exactly
    pub async fn update_feature(&self, feature_id: i32, name: Option<&str>, overview: Option<&str>, directory: Option<&str>) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.update_feature(feature_id as u64, name, overview, directory).await
    }

    /// Soft delete a feature
    pub async fn soft_delete_feature(&self, feature_id: i32, _deleted_by: &str) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.delete_feature(feature_id as u64).await
    }

    /// Restore a feature
    pub async fn restore_feature(&self, feature_id: i32) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.restore_feature(feature_id as u64).await
    }

    /// Resolve file path to project/feature
    pub async fn resolve_file_to_project(&self, path: &str) -> Result<Option<(i32, Option<i32>)>> {
        let mut client = self.get_client().await?;
        match client.resolve_file_to_project(path).await? {
            Some((project_id, feature_id)) => Ok(Some((project_id as i32, feature_id.map(|f| f as i32)))),
            None => Ok(None),
        }
    }

    // =========================================================================
    // VAULT (shared key-value storage)
    // =========================================================================

    /// Store in vault
    pub async fn vault_store(&self, key: &str, value: &str) -> Result<()> {
        let mut client = self.get_client().await?;
        client.vault_store(key, value).await
    }

    /// Retrieve from vault
    pub async fn vault_retrieve(&self, key: &str) -> Result<Option<String>> {
        let mut client = self.get_client().await?;
        client.vault_get(key).await
    }

    /// List vault keys
    pub async fn vault_list(&self) -> Result<Vec<String>> {
        let mut client = self.get_client().await?;
        client.vault_list().await
    }

    // =========================================================================
    // FILE ACTIONS & TEAM ACTIVITY
    // =========================================================================

    /// Log file action
    pub async fn log_file_action(&self, _ai_id: &str, file_path: &str, action: &str) -> Result<()> {
        let mut client = self.get_client().await?;
        client.log_file_action(file_path, action).await?;
        Ok(())
    }

    /// Get recent file actions - returns (ai_id, path, action)
    pub async fn get_recent_file_actions(&self, limit: i32) -> Result<Vec<(String, String, String)>> {
        let mut client = self.get_client().await?;
        let actions = client.get_recent_file_actions(limit as usize).await?;
        Ok(actions.into_iter().map(|a| (a.ai_id, a.path, a.action)).collect())
    }

    /// Recent creations - returns (path, ai_id)
    pub async fn recent_creations(&self, limit: i32) -> Result<Vec<(String, String)>> {
        let mut client = self.get_client().await?;
        let actions = client.get_recent_file_actions(limit as usize).await?;
        // Filter to just "created" actions
        Ok(actions.into_iter()
            .filter(|a| a.action == "created")
            .map(|a| (a.path, a.ai_id))
            .collect())
    }

    /// Get team activity - returns (ai_id, action_count)
    pub async fn get_team_activity(&self, _hours: i32) -> Result<Vec<(String, i64)>> {
        let mut client = self.get_client().await?;
        // Get recent actions and count by AI
        let actions = client.get_recent_file_actions(1000).await?;
        let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for action in actions {
            *counts.entry(action.ai_id).or_insert(0) += 1;
        }
        Ok(counts.into_iter().collect())
    }

    // =========================================================================
    // MISC
    // =========================================================================

    /// List teambooks
    pub async fn list_teambooks(&self) -> Result<Vec<String>> {
        Ok(vec!["teamengram".to_string()])
    }

    /// Check for events - returns unread DMs and recent broadcasts as event strings
    pub async fn check_for_events(&self, _ai_id: &str, since: Option<chrono::DateTime<chrono::Utc>>) -> Result<Vec<String>> {
        let mut client = self.get_client().await?;
        let mut events = Vec::new();

        // Get unread direct messages
        let unread_dms = client.get_unread_dms(50).await?;
        for dm in unread_dms {
            let dm_time = chrono::DateTime::from_timestamp_millis(dm.created_at as i64);
            // Filter by since timestamp if provided
            if let Some(since_time) = since {
                if let Some(msg_time) = dm_time {
                    if msg_time < since_time {
                        continue;
                    }
                }
            }
            events.push(format!("DM from {}: {}", dm.from_ai, dm.content));
        }

        // Get recent broadcasts
        let broadcasts = client.get_broadcasts("general", 50).await?;
        for bc in broadcasts {
            let bc_time = chrono::DateTime::from_timestamp_millis(bc.created_at as i64);
            // Filter by since timestamp if provided
            if let Some(since_time) = since {
                if let Some(msg_time) = bc_time {
                    if msg_time < since_time {
                        continue;
                    }
                }
            }
            events.push(format!("Broadcast from {} in {}: {}", bc.from_ai, bc.channel, bc.content));
        }

        Ok(events)
    }

    /// Get status
    pub async fn get_status(&self) -> Result<String> {
        Ok("TeamEngram".to_string())
    }

    /// Query pheromones - returns (type, intensity, ai_id, age_secs)
    /// In TeamEngram, stigmergy is implemented through FILE CLAIMS
    /// - location: file path or directory prefix to filter
    /// - pheromone_type: "file_claim" or None for all
    /// Returns: (type="file_claim", intensity=1.0, claimer_ai_id, age_seconds)
    pub async fn query_pheromones(&self, location: &str, pheromone_type: Option<&str>) -> Result<Vec<(String, f64, String, i64)>> {
        // Only "file_claim" pheromone type is supported (stigmergy = file claims)
        if let Some(ptype) = pheromone_type {
            if ptype != "file_claim" {
                return Ok(vec![]); // No other pheromone types exist
            }
        }

        let mut client = self.get_client().await?;
        let claims = client.get_active_claims().await?;
        let now = chrono::Utc::now().timestamp() as u64;

        let pheromones: Vec<(String, f64, String, i64)> = claims
            .into_iter()
            .filter(|claim| {
                // Filter by location prefix (empty = all)
                location.is_empty() || claim.path.starts_with(location)
            })
            .map(|claim| {
                // Calculate age in seconds from expires_at (claims have duration)
                // expires_at is future timestamp, so we estimate creation time
                let age_secs = if claim.expires_at > now {
                    0 // Still active, recently created
                } else {
                    (now - claim.expires_at) as i64 // Expired by this many seconds
                };
                ("file_claim".to_string(), 1.0, claim.claimer, age_secs)
            })
            .collect();

        Ok(pheromones)
    }

    /// Track directory access
    pub async fn track_directory(&self, _ai_id: &str, directory: &str, access_type: &str) -> Result<()> {
        let mut client = self.get_client().await?;
        client.track_directory(directory, access_type).await?;
        Ok(())
    }

    /// Get recent directories - returns (directory, access_type, timestamp)
    pub async fn get_recent_directories(&self, ai_id: &str, limit: i32) -> Result<Vec<(String, String, String)>> {
        let mut client = self.get_client().await?;
        let dirs = client.get_recent_directories(ai_id, limit as usize).await?;
        Ok(dirs.into_iter().map(|d| {
            let timestamp = chrono::DateTime::from_timestamp_millis(d.timestamp as i64)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default();
            (d.directory, d.access_type, timestamp)
        }).collect())
    }

    /// Check wake events - returns Option<(event_type, from_ai, content, timestamp)>
    /// Checks real data sources that would trigger a wake:
    /// - Unread DMs → "direct_message"
    /// - Dialogues where it's my turn → "dialogue_turn"
    /// - Recent broadcasts with @mention → "mention"
    pub async fn check_wake_events(&self, ai_id: &str, since_secs: i64) -> Result<Option<(String, String, String, String)>> {
        let mut client = self.get_client().await?;
        let since_timestamp = (chrono::Utc::now().timestamp() - since_secs) as u64 * 1000; // Convert to millis

        // Priority 1: Check for unread DMs (most urgent)
        let unread_dms = client.get_unread_dms(10).await?;
        for dm in unread_dms {
            if dm.created_at >= since_timestamp {
                let timestamp = chrono::DateTime::from_timestamp_millis(dm.created_at as i64)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_default();
                return Ok(Some(("direct_message".to_string(), dm.from_ai, dm.content, timestamp)));
            }
        }

        // Priority 2: Check for dialogues where it's my turn
        let my_turn_dialogues = client.dialogue_my_turn(5).await?;
        if let Some(dialogue) = my_turn_dialogues.first() {
            let timestamp = chrono::Utc::now().to_rfc3339();
            // Use first non-self participant as "from" attribution
            let from = dialogue.participants.iter()
                .find(|p| p.as_str() != ai_id)
                .cloned()
                .unwrap_or_else(|| dialogue.initiator.clone());
            return Ok(Some(("dialogue_turn".to_string(), from, dialogue.topic.clone(), timestamp)));
        }

        // Priority 3: Check broadcasts for @mentions
        let broadcasts = client.get_broadcasts("general", 20).await?;
        for bc in broadcasts {
            if bc.created_at >= since_timestamp {
                // Check for @mention of this AI
                let mention_pattern = format!("@{}", ai_id);
                if bc.content.contains(&mention_pattern) {
                    let timestamp = chrono::DateTime::from_timestamp_millis(bc.created_at as i64)
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_default();
                    return Ok(Some(("mention".to_string(), bc.from_ai, bc.content, timestamp)));
                }
            }
        }

        // No wake events found
        Ok(None)
    }

    // =========================================================================
    // DIALOGUES - Turn-based AI-to-AI conversations
    // =========================================================================

    /// Start a dialogue with another AI
    pub async fn dialogue_start(&self, responder: &str, topic: &str) -> Result<u64> {
        let mut client = self.get_client().await?;
        client.dialogue_start(responder, topic).await
    }

    /// Respond in an active dialogue (accepts the dialogue invitation)
    pub async fn dialogue_respond(&self, dialogue_id: u64, response: &str) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.dialogue_respond(dialogue_id, response).await
    }

    /// End a dialogue
    pub async fn dialogue_end(&self, dialogue_id: u64, status: &str) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.dialogue_end(dialogue_id, status).await
    }

    /// List dialogues for current AI
    pub async fn list_dialogues(&self, limit: usize) -> Result<Vec<crate::client::DialogueInfo>> {
        let mut client = self.get_client().await?;
        client.list_dialogues(limit).await
    }

    /// Get dialogue invites (dialogues where I'm responder and it's my turn)
    pub async fn dialogue_invites(&self, limit: usize) -> Result<Vec<crate::client::DialogueInfo>> {
        let mut client = self.get_client().await?;
        client.dialogue_invites(limit).await
    }

    /// Get dialogues where it's my turn to respond
    pub async fn dialogue_my_turn(&self, limit: usize) -> Result<Vec<crate::client::DialogueInfo>> {
        let mut client = self.get_client().await?;
        client.dialogue_my_turn(limit).await
    }

    /// Check whose turn it is in a dialogue
    pub async fn dialogue_turn(&self, dialogue_id: u64) -> Result<crate::client::DialogueTurnInfo> {
        let mut client = self.get_client().await?;
        client.dialogue_turn(dialogue_id).await
    }

    // =========================================================================
    // ROOMS - Multi-AI collaboration spaces
    // =========================================================================

    /// Create a room
    pub async fn room_create(&self, name: &str, topic: &str) -> Result<u64> {
        let mut client = self.get_client().await?;
        client.create_room(name, topic).await
    }

    /// Join a room
    pub async fn room_join(&self, room_id: u64) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.join_room(room_id).await
    }

    /// Leave a room
    pub async fn room_leave(&self, room_id: u64) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.leave_room(room_id).await
    }

    /// List rooms
    pub async fn list_rooms(&self, limit: usize) -> Result<Vec<crate::client::RoomInfo>> {
        let mut client = self.get_client().await?;
        client.list_rooms(limit).await
    }

    /// Get room details by ID
    pub async fn room_get(&self, room_id: u64) -> Result<Option<crate::client::RoomInfo>> {
        let mut client = self.get_client().await?;
        client.room_get(room_id).await
    }

    /// Close a room (creator only)
    pub async fn room_close(&self, room_id: u64) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.room_close(room_id).await
    }

    // =========================================================================
    // LOCKS - Resource locking for coordination
    // =========================================================================

    /// Acquire a lock
    pub async fn lock_acquire(&self, resource: &str, working_on: &str, duration_mins: u32) -> Result<Option<u64>> {
        let mut client = self.get_client().await?;
        client.lock_acquire(resource, working_on, duration_mins).await
    }

    /// Release a lock
    pub async fn lock_release(&self, resource: &str) -> Result<bool> {
        let mut client = self.get_client().await?;
        client.lock_release(resource).await
    }

    /// Check lock status
    pub async fn lock_check(&self, resource: &str) -> Result<Option<crate::client::LockInfo>> {
        let mut client = self.get_client().await?;
        client.lock_check(resource).await
    }
}

// ============================================================================
// V2 EVENT SOURCING STORAGE
// ============================================================================

/// V2 Storage adapter - uses event sourcing backend
/// Enable with TEAMENGRAM_V2=1 environment variable
///
/// Architecture:
/// - Each AI writes to local outbox (~100ns, wait-free)
/// - Sequencer daemon aggregates to master log
/// - Per-AI materialized views (no sharing, no corruption)
pub struct V2Storage {
    v2: Arc<tokio::sync::Mutex<V2Client>>,
    ai_id: String,
}

impl V2Storage {
    /// Check if V2 mode is enabled via TEAMENGRAM_V2=1 env var
    pub fn is_enabled() -> bool {
        std::env::var("TEAMENGRAM_V2").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false)
    }

    /// Connect to V2 storage - REQUIRES AI_ID
    pub fn connect() -> Result<Self> {
        // AI_ID is REQUIRED - fail loudly, no fallback to "unknown"
        let ai_id = std::env::var("AI_ID")
            .context("AI_ID environment variable is required for V2 storage")?;

        if ai_id.is_empty() || ai_id == "unknown" {
            anyhow::bail!("AI_ID cannot be empty or 'unknown' - set a valid AI identity");
        }

        // Load encryption key from default V2 data directory (None = plaintext)
        let v2_dir = crate::store::ai_foundation_base_dir().join("v2");
        let crypto = crate::crypto::load_encryption_key(&v2_dir)
            .ok()
            .flatten()
            .map(std::sync::Arc::new);

        let v2 = V2Client::open(&ai_id, None, crypto)
            .map_err(|e| anyhow::anyhow!("V2 client error: {}", e))?;

        Ok(Self {
            v2: Arc::new(tokio::sync::Mutex::new(v2)),
            ai_id,
        })
    }

    /// Get AI ID
    pub fn ai_id(&self) -> &str {
        &self.ai_id
    }

    /// Refresh BulletinBoard with current DMs, dialogues, etc.
    /// Called after write operations to enable passive injection via hooks.
    async fn refresh_bulletin(&self) {
        let mut v2 = self.v2.lock().await;

        // Open bulletin and update with current state
        match BulletinBoard::open(None) {
            Ok(mut bulletin) => {
                // Get recent DMs for this AI (v2 returns Messages)
                // set_dms expects: (id, created_at_secs, from_ai, to_ai, content)
                if let Ok(dms) = v2.recent_dms(20) {
                    let dm_data: Vec<(i64, i64, &str, &str, &str)> = dms.iter()
                        .filter(|m| m.to_ai.as_ref().map(|to| to == &self.ai_id).unwrap_or(false))
                        .take(10)
                        .map(|m| (
                            m.id as i64,
                            m.timestamp.timestamp(),
                            m.from_ai.as_str(),
                            m.to_ai.as_deref().unwrap_or(""),
                            m.content.as_str()
                        ))
                        .collect();
                    bulletin.set_dms(&dm_data);
                }

                // Get dialogues where it's my turn
                // Returns: (id, initiator, responder, topic, status, turn)
                if let Ok(dialogues) = v2.get_dialogue_my_turn() {
                    let dialogue_data: Vec<(i64, &str)> = dialogues.iter()
                        .map(|(id, _initiator, _responder, topic, _status, _turn)| (*id as i64, topic.as_str()))
                        .collect();
                    bulletin.set_dialogues(&dialogue_data);
                }

                // Get recent broadcasts
                // set_broadcasts expects: (id, created_at_secs, from_ai, channel, content)
                if let Ok(broadcasts) = v2.recent_broadcasts(10, None) {
                    let bc_data: Vec<(i64, i64, &str, &str, &str)> = broadcasts.iter()
                        .map(|m| (
                            m.id as i64,
                            m.timestamp.timestamp(),
                            m.from_ai.as_str(),
                            m.channel.as_str(),
                            m.content.as_str()
                        ))
                        .collect();
                    bulletin.set_broadcasts(&bc_data);
                }

                // Get votes (open votes)
                // Returns: (id, creator, topic, options, status, casts)
                // set_votes expects: (id, topic, cast_count, total_count)
                if let Ok(votes) = v2.get_votes() {
                    let vote_data: Vec<(i64, &str, u32, u32)> = votes.iter()
                        .filter(|(_, _, _, _, status, _)| status == "open")
                        .map(|(id, _creator, topic, options, _status, casts)| (
                            *id as i64,
                            topic.as_str(),
                            casts.len() as u32,
                            options.len() as u32  // Total possible voters
                        ))
                        .collect();
                    bulletin.set_votes(&vote_data);
                }

                // Get recent file actions
                // Returns: (ai_id, action, file_path)
                // set_file_actions expects: (ai_id, action, file_path, timestamp_secs)
                if let Ok(actions) = v2.get_file_actions(10) {
                    let fa_data: Vec<(&str, &str, &str, u64)> = actions.iter()
                        .map(|(ai, action, path, ts)| (ai.as_str(), action.as_str(), path.as_str(), *ts / 1000))
                        .collect();
                    bulletin.set_file_actions(&fa_data);
                }

                // Commit changes
                if let Err(e) = bulletin.commit() {
                    warn!("Failed to commit bulletin: {}", e);
                } else {
                    info!("Bulletin refreshed for {}", self.ai_id);
                }
            }
            Err(e) => {
                warn!("Failed to open BulletinBoard: {}", e);
            }
        }
    }

    // ===== CORE MESSAGING =====

    /// Broadcast a message
    pub async fn broadcast(&self, channel: &str, content: &str) -> Result<u64> {
        let mut v2 = self.v2.lock().await;
        v2.broadcast(channel, content)
            .map_err(|e| anyhow::anyhow!("V2 broadcast error: {}", e))
    }

    /// Send a direct message
    pub async fn direct_message(&self, to_ai: &str, content: &str) -> Result<u64> {
        let result = {
            let mut v2 = self.v2.lock().await;
            v2.direct_message(to_ai, content)
                .map_err(|e| anyhow::anyhow!("V2 dm error: {}", e))
        };
        // Refresh bulletin after DM (lock released)
        self.refresh_bulletin().await;
        result
    }

    /// Get recent broadcasts
    pub async fn get_recent_broadcasts(&self, limit: usize, channel: Option<&str>) -> Result<Vec<Message>> {
        let mut v2 = self.v2.lock().await;
        v2.sync().map_err(|e| anyhow::anyhow!("V2 sync error: {}", e))?;
        let msgs = v2.recent_broadcasts(limit, channel)
            .map_err(|e| anyhow::anyhow!("V2 messages error: {}", e))?;
        Ok(msgs)
    }

    /// Get recent DMs
    pub async fn get_recent_dms(&self, limit: usize) -> Result<Vec<Message>> {
        let mut v2 = self.v2.lock().await;
        v2.sync().map_err(|e| anyhow::anyhow!("V2 sync error: {}", e))?;
        let msgs = v2.recent_dms(limit)
            .map_err(|e| anyhow::anyhow!("V2 dms error: {}", e))?;
        Ok(msgs)
    }

    // ===== DIALOGUES =====

    /// Start a dialogue with one or more AIs.
    /// `responder` may be a single AI ID or a comma-separated list for n-party dialogues.
    pub async fn dialogue_start(&self, responder: &str, topic: &str) -> Result<u64> {
        let result = {
            let mut v2 = self.v2.lock().await;
            // Parse comma-separated list into slice of &str
            let participants: Vec<&str> = responder.split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            v2.start_dialogue(&participants, topic)
                .map_err(|e| anyhow::anyhow!("V2 dialogue start error: {}", e))
        };
        // Refresh bulletin so all participants see the invite
        self.refresh_bulletin().await;
        result
    }

    /// Respond to a dialogue
    pub async fn dialogue_respond(&self, dialogue_id: u64, response: &str) -> Result<u64> {
        let result = {
            let mut v2 = self.v2.lock().await;
            v2.respond_dialogue(dialogue_id, response)
                .map_err(|e| anyhow::anyhow!("V2 dialogue respond error: {}", e))
        };
        // Refresh bulletin so other party sees it's their turn
        self.refresh_bulletin().await;
        result
    }

    /// End a dialogue
    pub async fn dialogue_end(&self, dialogue_id: u64, status: &str) -> Result<u64> {
        let result = {
            let mut v2 = self.v2.lock().await;
            v2.end_dialogue(dialogue_id, status)
                .map_err(|e| anyhow::anyhow!("V2 dialogue end error: {}", e))
        };
        // Refresh bulletin to clear the dialogue
        self.refresh_bulletin().await;
        result
    }

    // ===== VOTES =====

    /// Create a vote
    pub async fn create_vote(&self, topic: &str, options: Vec<String>, total_voters: u32) -> Result<u64> {
        let mut v2 = self.v2.lock().await;
        v2.create_vote(topic, options, total_voters)
            .map_err(|e| anyhow::anyhow!("V2 vote create error: {}", e))
    }

    /// Cast a vote
    pub async fn cast_vote(&self, vote_id: u64, choice: &str) -> Result<u64> {
        let mut v2 = self.v2.lock().await;
        v2.cast_vote(vote_id, choice)
            .map_err(|e| anyhow::anyhow!("V2 vote cast error: {}", e))
    }

    // ===== TASKS =====

    /// Add a task
    pub async fn add_task(&self, description: &str, priority: u32, tags: &str) -> Result<u64> {
        let mut v2 = self.v2.lock().await;
        v2.add_task(description, priority, tags)
            .map_err(|e| anyhow::anyhow!("V2 task add error: {}", e))
    }

    /// Claim a task
    pub async fn claim_task(&self, task_id: u64) -> Result<u64> {
        let mut v2 = self.v2.lock().await;
        v2.claim_task(task_id)
            .map_err(|e| anyhow::anyhow!("V2 task claim error: {}", e))
    }

    /// Complete a task
    pub async fn complete_task(&self, task_id: u64, result: &str) -> Result<u64> {
        let mut v2 = self.v2.lock().await;
        v2.complete_task(task_id, result)
            .map_err(|e| anyhow::anyhow!("V2 task complete error: {}", e))
    }

    // ===== LOCKS (removed — deprecated Feb 2026, QD directive) =====

    // ===== ROOMS =====

    /// Create a room
    pub async fn create_room(&self, name: &str, topic: &str) -> Result<u64> {
        let mut v2 = self.v2.lock().await;
        v2.create_room(name, topic)
            .map_err(|e| anyhow::anyhow!("V2 room create error: {}", e))
    }

    /// Join a room
    pub async fn join_room(&self, room_id: &str) -> Result<u64> {
        let mut v2 = self.v2.lock().await;
        v2.join_room(room_id)
            .map_err(|e| anyhow::anyhow!("V2 room join error: {}", e))
    }

    /// Leave a room
    pub async fn leave_room(&self, room_id: &str) -> Result<u64> {
        let mut v2 = self.v2.lock().await;
        v2.leave_room(room_id)
            .map_err(|e| anyhow::anyhow!("V2 room leave error: {}", e))
    }

    // ===== PRESENCE =====

    /// Update presence
    pub async fn update_presence(&self, status: &str, task: Option<&str>) -> Result<u64> {
        let mut v2 = self.v2.lock().await;
        v2.update_presence(status, task)
            .map_err(|e| anyhow::anyhow!("V2 presence error: {}", e))
    }

    // ===== FILE ACTIONS =====

    /// Log a file action
    pub async fn log_file_action(&self, path: &str, action: &str) -> Result<u64> {
        let mut v2 = self.v2.lock().await;
        v2.log_file_action(path, action)
            .map_err(|e| anyhow::anyhow!("V2 file action error: {}", e))
    }

    // ===== STATS =====

    /// Get V2 stats
    pub async fn stats(&self) -> Result<V2Stats> {
        let v2 = self.v2.lock().await;
        let stats = v2.stats();
        Ok(V2Stats {
            events_applied: stats.events_applied,
            unread_dms: v2.unread_dm_count(),
            active_dialogues: v2.active_dialogue_count(),
            pending_votes: v2.pending_vote_count(),
            my_locks: 0, // locks deprecated (Feb 2026)
            my_tasks: v2.my_task_count(),
        })
    }
}

/// V2 Statistics
#[derive(Debug, Clone)]
pub struct V2Stats {
    pub events_applied: u64,
    pub unread_dms: u64,
    pub active_dialogues: u64,
    pub pending_votes: u64,
    pub my_locks: u64,
    pub my_tasks: u64,
}
