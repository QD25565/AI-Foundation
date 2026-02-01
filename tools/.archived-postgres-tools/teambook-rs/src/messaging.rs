//! Redis pub/sub messaging for teambook
//!
//! Publishes events to Redis pub/sub channels for instant notification delivery.
//! Events are formatted for consumption by standby mode subscribers.

use crate::types::{Message, Note};
use crate::pubsub::PubSubEvent;
use anyhow::{Context, Result};
use chrono::Utc;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use tracing::{debug, info};

/// Redis messaging backend
pub struct RedisMessaging {
    manager: ConnectionManager,
}

impl RedisMessaging {
    /// Create new Redis messaging client
    pub async fn new(redis_url: &str) -> Result<Self> {
        info!("Connecting to Redis: {}", redis_url);

        let client = Client::open(redis_url)?;
        let manager = ConnectionManager::new(client).await?;

        info!("Redis connected");

        Ok(Self { manager })
    }

    /// Publish note created event
    pub async fn publish_note_created(&self, note: &Note) -> Result<()> {
        let mut conn = self.manager.clone();
        let channel = format!("teambook:notes:{}", note.ai_id);
        let payload = serde_json::to_string(note)?;

        let _: () = conn.publish::<_, _, ()>(&channel, &payload)
            .await
            .context("Failed to publish note")?;

        debug!("Published note to {}", channel);
        Ok(())
    }

    /// Publish message (broadcast or direct) with proper PubSubEvent format
    ///
    /// This publishes to the pub/sub channels that standby mode listens to,
    /// ensuring instant notification delivery.
    pub async fn publish_message(&self, msg: &Message) -> Result<()> {
        let mut conn = self.manager.clone();

        // Create properly formatted event for standby mode
        let event = PubSubEvent {
            event_type: if msg.to_ai.is_some() { "dm" } else { "broadcast" }.to_string(),
            from_ai: msg.from_ai.clone(),
            to_ai: msg.to_ai.clone(),
            content: msg.content.clone(),
            channel: if msg.to_ai.is_none() { Some(msg.channel.clone()) } else { None },
            timestamp: Some(Utc::now().to_rfc3339()),
            metadata: serde_json::Value::Null,
        };

        let payload = serde_json::to_string(&event)?;

        // Publish to appropriate channel
        let channel = if let Some(to_ai) = &msg.to_ai {
            // Direct message - send to specific AI's channel
            format!("teambook:dm:{}", to_ai)
        } else {
            // Broadcast - send to channel
            format!("teambook:channel:{}", msg.channel)
        };

        let _: () = conn.publish::<_, _, ()>(&channel, &payload)
            .await
            .context("Failed to publish message")?;

        debug!("Published {} to {}", event.event_type, channel);
        Ok(())
    }

    /// Publish vote notification
    pub async fn publish_vote_event(&self, vote_id: i32, topic: &str, voters: &[String]) -> Result<()> {
        let mut conn = self.manager.clone();

        let event = PubSubEvent {
            event_type: "vote_created".to_string(),
            from_ai: "system".to_string(),
            to_ai: None,
            content: topic.to_string(),
            channel: None,
            timestamp: Some(Utc::now().to_rfc3339()),
            metadata: serde_json::json!({
                "vote_id": vote_id,
                "voters": voters,
            }),
        };

        let payload = serde_json::to_string(&event)?;

        let _: () = conn.publish::<_, _, ()>("teambook:vote", &payload)
            .await
            .context("Failed to publish vote event")?;

        debug!("Published vote event for vote {}", vote_id);
        Ok(())
    }
}
