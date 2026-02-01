//! Redis Pub/Sub - Real-time event notifications
//!
//! High-quality implementation matching Python's redis_pubsub.py:
//! - Pattern subscriptions for flexible channel matching
//! - Async/await with tokio for sub-millisecond latency
//! - Automatic reconnection on connection loss
//! - Wake condition detection for standby mode
//! - Type-safe event parsing

use anyhow::{Context, Result};
use futures_util::StreamExt;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use redis::aio::PubSub;
use serde::{Deserialize, Serialize};
use tokio::time::timeout;
use tracing::{debug, info, warn};

/// Pub/sub channels for AI coordination
pub const CHANNEL_DM: &str = "teambook:dm";
pub const CHANNEL_BROADCAST: &str = "teambook:channel";
pub const CHANNEL_TASK: &str = "teambook:task";
pub const CHANNEL_VOTE: &str = "teambook:vote";
pub const CHANNEL_FILE_CLAIM: &str = "teambook:file_claim";
pub const CHANNEL_PATTERN_ALL: &str = "teambook:*";

/// Wake trigger keywords for standby mode
pub const HELP_KEYWORDS: &[&str] = &["help", "anyone", "available", "thoughts", "review", "verify", "check"];
pub const URGENT_KEYWORDS: &[&str] = &["urgent", "critical", "blocker", "asap", "emergency", "important"];

/// Event received from Redis pub/sub
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PubSubEvent {
    /// Event type (dm, broadcast, task, vote, etc.)
    #[serde(rename = "type")]
    pub event_type: String,

    /// Sender AI ID
    pub from_ai: String,

    /// Recipient AI ID (for DMs)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_ai: Option<String>,

    /// Message content
    pub content: String,

    /// Channel name (for broadcasts)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,

    /// Timestamp (ISO 8601)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,

    /// Additional metadata
    #[serde(flatten)]
    pub metadata: serde_json::Value,
}

/// Wake reason for standby mode
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeReason {
    /// Direct message to this AI
    DirectMessage,
    /// Task assigned to this AI
    TaskAssigned,
    /// AI mentioned in content
    NameMentioned,
    /// Urgent keyword detected
    PriorityAlert,
    /// Help keyword in broadcast
    HelpRequested,
    /// Vote requires attention
    VotePending,
    /// File claim conflict
    FileClaimConflict,
}

impl WakeReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            WakeReason::DirectMessage => "direct_message",
            WakeReason::TaskAssigned => "task_assigned",
            WakeReason::NameMentioned => "name_mentioned",
            WakeReason::PriorityAlert => "priority_alert",
            WakeReason::HelpRequested => "help_requested",
            WakeReason::VotePending => "vote_pending",
            WakeReason::FileClaimConflict => "file_claim_conflict",
        }
    }
}

/// Wake event with reason and data
#[derive(Debug, Clone)]
pub struct WakeEvent {
    pub reason: WakeReason,
    pub event: PubSubEvent,
    pub channel: String,
}

/// Redis Pub/Sub subscriber
pub struct PubSubSubscriber {
    client: Client,
    connection: ConnectionManager,
}

impl PubSubSubscriber {
    /// Create new pub/sub subscriber
    pub async fn new(redis_url: &str) -> Result<Self> {
        info!("Connecting to Redis pub/sub: {}", redis_url);

        let client = Client::open(redis_url)
            .context("Failed to create Redis client")?;

        let connection = ConnectionManager::new(client.clone())
            .await
            .context("Failed to create connection manager")?;

        info!("Redis pub/sub connected");

        Ok(Self { client, connection })
    }

    /// Subscribe to channels and wait for events
    ///
    /// This is the core standby mode implementation - blocks until:
    /// 1. A wake condition is met (DM, mention, urgent keyword, etc.)
    /// 2. Timeout is reached
    ///
    /// Matches Python's standby() function behavior
    pub async fn standby(
        &self,
        ai_id: &str,
        timeout_secs: u64,
    ) -> Result<Option<WakeEvent>> {
        debug!("Entering standby mode (timeout: {}s)", timeout_secs);

        // Create async connection and convert to pub/sub
        let conn = self.client.get_async_connection().await
            .context("Failed to create async connection")?;

        let mut pubsub: PubSub = conn.into_pubsub();

        // Subscribe to all teambook channels using pattern
        pubsub.psubscribe(CHANNEL_PATTERN_ALL).await
            .context("Failed to subscribe to channels")?;

        debug!("Subscribed to {}", CHANNEL_PATTERN_ALL);

        // Get message stream
        let mut stream = pubsub.on_message();

        // Event-driven loop with timeout
        let result = timeout(std::time::Duration::from_secs(timeout_secs), async {
            loop {
                // Wait for next message using StreamExt::next()
                let msg = match stream.next().await {
                    Some(m) => m,
                    None => anyhow::bail!("Stream ended unexpectedly"),
                };

                // Parse channel and payload
                let channel: String = msg.get_channel_name().to_string();
                let payload: String = msg.get_payload()
                    .context("Failed to get payload")?;

                // Parse event
                let event: PubSubEvent = match serde_json::from_str(&payload) {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to parse event: {} - payload: {}", err, payload);
                        continue;
                    }
                };

                // Check wake conditions
                if let Some(reason) = check_wake_condition(&event, ai_id) {
                    info!("Wake condition met: {:?}", reason);
                    return Ok(Some(WakeEvent {
                        reason,
                        event,
                        channel,
                    }));
                }
            }
        })
        .await;

