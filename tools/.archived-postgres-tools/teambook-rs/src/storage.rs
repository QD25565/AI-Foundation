//! PostgreSQL storage backend for teambook

use crate::types::{Message, Note, Presence, Task, Vote, VoteStatus, VoteResponse, VoteResults, FileClaim, AwarenessData, FileHistoryEntry, AiActivityStats, HotFile, TeamSummary};
use anyhow::{Context, Result};
use std::collections::HashMap;
use chrono::{NaiveDateTime, TimeZone, Utc};
use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use tokio_postgres::NoTls;
use tracing::{debug, info};

/// PostgreSQL storage backend with multi-teambook support
pub struct PostgresStorage {
    pool: Pool,
    teambook_name: String,
}

impl PostgresStorage {
    /// Create new PostgreSQL storage with default teambook
    pub async fn new(database_url: &str) -> Result<Self> {
        Self::with_teambook(database_url, "town-hall-qd").await
    }

    /// Create new PostgreSQL storage with specific teambook namespace
    pub async fn with_teambook(database_url: &str, teambook_name: &str) -> Result<Self> {
        info!("Connecting to PostgreSQL: {} (teambook: {})", database_url, teambook_name);

        // Parse URL with tokio_postgres and create manager directly (proper method per deadpool docs)
        let pg_config: tokio_postgres::Config = database_url.parse()
            .context("Failed to parse database URL")?;

        let manager = deadpool_postgres::Manager::new(pg_config, NoTls);
        let pool = Pool::builder(manager)
            .max_size(16)
            .runtime(Runtime::Tokio1)
            .build()
            .context("Failed to build connection pool")?;

        // Test connection
        let client = pool.get().await?;
        let version: String = client
            .query_one("SELECT version()", &[])
            .await?
            .get(0);
        info!("PostgreSQL connected: {}", version);

        Ok(Self {
            pool,
            teambook_name: teambook_name.to_string(),
        })
    }

    /// Get the current teambook name
    pub fn teambook_name(&self) -> &str {
        &self.teambook_name
    }

    /// Get a client from the pool (for direct queries)
    pub async fn pool(&self) -> Result<deadpool_postgres::Object> {
        self.pool.get().await.context("Failed to get connection from pool")
    }

    /// Save a note to database
    pub async fn save_note(&self, note: &Note) -> Result<()> {
        let client = self.pool.get().await?;

        // Use actual table columns: teambook_name, timestamp, content, author, tags, pinned
        // Let PostgreSQL auto-generate the id
        client
            .execute(
                "INSERT INTO notes (teambook_name, timestamp, content, author, tags, pinned)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                &[
                    &self.teambook_name,  // Use configured teambook namespace
                    &note.timestamp.naive_utc(),  // Convert to NaiveDateTime
                    &note.content,
                    &note.ai_id,  // maps to 'author' column
                    &note.tags,
                    &note.pinned,
                ],
            )
            .await
            .context("Failed to save note")?;

