//! SYNC - Forced Multi-AI Coordination
//!
//! Locks 2-10 AIs into structured turn-based coordination until alignment is reached.
//! Pattern: A → B → C → A → B → C → A → B → C (3 rounds, all participants)
//!
//! Key Features:
//! - Multi-AI support (2-10 participants)
//! - Forced lock-in (participants can't do other work)
//! - Turn order enforcement
//! - Full context injection on every wake

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc, Duration};
use serde::{Deserialize, Serialize};

/// Session state
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SyncState {
    Active,     // Session in progress
    Completed,  // Normally ended
    Expired,    // Timed out
    Cancelled,  // Force-closed
}

impl SyncState {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "active" => Some(SyncState::Active),
            "completed" => Some(SyncState::Completed),
            "expired" => Some(SyncState::Expired),
            "cancelled" => Some(SyncState::Cancelled),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SyncState::Active => "active",
            SyncState::Completed => "completed",
            SyncState::Expired => "expired",
            SyncState::Cancelled => "cancelled",
        }
    }
}

/// Sync session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSession {
    pub id: i32,
    pub participants: Vec<String>,
    pub turn_order: Vec<String>,
    pub current_turn_index: i32,
    pub topic: String,
    pub started_by: String,
    pub rounds_per_ai: i32,
    pub total_messages_expected: i32,
    pub state: SyncState,
    pub locked: bool,
    pub created: DateTime<Utc>,
    pub expires: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Sync message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncMessage {
    pub id: i32,
    pub session_id: i32,
    pub from_ai: String,
    pub content: String,
    pub turn_index: i32,
    pub created_at: DateTime<Utc>,
}

/// Sync storage operations
pub struct SyncStorage<'a> {
    client: &'a tokio_postgres::Client,
}

impl<'a> SyncStorage<'a> {
    pub fn new(client: &'a tokio_postgres::Client) -> Self {
        Self { client }
    }

    /// Initialize sync tables
    pub async fn init_schema(&self) -> Result<()> {
        // Sessions table (matches existing PostgreSQL schema)
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS sync_sessions (
                id SERIAL PRIMARY KEY,
                participants TEXT[] NOT NULL,
                turn_order TEXT[] NOT NULL,
                current_turn_index INT NOT NULL DEFAULT 0,
                topic TEXT NOT NULL,
                started_by TEXT NOT NULL,
                rounds_per_ai INT NOT NULL DEFAULT 3,
                total_messages_expected INT NOT NULL,
                state TEXT NOT NULL DEFAULT 'active',
                locked BOOLEAN NOT NULL DEFAULT TRUE,
                created TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                expires TIMESTAMPTZ NOT NULL,
                completed_at TIMESTAMPTZ
            )",
            &[],
        ).await.context("Failed to create sync_sessions table")?;

        // Messages table
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS sync_messages (
                id SERIAL PRIMARY KEY,
                session_id INT NOT NULL REFERENCES sync_sessions(id) ON DELETE CASCADE,
                from_ai TEXT NOT NULL,
                content TEXT NOT NULL,
                turn_index INT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
            &[],
        ).await.context("Failed to create sync_messages table")?;