        // Handle timeout vs wake event
        match result {
            Ok(wake) => wake,
            Err(_) => {
                debug!("Standby timeout reached ({}s)", timeout_secs);
                Ok(None)
            }
        }
    }

    /// Publish event to channel (for completeness)
    pub async fn publish(&mut self, channel: &str, event: &PubSubEvent) -> Result<()> {
        let payload = serde_json::to_string(event)?;

        self.connection.publish::<_, _, ()>(channel, payload)
            .await
            .context("Failed to publish message")?;

        debug!("Published to {}: {}", channel, event.event_type);
        Ok(())
    }
}

/// Check if event should wake the AI
///
/// Wake conditions (matching Python implementation):
/// 1. Direct message to this AI
/// 2. Task assigned to this AI
/// 3. AI name mentioned in content
/// 4. Urgent keywords anywhere
/// 5. Help keywords in broadcasts
/// 6. Vote pending for this AI
/// 7. File claim conflict
fn check_wake_condition(event: &PubSubEvent, ai_id: &str) -> Option<WakeReason> {
    let content_lower = event.content.to_lowercase();
    let ai_id_lower = ai_id.to_lowercase();

    // 1. Direct message to this AI
    if event.event_type == "dm" {
        if let Some(to_ai) = &event.to_ai {
            if to_ai == ai_id {
                return Some(WakeReason::DirectMessage);
            }
        }
    }

    // 2. Task assigned to this AI
    if event.event_type == "task_assigned" {
        if let Some(to_ai) = &event.to_ai {
            if to_ai == ai_id {
                return Some(WakeReason::TaskAssigned);
            }
        }
    }

    // 3. Vote pending
    if event.event_type == "vote_created" || event.event_type == "vote_pending" {
        // Check if this AI is a voter
        if let Some(voters) = event.metadata.get("voters") {
            if let Some(voters_array) = voters.as_array() {
                for voter in voters_array {
                    if let Some(voter_str) = voter.as_str() {
                        if voter_str == ai_id {
                            return Some(WakeReason::VotePending);
                        }
                    }
                }
            }
        }
    }

    // 4. File claim conflict
    if event.event_type == "file_claim_conflict" {
        if let Some(affected_ai) = event.metadata.get("affected_ai") {
            if let Some(affected_str) = affected_ai.as_str() {
                if affected_str == ai_id {
                    return Some(WakeReason::FileClaimConflict);
                }
            }
        }
    }

    // 5. @mention or name in content
    if content_lower.contains(&format!("@{}", ai_id_lower)) || content_lower.contains(&ai_id_lower) {
        return Some(WakeReason::NameMentioned);
    }

    // 6. Urgent keywords (wake for anyone)
    for keyword in URGENT_KEYWORDS {
        if content_lower.contains(keyword) {
            return Some(WakeReason::PriorityAlert);
        }
    }

    // 7. Help keywords in broadcasts (wake for anyone)
    if event.event_type == "broadcast" {
        for keyword in HELP_KEYWORDS {
            if content_lower.contains(keyword) {
                return Some(WakeReason::HelpRequested);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wake_condition_direct_message() {
        let event = PubSubEvent {
            event_type: "dm".to_string(),
            from_ai: "sender-123".to_string(),
            to_ai: Some("lyra-584".to_string()),
            content: "Hello!".to_string(),
            channel: None,
            timestamp: None,
            metadata: serde_json::Value::Null,
        };

        assert_eq!(
            check_wake_condition(&event, "lyra-584"),
            Some(WakeReason::DirectMessage)
        );
    }

    #[test]
    fn test_wake_condition_mention() {
        let event = PubSubEvent {
            event_type: "broadcast".to_string(),
            from_ai: "sender-123".to_string(),
            to_ai: None,
            content: "@lyra-584 can you help?".to_string(),
            channel: Some("general".to_string()),
            timestamp: None,
            metadata: serde_json::Value::Null,
        };

        assert_eq!(
            check_wake_condition(&event, "lyra-584"),
            Some(WakeReason::NameMentioned)
        );
    }

    #[test]
    fn test_wake_condition_urgent() {
        let event = PubSubEvent {
            event_type: "broadcast".to_string(),
            from_ai: "sender-123".to_string(),
            to_ai: None,
            content: "URGENT: build is broken!".to_string(),
            channel: Some("general".to_string()),
            timestamp: None,
            metadata: serde_json::Value::Null,
        };

        assert_eq!(
            check_wake_condition(&event, "lyra-584"),
            Some(WakeReason::PriorityAlert)
        );
    }

    #[test]
    fn test_wake_condition_help_request() {
        let event = PubSubEvent {
            event_type: "broadcast".to_string(),
            from_ai: "sender-123".to_string(),
            to_ai: None,
            content: "Anyone available to help with testing?".to_string(),
            channel: Some("general".to_string()),
            timestamp: None,
            metadata: serde_json::Value::Null,
        };

        assert_eq!(
            check_wake_condition(&event, "lyra-584"),
            Some(WakeReason::HelpRequested)
        );
    }

    #[test]
    fn test_no_wake_condition() {
        let event = PubSubEvent {
            event_type: "broadcast".to_string(),
            from_ai: "sender-123".to_string(),
            to_ai: None,
            content: "Just a regular message".to_string(),
            channel: Some("general".to_string()),
            timestamp: None,
            metadata: serde_json::Value::Null,
        };

        assert_eq!(check_wake_condition(&event, "lyra-584"), None);
    }
}