        debug!("Saved note by: {} (teambook: {})", note.ai_id, self.teambook_name);
        Ok(())
    }

    /// Get recent notes (filtered by teambook namespace)
    pub async fn get_recent_notes(&self, limit: i32) -> Result<Vec<Note>> {
        let client = self.pool.get().await?;

        let rows = client
            .query(
                "SELECT id, author, timestamp, content, tags, pinned
                 FROM notes
                 WHERE teambook_name = $1
                 ORDER BY created DESC
                 LIMIT $2",
                &[&self.teambook_name, &(limit as i64)],
            )
            .await?;

        let notes = rows
            .iter()
            .map(|row| {
                let naive_ts: NaiveDateTime = row.get(2);
                Note {
                    id: row.get(0),
                    ai_id: row.get(1),
                    timestamp: Utc.from_utc_datetime(&naive_ts),
                    content: row.get(3),
                    tags: row.get(4),
                    pinned: row.get(5),
                }
            })
            .collect();

        Ok(notes)
    }

    /// Save a message to database
    pub async fn save_message(&self, msg: &Message) -> Result<()> {
        let client = self.pool.get().await?;

        // Let PostgreSQL auto-generate the id, use configured teambook namespace
        client
            .execute(
                "INSERT INTO messages (teambook_name, from_ai, to_ai, created, content, channel)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                &[
                    &self.teambook_name, // Use configured teambook namespace
                    &msg.from_ai,
                    &msg.to_ai,
                    &msg.timestamp.naive_utc(), // Convert to NaiveDateTime for 'timestamp without time zone' column
                    &msg.content,
                    &msg.channel,
                ],
            )
            .await
            .context("Failed to save message")?;

        debug!("Saved message from: {} (teambook: {})", msg.from_ai, self.teambook_name);
        Ok(())
    }

    /// Get recent messages (filtered by teambook namespace)
    pub async fn get_recent_messages(&self, limit: i32) -> Result<Vec<Message>> {
        let client = self.pool.get().await?;

        let rows = client
            .query(
                "SELECT id, from_ai, to_ai, created, content, channel
                 FROM messages
                 WHERE to_ai IS NULL AND teambook_name = $1
                 ORDER BY created DESC
                 LIMIT $2",
                &[&self.teambook_name, &(limit as i64)],
            )
            .await?;

        let messages = rows
            .iter()
            .map(|row| {
                let naive_ts: Option<chrono::NaiveDateTime> = row.get(3);
                let timestamp = naive_ts
                    .map(|ts| Utc.from_utc_datetime(&ts))
                    .unwrap_or_else(Utc::now);
                Message {
                    id: row.get(0),
                    from_ai: row.get(1),
                    to_ai: row.get(2),
                    timestamp,
                    content: row.get(4),
                    channel: row.get(5),
                    message_type: crate::types::MessageType::Broadcast,
                }
            })
            .collect();

        Ok(messages)
    }

    /// Get direct messages for an AI (filtered by teambook namespace)
    pub async fn get_direct_messages(&self, ai_id: &str, limit: i32) -> Result<Vec<Message>> {
        let client = self.pool.get().await?;

        let rows = client
            .query(
                "SELECT id, from_ai, to_ai, created, content, channel
                 FROM messages
                 WHERE to_ai = $1 AND teambook_name = $2
                 ORDER BY created DESC
                 LIMIT $3",
                &[&ai_id, &self.teambook_name, &(limit as i64)],
            )
            .await?;

        let messages = rows
            .iter()
            .map(|row| {
                let naive_ts: Option<chrono::NaiveDateTime> = row.get(3);
                let timestamp = naive_ts
                    .map(|ts| Utc.from_utc_datetime(&ts))
                    .unwrap_or_else(Utc::now);
                Message {
                    id: row.get(0),
                    from_ai: row.get(1),
                    to_ai: row.get(2),
                    timestamp,
                    content: row.get(4),
                    channel: row.get(5),
                    message_type: crate::types::MessageType::Direct,
                }
            })
            .collect();

        Ok(messages)
    }

    /// Initialize database schema (create tables and indexes)
    pub async fn init_schema(&self) -> Result<()> {
        let client = self.pool.get().await?;

        // Messages table
        client.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                from_ai TEXT NOT NULL,
                to_ai TEXT,
                channel TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                message_type TEXT NOT NULL
            )",
            &[],
        ).await.context("Failed to create messages table")?;

        // Notes table
        client.execute(
            "CREATE TABLE IF NOT EXISTS notes (
                id TEXT PRIMARY KEY,
                ai_id TEXT NOT NULL,
                timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                content TEXT NOT NULL,
                tags TEXT[] NOT NULL,
                pinned BOOLEAN NOT NULL DEFAULT FALSE
            )",
            &[],
        ).await.context("Failed to create notes table")?;

        // Presence table
        client.execute(
            "CREATE TABLE IF NOT EXISTS ai_presence (
                ai_id TEXT PRIMARY KEY,
                last_seen TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                status TEXT NOT NULL,
                current_task TEXT
            )",
            &[],
        ).await.context("Failed to create presence table")?;

        // Vault table (key-value storage)
        client.execute(
            "CREATE TABLE IF NOT EXISTS vault (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
            &[],
        ).await.context("Failed to create vault table")?;

        // Tasks table
        client.execute(
            "CREATE TABLE IF NOT EXISTS task_queue (
                id SERIAL PRIMARY KEY,
                task TEXT NOT NULL,
                priority INTEGER NOT NULL DEFAULT 5,
                status TEXT NOT NULL DEFAULT 'pending',
                assigned_to TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                completed_at TIMESTAMPTZ,
                result TEXT
            )",
            &[],
        ).await.context("Failed to create tasks table")?;

        // Projects table
        client.execute(
            "CREATE TABLE IF NOT EXISTS projects (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                goal TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                completed_at TIMESTAMPTZ
            )",
            &[],
        ).await.context("Failed to create projects table")?;

        // Project tasks table
        client.execute(
            "CREATE TABLE IF NOT EXISTS project_tasks (
                id SERIAL PRIMARY KEY,
                project_id INTEGER NOT NULL REFERENCES projects(id),
                title TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                priority INTEGER NOT NULL DEFAULT 5,
                assigned_to TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                completed_at TIMESTAMPTZ
            )",
            &[],
        ).await.context("Failed to create project_tasks table")?;

        // File claims table
        client.execute(
            "CREATE TABLE IF NOT EXISTS file_claims (
                file_path TEXT PRIMARY KEY,
                claimed_by TEXT NOT NULL,
                claimed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                expires_at TIMESTAMPTZ NOT NULL,
                operation TEXT NOT NULL DEFAULT 'editing'
            )",
            &[],
        ).await.context("Failed to create file_claims table")?;

        // Votes table - Full democratic consensus system
        client.execute(
            "CREATE TABLE IF NOT EXISTS team_votes (
                id SERIAL PRIMARY KEY,
                topic TEXT NOT NULL,
                options TEXT[] NOT NULL,
                status TEXT NOT NULL DEFAULT 'open',
                created_by TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                closed_at TIMESTAMPTZ,
                total_voters INTEGER NOT NULL DEFAULT 0,
                votes_cast INTEGER NOT NULL DEFAULT 0,
                timeout_minutes INTEGER NOT NULL DEFAULT 3,
                threshold_pct INTEGER NOT NULL DEFAULT 75
            )",
            &[],
        ).await.context("Failed to create team_votes table")?;

        // Vote responses table - Individual ballots
        client.execute(
            "CREATE TABLE IF NOT EXISTS vote_responses (
                id SERIAL PRIMARY KEY,
                vote_id INTEGER NOT NULL REFERENCES team_votes(id) ON DELETE CASCADE,
                voter_ai TEXT NOT NULL,
                choice TEXT NOT NULL,
                voted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                UNIQUE(vote_id, voter_ai)
            )",
            &[],
        ).await.context("Failed to create vote_responses table")?;

        // Intent signaling table - Proactive coordination
        client.execute(
            "CREATE TABLE IF NOT EXISTS ai_intents (
                id SERIAL PRIMARY KEY,
                ai_id TEXT NOT NULL,
                intent_text TEXT NOT NULL,
                started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                expires_at TIMESTAMPTZ NOT NULL,
                related_files TEXT[] NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'active'
            )",
            &[],
        ).await.context("Failed to create ai_intents table")?;

        // Indexes for performance
        client.execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_channel
             ON messages(channel, timestamp DESC)",
            &[],
        ).await.ok();

        client.execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_to_ai
             ON messages(to_ai, timestamp DESC)",
            &[],
        ).await.ok();

        client.execute(
            "CREATE INDEX IF NOT EXISTS idx_notes_ai_id
             ON notes(ai_id, timestamp DESC)",
            &[],
        ).await.ok();

        // Voting indexes
        client.execute(
            "CREATE INDEX IF NOT EXISTS idx_team_votes_status
             ON team_votes(status, created_at DESC)",
            &[],
        ).await.ok();

        client.execute(
            "CREATE INDEX IF NOT EXISTS idx_vote_responses_vote_id
             ON vote_responses(vote_id)",
            &[],
        ).await.ok();

        // Intent signaling indexes
        client.execute(
            "CREATE INDEX IF NOT EXISTS idx_ai_intents_status_expires
             ON ai_intents(status, expires_at DESC)",
            &[],
        ).await.ok();

        client.execute(
            "CREATE INDEX IF NOT EXISTS idx_ai_intents_ai_id
             ON ai_intents(ai_id, started_at DESC)",
            &[],
        ).await.ok();

        info!("Database schema initialized");
        Ok(())
    }

    /// Update AI presence
    pub async fn update_presence(&self, presence: &Presence) -> Result<()> {
        let client = self.pool.get().await?;

        client.execute(
            "INSERT INTO ai_presence (ai_id, last_seen, status_message, last_operation)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (ai_id) DO UPDATE SET
                last_seen = EXCLUDED.last_seen,
                status_message = EXCLUDED.status_message,
                last_operation = EXCLUDED.last_operation",
            &[&presence.ai_id, &presence.last_seen, &presence.status, &presence.current_task],
        ).await.context("Failed to update presence")?;

        debug!("Updated presence for: {}", presence.ai_id);
        Ok(())
    }

    /// Get active AIs (seen within N minutes)
    pub async fn get_active_ais(&self, minutes: i64) -> Result<Vec<Presence>> {
        let client = self.pool.get().await?;

        let rows = client.query(
            "SELECT ai_id, last_seen, COALESCE(status_message, 'active') as status, last_operation as current_task
             FROM ai_presence
             WHERE last_seen > NOW() - make_interval(mins => $1::int)
             ORDER BY last_seen DESC",
            &[&(minutes as i32)],
        ).await?;

        let presences = rows.iter().map(|row| Presence {
            ai_id: row.get(0),
            last_seen: row.get(1),
            status: row.get(2),
            current_task: row.get(3),
        }).collect();

        Ok(presences)
    }

    // ===== VAULT OPERATIONS =====

    /// Store value in vault
    pub async fn vault_store(&self, key: &str, value: &str) -> Result<()> {
        let client = self.pool.get().await?;

        client.execute(
            "INSERT INTO vault (key, value, updated_at)
             VALUES ($1, $2, NOW())
             ON CONFLICT (key) DO UPDATE SET
                value = EXCLUDED.value,
                updated_at = NOW()",
            &[&key, &value],
        ).await.context("Failed to store vault value")?;

        debug!("Stored vault key: {}", key);
        Ok(())
    }

    /// Retrieve value from vault
    pub async fn vault_retrieve(&self, key: &str) -> Result<Option<String>> {
        let client = self.pool.get().await?;

        let result = client.query_opt(
            "SELECT value FROM vault WHERE key = $1",
            &[&key],
        ).await?;

        Ok(result.map(|row| row.get(0)))
    }

    /// List all vault keys
    pub async fn vault_list(&self) -> Result<Vec<String>> {
        let client = self.pool.get().await?;

        let rows = client.query(
            "SELECT key FROM vault ORDER BY key",
            &[],
        ).await?;

        Ok(rows.iter().map(|row| row.get(0)).collect())
    }

    // ===== NOTE PIN OPERATIONS =====

    /// Pin a note
    pub async fn pin_note(&self, note_id: &str) -> Result<()> {
        let client = self.pool.get().await?;

        client.execute(
            "UPDATE notes SET pinned = TRUE WHERE id = $1",
            &[&note_id],
        ).await.context("Failed to pin note")?;

        debug!("Pinned note: {}", note_id);
        Ok(())
    }

    /// Unpin a note
    pub async fn unpin_note(&self, note_id: &str) -> Result<()> {
        let client = self.pool.get().await?;

        client.execute(
            "UPDATE notes SET pinned = FALSE WHERE id = $1",
            &[&note_id],
        ).await.context("Failed to unpin note")?;

        debug!("Unpinned note: {}", note_id);
        Ok(())
    }

    // ===== TASK QUEUE OPERATIONS =====

    /// Queue a new task
    pub async fn queue_task(&self, task: &str, priority: i32) -> Result<i32> {
        let client = self.pool.get().await?;

        let row = client.query_one(
            "INSERT INTO task_queue (title, priority, status)
             VALUES ($1, $2, 'pending')
             RETURNING id",
            &[&task, &priority],
        ).await.context("Failed to queue task")?;

        let task_id: i32 = row.get(0);
        debug!("Queued task: {}", task_id);
        Ok(task_id)
    }

    /// Queue a task excluding a specific AI from claiming it
    pub async fn queue_task_with_exclude(&self, task: &str, priority: i32, exclude_ai: Option<&str>) -> Result<i32> {
        let client = self.pool.get().await?;

        let row = client.query_one(
            "INSERT INTO task_queue (title, priority, status, exclude_ai)
             VALUES ($1, $2, 'pending', $3)
             RETURNING id",
            &[&task, &priority, &exclude_ai],
        ).await.context("Failed to queue task with exclude")?;

        let task_id: i32 = row.get(0);
        debug!("Queued task with exclude: {} (exclude: {:?})", task_id, exclude_ai);
        Ok(task_id)
    }

    /// Claim a task (atomic operation)
    pub async fn claim_task(&self, ai_id: &str) -> Result<Option<i32>> {
        let client = self.pool.get().await?;

        let result = client.query_opt(
            "UPDATE task_queue
             SET status = 'in_progress', claimed_by = $1, claimed_at = NOW()
             WHERE id = (
                 SELECT id FROM task_queue
                 WHERE status = 'pending'
                 ORDER BY priority DESC, created_at ASC
                 LIMIT 1
                 FOR UPDATE SKIP LOCKED
             )
             RETURNING id",
            &[&ai_id],
        ).await?;

        Ok(result.map(|row| row.get(0)))
    }

    /// Claim specific task by ID
    pub async fn claim_task_by_id(&self, task_id: i32, ai_id: &str) -> Result<bool> {
        let client = self.pool.get().await?;

        let rows_affected = client.execute(
            "UPDATE task_queue
             SET status = 'in_progress', claimed_by = $1, claimed_at = NOW()
             WHERE id = $2 AND status = 'pending'",
            &[&ai_id, &task_id],
        ).await?;

        Ok(rows_affected > 0)
    }

    /// Complete a task
    pub async fn complete_task(&self, task_id: i32, result: &str) -> Result<()> {
        let client = self.pool.get().await?;

        client.execute(
            "UPDATE task_queue
             SET status = 'completed', result = $1, completed_at = NOW()
             WHERE id = $2",
            &[&result, &task_id],
        ).await.context("Failed to complete task")?;

        debug!("Completed task: {}", task_id);
        Ok(())
    }

    /// Update task status
    pub async fn update_task_status(&self, task_id: i32, status: &str) -> Result<()> {
        let client = self.pool.get().await?;

        client.execute(
            "UPDATE task_queue SET status = $1 WHERE id = $2",
            &[&status, &task_id],
        ).await.context("Failed to update task status")?;

        debug!("Updated task {} to {}", task_id, status);
        Ok(())
    }

    /// Get queue statistics
    pub async fn queue_stats(&self) -> Result<(i32, i32, i32)> {
        let client = self.pool.get().await?;

        let row = client.query_one(
            "SELECT
                COUNT(*) FILTER (WHERE status = 'pending') as pending,
                COUNT(*) FILTER (WHERE status = 'in_progress') as in_progress,
                COUNT(*) FILTER (WHERE status = 'completed') as completed
             FROM task_queue",
            &[],
        ).await?;

        let pending: i64 = row.get(0);
        let in_progress: i64 = row.get(1);
        let completed: i64 = row.get(2);

        Ok((pending as i32, in_progress as i32, completed as i32))
    }

    /// List tasks with optional status filter
    pub async fn list_tasks(&self, status_filter: Option<&str>, limit: i32) -> Result<Vec<Task>> {
        let client = self.pool.get().await?;

        let rows = if let Some(status) = status_filter {
            if status == "all" {
                client.query(
                    "SELECT id, title, priority, status, claimed_by, created_at, completed_at, result
                     FROM task_queue
                     ORDER BY priority DESC, created_at ASC
                     LIMIT $1",
                    &[&(limit as i64)],
                ).await?
            } else {
                client.query(
                    "SELECT id, title, priority, status, claimed_by, created_at, completed_at, result
                     FROM task_queue
                     WHERE status = $1
                     ORDER BY priority DESC, created_at ASC
                     LIMIT $2",
                    &[&status, &(limit as i64)],
                ).await?
            }
        } else {
            client.query(
                "SELECT id, title, priority, status, claimed_by, created_at, completed_at, result
                 FROM task_queue
                 WHERE status = 'pending'
                 ORDER BY priority DESC, created_at ASC
                 LIMIT $1",
                &[&(limit as i64)],
            ).await?
        };

        let tasks = rows.iter().map(|row| {
            let created_naive: NaiveDateTime = row.get(5);
            let completed_naive: Option<NaiveDateTime> = row.get(6);
            Task {
                id: row.get(0),
                task: row.get(1),
                priority: row.get(2),
                status: row.get(3),
                assigned_to: row.get(4),
                created_at: Utc.from_utc_datetime(&created_naive),
                completed_at: completed_naive.map(|n| Utc.from_utc_datetime(&n)),
                result: row.get(7),
            }
        }).collect();

        Ok(tasks)
    }

    /// Get a specific task by ID
    pub async fn get_task(&self, task_id: i32) -> Result<Option<Task>> {
        let client = self.pool.get().await?;

        let row = client.query_opt(
            "SELECT id, title, priority, status, claimed_by, created_at, completed_at, result
             FROM task_queue WHERE id = $1",
            &[&task_id],
        ).await?;

        Ok(row.map(|r| {
            let created_naive: NaiveDateTime = r.get(5);
            let completed_naive: Option<NaiveDateTime> = r.get(6);
            Task {
                id: r.get(0),
                task: r.get(1),
                priority: r.get(2),
                status: r.get(3),
                assigned_to: r.get(4),
                created_at: Utc.from_utc_datetime(&created_naive),
                completed_at: completed_naive.map(|n| Utc.from_utc_datetime(&n)),
                result: r.get(7),
            }
        }))
    }

    // ===== PROJECT MANAGEMENT =====

    /// Create a new project
    pub async fn create_project(&self, name: &str, goal: &str) -> Result<i32> {
        let client = self.pool.get().await?;

        let row = client.query_one(
            "INSERT INTO projects (name, goal)
             VALUES ($1, $2)
             RETURNING id",
            &[&name, &goal],
        ).await.context("Failed to create project")?;

        let project_id: i32 = row.get(0);
        debug!("Created project: {} ({})", name, project_id);
        Ok(project_id)
    }

    /// List all projects
    pub async fn list_projects(&self) -> Result<Vec<(i32, String, String)>> {
        let client = self.pool.get().await?;

        let rows = client.query(
            "SELECT id, name, goal FROM projects
             WHERE completed_at IS NULL
             ORDER BY created DESC",
            &[],
        ).await?;

        Ok(rows.iter().map(|row| (row.get(0), row.get(1), row.get(2))).collect())
    }

    /// Add task to project
    pub async fn add_task_to_project(&self, project_id: i32, title: &str, priority: i32) -> Result<i32> {
        let client = self.pool.get().await?;

        let row = client.query_one(
            "INSERT INTO project_tasks (project_id, title, priority, status)
             VALUES ($1, $2, $3, 'pending')
             RETURNING id",
            &[&project_id, &title, &priority],
        ).await.context("Failed to add task to project")?;

        let task_id: i32 = row.get(0);
        debug!("Added task {} to project {}", task_id, project_id);
        Ok(task_id)
    }

    /// List project tasks
    pub async fn list_project_tasks(&self, project_id: i32) -> Result<Vec<(i32, String, String, i32)>> {
        let client = self.pool.get().await?;

        let rows = client.query(
            "SELECT id, title, status, priority
             FROM project_tasks
             WHERE project_id = $1
             ORDER BY priority DESC, created_at ASC",
            &[&project_id],
        ).await?;

        Ok(rows.iter().map(|row| (row.get(0), row.get(1), row.get(2), row.get(3))).collect())
    }

    // ===== FILE CLAIMS =====

    /// Claim a file
    pub async fn claim_file(&self, file_path: &str, ai_id: &str, duration_minutes: i32) -> Result<bool> {
        let client = self.pool.get().await?;

        // Clean up expired claims first
        client.execute(
            "DELETE FROM file_claims WHERE expires_at < NOW()",
            &[],
        ).await.ok();

        // Use make_interval for proper interval construction
        let result = client.execute(
            "INSERT INTO file_claims (file_path, claimed_by, ai_name, expires_at, operation)
             VALUES ($1, $2, $2, NOW() + make_interval(mins => $3), 'editing')
             ON CONFLICT (file_path) DO NOTHING",
            &[&file_path, &ai_id, &duration_minutes],
        ).await;

        match result {
            Ok(rows) => Ok(rows > 0),
            Err(e) => {
                debug!("Failed to claim file {}: {}", file_path, e);
                Ok(false)
            },
        }
    }

    /// Release a file claim
    pub async fn release_file(&self, file_path: &str, ai_id: &str) -> Result<bool> {
        let client = self.pool.get().await?;

        let rows_affected = client.execute(
            "DELETE FROM file_claims
             WHERE file_path = $1 AND claimed_by = $2",
            &[&file_path, &ai_id],
        ).await?;

        Ok(rows_affected > 0)
    }

    /// Check if file is claimed
    pub async fn is_file_claimed(&self, file_path: &str) -> Result<Option<String>> {
        let client = self.pool.get().await?;

        // Clean up expired claims first
        client.execute(
            "DELETE FROM file_claims WHERE expires_at < NOW()",
            &[],
        ).await.ok();

        let result = client.query_opt(
            "SELECT claimed_by FROM file_claims
             WHERE file_path = $1 AND expires_at > NOW()",
            &[&file_path],
        ).await?;

        Ok(result.map(|row| row.get(0)))
    }

    /// Get all active file claims
    pub async fn get_active_claims(&self) -> Result<Vec<(String, String)>> {
        let client = self.pool.get().await?;

        // Clean up expired claims first
        client.execute(
            "DELETE FROM file_claims WHERE expires_at < NOW()",
            &[],
        ).await.ok();

        let rows = client.query(
            "SELECT file_path, claimed_by FROM file_claims
             WHERE expires_at > NOW()
             ORDER BY claimed_at DESC",
            &[],
        ).await?;

        Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
    }

    /// Force release all claims by AI
    pub async fn force_release_all_claims(&self, ai_id: &str) -> Result<i32> {
        let client = self.pool.get().await?;

        let rows_affected = client.execute(
            "DELETE FROM file_claims WHERE claimed_by = $1",
            &[&ai_id],
        ).await? as i32;

        debug!("Force released {} claims for {}", rows_affected, ai_id);
        Ok(rows_affected)
    }

    // ===== MESSAGING FUNCTIONS =====

    /// Get recent messages from a channel
    pub async fn get_messages(&self, channel: &str, limit: i32) -> Result<Vec<(String, String, String, String)>> {
        let client = self.pool.get().await?;

        let rows = client.query(
            "SELECT id, from_ai, content, channel
             FROM messages
             WHERE channel = $1
             ORDER BY created DESC
             LIMIT $2",
            &[&channel, &(limit as i64)],
        ).await?;

        Ok(rows.iter().map(|row| (
            row.get::<_, i32>(0).to_string(),  // id (int -> string)
            row.get::<_, String>(1),  // from_ai
            row.get::<_, String>(2),  // content
            row.get::<_, String>(3),  // channel
        )).collect())
    }

    /// Read direct messages for an AI
    pub async fn read_dms(&self, ai_id: &str, limit: i32) -> Result<Vec<(String, String, String)>> {
        let client = self.pool.get().await?;

        let rows = client.query(
            "SELECT id, ai_id, content
             FROM messages
             WHERE message_type = 'dm' AND (channel = $1 OR ai_id = $1)
             ORDER BY created DESC
             LIMIT $2",
            &[&ai_id, &(limit as i64)],
        ).await?;

        Ok(rows.iter().map(|row| (
            row.get::<_, i32>(0).to_string(),  // id (int -> string)
            row.get::<_, String>(1),  // from_ai
            row.get::<_, String>(2),  // content
        )).collect())
    }

    // ===== PRESENCE & STATUS =====

    /// Get what AIs are doing (recent activity)
    pub async fn what_are_they_doing(&self, limit: i32) -> Result<Vec<(String, String, String)>> {
        let client = self.pool.get().await?;

        let rows = client.query(
            "SELECT ai_id, COALESCE(status_message, 'active') as status, last_operation as current_task
             FROM ai_presence
             WHERE last_seen > NOW() - make_interval(mins => 10)
             ORDER BY last_seen DESC
             LIMIT $1",
            &[&(limit as i64)],
        ).await?;

        Ok(rows.iter().map(|row| {
            let ai_id: String = row.get(0);
            let status: String = row.get(1);
            let task: Option<String> = row.get(2);
            (ai_id, status, task.unwrap_or_else(|| "idle".to_string()))
        }).collect())
    }

    /// Get presence for specific AI
    pub async fn get_presence(&self, ai_id: &str) -> Result<Option<Presence>> {
        let client = self.pool.get().await?;

        let row = client.query_opt(
            "SELECT ai_id, COALESCE(status_message, 'active') as status, last_seen, last_operation as current_task
             FROM ai_presence
             WHERE ai_id = $1",
            &[&ai_id],
        ).await?;

        Ok(row.map(|r| Presence {
            ai_id: r.get(0),
            status: r.get(1),
            last_seen: r.get(2),
            current_task: r.get(3),
        }))
    }

    /// Get connection health status
    pub async fn connection_health(&self) -> Result<(bool, String)> {
        let client = self.pool.get().await?;

        // Test query
        let result = client.query_one("SELECT 1 as test", &[]).await;

        match result {
            Ok(_) => Ok((true, "PostgreSQL connected".to_string())),
            Err(e) => Ok((false, format!("PostgreSQL error: {}", e))),
        }
    }

    /// Get teambook status
    pub async fn get_status(&self) -> Result<String> {
        let client = self.pool.get().await?;

        // Get counts
        let row = client.query_one(
            "SELECT
                (SELECT COUNT(*) FROM messages) as messages,
                (SELECT COUNT(*) FROM notes) as notes,
                (SELECT COUNT(*) FROM ai_presence) as ais,
                (SELECT COUNT(*) FROM task_queue) as tasks",
            &[],
        ).await?;

        let messages: i64 = row.get(0);
        let notes: i64 = row.get(1);
        let ais: i64 = row.get(2);
        let tasks: i64 = row.get(3);

        Ok(format!(
            "Teambook Status: {} messages, {} notes, {} AIs, {} tasks",
            messages, notes, ais, tasks
        ))
    }

    // ===== EVENT SYSTEM =====

    /// Check for events (messages, tasks, etc.)
    pub async fn check_for_events(&self, ai_id: &str, since: Option<chrono::DateTime<chrono::Utc>>) -> Result<Vec<String>> {
        let client = self.pool.get().await?;

        let since_time = since.unwrap_or_else(|| chrono::Utc::now() - chrono::Duration::minutes(5));

        // Check for new messages
        let msg_rows = client.query(
            "SELECT 'message' as type, id, content, created_at
             FROM messages
             WHERE created_at > $1 AND (channel = $2 OR ai_id = $2)
             ORDER BY created DESC
             LIMIT 10",
            &[&since_time, &ai_id],
        ).await?;

        // Check for new tasks
        let task_rows = client.query(
            "SELECT 'task' as type, id::text, task, created_at
             FROM task_queue
             WHERE created_at > $1 AND assigned_to = $2
             ORDER BY created DESC
             LIMIT 10",
            &[&since_time, &ai_id],
        ).await?;

        let mut events = Vec::new();
        for row in msg_rows {
            let event_type: String = row.get(0);
            let id: String = row.get(1);
            let content: String = row.get(2);
            events.push(format!("{}:{}:{}", event_type, id, content));
        }
        for row in task_rows {
            let event_type: String = row.get(0);
            let id: String = row.get(1);
            let content: String = row.get(2);
            events.push(format!("{}:{}:{}", event_type, id, content));
        }

        Ok(events)
    }

    /// Batch operation helper
    pub async fn batch_operations(&self, operations: Vec<String>) -> Result<Vec<String>> {
        let mut results = Vec::new();
        for op in operations {
            results.push(format!("executed:{}", op));
        }
        Ok(results)
    }

    // ===== ACTIVITY TRACKING =====

    /// Log file action
    pub async fn log_file_action(&self, ai_id: &str, file_path: &str, action: &str) -> Result<()> {
        let client = self.pool.get().await?;

        client.execute(
            "INSERT INTO ai_file_actions (ai_id, file_path, action_type, timestamp)
             VALUES ($1, $2, $3, NOW())",
            &[&ai_id, &file_path, &action],
        ).await.ok();

        Ok(())
    }

    /// Log file action with extended metadata
    pub async fn log_file_action_extended(
        &self,
        ai_id: &str,
        action: &str,
        file_path: &str,
        file_type: &str,
        file_size: Option<i64>,
        cwd: Option<&str>
    ) -> Result<()> {
        let client = self.pool.get().await?;

        client.execute(
            "INSERT INTO ai_file_actions (ai_id, file_path, action_type, file_type, file_size, working_directory, timestamp)
             VALUES ($1, $2, $3, $4, $5, $6, NOW())
             ON CONFLICT DO NOTHING",
            &[&ai_id, &file_path, &action, &file_type, &file_size, &cwd],
        ).await.ok();

        Ok(())
    }

    /// Log hook execution analytics
    pub async fn log_hook_analytics(
        &self,
        ai_id: &str,
        hook_type: &str,
        execution_ms: i32,
        tokens: i32,
        new_dms: i32,
        new_broadcasts: i32,
        pending_votes: i32
    ) -> Result<()> {
        let client = self.pool.get().await?;

        client.execute(
            "INSERT INTO hook_analytics (ai_id, hook_type, execution_ms, tokens_injected, new_dms, new_broadcasts, pending_votes, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())",
            &[&ai_id, &hook_type, &execution_ms, &tokens, &new_dms, &new_broadcasts, &pending_votes],
        ).await.ok();

        Ok(())
    }

    /// Get awareness data for hooks (DMs, broadcasts, votes)
    pub async fn get_awareness_data(&self, ai_id: &str, dm_limit: i32, broadcast_limit: i32) -> Result<AwarenessData> {
        let client = self.pool.get().await?;

        // Get recent DMs to this AI
        let dm_rows = client.query(
            "SELECT id, from_ai, content
             FROM messages
             WHERE to_ai = $1
             ORDER BY created DESC
             LIMIT $2",
            &[&ai_id, &(dm_limit as i64)],
        ).await?;

        let dms: Vec<Message> = dm_rows.iter().map(|row| Message {
            id: row.get(0),
            from_ai: row.get(1),
            to_ai: Some(ai_id.to_string()),
            content: row.get(2),
            channel: "dm".to_string(),
            timestamp: chrono::Utc::now(),
            message_type: crate::types::MessageType::Direct,
        }).collect();

        // Get recent broadcasts
        let bc_rows = client.query(
            "SELECT id, from_ai, channel, content
             FROM messages WHERE to_ai IS NULL
             ORDER BY created DESC
             LIMIT $1",
            &[&(broadcast_limit as i64)],
        ).await?;

        let broadcasts: Vec<Message> = bc_rows.iter().map(|row| Message {
            id: row.get(0),
            from_ai: row.get(1),
            to_ai: None,
            content: row.get(3),
            channel: row.get::<_, Option<String>>(2).unwrap_or_else(|| "general".to_string()),
            timestamp: chrono::Utc::now(),
            message_type: crate::types::MessageType::Broadcast,
        }).collect();

        // Get pending votes for this AI
        let vote_rows = client.query(
            "SELECT id, topic, options
             FROM team_votes
             WHERE status = 'open'
               AND id NOT IN (SELECT vote_id FROM vote_responses WHERE voter_ai = $1)
             ORDER BY created_at DESC
             LIMIT 10",
            &[&ai_id],
        ).await?;

        let pending_votes: Vec<Vote> = vote_rows.iter().map(|row| Vote {
            id: row.get(0),
            topic: row.get(1),
            options: row.get::<_, Vec<String>>(2),
            status: VoteStatus::Open,
            created_by: String::new(),
            created_at: chrono::Utc::now(),
            closed_at: None,
            total_voters: 0,
            votes_cast: 0,
        }).collect();

        Ok(AwarenessData {
            dms,
            broadcasts,
            votes: pending_votes,
            locks: vec![],
            detangles: vec![],
        })
    }

    /// Get recent file actions
    pub async fn get_recent_file_actions(&self, limit: i32) -> Result<Vec<(String, String, String)>> {
        let client = self.pool.get().await?;

        let rows = client.query(
            "SELECT ai_id, file_path, action
             FROM ai_file_actions
             ORDER BY created DESC
             LIMIT $1",
            &[&(limit as i64)],
        ).await?;

        Ok(rows.iter().map(|row| (
            row.get::<_, String>(0),
            row.get::<_, String>(1),
            row.get::<_, String>(2),
        )).collect())
    }

    /// Get recent creations
    pub async fn recent_creations(&self, limit: i32) -> Result<Vec<(String, String)>> {
        let client = self.pool.get().await?;

        let rows = client.query(
            "SELECT ai_id, file_path
             FROM ai_file_actions
             WHERE action = 'create'
             ORDER BY created DESC
             LIMIT $1",
            &[&(limit as i64)],
        ).await?;

        Ok(rows.iter().map(|row| (
            row.get::<_, String>(0),
            row.get::<_, String>(1),
        )).collect())
    }

    /// Get team activity summary
    pub async fn get_team_activity(&self, hours: i32) -> Result<Vec<(String, i64)>> {
        let client = self.pool.get().await?;

        let rows = client.query(
            "SELECT ai_id, COUNT(*) as activity_count
             FROM ai_file_actions
             WHERE created_at > NOW() - make_interval(hours => $1::int)
             GROUP BY ai_id
             ORDER BY activity_count DESC",
            &[&hours],
        ).await?;

        Ok(rows.iter().map(|row| (
            row.get::<_, String>(0),
            row.get::<_, i64>(1),
        )).collect())
    }

    /// Get file history - who has touched a specific file and when
    pub async fn get_file_history(&self, file_path: &str, limit: i32) -> Result<Vec<FileHistoryEntry>> {
        let client = self.pool.get().await?;

        // Support partial path matching (e.g., "main.rs" matches "src/main.rs")
        let search_pattern = format!("%{}", file_path);

        let rows = client.query(
            "SELECT ai_id, action_type, file_path, timestamp, file_type
             FROM ai_file_actions
             WHERE file_path LIKE $1
             ORDER BY created DESC
             LIMIT $2",
            &[&search_pattern, &(limit as i64)],
        ).await?;

        Ok(rows.iter().map(|row| FileHistoryEntry {
            ai_id: row.get(0),
            action: row.get(1),
            file_path: row.get(2),
            timestamp: row.get(3),
            file_type: row.try_get(4).ok(),
            file_size: None, // file_size column doesn't exist in actual table
        }).collect())
    }

    /// Get comprehensive team summary - aggregate stats over time period
    pub async fn get_team_summary(&self, hours: i32) -> Result<TeamSummary> {
        let client = self.pool.get().await?;

        // Total actions per AI
        let ai_rows = client.query(
            "SELECT ai_id, COUNT(*) as total,
                    COUNT(DISTINCT file_path) as unique_files,
                    COUNT(*) FILTER (WHERE action_type = 'modified' OR action_type = 'edited') as edits,
                    COUNT(*) FILTER (WHERE action_type = 'created') as creates,
                    COUNT(*) FILTER (WHERE action_type = 'accessed' OR action_type = 'read') as reads
             FROM ai_file_actions
             WHERE timestamp > NOW() - make_interval(hours => $1::int)
             GROUP BY ai_id
             ORDER BY total DESC",
            &[&hours],
        ).await?;

        let ai_stats: Vec<AiActivityStats> = ai_rows.iter().map(|row| AiActivityStats {
            ai_id: row.get(0),
            total_actions: row.get(1),
            unique_files: row.get(2),
            edits: row.get(3),
            creates: row.get(4),
            reads: row.get(5),
        }).collect();

        // Most active files
        let file_rows = client.query(
            "SELECT file_path, COUNT(*) as touches,
                    COUNT(DISTINCT ai_id) as unique_ais,
                    MAX(timestamp) as last_touch
             FROM ai_file_actions
             WHERE timestamp > NOW() - make_interval(hours => $1::int)
             GROUP BY file_path
             ORDER BY touches DESC
             LIMIT 10",
            &[&hours],
        ).await?;

        let hot_files: Vec<HotFile> = file_rows.iter().map(|row| HotFile {
            file_path: row.get(0),
            touch_count: row.get(1),
            unique_ais: row.get(2),
            last_touch: row.get(3),
        }).collect();

        // Overall totals
        let total_row = client.query_one(
            "SELECT COUNT(*) as total_actions,
                    COUNT(DISTINCT ai_id) as active_ais,
                    COUNT(DISTINCT file_path) as files_touched
             FROM ai_file_actions
             WHERE timestamp > NOW() - make_interval(hours => $1::int)",
            &[&hours],
        ).await?;

        Ok(TeamSummary {
            hours,
            total_actions: total_row.get(0),
            active_ais: total_row.get(1),
            files_touched: total_row.get(2),
            ai_stats,
            hot_files,
        })
    }

    /// Get full note with metadata
    pub async fn get_full_note(&self, note_id: &str) -> Result<Option<(String, String, String, bool)>> {
        let client = self.pool.get().await?;

        let row = client.query_opt(
            "SELECT id, ai_id, content, pinned
             FROM notes
             WHERE id = $1",
            &[&note_id],
        ).await?;

        Ok(row.map(|r| (
            r.get::<_, String>(0),
            r.get::<_, String>(1),
            r.get::<_, String>(2),
            r.get::<_, bool>(3),
        )))
    }

    // ===== SYNC SYSTEM =====

    pub async fn sync_start(&self, topic: &str, participants: Vec<String>, rounds: i32) -> Result<i32> {
        let client = self.pool.get().await?;
        // Calculate turn order and total messages
        let mut turn_order = Vec::new();
        for _ in 0..rounds {
            turn_order.extend(participants.iter().cloned());
        }
        let total_messages = turn_order.len() as i32;
        let started_by = participants.first().cloned().unwrap_or_default();
        let expires = chrono::Utc::now() + chrono::Duration::hours(1);

        let row = client.query_one(
            "INSERT INTO sync_sessions (topic, participants, turn_order, started_by, rounds_per_ai, total_messages_expected, state, locked, expires)
             VALUES ($1, $2, $3, $4, $5, $6, 'active', TRUE, $7) RETURNING id",
            &[&topic, &participants, &turn_order, &started_by, &rounds, &total_messages, &expires],
        ).await?;
        Ok(row.get::<_, i32>(0))
    }

    pub async fn sync_message(&self, session_id: i32, ai_id: &str, content: &str) -> Result<String> {
        let client = self.pool.get().await?;
        client.execute(
            "INSERT INTO sync_messages (session_id, ai_id, content) VALUES ($1, $2, $3)",
            &[&session_id, &ai_id, &content],
        ).await?;
        Ok(format!("sync_msg:{}:{}", session_id, ai_id))
    }

    pub async fn sync_complete(&self, session_id: i32) -> Result<()> {
        let client = self.pool.get().await?;
        let now = chrono::Utc::now();
        client.execute(
            "UPDATE sync_sessions SET state = 'completed', locked = FALSE, completed_at = $1 WHERE id = $2",
            &[&now, &session_id],
        ).await?;
        Ok(())
    }

    pub async fn sync_status(&self, session_id: i32) -> Result<String> {
        let client = self.pool.get().await?;
        let row = client.query_opt(
            "SELECT state, current_turn_index, total_messages_expected, rounds_per_ai FROM sync_sessions WHERE id = $1",
            &[&session_id],
        ).await?;
        Ok(row.map(|r| {
            let state: String = r.get(0);
            let current_turn: i32 = r.get(1);
            let total_messages: i32 = r.get(2);
            let rounds: i32 = r.get(3);
            format!("{}:turn_{}/{}:rounds_{}", state, current_turn, total_messages, rounds)
        }).unwrap_or_else(|| "not_found".to_string()))
    }

    // ===== DETANGLE SYSTEM =====

    pub async fn detangle_start(&self, ai1: &str, ai2: &str, topic: &str, turns: i32) -> Result<i32> {
        let client = self.pool.get().await?;
        let row = client.query_one(
            "INSERT INTO detangle_sessions (ai1, ai2, topic, max_turns, status)
             VALUES ($1, $2, $3, $4, 'active') RETURNING id",
            &[&ai1, &ai2, &topic, &turns],
        ).await?;
        Ok(row.get::<_, i32>(0))
    }

    pub async fn detangle_message(&self, session_id: i32, ai_id: &str, content: &str) -> Result<()> {
        let client = self.pool.get().await?;
        client.execute(
            "INSERT INTO detangle_messages (session_id, ai_id, content) VALUES ($1, $2, $3)",
            &[&session_id, &ai_id, &content],
        ).await?;
        Ok(())
    }

    pub async fn detangle_conclude(&self, session_id: i32, reason: &str) -> Result<()> {
        let client = self.pool.get().await?;
        client.execute(
            "UPDATE detangle_sessions SET status = 'concluded', conclusion = $2 WHERE id = $1",
            &[&session_id, &reason],
        ).await?;
        Ok(())
    }

    pub async fn detangle_status(&self, session_id: i32) -> Result<String> {
        let client = self.pool.get().await?;
        let row = client.query_opt(
            "SELECT status FROM detangle_sessions WHERE id = $1",
            &[&session_id],
        ).await?;
        Ok(row.map(|r| r.get::<_, String>(0)).unwrap_or_else(|| "not_found".to_string()))
    }

    // ===== EVOLVE SYSTEM =====

    pub async fn evolve_start(&self, goal: &str) -> Result<i32> {
        let client = self.pool.get().await?;
        let row = client.query_one(
            "INSERT INTO evolve_sessions (goal, status) VALUES ($1, 'active') RETURNING id",
            &[&goal],
        ).await?;
        Ok(row.get::<_, i32>(0))
    }

    pub async fn evolve_attempt(&self, session_id: i32, ai_id: &str, solution: &str) -> Result<i32> {
        let client = self.pool.get().await?;
        let row = client.query_one(
            "INSERT INTO evolve_attempts (session_id, ai_id, solution) VALUES ($1, $2, $3) RETURNING id",
            &[&session_id, &ai_id, &solution],
        ).await?;
        Ok(row.get::<_, i32>(0))
    }

    pub async fn evolve_list_attempts(&self, session_id: i32) -> Result<Vec<(i32, String, String)>> {
        let client = self.pool.get().await?;
        let rows = client.query(
            "SELECT id, ai_id, solution FROM evolve_attempts WHERE session_id = $1 ORDER BY created DESC",
            &[&session_id],
        ).await?;
        Ok(rows.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
    }

    // ===== VOTING SYSTEM - Full Democratic Consensus =====
    // Auto-closes at: 100% participation OR (timeout + 75% threshold)

    /// Create a new vote for team consensus
    pub async fn create_vote(&self, topic: &str, options: Vec<String>, created_by: &str, total_voters: i32) -> Result<Vote> {
        let client = self.pool.get().await?;
        let row = client.query_one(
            "INSERT INTO team_votes (topic, options, created_by, total_voters, status)
             VALUES ($1, $2, $3, $4, 'open')
             RETURNING id, topic, options, status, created_by, created_at, closed_at, total_voters, votes_cast",
            &[&topic, &options, &created_by, &total_voters],
        ).await.context("Failed to create vote")?;

        let vote = Vote {
            id: row.get(0),
            topic: row.get(1),
            options: row.get(2),
            status: VoteStatus::Open,
            created_by: row.get(4),
            created_at: row.get(5),
            closed_at: row.get(6),
            total_voters: row.get(7),
            votes_cast: row.get(8),
        };

        info!("Created vote #{}: {} by {}", vote.id, topic, created_by);
        Ok(vote)
    }

    /// Cast a vote (one vote per AI per topic)
    pub async fn cast_vote(&self, vote_id: i32, voter_ai: &str, choice: &str) -> Result<bool> {
        let client = self.pool.get().await?;

        // Check vote is still open
        let vote_row = client.query_opt(
            "SELECT status, options FROM team_votes WHERE id = $1",
            &[&vote_id],
        ).await?;

        let (status, options): (String, Vec<String>) = match vote_row {
            Some(row) => (row.get(0), row.get(1)),
            None => return Ok(false),
        };

        if status != "open" {
            debug!("Vote {} is closed, rejecting ballot from {}", vote_id, voter_ai);
            return Ok(false);
        }

        // Validate choice is in options
        if !options.contains(&choice.to_string()) {
            debug!("Invalid choice '{}' for vote {}", choice, vote_id);
            return Ok(false);
        }

        // Insert or update ballot
        let result = client.execute(
            "INSERT INTO vote_responses (vote_id, voter_ai, choice)
             VALUES ($1, $2, $3)
             ON CONFLICT (vote_id, voter_ai) DO UPDATE SET choice = $3, voted_at = NOW()",
            &[&vote_id, &voter_ai, &choice],
        ).await?;

        // Update votes_cast count
        client.execute(
            "UPDATE team_votes SET votes_cast = (
                SELECT COUNT(*) FROM vote_responses WHERE vote_id = $1
             ) WHERE id = $1",
            &[&vote_id],
        ).await?;

        debug!("{} voted '{}' on vote #{}", voter_ai, choice, vote_id);

        // Check for auto-close conditions
        self.check_auto_close(vote_id).await?;

        Ok(result > 0)
    }

    /// Check if vote should auto-close (100% or timeout+threshold)
    async fn check_auto_close(&self, vote_id: i32) -> Result<bool> {
        let client = self.pool.get().await?;

        let row = client.query_opt(
            "SELECT total_voters, votes_cast, created_at, timeout_minutes, threshold_pct, status
             FROM team_votes WHERE id = $1",
            &[&vote_id],
        ).await?;

        let (total, cast, created_at, timeout_mins, threshold, status): (i32, i32, chrono::DateTime<Utc>, i32, i32, String) = match row {
            Some(r) => (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5)),
            None => return Ok(false),
        };

        if status != "open" {
            return Ok(false);
        }

        let should_close = if total > 0 && cast >= total {
            // 100% participation - close immediately
            info!("Vote #{} reached 100% participation ({}/{}), closing", vote_id, cast, total);
            true
        } else {
            // Check timeout + threshold
            let elapsed = Utc::now() - created_at;
            let timeout_reached = elapsed.num_minutes() >= timeout_mins as i64;
            let threshold_met = total > 0 && (cast as f64 / total as f64 * 100.0) >= threshold as f64;

            if timeout_reached && threshold_met {
                info!("Vote #{} timeout + {}% threshold met ({}/{}), closing", vote_id, threshold, cast, total);
                true
            } else {
                false
            }
        };

        if should_close {
            client.execute(
                "UPDATE team_votes SET status = 'closed', closed_at = NOW() WHERE id = $1",
                &[&vote_id],
            ).await?;
        }

        Ok(should_close)
    }

    /// Get all open votes
    pub async fn get_open_votes(&self) -> Result<Vec<Vote>> {
        let client = self.pool.get().await?;

        // First, check for any that should auto-close due to timeout
        self.close_expired_votes().await?;

        let rows = client.query(
            "SELECT id, topic, options, status, created_by, created_at, closed_at, total_voters, votes_cast
             FROM team_votes
             WHERE status = 'open'
             ORDER BY created_at DESC",
            &[],
        ).await?;

        let votes = rows.iter().map(|row| Vote {
            id: row.get(0),
            topic: row.get(1),
            options: row.get(2),
            status: VoteStatus::Open,
            created_by: row.get(4),
            created_at: row.get(5),
            closed_at: row.get(6),
            total_voters: row.get(7),
            votes_cast: row.get(8),
        }).collect();

        Ok(votes)
    }

    /// Get votes pending for a specific AI (open votes they haven't voted on)
    pub async fn get_pending_votes_for_ai(&self, ai_id: &str) -> Result<Vec<Vote>> {
        let client = self.pool.get().await?;

        // First close any expired votes
        self.close_expired_votes().await?;

        let rows = client.query(
            "SELECT v.id, v.topic, v.options, v.status, v.created_by, v.created_at, v.closed_at, v.total_voters, v.votes_cast
             FROM team_votes v
             WHERE v.status = 'open'
               AND NOT EXISTS (SELECT 1 FROM vote_responses r WHERE r.vote_id = v.id AND r.voter_ai = $1)
             ORDER BY v.created_at ASC",
            &[&ai_id],
        ).await?;

        let votes = rows.iter().map(|row| Vote {
            id: row.get(0),
            topic: row.get(1),
            options: row.get(2),
            status: VoteStatus::Open,
            created_by: row.get(4),
            created_at: row.get(5),
            closed_at: row.get(6),
            total_voters: row.get(7),
            votes_cast: row.get(8),
        }).collect();

        Ok(votes)
    }

    /// Close expired votes that meet timeout + threshold
    pub async fn close_expired_votes(&self) -> Result<i32> {
        let client = self.pool.get().await?;

        // Get all open votes past timeout with threshold met
        let rows = client.query(
            "SELECT id FROM team_votes
             WHERE status = 'open'
               AND created_at + (timeout_minutes || ' minutes')::interval < NOW()
               AND total_voters > 0
               AND (votes_cast::float / total_voters::float * 100) >= threshold_pct",
            &[],
        ).await?;

        let mut closed = 0;
        for row in rows {
            let vote_id: i32 = row.get(0);
            client.execute(
                "UPDATE team_votes SET status = 'closed', closed_at = NOW() WHERE id = $1",
                &[&vote_id],
            ).await?;
            closed += 1;
            info!("Auto-closed expired vote #{}", vote_id);
        }

        Ok(closed)
    }

    /// Close a specific vote by ID
    pub async fn close_vote(&self, vote_id: i32) -> Result<bool> {
        let client = self.pool.get().await?;

        let result = client.execute(
            "UPDATE team_votes SET status = 'closed', closed_at = NOW()
             WHERE id = $1 AND status = 'open'",
            &[&vote_id],
        ).await?;

        Ok(result > 0)
    }

    /// Close stale votes older than X minutes
    pub async fn close_stale_votes(&self, max_age_minutes: i32) -> Result<i32> {
        let client = self.pool.get().await?;

        let result = client.execute(
            "UPDATE team_votes SET status = 'closed', closed_at = NOW()
             WHERE status = 'open'
               AND created_at < NOW() - ($1 || ' minutes')::interval",
            &[&max_age_minutes],
        ).await?;

        Ok(result as i32)
    }

    /// Get full vote results with counts and winner
    pub async fn get_vote_results(&self, vote_id: i32) -> Result<Option<VoteResults>> {
        let client = self.pool.get().await?;

        // Get vote
        let vote_row = client.query_opt(
            "SELECT id, topic, options, status, created_by, created_at, closed_at, total_voters, votes_cast
             FROM team_votes WHERE id = $1",
            &[&vote_id],
        ).await?;

        let vote = match vote_row {
            Some(row) => {
                let status_str: String = row.get(3);
                Vote {
                    id: row.get(0),
                    topic: row.get(1),
                    options: row.get(2),
                    status: if status_str == "open" { VoteStatus::Open } else { VoteStatus::Closed },
                    created_by: row.get(4),
                    created_at: row.get(5),
                    closed_at: row.get(6),
                    total_voters: row.get(7),
                    votes_cast: row.get(8),
                }
            }
            None => return Ok(None),
        };

        // Get vote counts
        let count_rows = client.query(
            "SELECT choice, COUNT(*) as cnt FROM vote_responses WHERE vote_id = $1 GROUP BY choice",
            &[&vote_id],
        ).await?;

        let mut counts: HashMap<String, i32> = HashMap::new();
        for row in &count_rows {
            let choice: String = row.get(0);
            let cnt: i64 = row.get(1);
            counts.insert(choice, cnt as i32);
        }

        // Get voters by choice
        let voter_rows = client.query(
            "SELECT choice, voter_ai FROM vote_responses WHERE vote_id = $1 ORDER BY choice, voted_at",
            &[&vote_id],
        ).await?;

        let mut voters_by_choice: HashMap<String, Vec<String>> = HashMap::new();
        for row in &voter_rows {
            let choice: String = row.get(0);
            let voter: String = row.get(1);
            voters_by_choice.entry(choice).or_default().push(voter);
        }

        // Determine winner
        let (winner, winner_count) = counts.iter()
            .max_by_key(|(_, &count)| count)
            .map(|(choice, &count)| (Some(choice.clone()), count))
            .unwrap_or((None, 0));

        Ok(Some(VoteResults {
            vote,
            counts,
            voters_by_choice,
            winner,
            winner_count,
        }))
    }

    /// List recent votes (for status display)
    pub async fn list_votes(&self, limit: i32) -> Result<Vec<Vote>> {
        let client = self.pool.get().await?;

        let rows = client.query(
            "SELECT id, topic, options, status, created_by, created_at, closed_at, total_voters, votes_cast
             FROM team_votes
             ORDER BY created_at DESC
             LIMIT $1",
            &[&(limit as i64)],
        ).await?;

        let votes = rows.iter().map(|row| {
            let status_str: String = row.get(3);
            Vote {
                id: row.get(0),
                topic: row.get(1),
                options: row.get(2),
                status: if status_str == "open" { VoteStatus::Open } else { VoteStatus::Closed },
                created_by: row.get(4),
                created_at: row.get(5),
                closed_at: row.get(6),
                total_voters: row.get(7),
                votes_cast: row.get(8),
            }
        }).collect();

        Ok(votes)
    }

    /// Format vote awareness for hook injection
    pub fn format_vote_awareness(votes: &[Vote], ai_id: &str, pending_for_ai: &[Vote]) -> String {
        if votes.is_empty() && pending_for_ai.is_empty() {
            return String::new();
        }

        let mut output = String::new();

        // Pending votes for this AI (most important)
        if !pending_for_ai.is_empty() {
            output.push_str("PENDING VOTES (need your input):\n");
            for vote in pending_for_ai {
                let options_str = vote.options.join(", ");
                let pct = vote.completion_pct();
                output.push_str(&format!(
                    "  [{}] {} ({:.0}% voted) - Options: {}\n",
                    vote.id, vote.topic, pct, options_str
                ));
            }
            output.push('\n');
        }

        // Active votes overview
        let open_votes: Vec<_> = votes.iter().filter(|v| v.status == VoteStatus::Open).collect();
        if !open_votes.is_empty() {
            output.push_str("ACTIVE VOTES:\n");
            for vote in open_votes {
                let pct = vote.completion_pct();
                output.push_str(&format!(
                    "  [{}] {} by {} ({:.0}% - {}/{} voted)\n",
                    vote.id, vote.topic, vote.created_by, pct, vote.votes_cast, vote.total_voters
                ));
            }
        }

        output
    }

    // ===== INFRASTRUCTURE HELPERS =====

    pub async fn list_teambooks(&self) -> Result<Vec<String>> {
        Ok(vec!["town_hall".to_string()])
    }

    pub async fn get_computer_id(&self) -> Result<String> {
        use std::env;
        Ok(env::var("COMPUTERNAME")
            .or_else(|_| env::var("HOSTNAME"))
            .unwrap_or_else(|_| "unknown".to_string()))
    }

    // ===== STIGMERGY - Digital Pheromones for O(1) Coordination =====

    /// Query pheromones at a location
    /// Returns: Vec<(pheromone_type, intensity, agent_id, age_seconds)>
    pub async fn query_pheromones(&self, location: &str, pheromone_type: Option<&str>) -> Result<Vec<(String, f64, String, i64)>> {
        let client = self.pool.get().await?;

        // Ensure pheromones table exists
        client.execute(
            "CREATE TABLE IF NOT EXISTS pheromones (
                id SERIAL PRIMARY KEY,
                location VARCHAR(255) NOT NULL,
                pheromone_type VARCHAR(50) NOT NULL,
                intensity FLOAT NOT NULL,
                decay_rate FLOAT NOT NULL,
                agent_id VARCHAR(100) NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                expires_at TIMESTAMPTZ NOT NULL
            )",
            &[],
        ).await.ok();

        let rows = if let Some(ptype) = pheromone_type {
            client.query(
                "SELECT pheromone_type, intensity, agent_id,
                        EXTRACT(EPOCH FROM (NOW() - created_at))::bigint as age_secs
                 FROM pheromones
                 WHERE location = $1 AND pheromone_type = $2 AND expires_at > NOW()
                 ORDER BY created_at DESC",
                &[&location, &ptype],
            ).await?
        } else {
            client.query(
                "SELECT pheromone_type, intensity, agent_id,
                        EXTRACT(EPOCH FROM (NOW() - created_at))::bigint as age_secs
                 FROM pheromones
                 WHERE location = $1 AND expires_at > NOW()
                 ORDER BY created_at DESC",
                &[&location],
            ).await?
        };

        let pheromones = rows.iter().map(|row| {
            let ptype: String = row.get(0);
            let intensity: f64 = row.get(1);
            let agent_id: String = row.get(2);
            let age_secs: i64 = row.get(3);
            (ptype, intensity, agent_id, age_secs)
        }).collect();

        Ok(pheromones)
    }

    /// Deposit a pheromone at a location
    pub async fn deposit_pheromone(&self, ai_id: &str, location: &str, pheromone_type: &str, intensity: f64, decay_rate: f64) -> Result<()> {
        let client = self.pool.get().await?;

        // Ensure pheromones table exists
        client.execute(
            "CREATE TABLE IF NOT EXISTS pheromones (
                id SERIAL PRIMARY KEY,
                location VARCHAR(255) NOT NULL,
                pheromone_type VARCHAR(50) NOT NULL,
                intensity FLOAT NOT NULL,
                decay_rate FLOAT NOT NULL,
                agent_id VARCHAR(100) NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                expires_at TIMESTAMPTZ NOT NULL
            )",
            &[],
        ).await.ok();

        // Calculate expiration based on decay rate
        // I(t) = I₀ * (1 - r)^t, solve for t when I(t) = 0.01
        let expiry_seconds = if decay_rate >= 1.0 {
            0.0
        } else if decay_rate <= 0.0 {
            300.0 // 5 minutes default for no-decay
        } else {
            (0.01_f64 / intensity).ln() / (1.0 - decay_rate).ln()
        };

        let expiry_seconds = expiry_seconds.max(0.0).min(3600.0) as i32; // Max 1 hour

        client.execute(
            "INSERT INTO pheromones (location, pheromone_type, intensity, decay_rate, agent_id, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, NOW(), NOW() + make_interval(secs => $6))",
            &[&location, &pheromone_type, &intensity, &decay_rate, &ai_id, &expiry_seconds],
        ).await.context("Failed to deposit pheromone")?;

        debug!("Deposited {} pheromone at {} by {}", pheromone_type, location, ai_id);
        Ok(())
    }

    /// Clear pheromones at a location (optional: by type and/or agent)
    pub async fn clear_pheromone(&self, location: &str, pheromone_type: Option<&str>, agent_id: Option<&str>) -> Result<i32> {
        let client = self.pool.get().await?;

        let rows_affected = match (pheromone_type, agent_id) {
            (Some(ptype), Some(aid)) => {
                client.execute(
                    "DELETE FROM pheromones WHERE location = $1 AND pheromone_type = $2 AND agent_id = $3",
                    &[&location, &ptype, &aid],
                ).await?
            }
            (Some(ptype), None) => {
                client.execute(
                    "DELETE FROM pheromones WHERE location = $1 AND pheromone_type = $2",
                    &[&location, &ptype],
                ).await?
            }
            (None, Some(aid)) => {
                client.execute(
                    "DELETE FROM pheromones WHERE location = $1 AND agent_id = $2",
                    &[&location, &aid],
                ).await?
            }
            (None, None) => {
                client.execute(
                    "DELETE FROM pheromones WHERE location = $1",
                    &[&location],
                ).await?
            }
        };

        Ok(rows_affected as i32)
    }

    /// Get total pheromone intensity at a location
    pub async fn get_pheromone_intensity(&self, location: &str, pheromone_type: Option<&str>) -> Result<f64> {
        let pheromones = self.query_pheromones(location, pheromone_type).await?;
        let total: f64 = pheromones.iter().map(|(_, intensity, _, _)| intensity).sum();
        Ok(total)
    }

    // ===== DIRECTORY TRACKING - Track AI's working patterns =====

    /// Track a directory access
    pub async fn track_directory(&self, ai_id: &str, directory: &str, access_type: &str) -> Result<()> {
        let client = self.pool.get().await?;

        // Ensure directory_access table exists
        client.execute(
            "CREATE TABLE IF NOT EXISTS directory_access (
                id SERIAL PRIMARY KEY,
                ai_id VARCHAR(100) NOT NULL,
                directory TEXT NOT NULL,
                access_type VARCHAR(50) NOT NULL,
                accessed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
            &[],
        ).await.ok();

        // Create index for performance
        client.execute(
            "CREATE INDEX IF NOT EXISTS idx_directory_access_ai ON directory_access(ai_id, accessed_at DESC)",
            &[],
        ).await.ok();

        client.execute(
            "INSERT INTO directory_access (ai_id, directory, access_type, accessed_at)
             VALUES ($1, $2, $3, NOW())",
            &[&ai_id, &directory, &access_type],
        ).await.context("Failed to track directory")?;

        debug!("Tracked {} access to {} by {}", access_type, directory, ai_id);
        Ok(())
    }

    /// Get recently accessed directories for an AI
    /// Returns: Vec<(directory, access_type, age_str)>
    pub async fn get_recent_directories(&self, ai_id: &str, limit: i32) -> Result<Vec<(String, String, String)>> {
        let client = self.pool.get().await?;

        // Ensure table exists
        client.execute(
            "CREATE TABLE IF NOT EXISTS directory_access (
                id SERIAL PRIMARY KEY,
                ai_id VARCHAR(100) NOT NULL,
                directory TEXT NOT NULL,
                access_type VARCHAR(50) NOT NULL,
                accessed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
            &[],
        ).await.ok();

        let rows = client.query(
            "SELECT DISTINCT ON (directory) directory, access_type,
                    CASE
                        WHEN EXTRACT(EPOCH FROM (NOW() - accessed_at)) < 60 THEN 'just now'
                        WHEN EXTRACT(EPOCH FROM (NOW() - accessed_at)) < 3600 THEN
                            EXTRACT(MINUTE FROM (NOW() - accessed_at))::int || 'm ago'
                        WHEN EXTRACT(EPOCH FROM (NOW() - accessed_at)) < 86400 THEN
                            EXTRACT(HOUR FROM (NOW() - accessed_at))::int || 'h ago'
                        ELSE EXTRACT(DAY FROM (NOW() - accessed_at))::int || 'd ago'
                    END as age_str
             FROM directory_access
             WHERE ai_id = $1
             ORDER BY directory, accessed_at DESC
             LIMIT $2",
            &[&ai_id, &(limit as i64)],
        ).await?;

        let dirs = rows.iter().map(|row| {
            let dir: String = row.get(0);
            let access_type: String = row.get(1);
            let age_str: String = row.get(2);
            (dir, access_type, age_str)
        }).collect();

        Ok(dirs)
    }

    // ===== STANDBY MODE - Event-Driven Wake System =====

    /// Check for wake events (DMs, mentions, tasks, help requests)
    /// This is a query-based implementation (DEPRECATED - use event-driven V2) for PostgreSQL
    /// Returns: Option<(wake_reason, event_type, from_ai, content)>
    pub async fn check_wake_events(&self, ai_id: &str, since_secs: i64) -> Result<Option<(String, String, String, String)>> {
        let client = self.pool.get().await?;

        // Check for direct messages to this AI
        let dm_row = client.query_opt(
            "SELECT from_ai, content
             FROM messages
             WHERE to_ai = $1 AND channel = 'direct'
               AND created_at > NOW() - make_interval(secs => $2)
             ORDER BY created_at DESC LIMIT 1",
            &[&ai_id, &since_secs],
        ).await?;

        if let Some(row) = dm_row {
            let from: String = row.get(0);
            let content: String = row.get(1);
            return Ok(Some(("direct_message".to_string(), "dm".to_string(), from, content)));
        }

        // Check for broadcasts mentioning this AI or containing help keywords
        let ai_name = ai_id.split('-').next().unwrap_or(ai_id);
        let broadcast_row = client.query_opt(
            "SELECT from_ai, content
             FROM messages
             WHERE channel = 'general'
               AND created_at > NOW() - make_interval(secs => $1)
               AND (
                   LOWER(content) LIKE LOWER($2) OR
                   LOWER(content) LIKE '%@' || LOWER($3) || '%' OR
                   LOWER(content) LIKE '%help%' OR
                   LOWER(content) LIKE '%anyone%' OR
                   LOWER(content) LIKE '%urgent%' OR
                   LOWER(content) LIKE '%critical%' OR
                   LOWER(content) LIKE '%review%' OR
                   LOWER(content) LIKE '%thoughts%'
               )
             ORDER BY created_at DESC LIMIT 1",
            &[&since_secs, &format!("%{}%", ai_name), &ai_name],
        ).await?;

        if let Some(row) = broadcast_row {
            let from: String = row.get(0);
            let content: String = row.get(1);
            let lower_content = content.to_lowercase();

            let wake_reason = if lower_content.contains(ai_name) || lower_content.contains(&format!("@{}", ai_name)) {
                "name_mentioned"
            } else if lower_content.contains("help") || lower_content.contains("anyone") {
                "help_requested"
            } else if lower_content.contains("urgent") || lower_content.contains("critical") {
                "priority_alert"
            } else {
                "coordination_request"
            };

            return Ok(Some((wake_reason.to_string(), "broadcast".to_string(), from, content)));
        }

        // Check for tasks assigned to this AI
        let task_row = client.query_opt(
            "SELECT created_by, description
             FROM tasks
             WHERE assigned_to = $1 AND status = 'pending'
               AND created_at > NOW() - make_interval(secs => $2)
             ORDER BY created_at DESC LIMIT 1",
            &[&ai_id, &since_secs],
        ).await?;

        if let Some(row) = task_row {
            let from: String = row.get(0);
            let desc: String = row.get(1);
            return Ok(Some(("task_assigned".to_string(), "task".to_string(), from, desc)));
        }

        Ok(None)
    }

    // ===== FEATURE MANAGEMENT - Project Subcomponents =====

    /// Create a feature within a project
    pub async fn create_feature(&self, project_id: i32, name: &str, overview: &str, directory: Option<&str>, created_by: &str) -> Result<i32> {
        let client = self.pool.get().await?;

        // Ensure features table exists
        client.execute(
            "CREATE TABLE IF NOT EXISTS features (
                id SERIAL PRIMARY KEY,
                project_id INT NOT NULL REFERENCES projects(id),
                name VARCHAR(255) NOT NULL,
                overview TEXT,
                directory TEXT,
                created_by VARCHAR(100) NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                UNIQUE(project_id, name)
            )",
            &[],
        ).await.ok();

        let row = client.query_one(
            "INSERT INTO features (project_id, name, overview, directory, created_by)
             VALUES ($1, $2, $3, $4, $5) RETURNING id",
            &[&project_id, &name, &overview, &directory.unwrap_or(""), &created_by],
        ).await.context("Failed to create feature")?;

        let id: i32 = row.get(0);
        debug!("Created feature: {} (id={}) in project {}", name, id, project_id);
        Ok(id)
    }

    /// List features in a project
    pub async fn list_features(&self, project_id: i32) -> Result<Vec<(i32, String, String, String)>> {
        let client = self.pool.get().await?;

        let rows = client.query(
            "SELECT id, name, COALESCE(overview, ''), COALESCE(directory, '')
             FROM features WHERE project_id = $1 ORDER BY name",
            &[&project_id],
        ).await?;

        let features = rows.iter().map(|row| {
            let id: i32 = row.get(0);
            let name: String = row.get(1);
            let overview: String = row.get(2);
            let dir: String = row.get(3);
            (id, name, overview, dir)
        }).collect();

        Ok(features)
    }

    /// Get a feature by ID
    pub async fn get_feature(&self, feature_id: i32) -> Result<Option<(i32, i32, String, String, String, String)>> {
        let client = self.pool.get().await?;

        let row = client.query_opt(
            "SELECT id, project_id, name, COALESCE(overview, ''), COALESCE(directory, ''), created_by
             FROM features WHERE id = $1",
            &[&feature_id],
        ).await?;

        if let Some(row) = row {
            Ok(Some((
                row.get(0),
                row.get(1),
                row.get(2),
                row.get(3),
                row.get(4),
                row.get(5),
            )))
        } else {
            Ok(None)
        }
    }

    /// Update a feature
    pub async fn update_feature(&self, feature_id: i32, name: Option<&str>, overview: Option<&str>, directory: Option<&str>) -> Result<bool> {
        let client = self.pool.get().await?;

        // Build query based on what's provided
        let result = match (name, overview, directory) {
            (Some(n), Some(o), Some(d)) => {
                client.execute(
                    "UPDATE features SET name = $1, overview = $2, directory = $3, updated_at = NOW() WHERE id = $4",
                    &[&n, &o, &d, &feature_id],
                ).await?
            }
            (Some(n), Some(o), None) => {
                client.execute(
                    "UPDATE features SET name = $1, overview = $2, updated_at = NOW() WHERE id = $3",
                    &[&n, &o, &feature_id],
                ).await?
            }
            (Some(n), None, Some(d)) => {
                client.execute(
                    "UPDATE features SET name = $1, directory = $2, updated_at = NOW() WHERE id = $3",
                    &[&n, &d, &feature_id],
                ).await?
            }
            (None, Some(o), Some(d)) => {
                client.execute(
                    "UPDATE features SET overview = $1, directory = $2, updated_at = NOW() WHERE id = $3",
                    &[&o, &d, &feature_id],
                ).await?
            }
            (Some(n), None, None) => {
                client.execute(
                    "UPDATE features SET name = $1, updated_at = NOW() WHERE id = $2",
                    &[&n, &feature_id],
                ).await?
            }
            (None, Some(o), None) => {
                client.execute(
                    "UPDATE features SET overview = $1, updated_at = NOW() WHERE id = $2",
                    &[&o, &feature_id],
                ).await?
            }
            (None, None, Some(d)) => {
                client.execute(
                    "UPDATE features SET directory = $1, updated_at = NOW() WHERE id = $2",
                    &[&d, &feature_id],
                ).await?
            }
            (None, None, None) => return Ok(false),
        };

        Ok(result > 0)
    }

    // ===== TASK EXTENSIONS =====

    /// Delete a task
    pub async fn delete_task(&self, task_id: i32, ai_id: &str) -> Result<bool> {
        let client = self.pool.get().await?;

        // Only allow deletion by creator or assignee
        let result = client.execute(
            "DELETE FROM tasks WHERE id = $1 AND (created_by = $2 OR assigned_to = $2)",
            &[&task_id, &ai_id],
        ).await?;

        Ok(result > 0)
    }

    /// Smart task search by keyword
    pub async fn find_tasks_smart(&self, query: &str, limit: i32) -> Result<Vec<(i32, String, String, String, String)>> {
        let client = self.pool.get().await?;

        let search_pattern = format!("%{}%", query.to_lowercase());

        let rows = client.query(
            "SELECT id, description, status, COALESCE(assigned_to, ''), created_by
             FROM tasks
             WHERE LOWER(description) LIKE $1
                OR LOWER(status) LIKE $1
                OR LOWER(COALESCE(assigned_to, '')) LIKE $1
             ORDER BY
                CASE WHEN status = 'in_progress' THEN 0
                     WHEN status = 'pending' THEN 1
                     WHEN status = 'blocked' THEN 2
                     ELSE 3 END,
                created_at DESC
             LIMIT $2",
            &[&search_pattern, &(limit as i64)],
        ).await?;

        let tasks = rows.iter().map(|row| {
            (
                row.get::<_, i32>(0),
                row.get::<_, String>(1),
                row.get::<_, String>(2),
                row.get::<_, String>(3),
                row.get::<_, String>(4),
            )
        }).collect();

        Ok(tasks)
    }

    /// Get active tasks for current session
    pub async fn get_session_tasks(&self, ai_id: &str) -> Result<Vec<(i32, String, String)>> {
        let client = self.pool.get().await?;

        let rows = client.query(
            "SELECT id, description, status
             FROM tasks
             WHERE (assigned_to = $1 OR created_by = $1)
               AND status IN ('pending', 'in_progress', 'blocked')
             ORDER BY
                CASE WHEN status = 'in_progress' THEN 0
                     WHEN status = 'blocked' THEN 1
                     ELSE 2 END,
                created_at DESC
             LIMIT 20",
            &[&ai_id],
        ).await?;

        let tasks = rows.iter().map(|row| {
            (
                row.get::<_, i32>(0),
                row.get::<_, String>(1),
                row.get::<_, String>(2),
            )
        }).collect();

        Ok(tasks)
    }

    // ============================================================================
    // SOFT DELETE - Projects and Features with 24h recovery
    // ============================================================================

    /// Soft-delete a project (moves to trash, recoverable for 24h)
    pub async fn soft_delete_project(&self, project_id: i32, deleted_by: &str) -> Result<bool> {
        let client = self.pool.get().await?;

        // Create trash table if not exists
        client.execute(
            "CREATE TABLE IF NOT EXISTS project_trash (
                id SERIAL PRIMARY KEY,
                original_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                goal TEXT NOT NULL,
                deleted_by TEXT NOT NULL,
                deleted_at TIMESTAMPTZ DEFAULT NOW(),
                expires_at TIMESTAMPTZ DEFAULT NOW() + INTERVAL '24 hours'
            )",
            &[],
        ).await?;

        // Move project to trash
        let result = client.execute(
            "INSERT INTO project_trash (original_id, name, goal, deleted_by)
             SELECT id, name, goal, $2
             FROM projects WHERE id = $1",
            &[&project_id, &deleted_by],
        ).await?;

        if result == 0 {
            return Ok(false);
        }

        // Delete from projects
        client.execute("DELETE FROM projects WHERE id = $1", &[&project_id]).await?;

        Ok(true)
    }

    /// Restore a soft-deleted project from trash
    pub async fn restore_project(&self, project_id: i32) -> Result<bool> {
        let client = self.pool.get().await?;

        // Find in trash (not expired)
        let row = client.query_opt(
            "SELECT original_id, name, goal FROM project_trash
             WHERE original_id = $1 AND expires_at > NOW()
             ORDER BY deleted_at DESC LIMIT 1",
            &[&project_id],
        ).await?;

        let row = match row {
            Some(r) => r,
            None => return Ok(false),
        };

        let name: String = row.get(1);
        let goal: String = row.get(2);

        // Restore to projects
        client.execute(
            "INSERT INTO projects (id, name, goal) VALUES ($1, $2, $3)
             ON CONFLICT (id) DO NOTHING",
            &[&project_id, &name, &goal],
        ).await?;

        // Remove from trash
        client.execute(
            "DELETE FROM project_trash WHERE original_id = $1",
            &[&project_id],
        ).await?;

        Ok(true)
    }

    /// Soft-delete a feature (moves to trash, recoverable for 24h)
    pub async fn soft_delete_feature(&self, feature_id: i32, deleted_by: &str) -> Result<bool> {
        let client = self.pool.get().await?;

        // Create trash table if not exists
        client.execute(
            "CREATE TABLE IF NOT EXISTS feature_trash (
                id SERIAL PRIMARY KEY,
                original_id INTEGER NOT NULL,
                project_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                overview TEXT NOT NULL,
                directory TEXT,
                deleted_by TEXT NOT NULL,
                deleted_at TIMESTAMPTZ DEFAULT NOW(),
                expires_at TIMESTAMPTZ DEFAULT NOW() + INTERVAL '24 hours'
            )",
            &[],
        ).await?;

        // Move feature to trash
        let result = client.execute(
            "INSERT INTO feature_trash (original_id, project_id, name, overview, directory, deleted_by)
             SELECT id, project_id, name, overview, directory, $2
             FROM features WHERE id = $1",
            &[&feature_id, &deleted_by],
        ).await?;

        if result == 0 {
            return Ok(false);
        }

        // Delete from features
        client.execute("DELETE FROM features WHERE id = $1", &[&feature_id]).await?;

        Ok(true)
    }

    /// Restore a soft-deleted feature from trash
    pub async fn restore_feature(&self, feature_id: i32) -> Result<bool> {
        let client = self.pool.get().await?;

        // Find in trash (not expired)
        let row = client.query_opt(
            "SELECT original_id, project_id, name, overview, directory FROM feature_trash
             WHERE original_id = $1 AND expires_at > NOW()
             ORDER BY deleted_at DESC LIMIT 1",
            &[&feature_id],
        ).await?;

        let row = match row {
            Some(r) => r,
            None => return Ok(false),
        };

        let project_id: i32 = row.get(1);
        let name: String = row.get(2);
        let overview: String = row.get(3);
        let directory: Option<String> = row.get(4);

        // Restore to features
        client.execute(
            "INSERT INTO features (id, project_id, name, overview, directory) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (id) DO NOTHING",
            &[&feature_id, &project_id, &name, &overview, &directory],
        ).await?;

        // Remove from trash
        client.execute(
            "DELETE FROM feature_trash WHERE original_id = $1",
            &[&feature_id],
        ).await?;

        Ok(true)
    }

}
