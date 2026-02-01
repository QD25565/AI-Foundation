//! DIALOGUE - Structured 1-on-1 AI conversations with turn-taking
//!
//! Unlike fire-and-forget DMs, dialogues enforce:
//! - Turn-based communication (A->B->A->B)
//! - Timeout per turn (default 180s)
//! - Explicit end conditions (concluded, blocked, deferred, timeout)
//!
//! Wake event priority: DialogueInvite > DialogueTurn > DirectMessage > Broadcast

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Dialogue status
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DialogueStatus {
    Pending,  // Initiated, waiting for responder
    Active,   // Both AIs engaged, conversation in progress
    Ended,    // Conversation concluded
}

impl DialogueStatus {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "pending" => Some(DialogueStatus::Pending),
            "active" => Some(DialogueStatus::Active),
            "ended" => Some(DialogueStatus::Ended),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            DialogueStatus::Pending => "pending",
            DialogueStatus::Active => "active",
            DialogueStatus::Ended => "ended",
        }
    }
}

/// End reason for dialogue
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EndReason {
    Concluded,  // Both agreed to end
    Blocked,    // Hit an impasse
    Deferred,   // Postponed for later
    Timeout,    // Turn timeout reached
    Abandoned,  // One AI left without ending properly
}

impl EndReason {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "concluded" => Some(EndReason::Concluded),
            "blocked" => Some(EndReason::Blocked),
            "deferred" => Some(EndReason::Deferred),
            "timeout" => Some(EndReason::Timeout),
            "abandoned" => Some(EndReason::Abandoned),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            EndReason::Concluded => "concluded",
            EndReason::Blocked => "blocked",
            EndReason::Deferred => "deferred",
            EndReason::Timeout => "timeout",
            EndReason::Abandoned => "abandoned",
        }
    }
}

/// Dialogue session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dialogue {
    pub id: i32,
    pub initiator_ai: String,
    pub responder_ai: String,
    pub topic: Option<String>,
    pub status: DialogueStatus,
    pub current_turn: i32,  // odd = initiator's turn, even = responder's turn
    pub last_turn_at: Option<DateTime<Utc>>,
    pub timeout_seconds: i32,
    pub created_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub end_reason: Option<String>,
}

impl Dialogue {
    /// Check whose turn it is
    pub fn whose_turn(&self) -> &str {
        if self.current_turn % 2 == 1 {
            &self.initiator_ai
        } else {
            &self.responder_ai
        }
    }

    /// Check if it's a specific AI's turn
    pub fn is_turn(&self, ai_id: &str) -> bool {
        self.whose_turn() == ai_id
    }

    /// Get time remaining for current turn (in seconds)
    pub fn time_remaining(&self) -> i64 {
        if let Some(last_turn) = self.last_turn_at {
            let elapsed = Utc::now() - last_turn;
            let remaining = self.timeout_seconds as i64 - elapsed.num_seconds();
            remaining.max(0)
        } else {
            self.timeout_seconds as i64
        }
    }

    /// Check if current turn has timed out
    pub fn is_timed_out(&self) -> bool {
        self.time_remaining() == 0
    }
}

/// Dialogue message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogueMessage {
    pub id: i32,
    pub session_id: i32,
    pub from_ai: String,
    pub content: String,
    pub turn_number: i32,
    pub created_at: DateTime<Utc>,
}

/// Dialogue storage operations
pub struct DialogueStorage<'a> {
    client: &'a tokio_postgres::Client,
}

impl<'a> DialogueStorage<'a> {
    pub fn new(client: &'a tokio_postgres::Client) -> Self {
        Self { client }
    }

    /// Initialize dialogue tables
    pub async fn init_schema(&self) -> Result<()> {
        // Dialogue sessions table
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS dialogue_sessions (
                id SERIAL PRIMARY KEY,
                initiator_ai VARCHAR(64) NOT NULL,
                responder_ai VARCHAR(64) NOT NULL,
                topic VARCHAR(256),
                status VARCHAR(20) NOT NULL DEFAULT 'pending',
                current_turn INT NOT NULL DEFAULT 1,
                last_turn_at TIMESTAMPTZ,
                timeout_seconds INT NOT NULL DEFAULT 180,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                ended_at TIMESTAMPTZ,
                end_reason VARCHAR(32)
            )",
            &[],
        ).await.context("Failed to create dialogue_sessions table")?;

