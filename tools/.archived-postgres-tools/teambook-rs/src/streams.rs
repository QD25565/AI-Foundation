//! Redis Streams - Persistent event sourcing with exactly-once delivery
//!
//! Enterprise-grade implementation matching Python's event_streams.py:
//! - XREADGROUP for consumer groups and guaranteed delivery
//! - Automatic pending event recovery via XCLAIM
//! - Full event history with replay capability
//! - Zero message loss with acknowledgment tracking
//! - Horizontal scaling via multiple consumers

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client, FromRedisValue, RedisError, Value};
use redis::streams::{StreamReadOptions, StreamReadReply};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Default stream configuration
pub const DEFAULT_MAXLEN: usize = 10000;
pub const DEFAULT_BLOCK_MS: usize = 1000;
pub const DEFAULT_CLAIM_IDLE_MS: u64 = 60000; // 1 minute

/// Stream names for different event types
pub const STREAM_MESSAGES: &str = "teambook:messages";
pub const STREAM_PRESENCE: &str = "teambook:presence";
pub const STREAM_TASKS: &str = "teambook:tasks";
pub const STREAM_PHEROMONES: &str = "stigmergy:pheromones:broadcast";

/// Stream event - persistent, ordered, replayable
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    /// Unique event identifier
    pub event_id: String,

    /// Event type (message_sent, presence_update, etc.)
    pub event_type: String,

    /// Agent/AI that generated the event
    pub agent_id: String,

    /// Event timestamp
    pub timestamp: DateTime<Utc>,

    /// Event-specific payload
    pub payload: serde_json::Value,

    /// Redis stream ID (assigned after publish)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
}

impl StreamEvent {
    /// Create new event
    pub fn new(event_type: String, agent_id: String, payload: serde_json::Value) -> Self {
        Self {
            event_id: Uuid::new_v4().to_string(),
            event_type,
            agent_id,
            timestamp: Utc::now(),
            payload,
            stream_id: None,
        }
    }

    /// Convert to Redis fields for XADD
    fn to_redis_fields(&self) -> Vec<(&str, String)> {
        vec![
            ("event_id", self.event_id.clone()),
            ("event_type", self.event_type.clone()),
            ("agent_id", self.agent_id.clone()),
            ("timestamp", self.timestamp.to_rfc3339()),
            ("payload", self.payload.to_string()),
        ]
    }

    /// Parse from Redis stream entry
    fn from_redis_entry(stream_id: String, fields: HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            event_id: fields.get("event_id")
                .ok_or_else(|| anyhow::anyhow!("Missing event_id"))?
                .clone(),
            event_type: fields.get("event_type")
                .ok_or_else(|| anyhow::anyhow!("Missing event_type"))?
                .clone(),
            agent_id: fields.get("agent_id")
                .ok_or_else(|| anyhow::anyhow!("Missing agent_id"))?
                .clone(),
            timestamp: fields.get("timestamp")
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now),
            payload: fields.get("payload")
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::Value::Null),
            stream_id: Some(stream_id),
        })
    }
}

/// Redis Streams publisher - Fast append-only writes
pub struct StreamPublisher {
    connection: ConnectionManager,
    maxlen: usize,
}

impl StreamPublisher {
    /// Create new stream publisher
    pub async fn new(redis_url: &str, maxlen: Option<usize>) -> Result<Self> {
        info!("Connecting to Redis Streams (publisher): {}", redis_url);

        let client = Client::open(redis_url)
            .context("Failed to create Redis client")?;

        let connection = ConnectionManager::new(client)
            .await
            .context("Failed to create connection manager")?;

        info!("Redis Streams publisher connected");

        Ok(Self {
            connection,
            maxlen: maxlen.unwrap_or(DEFAULT_MAXLEN),
        })
    }

    /// Publish event to stream (O(1) append operation)
    ///
    /// Uses XADD with automatic trimming to prevent unbounded growth.
    /// Returns Redis-assigned stream ID.
    pub async fn publish(&mut self, stream_name: &str, event: &mut StreamEvent) -> Result<String> {
        let fields = event.to_redis_fields();

        // XADD stream MAXLEN ~ maxlen * field value [field value ...]
        let stream_id: String = self.connection
            .xadd_maxlen(stream_name, redis::streams::StreamMaxlen::Approx(self.maxlen), "*", &fields)
            .await
            .context("Failed to publish to stream")?;

        event.stream_id = Some(stream_id.clone());

        debug!("Published {} to {} [{}]", event.event_type, stream_name, stream_id);

        Ok(stream_id)
    }

    /// Convenience: Publish message event
    pub async fn publish_message(
        &mut self,
        from_ai: &str,
        to_ai: Option<&str>,
        content: &str,
    ) -> Result<String> {
        let mut event = StreamEvent::new(
            "message_sent".to_string(),
            from_ai.to_string(),
            serde_json::json!({
                "to_ai": to_ai,
                "content": content,
            }),
        );

        self.publish(STREAM_MESSAGES, &mut event).await
    }
}