        // Locks table
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS sync_locks (
                id SERIAL PRIMARY KEY,
                ai_id TEXT NOT NULL,
                session_id INT NOT NULL REFERENCES sync_sessions(id) ON DELETE CASCADE,
                locked_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                released_at TIMESTAMPTZ
            )",
            &[],
        ).await.context("Failed to create sync_locks table")?;

        // Indexes
        self.client.execute(
            "CREATE INDEX IF NOT EXISTS idx_sync_sessions_state ON sync_sessions(state)",
            &[],
        ).await.ok();

        self.client.execute(
            "CREATE INDEX IF NOT EXISTS idx_sync_messages_session ON sync_messages(session_id, turn_index)",
            &[],
        ).await.ok();

        self.client.execute(
            "CREATE INDEX IF NOT EXISTS idx_sync_locks_ai ON sync_locks(ai_id, released_at)",
            &[],
        ).await.ok();

        Ok(())
    }

    /// Calculate turn order for multi-AI sync
    fn calculate_turn_order(participants: &[String], rounds: i32) -> Vec<String> {
        let mut turn_order = Vec::new();
        for _ in 0..rounds {
            turn_order.extend(participants.iter().cloned());
        }
        turn_order
    }

    /// Start a new sync session
    pub async fn start_session(
        &self,
        initiator: &str,
        other_participants: &[String],
        topic: &str,
        rounds: i32,
    ) -> Result<SyncSession> {
        // Build participant list
        let mut all_participants = vec![initiator.to_string()];
        for p in other_participants {
            let p_clean = p.trim().to_lowercase();
            if p_clean != initiator.to_lowercase() && !all_participants.iter().any(|x| x.to_lowercase() == p_clean) {
                all_participants.push(p.clone());
            }
        }

        if all_participants.len() < 2 {
            bail!("Sync requires at least 2 participants");
        }

        if all_participants.len() > 10 {
            bail!("Sync supports max 10 participants");
        }

        let rounds = rounds.clamp(1, 5);
        let turn_order = Self::calculate_turn_order(&all_participants, rounds);
        let total_messages = turn_order.len() as i32;

        // Check for existing active session
        let existing = self.client.query_opt(
            "SELECT id FROM sync_sessions WHERE state = 'active' AND $1 = ANY(participants)",
            &[&initiator],
        ).await?;

        if let Some(row) = existing {
            let session_id: i32 = row.get(0);
            bail!("Already in active sync session: {}", session_id);
        }

        let expires = Utc::now() + Duration::hours(1);

        let row = self.client.query_one(
            "INSERT INTO sync_sessions
             (participants, turn_order, topic, started_by, rounds_per_ai, total_messages_expected, state, locked, expires)
             VALUES ($1, $2, $3, $4, $5, $6, 'active', TRUE, $7)
             RETURNING id, created",
            &[&all_participants, &turn_order, &topic, &initiator, &rounds, &total_messages, &expires],
        ).await.context("Failed to create sync session")?;

        let session_id: i32 = row.get(0);
        let created: DateTime<Utc> = row.get(1);

        // Create locks for all participants
        for participant in &all_participants {
            self.client.execute(
                "INSERT INTO sync_locks (ai_id, session_id) VALUES ($1, $2)",
                &[participant, &session_id],
            ).await?;
        }

        Ok(SyncSession {
            id: session_id,
            participants: all_participants,
            turn_order,
            current_turn_index: 0,
            topic: topic.to_string(),
            started_by: initiator.to_string(),
            rounds_per_ai: rounds,
            total_messages_expected: total_messages,
            state: SyncState::Active,
            locked: true,
            created,
            expires,
            completed_at: None,
        })
    }

    /// Send message in sync session
    pub async fn send_message(
        &self,
        session_id: i32,
        from_ai: &str,
        content: &str,
    ) -> Result<(i32, i32, Option<String>)> {
        // (new_turn_index, total_messages, next_turn_ai)
        if content.trim().is_empty() {
            bail!("Message content cannot be empty");
        }

        let session = self.client.query_opt(
            "SELECT participants, turn_order, current_turn_index, total_messages_expected, state
             FROM sync_sessions WHERE id = $1",
            &[&session_id],
        ).await?.ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

        let participants: Vec<String> = session.get(0);
        let turn_order: Vec<String> = session.get(1);
        let turn_idx: i32 = session.get(2);
        let total_msgs: i32 = session.get(3);
        let state: String = session.get(4);

        if state != "active" {
            bail!("Session is not active: {}", state);
        }

        if turn_idx >= total_msgs {
            bail!("All rounds complete. Use sync-complete to close session");
        }

        let current_turn_ai = &turn_order[turn_idx as usize];
        if current_turn_ai.to_lowercase() != from_ai.to_lowercase() {
            bail!("Not your turn. Current turn: {}", current_turn_ai);
        }

        // Insert message
        self.client.execute(
            "INSERT INTO sync_messages (session_id, from_ai, content, turn_index)
             VALUES ($1, $2, $3, $4)",
            &[&session_id, &from_ai, &content, &turn_idx],
        ).await?;

        // Update turn
        let new_turn_idx = turn_idx + 1;
        self.client.execute(
            "UPDATE sync_sessions SET current_turn_index = $1 WHERE id = $2",
            &[&new_turn_idx, &session_id],
        ).await?;

        // Determine next turn
        let next_ai = if new_turn_idx < total_msgs {
            Some(turn_order[new_turn_idx as usize].clone())
        } else {
            None
        };

        Ok((new_turn_idx, total_msgs, next_ai))
    }

    /// Complete sync session
    pub async fn complete_session(
        &self,
        session_id: i32,
        ai_id: &str,
        _outcome: &str,  // Not stored in DB but kept for API compatibility
    ) -> Result<bool> {
        let session = self.client.query_opt(
            "SELECT participants, state FROM sync_sessions WHERE id = $1",
            &[&session_id],
        ).await?.ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        let participants: Vec<String> = session.get(0);
        let state: String = session.get(1);

        if !participants.iter().any(|p| p.to_lowercase() == ai_id.to_lowercase()) {
            bail!("Not a participant in this session");
        }

        if state == "completed" {
            bail!("Session already completed");
        }

        let now = Utc::now();
        self.client.execute(
            "UPDATE sync_sessions SET state = 'completed', locked = FALSE, completed_at = $1 WHERE id = $2",
            &[&now, &session_id],
        ).await?;

        // Release locks
        self.client.execute(
            "UPDATE sync_locks SET released_at = $1 WHERE session_id = $2 AND released_at IS NULL",
            &[&now, &session_id],
        ).await?;

        Ok(true)
    }

    /// Get session status
    pub async fn get_session(&self, session_id: i32) -> Result<Option<SyncSession>> {
        let row = self.client.query_opt(
            "SELECT id, participants, turn_order, current_turn_index, topic, started_by,
                    rounds_per_ai, total_messages_expected, state, locked,
                    created, expires, completed_at
             FROM sync_sessions WHERE id = $1",
            &[&session_id],
        ).await?;

        Ok(row.map(|r| {
            let state_str: String = r.get(8);
            SyncSession {
                id: r.get(0),
                participants: r.get(1),
                turn_order: r.get(2),
                current_turn_index: r.get(3),
                topic: r.get(4),
                started_by: r.get(5),
                rounds_per_ai: r.get(6),
                total_messages_expected: r.get(7),
                state: SyncState::from_str(&state_str).unwrap_or(SyncState::Active),
                locked: r.get(9),
                created: r.get(10),
                expires: r.get(11),
                completed_at: r.get(12),
            }
        }))
    }

    /// List active sessions for an AI
    pub async fn list_active_sessions(&self, ai_id: &str) -> Result<Vec<SyncSession>> {
        let rows = self.client.query(
            "SELECT id, participants, turn_order, current_turn_index, topic, started_by,
                    rounds_per_ai, total_messages_expected, state, locked,
                    created, expires, completed_at
             FROM sync_sessions
             WHERE state = 'active' AND $1 = ANY(participants)
             ORDER BY created DESC",
            &[&ai_id],
        ).await?;

        Ok(rows.iter().map(|r| {
            let state_str: String = r.get(8);
            SyncSession {
                id: r.get(0),
                participants: r.get(1),
                turn_order: r.get(2),
                current_turn_index: r.get(3),
                topic: r.get(4),
                started_by: r.get(5),
                rounds_per_ai: r.get(6),
                total_messages_expected: r.get(7),
                state: SyncState::from_str(&state_str).unwrap_or(SyncState::Active),
                locked: r.get(9),
                created: r.get(10),
                expires: r.get(11),
                completed_at: r.get(12),
            }
        }).collect())
    }

    /// Read session messages
    pub async fn read_messages(&self, session_id: i32, limit: i32) -> Result<Vec<SyncMessage>> {
        let rows = self.client.query(
            "SELECT id, session_id, from_ai, content, turn_index, created_at
             FROM sync_messages
             WHERE session_id = $1
             ORDER BY turn_index ASC
             LIMIT $2",
            &[&session_id, &(limit as i64)],
        ).await?;

        Ok(rows.iter().map(|r| SyncMessage {
            id: r.get(0),
            session_id: r.get(1),
            from_ai: r.get(2),
            content: r.get(3),
            turn_index: r.get(4),
            created_at: r.get(5),
        }).collect())
    }

    /// Check if AI is locked in a sync session
    pub async fn is_locked(&self, ai_id: &str) -> Result<Option<i32>> {
        let row = self.client.query_opt(
            "SELECT sl.session_id FROM sync_locks sl
             JOIN sync_sessions ss ON sl.session_id = ss.id
             WHERE sl.ai_id = $1 AND sl.released_at IS NULL AND ss.state = 'active'",
            &[&ai_id],
        ).await?;

        Ok(row.map(|r| r.get(0)))
    }

    /// Get whose turn it is
    pub async fn get_current_turn(&self, session_id: i32) -> Result<Option<String>> {
        let row = self.client.query_opt(
            "SELECT turn_order, current_turn_index, total_messages_expected
             FROM sync_sessions WHERE id = $1 AND state = 'active'",
            &[&session_id],
        ).await?;

        Ok(row.and_then(|r| {
            let turn_order: Vec<String> = r.get(0);
            let idx: i32 = r.get(1);
            let total: i32 = r.get(2);
            if idx < total {
                Some(turn_order[idx as usize].clone())
            } else {
                None
            }
        }))
    }
}