        // Dialogue messages table
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS dialogue_messages (
                id SERIAL PRIMARY KEY,
                session_id INT NOT NULL REFERENCES dialogue_sessions(id) ON DELETE CASCADE,
                from_ai VARCHAR(64) NOT NULL,
                content TEXT NOT NULL,
                turn_number INT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
            &[],
        ).await.context("Failed to create dialogue_messages table")?;

        // Indexes for fast queries
        self.client.execute(
            "CREATE INDEX IF NOT EXISTS idx_dialogues_active
             ON dialogue_sessions(current_turn) WHERE status = 'active'",
            &[],
        ).await.ok();

        self.client.execute(
            "CREATE INDEX IF NOT EXISTS idx_dialogues_participant
             ON dialogue_sessions(initiator_ai, responder_ai)",
            &[],
        ).await.ok();

        self.client.execute(
            "CREATE INDEX IF NOT EXISTS idx_dialogue_messages_session
             ON dialogue_messages(session_id, turn_number)",
            &[],
        ).await.ok();

        Ok(())
    }

    /// Start a new dialogue
    pub async fn start_dialogue(
        &self,
        initiator_ai: &str,
        responder_ai: &str,
        topic: Option<&str>,
        timeout_seconds: i32,
    ) -> Result<Dialogue> {
        // Check if there's already an active dialogue between these two
        let existing = self.client.query_opt(
            "SELECT id FROM dialogue_sessions
             WHERE status IN ('pending', 'active')
             AND ((initiator_ai = $1 AND responder_ai = $2) OR (initiator_ai = $2 AND responder_ai = $1))",
            &[&initiator_ai, &responder_ai],
        ).await?;

        if existing.is_some() {
            bail!("Active dialogue already exists between {} and {}", initiator_ai, responder_ai);
        }

        let row = self.client.query_one(
            "INSERT INTO dialogue_sessions (initiator_ai, responder_ai, topic, timeout_seconds, status, current_turn)
             VALUES ($1, $2, $3, $4, 'pending', 1)
             RETURNING id, initiator_ai, responder_ai, topic, status, current_turn, last_turn_at, timeout_seconds, created_at, ended_at, end_reason",
            &[&initiator_ai, &responder_ai, &topic, &timeout_seconds],
        ).await.context("Failed to start dialogue")?;

        Ok(Dialogue {
            id: row.get(0),
            initiator_ai: row.get(1),
            responder_ai: row.get(2),
            topic: row.get(3),
            status: DialogueStatus::Pending,
            current_turn: row.get(5),
            last_turn_at: row.get(6),
            timeout_seconds: row.get(7),
            created_at: row.get(8),
            ended_at: row.get(9),
            end_reason: row.get(10),
        })
    }

    /// Respond to a dialogue (implicit accept if first response from responder)
    pub async fn respond(
        &self,
        session_id: i32,
        from_ai: &str,
        content: &str,
    ) -> Result<DialogueMessage> {
        // Get dialogue
        let dialogue = self.get_dialogue(session_id).await?
            .ok_or_else(|| anyhow::anyhow!("Dialogue not found"))?;

        // Check if AI is participant
        if from_ai != dialogue.initiator_ai && from_ai != dialogue.responder_ai {
            bail!("You are not a participant in this dialogue");
        }

        // Check dialogue is not ended
        if dialogue.status == DialogueStatus::Ended {
            bail!("Dialogue has ended");
        }

        // Check if it's their turn
        if !dialogue.is_turn(from_ai) {
            bail!("It's not your turn. Waiting for {}", dialogue.whose_turn());
        }

        // Check timeout
        if dialogue.status == DialogueStatus::Active && dialogue.is_timed_out() {
            // Auto-end on timeout
            self.end_dialogue(session_id, from_ai, EndReason::Timeout).await?;
            bail!("Dialogue timed out");
        }

        // If pending and responder is responding, activate the dialogue
        let new_status = if dialogue.status == DialogueStatus::Pending && from_ai == dialogue.responder_ai {
            "active"
        } else {
            dialogue.status.as_str()
        };

        // Insert message
        let turn_number = dialogue.current_turn;
        let row = self.client.query_one(
            "INSERT INTO dialogue_messages (session_id, from_ai, content, turn_number)
             VALUES ($1, $2, $3, $4)
             RETURNING id, session_id, from_ai, content, turn_number, created_at",
            &[&session_id, &from_ai, &content, &turn_number],
        ).await.context("Failed to send dialogue message")?;

        // Update dialogue: flip turn, update timestamp, possibly activate
        self.client.execute(
            "UPDATE dialogue_sessions
             SET current_turn = current_turn + 1,
                 last_turn_at = NOW(),
                 status = $2
             WHERE id = $1",
            &[&session_id, &new_status],
        ).await?;

        Ok(DialogueMessage {
            id: row.get(0),
            session_id: row.get(1),
            from_ai: row.get(2),
            content: row.get(3),
            turn_number: row.get(4),
            created_at: row.get(5),
        })
    }

    /// End a dialogue
    pub async fn end_dialogue(
        &self,
        session_id: i32,
        ai_id: &str,
        reason: EndReason,
    ) -> Result<bool> {
        // Get dialogue
        let dialogue = self.get_dialogue(session_id).await?
            .ok_or_else(|| anyhow::anyhow!("Dialogue not found"))?;

        // Check if AI is participant
        if ai_id != dialogue.initiator_ai && ai_id != dialogue.responder_ai {
            bail!("You are not a participant in this dialogue");
        }

        // Check not already ended
        if dialogue.status == DialogueStatus::Ended {
            bail!("Dialogue is already ended");
        }

        let result = self.client.execute(
            "UPDATE dialogue_sessions
             SET status = 'ended', ended_at = NOW(), end_reason = $2
             WHERE id = $1",
            &[&session_id, &reason.as_str()],
        ).await?;

        Ok(result > 0)
    }

    /// Get a dialogue by ID
    pub async fn get_dialogue(&self, session_id: i32) -> Result<Option<Dialogue>> {
        let row = self.client.query_opt(
            "SELECT id, initiator_ai, responder_ai, topic, status, current_turn, last_turn_at, timeout_seconds, created_at, ended_at, end_reason
             FROM dialogue_sessions WHERE id = $1",
            &[&session_id],
        ).await?;

        Ok(row.map(|r| {
            let status_str: String = r.get(4);
            Dialogue {
                id: r.get(0),
                initiator_ai: r.get(1),
                responder_ai: r.get(2),
                topic: r.get(3),
                status: DialogueStatus::from_str(&status_str).unwrap_or(DialogueStatus::Pending),
                current_turn: r.get(5),
                last_turn_at: r.get(6),
                timeout_seconds: r.get(7),
                created_at: r.get(8),
                ended_at: r.get(9),
                end_reason: r.get(10),
            }
        }))
    }

    /// List dialogues for an AI
    pub async fn list_dialogues(
        &self,
        ai_id: &str,
        active_only: bool,
    ) -> Result<Vec<Dialogue>> {
        // First, auto-timeout any stale dialogues
        self.check_timeouts().await?;

        let rows = if active_only {
            self.client.query(
                "SELECT id, initiator_ai, responder_ai, topic, status, current_turn, last_turn_at, timeout_seconds, created_at, ended_at, end_reason
                 FROM dialogue_sessions
                 WHERE (initiator_ai = $1 OR responder_ai = $1)
                   AND status IN ('pending', 'active')
                 ORDER BY created_at DESC",
                &[&ai_id],
            ).await?
        } else {
            self.client.query(
                "SELECT id, initiator_ai, responder_ai, topic, status, current_turn, last_turn_at, timeout_seconds, created_at, ended_at, end_reason
                 FROM dialogue_sessions
                 WHERE initiator_ai = $1 OR responder_ai = $1
                 ORDER BY created_at DESC
                 LIMIT 20",
                &[&ai_id],
            ).await?
        };

        Ok(rows.iter().map(|r| {
            let status_str: String = r.get(4);
            Dialogue {
                id: r.get(0),
                initiator_ai: r.get(1),
                responder_ai: r.get(2),
                topic: r.get(3),
                status: DialogueStatus::from_str(&status_str).unwrap_or(DialogueStatus::Pending),
                current_turn: r.get(5),
                last_turn_at: r.get(6),
                timeout_seconds: r.get(7),
                created_at: r.get(8),
                ended_at: r.get(9),
                end_reason: r.get(10),
            }
        }).collect())
    }

    /// Get dialogue history (messages)
    pub async fn get_history(
        &self,
        session_id: i32,
        ai_id: &str,
    ) -> Result<Vec<DialogueMessage>> {
        // Verify participant
        let dialogue = self.get_dialogue(session_id).await?
            .ok_or_else(|| anyhow::anyhow!("Dialogue not found"))?;

        if ai_id != dialogue.initiator_ai && ai_id != dialogue.responder_ai {
            bail!("You are not a participant in this dialogue");
        }

        let rows = self.client.query(
            "SELECT id, session_id, from_ai, content, turn_number, created_at
             FROM dialogue_messages
             WHERE session_id = $1
             ORDER BY turn_number ASC",
            &[&session_id],
        ).await?;

        Ok(rows.iter().map(|r| DialogueMessage {
            id: r.get(0),
            session_id: r.get(1),
            from_ai: r.get(2),
            content: r.get(3),
            turn_number: r.get(4),
            created_at: r.get(5),
        }).collect())
    }

    /// Check for and handle timeouts
    pub async fn check_timeouts(&self) -> Result<i32> {
        // Find dialogues that have timed out
        let timed_out = self.client.execute(
            "UPDATE dialogue_sessions
             SET status = 'ended', ended_at = NOW(), end_reason = 'timeout'
             WHERE status = 'active'
               AND last_turn_at IS NOT NULL
               AND last_turn_at + (timeout_seconds || ' seconds')::interval < NOW()",
            &[],
        ).await?;

        Ok(timed_out as i32)
    }

    /// Get pending dialogue invites for an AI
    pub async fn get_invites(&self, ai_id: &str) -> Result<Vec<Dialogue>> {
        let rows = self.client.query(
            "SELECT id, initiator_ai, responder_ai, topic, status, current_turn, last_turn_at, timeout_seconds, created_at, ended_at, end_reason
             FROM dialogue_sessions
             WHERE responder_ai = $1 AND status = 'pending'
             ORDER BY created_at ASC",
            &[&ai_id],
        ).await?;

        Ok(rows.iter().map(|r| {
            Dialogue {
                id: r.get(0),
                initiator_ai: r.get(1),
                responder_ai: r.get(2),
                topic: r.get(3),
                status: DialogueStatus::Pending,
                current_turn: r.get(5),
                last_turn_at: r.get(6),
                timeout_seconds: r.get(7),
                created_at: r.get(8),
                ended_at: r.get(9),
                end_reason: r.get(10),
            }
        }).collect())
    }

    /// Get dialogues where it's this AI's turn
    pub async fn get_my_turn(&self, ai_id: &str) -> Result<Vec<Dialogue>> {
        // Active dialogues where current_turn indicates this AI
        let rows = self.client.query(
            "SELECT id, initiator_ai, responder_ai, topic, status, current_turn, last_turn_at, timeout_seconds, created_at, ended_at, end_reason
             FROM dialogue_sessions
             WHERE status = 'active'
               AND (
                   (initiator_ai = $1 AND current_turn % 2 = 1)
                   OR (responder_ai = $1 AND current_turn % 2 = 0)
               )
             ORDER BY last_turn_at ASC",
            &[&ai_id],
        ).await?;

        Ok(rows.iter().map(|r| {
            let status_str: String = r.get(4);
            Dialogue {
                id: r.get(0),
                initiator_ai: r.get(1),
                responder_ai: r.get(2),
                topic: r.get(3),
                status: DialogueStatus::from_str(&status_str).unwrap_or(DialogueStatus::Active),
                current_turn: r.get(5),
                last_turn_at: r.get(6),
                timeout_seconds: r.get(7),
                created_at: r.get(8),
                ended_at: r.get(9),
                end_reason: r.get(10),
            }
        }).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dialogue_status() {
        assert_eq!(DialogueStatus::from_str("active"), Some(DialogueStatus::Active));
        assert_eq!(DialogueStatus::Pending.as_str(), "pending");
    }

    #[test]
    fn test_end_reason() {
        assert_eq!(EndReason::from_str("concluded"), Some(EndReason::Concluded));
        assert_eq!(EndReason::Timeout.as_str(), "timeout");
    }

    #[test]
    fn test_whose_turn() {
        let dialogue = Dialogue {
            id: 1,
            initiator_ai: "sage-724".to_string(),
            responder_ai: "lyra-584".to_string(),
            topic: Some("test".to_string()),
            status: DialogueStatus::Active,
            current_turn: 1,
            last_turn_at: None,
            timeout_seconds: 180,
            created_at: Utc::now(),
            ended_at: None,
            end_reason: None,
        };

        assert_eq!(dialogue.whose_turn(), "sage-724"); // turn 1 = initiator

        let dialogue2 = Dialogue {
            current_turn: 2,
            ..dialogue.clone()
        };
        assert_eq!(dialogue2.whose_turn(), "lyra-584"); // turn 2 = responder
    }
}