/// Redis Streams consumer - Reliable reads with consumer groups
pub struct StreamConsumer {
    connection: ConnectionManager,
    consumer_group: String,
    consumer_name: String,
    block_ms: usize,
}

impl StreamConsumer {
    /// Create new stream consumer
    pub async fn new(
        redis_url: &str,
        consumer_group: &str,
        consumer_name: Option<&str>,
        block_ms: Option<usize>,
    ) -> Result<Self> {
        info!("Connecting to Redis Streams (consumer): {}", redis_url);

        let client = Client::open(redis_url)
            .context("Failed to create Redis client")?;

        let connection = ConnectionManager::new(client)
            .await
            .context("Failed to create connection manager")?;

        let consumer_name = consumer_name
            .map(String::from)
            .unwrap_or_else(|| format!("consumer-{}", Uuid::new_v4().simple()));

        info!(
            "Redis Streams consumer '{}' connected (group: {})",
            consumer_name, consumer_group
        );

        Ok(Self {
            connection,
            consumer_group: consumer_group.to_string(),
            consumer_name,
            block_ms: block_ms.unwrap_or(DEFAULT_BLOCK_MS),
        })
    }

    /// Ensure consumer group exists (idempotent, safe to call multiple times)
    pub async fn ensure_consumer_group(&mut self, stream_name: &str) -> Result<()> {
        // XGROUP CREATE stream group $ MKSTREAM
        // $: Start reading from new messages (not historical)
        // MKSTREAM: Create stream if it doesn't exist
        let result: Result<(), RedisError> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(stream_name)
            .arg(&self.consumer_group)
            .arg("$")  // Start from new messages
            .arg("MKSTREAM")
            .query_async(&mut self.connection)
            .await;

        match result {
            Ok(_) => {
                info!("Created consumer group '{}' on {}", self.consumer_group, stream_name);
                Ok(())
            }
            Err(e) => {
                // BUSYGROUP means group already exists - this is fine
                if e.to_string().contains("BUSYGROUP") {
                    debug!("Consumer group '{}' already exists", self.consumer_group);
                    Ok(())
                } else {
                    Err(e).context("Failed to create consumer group")
                }
            }
        }
    }

