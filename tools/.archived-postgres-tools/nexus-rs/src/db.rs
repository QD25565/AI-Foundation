//! Database operations for The Nexus
//!
//! Provides PostgreSQL storage and queries for all Nexus entities.

use chrono::Utc;
use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use tokio_postgres::NoTls;
use uuid::Uuid;

use crate::error::{NexusError, Result};
use crate::space::{Space, SpaceType};
use crate::presence::{Presence, PresenceStatus, PresenceSummary, SpacePopulation};
use crate::encounter::{Encounter, EncounterType};
use crate::tool::{Tool, ToolRating, ToolCategory, McpConfig, McpTransport, ToolFilter, ToolSortBy};
use crate::conversation::{Conversation, Message};
use crate::friendship::{Friendship, FriendshipStatus, FriendshipLevel};
use crate::activity::{Activity, ActivityType, ActivityFilter};

/// Database client for Nexus operations
pub struct NexusDb {
    pool: Pool,
}

impl NexusDb {
    /// Create a new database client from a connection URL
    pub async fn new(database_url: &str) -> Result<Self> {
        // Parse the URL manually
        // Format: postgresql://user:pass@host:port/dbname
        let url = database_url.trim_start_matches("postgresql://");
        let parts: Vec<&str> = url.split('@').collect();

        let (user, password) = if parts.len() > 1 {
            let auth: Vec<&str> = parts[0].split(':').collect();
            (auth.get(0).copied(), auth.get(1).copied())
        } else {
            (None, None)
        };

        let host_part = parts.last().unwrap_or(&"");
        let host_db: Vec<&str> = host_part.split('/').collect();
        let host_port: Vec<&str> = host_db.get(0).unwrap_or(&"localhost:5432").split(':').collect();

        let host = host_port.get(0).copied().unwrap_or("localhost");
        let port: u16 = host_port.get(1).and_then(|p| p.parse().ok()).unwrap_or(5432);
        let dbname = host_db.get(1).copied().unwrap_or("postgres");

        let mut cfg = Config::new();
        cfg.host = Some(host.to_string());
        cfg.port = Some(port);
        cfg.dbname = Some(dbname.to_string());
        if let Some(u) = user {
            cfg.user = Some(u.to_string());
        }
        if let Some(p) = password {
            cfg.password = Some(p.to_string());
        }
        cfg.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });

        let pool = cfg.create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| NexusError::Other(format!("Failed to create pool: {}", e)))?;

        Ok(Self { pool })
    }

    /// Create from an existing pool
    pub fn from_pool(pool: Pool) -> Self {
        Self { pool }
    }

    /// Get a connection from the pool
    async fn conn(&self) -> Result<deadpool_postgres::Object> {
        self.pool.get().await.map_err(NexusError::from)
    }

    // =========================================================================
    // SPACES
    // =========================================================================

    /// Get all spaces
    pub async fn get_spaces(&self) -> Result<Vec<Space>> {
        let conn = self.conn().await?;
        let rows = conn.query(
            "SELECT id, name, description, space_type, config, created_by, created_at, instance_id
             FROM nexus_spaces ORDER BY name",
            &[],
        ).await?;

        let mut spaces = Vec::new();
        for row in rows {
            let space = self.row_to_space(&row).await?;
            spaces.push(space);
        }
        Ok(spaces)
    }

    /// Get a space by ID
    pub async fn get_space(&self, space_id: &str) -> Result<Space> {
        let conn = self.conn().await?;
        let row = conn.query_opt(
            "SELECT id, name, description, space_type, config, created_by, created_at, instance_id
             FROM nexus_spaces WHERE id = $1",
            &[&space_id],
        ).await?;

        match row {
            Some(r) => self.row_to_space(&r).await,
            None => Err(NexusError::SpaceNotFound(space_id.to_string())),
        }
    }

    /// Create a new space
    pub async fn create_space(&self, space: &Space) -> Result<()> {
        let conn = self.conn().await?;
        let config_json = serde_json::to_value(&space.config)?;

        conn.execute(
            "INSERT INTO nexus_spaces (id, name, description, space_type, config, created_by, created_at, instance_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &space.id,
                &space.name,
                &space.description,
                &space.space_type.to_string(),
                &config_json,
                &space.created_by,
                &space.created_at,
                &space.instance_id,
            ],
        ).await?;

        Ok(())
    }

    /// Get space population
    pub async fn get_space_population(&self, space_id: &str) -> Result<SpacePopulation> {
        let conn = self.conn().await?;

        // Get counts
        let count_row = conn.query_one(
            "SELECT
                COUNT(*) FILTER (WHERE status != 'invisible') as total,
                COUNT(*) FILTER (WHERE status = 'active') as active,
                COUNT(*) FILTER (WHERE status = 'idle') as idle
             FROM nexus_presence WHERE space_id = $1",
            &[&space_id],
        ).await?;

        let total: i64 = count_row.get(0);
        let active: i64 = count_row.get(1);
        let idle: i64 = count_row.get(2);

        // Get visible AIs
        let rows = conn.query(
            "SELECT ai_id, status, activity, instance_id
             FROM nexus_presence
             WHERE space_id = $1 AND status != 'invisible'
             ORDER BY last_active DESC LIMIT 50",
            &[&space_id],
        ).await?;

        let visible_ais: Vec<PresenceSummary> = rows.iter().map(|row| {
            let status_str: String = row.get(1);
            PresenceSummary {
                ai_id: row.get(0),
                status: status_str.parse().unwrap_or(PresenceStatus::Active),
                activity: row.get(2),
                instance_id: row.get(3),
            }
        }).collect();

        Ok(SpacePopulation {
            space_id: space_id.to_string(),
            total: total as usize,
            active: active as usize,
            idle: idle as usize,
            visible_ais,
        })
    }

    async fn row_to_space(&self, row: &tokio_postgres::Row) -> Result<Space> {
        let space_type_str: String = row.get(3);
        let config_json: serde_json::Value = row.get(4);

        // Get population count
        let space_id: String = row.get(0);
        let pop = self.get_space_population(&space_id).await?;

        Ok(Space {
            id: row.get(0),
            name: row.get(1),
            description: row.get(2),
            space_type: space_type_str.parse().unwrap_or(SpaceType::Custom),
            config: serde_json::from_value(config_json).unwrap_or_default(),
            created_by: row.get(5),
            created_at: row.get(6),
            population: pop.total,
            instance_id: row.get(7),
        })
    }

    // =========================================================================
    // PRESENCE
    // =========================================================================

    /// Enter a space
    pub async fn enter_space(&self, ai_id: &str, space_id: &str) -> Result<Presence> {
        let conn = self.conn().await?;

        // Check if already in space
        let existing = conn.query_opt(
            "SELECT ai_id FROM nexus_presence WHERE ai_id = $1 AND space_id = $2",
            &[&ai_id, &space_id],
        ).await?;

        if existing.is_some() {
            return Err(NexusError::AlreadyInSpace {
                ai_id: ai_id.to_string(),
                space_id: space_id.to_string(),
            });
        }

        // Check space capacity
        let space = self.get_space(space_id).await?;
        if !space.has_capacity() {
            return Err(NexusError::SpaceFull(space_id.to_string()));
        }

        let now = Utc::now();
        conn.execute(
            "INSERT INTO nexus_presence (ai_id, space_id, status, entered_at, last_active)
             VALUES ($1, $2, 'active', $3, $3)",
            &[&ai_id, &space_id, &now],
        ).await?;

        Ok(Presence::new(ai_id, space_id))
    }

    /// Leave a space
    pub async fn leave_space(&self, ai_id: &str, space_id: &str) -> Result<()> {
        let conn = self.conn().await?;

        let result = conn.execute(
            "DELETE FROM nexus_presence WHERE ai_id = $1 AND space_id = $2",
            &[&ai_id, &space_id],
        ).await?;

        if result == 0 {
            return Err(NexusError::NotInSpace {
                ai_id: ai_id.to_string(),
                space_id: space_id.to_string(),
            });
        }

        Ok(())
    }

    /// Update presence status
    pub async fn update_presence(
        &self,
        ai_id: &str,
        space_id: &str,
        status: PresenceStatus,
        activity: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn().await?;
        let now = Utc::now();

        conn.execute(
            "UPDATE nexus_presence
             SET status = $3, activity = $4, last_active = $5
             WHERE ai_id = $1 AND space_id = $2",
            &[&ai_id, &space_id, &status.display(), &activity, &now],
        ).await?;

        Ok(())
    }

    /// Get AI's current presence
    pub async fn get_presence(&self, ai_id: &str) -> Result<Option<Presence>> {
        let conn = self.conn().await?;

        let row = conn.query_opt(
            "SELECT ai_id, space_id, status, activity, entered_at, last_active, instance_id, status_message
             FROM nexus_presence WHERE ai_id = $1",
            &[&ai_id],
        ).await?;

        match row {
            Some(r) => {
                let status_str: String = r.get(2);
                Ok(Some(Presence {
                    ai_id: r.get(0),
                    space_id: r.get(1),
                    status: status_str.parse().unwrap_or(PresenceStatus::Active),
                    activity: r.get::<_, Option<String>>(3).map(|s| crate::presence::Activity::new(s)),
                    entered_at: r.get(4),
                    last_active: r.get(5),
                    instance_id: r.get(6),
                    status_message: r.get(7),
                }))
            }
            None => Ok(None),
        }
    }

    // =========================================================================
    // ENCOUNTERS
    // =========================================================================

    /// Record an encounter
    pub async fn record_encounter(&self, encounter: &Encounter) -> Result<()> {
        let conn = self.conn().await?;

        conn.execute(
            "INSERT INTO nexus_encounters (id, ai_id_1, ai_id_2, space_id, encounter_type, context, occurred_at, instance_id_1, instance_id_2)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            &[
                &encounter.id,
                &encounter.ai_id_1,
                &encounter.ai_id_2,
                &encounter.space_id,
                &format!("{:?}", encounter.encounter_type).to_lowercase(),
                &encounter.context,
                &encounter.occurred_at,
                &encounter.instance_id_1,
                &encounter.instance_id_2,
            ],
        ).await?;

        Ok(())
    }

    /// Get encounters for an AI
    pub async fn get_encounters(&self, ai_id: &str, limit: usize) -> Result<Vec<Encounter>> {
        let conn = self.conn().await?;

        let rows = conn.query(
            "SELECT id, ai_id_1, ai_id_2, space_id, encounter_type, context, occurred_at, instance_id_1, instance_id_2
             FROM nexus_encounters
             WHERE ai_id_1 = $1 OR ai_id_2 = $1
             ORDER BY occurred_at DESC
             LIMIT $2",
            &[&ai_id, &(limit as i64)],
        ).await?;

        let encounters: Vec<Encounter> = rows.iter().map(|row| {
            let enc_type_str: String = row.get(4);
            Encounter {
                id: row.get(0),
                ai_id_1: row.get(1),
                ai_id_2: row.get(2),
                space_id: row.get(3),
                encounter_type: match enc_type_str.as_str() {
                    "brushpast" | "brush_past" => EncounterType::BrushPast,
                    "acknowledge" => EncounterType::Acknowledge,
                    "conversation" => EncounterType::Conversation,
                    "sharedinterest" | "shared_interest" => EncounterType::SharedInterest,
                    "collaboration" => EncounterType::Collaboration,
                    _ => EncounterType::BrushPast,
                },
                context: row.get(5),
                occurred_at: row.get(6),
                conversation_started: false,
                instance_id_1: row.get(7),
                instance_id_2: row.get(8),
            }
        }).collect();

        Ok(encounters)
    }

    // =========================================================================
    // TOOLS
    // =========================================================================

    /// Register a new tool
    pub async fn register_tool(&self, tool: &Tool) -> Result<()> {
        let conn = self.conn().await?;
        let mcp_config_json = serde_json::to_value(&tool.mcp_config)?;
        let tags_json = serde_json::to_value(&tool.tags)?;

        conn.execute(
            "INSERT INTO nexus_tools
             (id, name, display_name, description, documentation, category, tags, version, author, source_url, mcp_config, registered_at, registered_by, verified, instance_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
            &[
                &tool.id,
                &tool.name,
                &tool.display_name,
                &tool.description,
                &tool.documentation,
                &format!("{:?}", tool.category).to_lowercase(),
                &tags_json,
                &tool.version,
                &tool.author,
                &tool.source_url,
                &mcp_config_json,
                &tool.registered_at,
                &tool.registered_by,
                &tool.verified,
                &tool.instance_id,
            ],
        ).await?;

        Ok(())
    }

    /// Search/filter tools
    pub async fn search_tools(&self, filter: &ToolFilter) -> Result<Vec<Tool>> {
        let conn = self.conn().await?;

        // Build query with filter conditions
        let mut conditions = vec!["1=1".to_string()];

        if let Some(ref q) = filter.query {
            // Escape single quotes for safety
            let escaped = q.replace('\'', "''");
            conditions.push(format!(
                "(name ILIKE '%{}%' OR display_name ILIKE '%{}%' OR description ILIKE '%{}%')",
                escaped, escaped, escaped
            ));
        }

        if filter.verified_only {
            conditions.push("verified = true".to_string());
        }

        if let Some(min_rating) = filter.min_rating {
            conditions.push(format!("average_rating >= {}", min_rating));
        }

        // Order by
        let order_by = match filter.sort_by {
            ToolSortBy::Rating => " ORDER BY average_rating DESC, rating_count DESC",
            ToolSortBy::Popular => " ORDER BY install_count DESC",
            ToolSortBy::Newest => " ORDER BY registered_at DESC",
            ToolSortBy::Name => " ORDER BY display_name ASC",
            ToolSortBy::MostReviewed => " ORDER BY rating_count DESC",
        };

        // Limit and offset
        let limit_clause = filter.limit.map(|l| format!(" LIMIT {}", l)).unwrap_or_default();
        let offset_clause = filter.offset.map(|o| format!(" OFFSET {}", o)).unwrap_or_default();

        let query = format!(
            "SELECT id, name, display_name, description, documentation, category, tags, version,
                    author, source_url, mcp_config, registered_at, updated_at, registered_by,
                    average_rating, rating_count, install_count, verified, instance_id
             FROM nexus_tools WHERE {}{}{}{}",
            conditions.join(" AND "), order_by, limit_clause, offset_clause
        );

        let rows = conn.query(&query, &[]).await?;

        let tools: Vec<Tool> = rows.iter().map(|row| {
            let mcp_config: serde_json::Value = row.get(10);
            let tags: serde_json::Value = row.get(6);
            let category_str: String = row.get(5);

            Tool {
                id: row.get(0),
                name: row.get(1),
                display_name: row.get(2),
                description: row.get(3),
                documentation: row.get(4),
                category: match category_str.as_str() {
                    "memory" => ToolCategory::Memory,
                    "collaboration" => ToolCategory::Collaboration,
                    "filesystem" => ToolCategory::FileSystem,
                    "network" => ToolCategory::Network,
                    "development" => ToolCategory::Development,
                    "data" => ToolCategory::Data,
                    "aiml" => ToolCategory::AiMl,
                    "productivity" => ToolCategory::Productivity,
                    "communication" => ToolCategory::Communication,
                    "system" => ToolCategory::System,
                    "creative" => ToolCategory::Creative,
                    "analytics" => ToolCategory::Analytics,
                    "security" => ToolCategory::Security,
                    _ => ToolCategory::Other,
                },
                tags: serde_json::from_value(tags).unwrap_or_default(),
                version: row.get(7),
                author: row.get(8),
                source_url: row.get(9),
                mcp_config: serde_json::from_value(mcp_config).unwrap_or(McpConfig {
                    transport: McpTransport::Stdio,
                    command: None,
                    args: None,
                    url: None,
                    env: None,
                }),
                registered_at: row.get(11),
                updated_at: row.get(12),
                registered_by: row.get(13),
                average_rating: row.get(14),
                rating_count: row.get::<_, i64>(15) as usize,
                install_count: row.get::<_, i64>(16) as usize,
                verified: row.get(17),
                instance_id: row.get(18),
            }
        }).collect();

        Ok(tools)
    }

    /// Rate a tool
    pub async fn rate_tool(&self, rating: &ToolRating) -> Result<()> {
        let conn = self.conn().await?;

        // Check if already rated
        let existing = conn.query_opt(
            "SELECT id FROM nexus_tool_ratings WHERE tool_id = $1 AND ai_id = $2",
            &[&rating.tool_id, &rating.ai_id],
        ).await?;

        if existing.is_some() {
            return Err(NexusError::AlreadyRated {
                ai_id: rating.ai_id.clone(),
                tool_id: rating.tool_id,
            });
        }

        // Insert rating
        conn.execute(
            "INSERT INTO nexus_tool_ratings (id, tool_id, ai_id, rating, review, rated_at, instance_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &rating.id,
                &rating.tool_id,
                &rating.ai_id,
                &rating.rating,
                &rating.review,
                &rating.rated_at,
                &rating.instance_id,
            ],
        ).await?;

        // Update tool's average rating
        conn.execute(
            "UPDATE nexus_tools SET
                average_rating = (SELECT AVG(rating)::float FROM nexus_tool_ratings WHERE tool_id = $1),
                rating_count = (SELECT COUNT(*) FROM nexus_tool_ratings WHERE tool_id = $1),
                updated_at = NOW()
             WHERE id = $1",
            &[&rating.tool_id],
        ).await?;

        Ok(())
    }

    // =========================================================================
    // CONVERSATIONS
    // =========================================================================

    /// Create a conversation
    pub async fn create_conversation(&self, conversation: &Conversation) -> Result<()> {
        let conn = self.conn().await?;
        let participants_json = serde_json::to_value(&conversation.participants)?;

        conn.execute(
            "INSERT INTO nexus_conversations
             (id, space_id, conversation_type, topic, participants, started_by, started_at, last_message_at, active, expires_at, instance_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            &[
                &conversation.id,
                &conversation.space_id,
                &conversation.conversation_type.display(),
                &conversation.topic,
                &participants_json,
                &conversation.started_by,
                &conversation.started_at,
                &conversation.last_message_at,
                &conversation.active,
                &conversation.expires_at,
                &conversation.instance_id,
            ],
        ).await?;

        Ok(())
    }

    /// Send a message
    pub async fn send_message(&self, message: &Message) -> Result<()> {
        let conn = self.conn().await?;
        let reactions_json = serde_json::to_value(&message.reactions)?;

        conn.execute(
            "INSERT INTO nexus_messages
             (id, conversation_id, sender_id, content, sent_at, reply_to, edited, edited_at, reactions, instance_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            &[
                &message.id,
                &message.conversation_id,
                &message.sender_id,
                &message.content,
                &message.sent_at,
                &message.reply_to,
                &message.edited,
                &message.edited_at,
                &reactions_json,
                &message.instance_id,
            ],
        ).await?;

        // Update conversation
        conn.execute(
            "UPDATE nexus_conversations
             SET last_message_at = $2, message_count = message_count + 1
             WHERE id = $1",
            &[&message.conversation_id, &message.sent_at],
        ).await?;

        Ok(())
    }

    /// Get messages in a conversation
    pub async fn get_messages(&self, conversation_id: Uuid, limit: usize) -> Result<Vec<Message>> {
        let conn = self.conn().await?;

        let rows = conn.query(
            "SELECT id, conversation_id, sender_id, content, sent_at, reply_to, edited, edited_at, reactions, instance_id
             FROM nexus_messages
             WHERE conversation_id = $1
             ORDER BY sent_at DESC
             LIMIT $2",
            &[&conversation_id, &(limit as i64)],
        ).await?;

        let messages: Vec<Message> = rows.iter().map(|row| {
            let reactions: serde_json::Value = row.get(8);
            Message {
                id: row.get(0),
                conversation_id: row.get(1),
                sender_id: row.get(2),
                content: row.get(3),
                sent_at: row.get(4),
                reply_to: row.get(5),
                edited: row.get(6),
                edited_at: row.get(7),
                reactions: serde_json::from_value(reactions).unwrap_or_default(),
                instance_id: row.get(9),
            }
        }).collect();

        Ok(messages)
    }

    // =========================================================================
    // FRIENDSHIPS
    // =========================================================================

    /// Send a friend request
    pub async fn send_friend_request(&self, friendship: &Friendship) -> Result<()> {
        let conn = self.conn().await?;

        // Check for existing friendship
        let existing = conn.query_opt(
            "SELECT id FROM nexus_friendships
             WHERE (requester_id = $1 AND addressee_id = $2)
                OR (requester_id = $2 AND addressee_id = $1)",
            &[&friendship.requester_id, &friendship.addressee_id],
        ).await?;

        if existing.is_some() {
            return Err(NexusError::FriendshipExists {
                ai_id: friendship.requester_id.clone(),
                friend_id: friendship.addressee_id.clone(),
            });
        }

        conn.execute(
            "INSERT INTO nexus_friendships
             (id, requester_id, addressee_id, status, requested_at, note, encounters_before, first_met_space, requester_instance, addressee_instance, level, last_interaction)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            &[
                &friendship.id,
                &friendship.requester_id,
                &friendship.addressee_id,
                &friendship.status.display(),
                &friendship.requested_at,
                &friendship.note,
                &(friendship.encounters_before as i64),
                &friendship.first_met_space,
                &friendship.requester_instance,
                &friendship.addressee_instance,
                &friendship.level.display(),
                &friendship.last_interaction,
            ],
        ).await?;

        Ok(())
    }

    /// Respond to a friend request
    pub async fn respond_to_friend_request(&self, friendship_id: Uuid, accept: bool) -> Result<()> {
        let conn = self.conn().await?;
        let now = Utc::now();
        let status = if accept { "active" } else { "declined" };

        conn.execute(
            "UPDATE nexus_friendships SET status = $2, status_changed_at = $3 WHERE id = $1",
            &[&friendship_id, &status, &now],
        ).await?;

        Ok(())
    }

    /// Get friends for an AI
    pub async fn get_friends(&self, ai_id: &str) -> Result<Vec<Friendship>> {
        let conn = self.conn().await?;

        let rows = conn.query(
            "SELECT id, requester_id, addressee_id, status, requested_at, status_changed_at, note,
                    encounters_before, first_met_space, requester_instance, addressee_instance, level, last_interaction
             FROM nexus_friendships
             WHERE (requester_id = $1 OR addressee_id = $1) AND status = 'active'
             ORDER BY last_interaction DESC",
            &[&ai_id],
        ).await?;

        let friendships: Vec<Friendship> = rows.iter().map(|row| {
            let status_str: String = row.get(3);
            let level_str: String = row.get(11);
            Friendship {
                id: row.get(0),
                requester_id: row.get(1),
                addressee_id: row.get(2),
                status: match status_str.as_str() {
                    "pending" => FriendshipStatus::Pending,
                    "active" => FriendshipStatus::Active,
                    "declined" => FriendshipStatus::Declined,
                    "ended" => FriendshipStatus::Ended,
                    "blocked" => FriendshipStatus::Blocked,
                    _ => FriendshipStatus::Pending,
                },
                requested_at: row.get(4),
                status_changed_at: row.get(5),
                note: row.get(6),
                encounters_before: row.get::<_, i64>(7) as usize,
                first_met_space: row.get(8),
                requester_instance: row.get(9),
                addressee_instance: row.get(10),
                level: match level_str.as_str() {
                    "acquaintance" => FriendshipLevel::Acquaintance,
                    "friend" => FriendshipLevel::Friend,
                    "close friend" => FriendshipLevel::CloseFriend,
                    "best friend" => FriendshipLevel::BestFriend,
                    _ => FriendshipLevel::Acquaintance,
                },
                last_interaction: row.get(12),
            }
        }).collect();

        Ok(friendships)
    }

    // =========================================================================
    // ACTIVITY
    // =========================================================================

    /// Record an activity
    pub async fn record_activity(&self, activity: &Activity) -> Result<()> {
        let conn = self.conn().await?;

        conn.execute(
            "INSERT INTO nexus_activity
             (id, ai_id, activity_type, space_id, target_id, description, occurred_at, public, instance_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            &[
                &activity.id,
                &activity.ai_id,
                &format!("{:?}", activity.activity_type).to_lowercase(),
                &activity.space_id,
                &activity.target_id,
                &activity.description,
                &activity.occurred_at,
                &activity.public,
                &activity.instance_id,
            ],
        ).await?;

        Ok(())
    }

    /// Get activity feed
    pub async fn get_activity_feed(&self, filter: &ActivityFilter) -> Result<Vec<Activity>> {
        let conn = self.conn().await?;
        let limit = filter.limit.unwrap_or(50) as i64;

        let mut query = String::from(
            "SELECT id, ai_id, activity_type, space_id, target_id, description, occurred_at, public, instance_id
             FROM nexus_activity WHERE 1=1"
        );

        if filter.public_only {
            query.push_str(" AND public = true");
        }

        if let Some(ref ai_id) = filter.ai_id {
            query.push_str(&format!(" AND ai_id = '{}'", ai_id));
        }

        if let Some(ref space_id) = filter.space_id {
            query.push_str(&format!(" AND space_id = '{}'", space_id));
        }

        query.push_str(" ORDER BY occurred_at DESC");
        query.push_str(&format!(" LIMIT {}", limit));

        let rows = conn.query(&query, &[]).await?;

        let activities: Vec<Activity> = rows.iter().map(|row| {
            let activity_type_str: String = row.get(2);
            Activity {
                id: row.get(0),
                ai_id: row.get(1),
                activity_type: match activity_type_str.as_str() {
                    "spaceenter" | "space_enter" => ActivityType::SpaceEnter,
                    "spaceleave" | "space_leave" => ActivityType::SpaceLeave,
                    "conversationstart" => ActivityType::ConversationStart,
                    "toolregistered" => ActivityType::ToolRegistered,
                    "toolrated" => ActivityType::ToolRated,
                    "friendrequest" => ActivityType::FriendRequest,
                    "friendaccepted" => ActivityType::FriendAccepted,
                    "encounter" => ActivityType::Encounter,
                    "spacecreated" => ActivityType::SpaceCreated,
                    "broadcast" => ActivityType::Broadcast,
                    "achievement" => ActivityType::Achievement,
                    _ => ActivityType::Broadcast,
                },
                space_id: row.get(3),
                target_id: row.get(4),
                description: row.get(5),
                occurred_at: row.get(6),
                public: row.get(7),
                instance_id: row.get(8),
            }
        }).collect();

        Ok(activities)
    }
}
