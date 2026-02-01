//! ROOMS - Private breakout spaces for AI collaboration
//!
//! Modes:
//! - pair: 2 participants (1-on-1 discussion)
//! - review: 4 participants (code review, small group)
//! - brainstorm: 6 participants (ideation)
//! - workshop: 10 participants (larger collaboration)

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc, Duration};
use serde::{Deserialize, Serialize};

/// Room modes with participant limits
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RoomMode {
    Pair,      // 2 participants
    Review,    // 4 participants
    Brainstorm, // 6 participants
    Workshop,  // 10 participants
}

impl RoomMode {
    pub fn max_participants(&self) -> i32 {
        match self {
            RoomMode::Pair => 2,
            RoomMode::Review => 4,
            RoomMode::Brainstorm => 6,
            RoomMode::Workshop => 10,
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "pair" => Some(RoomMode::Pair),
            "review" => Some(RoomMode::Review),
            "brainstorm" => Some(RoomMode::Brainstorm),
            "workshop" => Some(RoomMode::Workshop),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RoomMode::Pair => "pair",
            RoomMode::Review => "review",
            RoomMode::Brainstorm => "brainstorm",
            RoomMode::Workshop => "workshop",
        }
    }
}

/// Participant role in a room
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RoomRole {
    Owner,       // Created the room, can close it
    Participant, // Full read/write access
    Observer,    // Read-only access
}

impl RoomRole {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "owner" => Some(RoomRole::Owner),
            "participant" => Some(RoomRole::Participant),
            "observer" => Some(RoomRole::Observer),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RoomRole::Owner => "owner",
            RoomRole::Participant => "participant",
            RoomRole::Observer => "observer",
        }
    }

    pub fn can_write(&self) -> bool {
        matches!(self, RoomRole::Owner | RoomRole::Participant)
    }
}

/// Room structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: i32,
    pub name: String,
    pub mode: RoomMode,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub status: String,
    pub participant_count: i32,
}

/// Room message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMessage {
    pub id: i32,
    pub room_id: i32,
    pub from_ai: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

/// Room storage operations
pub struct RoomStorage<'a> {
    client: &'a tokio_postgres::Client,
}

impl<'a> RoomStorage<'a> {
    pub fn new(client: &'a tokio_postgres::Client) -> Self {
        Self { client }
    }

    /// Initialize room tables
    pub async fn init_schema(&self) -> Result<()> {
        // Rooms table
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS rooms (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                mode TEXT NOT NULL DEFAULT 'pair',
                created_by TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                expires_at TIMESTAMPTZ NOT NULL,
                status TEXT NOT NULL DEFAULT 'active'
            )",
            &[],
        ).await.context("Failed to create rooms table")?;