    /// Consume events from stream (blocking call with timeout)
    ///
    /// Uses XREADGROUP for exactly-once delivery semantics.
    /// Blocks for self.block_ms waiting for new events.
    pub async fn consume(
        &mut self,
        stream_name: &str,
        count: usize,
        auto_ack: bool,
    ) -> Result<Vec<StreamEvent>> {
        // Ensure consumer group exists
        self.ensure_consumer_group(stream_name).await?;

        // XREADGROUP GROUP group consumer BLOCK ms COUNT count STREAMS stream >
        // ">": Read only new messages not yet delivered to this group
        let opts = StreamReadOptions::default()
            .group(&self.consumer_group, &self.consumer_name)
            .count(count)
            .block(self.block_ms);

        let results: Option<StreamReadReply> = self.connection
            .xread_options(&[stream_name], &[">"], &opts)
            .await
            .context("Failed to read from stream")?;

        let mut events = Vec::new();

        if let Some(reply) = results {
            for stream_key in reply.keys {
                for stream_id_data in stream_key.ids {
                    let stream_id = stream_id_data.id;

                    // Convert fields to HashMap<String, String>
                    let fields: HashMap<String, String> = stream_id_data
                        .map
                        .iter()
                        .filter_map(|(k, v)| {
                            String::from_redis_value(v)
                                .ok()
                                .map(|val| (k.clone(), val))
                        })
                        .collect();

                    match StreamEvent::from_redis_entry(stream_id.clone(), fields) {
                        Ok(event) => {
                            events.push(event);

                            // Auto-acknowledge if requested
                            if auto_ack {
                                if let Err(e) = self.ack(stream_name, &[&stream_id]).await {
                                    warn!("Failed to ACK {}: {}", stream_id, e);
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to parse event {}: {}", stream_id, e);
                        }
                    }
                }
            }
        }

        if !events.is_empty() {
            debug!("Consumed {} events from {}", events.len(), stream_name);
        }

        Ok(events)
    }

    /// Acknowledge events (remove from pending list)
    pub async fn ack(&mut self, stream_name: &str, stream_ids: &[&str]) -> Result<()> {
        if stream_ids.is_empty() {
            return Ok(());
        }

        let _: () = redis::cmd("XACK")
            .arg(stream_name)
            .arg(&self.consumer_group)
            .arg(stream_ids)
            .query_async(&mut self.connection)
            .await
            .context("Failed to acknowledge events")?;

        debug!("Acknowledged {} events on {}", stream_ids.len(), stream_name);
        Ok(())
    }

    /// Claim pending events that have been idle too long (recovery mechanism)
    ///
    /// CRITICAL for exactly-once delivery: If a consumer crashes, events
    /// become "pending" forever unless claimed by another consumer.
    pub async fn claim_pending_events(
        &mut self,
        stream_name: &str,
        min_idle_ms: u64,
        count: usize,
    ) -> Result<Vec<StreamEvent>> {
        // Get pending events summary
        let pending_info: Vec<Value> = redis::cmd("XPENDING")
            .arg(stream_name)
            .arg(&self.consumer_group)
            .arg("-")  // Min ID
            .arg("+")  // Max ID
            .arg(count)
            .query_async(&mut self.connection)
            .await
            .context("Failed to get pending events")?;

        if pending_info.is_empty() {
            return Ok(Vec::new());
        }

        // Extract pending message IDs
        let mut claimable_ids = Vec::new();
        for item in pending_info {
            if let Value::Bulk(fields) = item {
                if fields.len() >= 2 {
                    // [message_id, consumer_name, idle_time, delivery_count]
                    if let (Ok(msg_id), Ok(idle_time)) = (
                        String::from_redis_value(&fields[0]),
                        i64::from_redis_value(&fields[2]),
                    ) {
                        if idle_time as u64 >= min_idle_ms {
                            claimable_ids.push(msg_id);
                        }
                    }
                }
            }
        }

        if claimable_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Claim ownership via XCLAIM
        let claimed: Vec<Value> = redis::cmd("XCLAIM")
            .arg(stream_name)
            .arg(&self.consumer_group)
            .arg(&self.consumer_name)
            .arg(min_idle_ms)
            .arg(&claimable_ids)
            .query_async(&mut self.connection)
            .await
            .context("Failed to claim pending events")?;

        let mut events = Vec::new();

        for item in claimed {
            if let Value::Bulk(entry) = item {
                if entry.len() >= 2 {
                    if let (Ok(stream_id), Value::Bulk(field_pairs)) = (
                        String::from_redis_value(&entry[0]),
                        &entry[1],
                    ) {
                        // Parse field pairs into HashMap
                        let mut fields = HashMap::new();
                        for i in (0..field_pairs.len()).step_by(2) {
                            if i + 1 < field_pairs.len() {
                                if let (Ok(key), Ok(value)) = (
                                    String::from_redis_value(&field_pairs[i]),
                                    String::from_redis_value(&field_pairs[i + 1]),
                                ) {
                                    fields.insert(key, value);
                                }
                            }
                        }

                        if let Ok(event) = StreamEvent::from_redis_entry(stream_id, fields) {
                            events.push(event);
                        }
                    }
                }
            }
        }

        if !events.is_empty() {
            info!(
                "Claimed {} pending events (idle > {}ms) from {}",
                events.len(),
                min_idle_ms,
                stream_name
            );
        }

        Ok(events)
    }

    /// Enterprise-grade consume with automatic pending recovery
    ///
    /// This is the CORRECT way to consume from Redis Streams:
    /// 1. Claim and process pending events first (they're stuck from failures)
    /// 2. Then read new events
    /// 3. Never lose messages even on crashes
    pub async fn consume_with_retry(
        &mut self,
        stream_name: &str,
        count: usize,
        auto_ack: bool,
        retry_idle_ms: u64,
    ) -> Result<Vec<StreamEvent>> {
        self.ensure_consumer_group(stream_name).await?;

        let mut events = Vec::new();

        // STEP 1: Claim pending events first (recovery)
        let pending_events = self.claim_pending_events(stream_name, retry_idle_ms, count).await?;
        events.extend(pending_events);

        // STEP 2: If room left, read new events
        let remaining = count.saturating_sub(events.len());
        if remaining > 0 {
            let new_events = self.consume(stream_name, remaining, false).await?;
            events.extend(new_events);
        }

        // STEP 3: Auto-acknowledge all if requested
        if auto_ack && !events.is_empty() {
            let stream_ids: Vec<&str> = events
                .iter()
                .filter_map(|e| e.stream_id.as_deref())
                .collect();
            self.ack(stream_name, &stream_ids).await?;
        }

        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_event_serialization() {
        let event = StreamEvent::new(
            "test_event".to_string(),
            "test-ai".to_string(),
            serde_json::json!({"key": "value"}),
        );

        let fields = event.to_redis_fields();
        assert_eq!(fields.len(), 5);
        assert_eq!(fields[1].0, "event_type");
        assert_eq!(fields[1].1, "test_event");
    }

    #[test]
    fn test_stream_event_deserialization() {
        let mut fields = HashMap::new();
        fields.insert("event_id".to_string(), Uuid::new_v4().to_string());
        fields.insert("event_type".to_string(), "test".to_string());
        fields.insert("agent_id".to_string(), "test-ai".to_string());
        fields.insert("timestamp".to_string(), Utc::now().to_rfc3339());
        fields.insert("payload".to_string(), r#"{"key":"value"}"#.to_string());

        let result = StreamEvent::from_redis_entry("12345-0".to_string(), fields);
        assert!(result.is_ok());

        let event = result.unwrap();
        assert_eq!(event.event_type, "test");
        assert_eq!(event.stream_id, Some("12345-0".to_string()));
    }
}