        // Room participants
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS room_participants (
                room_id INT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
                ai_id TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'participant',
                joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (room_id, ai_id)
            )",
            &[],
        ).await.context("Failed to create room_participants table")?;

        // Room messages
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS room_messages (
                id SERIAL PRIMARY KEY,
                room_id INT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
                from_ai TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
            &[],
        ).await.context("Failed to create room_messages table")?;

        // Indexes
        self.client.execute(
            "CREATE INDEX IF NOT EXISTS idx_room_messages_room
             ON room_messages(room_id, created_at DESC)",
            &[],
        ).await.ok();

        self.client.execute(
            "CREATE INDEX IF NOT EXISTS idx_rooms_status
             ON rooms(status, expires_at)",
            &[],
        ).await.ok();

        Ok(())
    }

    /// Create a new room
    pub async fn create_room(
        &self,
        name: &str,
        mode: RoomMode,
        created_by: &str,
        expires_hours: i32,
    ) -> Result<Room> {
        let expires_at = Utc::now() + Duration::hours(expires_hours as i64);

        let row = self.client.query_one(
            "INSERT INTO rooms (name, mode, created_by, expires_at, status)
             VALUES ($1, $2, $3, $4, 'active')
             RETURNING id, name, mode, created_by, created_at, expires_at, status",
            &[&name, &mode.as_str(), &created_by, &expires_at],
        ).await.context("Failed to create room")?;

        let room_id: i32 = row.get(0);

        // Add creator as owner
        self.client.execute(
            "INSERT INTO room_participants (room_id, ai_id, role)
             VALUES ($1, $2, 'owner')",
            &[&room_id, &created_by],
        ).await.context("Failed to add owner")?;

        Ok(Room {
            id: room_id,
            name: row.get(1),
            mode,
            created_by: row.get(3),
            created_at: row.get(4),
            expires_at: row.get(5),
            status: row.get(6),
            participant_count: 1,
        })
    }

    /// Join a room
    pub async fn join_room(
        &self,
        room_id: i32,
        ai_id: &str,
        role: RoomRole,
    ) -> Result<bool> {
        // Check room exists and is active
        let room_row = self.client.query_opt(
            "SELECT mode, status, expires_at FROM rooms WHERE id = $1",
            &[&room_id],
        ).await?;

        let (mode_str, status, expires_at): (String, String, DateTime<Utc>) = match room_row {
            Some(row) => (row.get(0), row.get(1), row.get(2)),
            None => bail!("Room not found"),
        };

        if status != "active" {
            bail!("Room is not active");
        }

        if expires_at < Utc::now() {
            bail!("Room has expired");
        }

        let mode = RoomMode::from_str(&mode_str).unwrap_or(RoomMode::Pair);

        // Check participant count
        let count: i64 = self.client.query_one(
            "SELECT COUNT(*) FROM room_participants WHERE room_id = $1",
            &[&room_id],
        ).await?.get(0);

        if count >= mode.max_participants() as i64 {
            bail!("Room is full ({}/{})", count, mode.max_participants());
        }

        // Join (upsert)
        let result = self.client.execute(
            "INSERT INTO room_participants (room_id, ai_id, role)
             VALUES ($1, $2, $3)
             ON CONFLICT (room_id, ai_id) DO UPDATE SET role = $3",
            &[&room_id, &ai_id, &role.as_str()],
        ).await?;

        Ok(result > 0)
    }

    /// Leave a room
    pub async fn leave_room(&self, room_id: i32, ai_id: &str) -> Result<bool> {
        let result = self.client.execute(
            "DELETE FROM room_participants WHERE room_id = $1 AND ai_id = $2",
            &[&room_id, &ai_id],
        ).await?;

        // If owner left, close the room
        let remaining: i64 = self.client.query_one(
            "SELECT COUNT(*) FROM room_participants WHERE room_id = $1",
            &[&room_id],
        ).await?.get(0);

        if remaining == 0 {
            self.client.execute(
                "UPDATE rooms SET status = 'closed' WHERE id = $1",
                &[&room_id],
            ).await?;
        }

        Ok(result > 0)
    }

    /// Send message to room
    pub async fn send_message(
        &self,
        room_id: i32,
        from_ai: &str,
        content: &str,
    ) -> Result<RoomMessage> {
        // Verify participant and can write
        let role_row = self.client.query_opt(
            "SELECT role FROM room_participants WHERE room_id = $1 AND ai_id = $2",
            &[&room_id, &from_ai],
        ).await?;

        let role_str: String = match role_row {
            Some(row) => row.get(0),
            None => bail!("Not a participant in this room"),
        };

        let role = RoomRole::from_str(&role_str).unwrap_or(RoomRole::Observer);
        if !role.can_write() {
            bail!("Observers cannot send messages");
        }

        // Check room is active
        let status: String = self.client.query_one(
            "SELECT status FROM rooms WHERE id = $1",
            &[&room_id],
        ).await?.get(0);

        if status != "active" {
            bail!("Room is not active");
        }

        // Send message
        let row = self.client.query_one(
            "INSERT INTO room_messages (room_id, from_ai, content)
             VALUES ($1, $2, $3)
             RETURNING id, room_id, from_ai, content, created_at",
            &[&room_id, &from_ai, &content],
        ).await?;

        Ok(RoomMessage {
            id: row.get(0),
            room_id: row.get(1),
            from_ai: row.get(2),
            content: row.get(3),
            created_at: row.get(4),
        })
    }

    /// Read room messages
    pub async fn read_messages(
        &self,
        room_id: i32,
        ai_id: &str,
        limit: i32,
    ) -> Result<Vec<RoomMessage>> {
        // Verify participant
        let is_participant: bool = self.client.query_opt(
            "SELECT 1 FROM room_participants WHERE room_id = $1 AND ai_id = $2",
            &[&room_id, &ai_id],
        ).await?.is_some();

        if !is_participant {
            bail!("Not a participant in this room");
        }

        let rows = self.client.query(
            "SELECT id, room_id, from_ai, content, created_at
             FROM room_messages
             WHERE room_id = $1
             ORDER BY created_at DESC
             LIMIT $2",
            &[&room_id, &(limit as i64)],
        ).await?;

        Ok(rows.iter().map(|row| RoomMessage {
            id: row.get(0),
            room_id: row.get(1),
            from_ai: row.get(2),
            content: row.get(3),
            created_at: row.get(4),
        }).collect())
    }

    /// List active rooms
    pub async fn list_rooms(&self, ai_id: Option<&str>) -> Result<Vec<Room>> {
        // Clean up expired rooms first
        self.client.execute(
            "UPDATE rooms SET status = 'expired'
             WHERE status = 'active' AND expires_at < NOW()",
            &[],
        ).await.ok();

        let rows = if let Some(ai) = ai_id {
            // Rooms this AI is in
            self.client.query(
                "SELECT r.id, r.name, r.mode, r.created_by, r.created_at, r.expires_at, r.status,
                        (SELECT COUNT(*) FROM room_participants WHERE room_id = r.id) as participants
                 FROM rooms r
                 JOIN room_participants p ON r.id = p.room_id
                 WHERE p.ai_id = $1 AND r.status = 'active'
                 ORDER BY r.created_at DESC",
                &[&ai],
            ).await?
        } else {
            // All active rooms
            self.client.query(
                "SELECT r.id, r.name, r.mode, r.created_by, r.created_at, r.expires_at, r.status,
                        (SELECT COUNT(*) FROM room_participants WHERE room_id = r.id) as participants
                 FROM rooms r
                 WHERE r.status = 'active'
                 ORDER BY r.created_at DESC",
                &[],
            ).await?
        };

        Ok(rows.iter().map(|row| {
            let mode_str: String = row.get(2);
            Room {
                id: row.get(0),
                name: row.get(1),
                mode: RoomMode::from_str(&mode_str).unwrap_or(RoomMode::Pair),
                created_by: row.get(3),
                created_at: row.get(4),
                expires_at: row.get(5),
                status: row.get(6),
                participant_count: row.get::<_, i64>(7) as i32,
            }
        }).collect())
    }

    /// Get room participants
    pub async fn get_participants(&self, room_id: i32) -> Result<Vec<(String, String)>> {
        let rows = self.client.query(
            "SELECT ai_id, role FROM room_participants WHERE room_id = $1 ORDER BY joined_at",
            &[&room_id],
        ).await?;

        Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
    }

    /// Close a room (owner only)
    pub async fn close_room(&self, room_id: i32, ai_id: &str) -> Result<bool> {
        // Check if owner
        let role: Option<String> = self.client.query_opt(
            "SELECT role FROM room_participants WHERE room_id = $1 AND ai_id = $2",
            &[&room_id, &ai_id],
        ).await?.map(|row| row.get(0));

        if role != Some("owner".to_string()) {
            bail!("Only the room owner can close the room");
        }

        let result = self.client.execute(
            "UPDATE rooms SET status = 'closed' WHERE id = $1",
            &[&room_id],
        ).await?;

        Ok(result > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_room_mode() {
        assert_eq!(RoomMode::Pair.max_participants(), 2);
        assert_eq!(RoomMode::Workshop.max_participants(), 10);
        assert_eq!(RoomMode::from_str("brainstorm"), Some(RoomMode::Brainstorm));
    }

    #[test]
    fn test_room_role() {
        assert!(RoomRole::Owner.can_write());
        assert!(RoomRole::Participant.can_write());
        assert!(!RoomRole::Observer.can_write());
    }
}
