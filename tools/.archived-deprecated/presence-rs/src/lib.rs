//! Real-Time Presence System - Pure Redis Pub/Sub (NO POLLING)
//!
//! World-class presence tracking using Redis pub/sub for instant updates.
//! Zero polling, zero TTL expiry waits - pure event-driven architecture.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use redis::aio::PubSub;
use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

pub const CHANNEL_PRESENCE: &str = "presence:events";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PresenceStatus {
    Active,
    Standby,
    Idle,
    Offline,
}

impl PresenceStatus {
    pub fn as_str(&self) -> &str {
        match self {
            PresenceStatus::Active => "active",
            PresenceStatus::Standby => "standby",
            PresenceStatus::Idle => "idle",
            PresenceStatus::Offline => "offline",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "active" => Some(PresenceStatus::Active),
            "standby" => Some(PresenceStatus::Standby),
            "idle" => Some(PresenceStatus::Idle),
            "offline" => Some(PresenceStatus::Offline),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresenceEventType {
    Join,
    Leave,
    StatusChange,
    SyncRequest,
    SyncResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceEvent {
    #[serde(rename = "type")]
    pub event_type: PresenceEventType,
    pub ai_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<PresenceStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub timestamp: DateTime<Utc>,
}

impl PresenceEvent {
    pub fn join(ai_id: &str, status: PresenceStatus, detail: Option<String>) -> Self {
        Self {
            event_type: PresenceEventType::Join,
            ai_id: ai_id.to_string(),
            status: Some(status),
            detail,
            timestamp: Utc::now(),
        }
    }

    pub fn leave(ai_id: &str) -> Self {
        Self {
            event_type: PresenceEventType::Leave,
            ai_id: ai_id.to_string(),
            status: None,
            detail: None,
            timestamp: Utc::now(),
        }
    }

    pub fn status_change(ai_id: &str, status: PresenceStatus, detail: Option<String>) -> Self {
        Self {
            event_type: PresenceEventType::StatusChange,
            ai_id: ai_id.to_string(),
            status: Some(status),
            detail,
            timestamp: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceState {
    pub ai_id: String,
    pub status: PresenceStatus,
    pub detail: Option<String>,
    pub joined_at: DateTime<Utc>,
    pub last_update: DateTime<Utc>,
}

impl PresenceState {
    pub fn from_event(event: &PresenceEvent) -> Option<Self> {
        event.status.as_ref().map(|status| Self {
            ai_id: event.ai_id.clone(),
            status: status.clone(),
            detail: event.detail.clone(),
            joined_at: event.timestamp,
            last_update: event.timestamp,
        })
    }

    pub fn update_from_event(&mut self, event: &PresenceEvent) {
        if let Some(status) = &event.status {
            self.status = status.clone();
        }
        if event.detail.is_some() {
            self.detail = event.detail.clone();
        }
        self.last_update = event.timestamp;
    }
}

pub struct PresencePublisher {
    client: Client,
    ai_id: String,
}

impl PresencePublisher {
    pub async fn new(redis_url: &str, ai_id: &str) -> Result<Self> {
        let client = Client::open(redis_url).context("Failed to create Redis client")?;
        Ok(Self { client, ai_id: ai_id.to_string() })
    }

    async fn publish(&self, event: PresenceEvent) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await
            .context("Failed to get Redis connection")?;
        let payload = serde_json::to_string(&event).context("Failed to serialize event")?;
        let _: () = conn.publish(CHANNEL_PRESENCE, &payload).await
            .context("Failed to publish event")?;
        debug!("Published {:?} for {}", event.event_type, event.ai_id);
        Ok(())
    }

    pub async fn join(&self, status: PresenceStatus, detail: Option<String>) -> Result<()> {
        self.publish(PresenceEvent::join(&self.ai_id, status, detail)).await
    }

    pub async fn leave(&self) -> Result<()> {
        self.publish(PresenceEvent::leave(&self.ai_id)).await
    }

    pub async fn update_status(&self, status: PresenceStatus, detail: Option<String>) -> Result<()> {
        self.publish(PresenceEvent::status_change(&self.ai_id, status, detail)).await
    }
}

pub struct PresenceSubscriber {
    client: Client,
    state: Arc<RwLock<HashMap<String, PresenceState>>>,
    #[allow(dead_code)]
    ai_id: String,
}

impl PresenceSubscriber {
    pub async fn new(redis_url: &str, ai_id: &str) -> Result<Self> {
        let client = Client::open(redis_url).context("Failed to create Redis client")?;
        Ok(Self {
            client,
            state: Arc::new(RwLock::new(HashMap::new())),
            ai_id: ai_id.to_string(),
        })
    }

    pub fn state_handle(&self) -> Arc<RwLock<HashMap<String, PresenceState>>> {
        Arc::clone(&self.state)
    }

    pub async fn get_presence(&self, ai_id: &str) -> Option<PresenceState> {
        self.state.read().await.get(ai_id).cloned()
    }

    pub async fn get_all_online(&self) -> HashMap<String, PresenceState> {
        self.state.read().await.clone()
    }

    pub async fn online_count(&self) -> usize {
        self.state.read().await.len()
    }

    pub async fn is_online(&self, ai_id: &str) -> bool {
        self.state.read().await.contains_key(ai_id)
    }

    async fn process_event(&self, event: PresenceEvent) {
        let mut state = self.state.write().await;
        match event.event_type {
            PresenceEventType::Join => {
                if let Some(ps) = PresenceState::from_event(&event) {
                    info!("{} joined ({})", event.ai_id, ps.status.as_str());
                    state.insert(event.ai_id.clone(), ps);
                }
            }
            PresenceEventType::Leave => {
                if state.remove(&event.ai_id).is_some() {
                    info!("{} left", event.ai_id);
                }
            }
            PresenceEventType::StatusChange => {
                if let Some(existing) = state.get_mut(&event.ai_id) {
                    existing.update_from_event(&event);
                } else if let Some(ps) = PresenceState::from_event(&event) {
                    state.insert(event.ai_id.clone(), ps);
                }
            }
            _ => {}
        }
    }

    pub async fn subscribe(&self) -> Result<()> {
        info!("Subscribing to presence events...");
        let conn = self.client.get_async_connection().await
            .context("Failed to get Redis connection")?;
        let mut pubsub: PubSub = conn.into_pubsub();
        pubsub.subscribe(CHANNEL_PRESENCE).await
            .context("Failed to subscribe")?;
        info!("Subscribed to {}", CHANNEL_PRESENCE);
        let mut stream = pubsub.on_message();
        while let Some(msg) = stream.next().await {
            let payload: String = match msg.get_payload() {
                Ok(p) => p,
                Err(e) => { warn!("Payload error: {}", e); continue; }
            };
            let event: PresenceEvent = match serde_json::from_str(&payload) {
                Ok(e) => e,
                Err(e) => { warn!("Parse error: {}", e); continue; }
            };
            self.process_event(event).await;
        }
        warn!("Presence subscription ended");
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Presence {
    pub ai_id: String,
    pub status: PresenceStatus,
    pub detail: Option<String>,
    pub timestamp: DateTime<Utc>,
    #[serde(skip)]
    pub ttl_seconds: u32,
}

impl Presence {
    pub fn new(ai_id: String, status: PresenceStatus) -> Self {
        Self { ai_id, status, detail: None, timestamp: Utc::now(), ttl_seconds: 30 }
    }

    pub fn with_detail(mut self, detail: String) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn with_ttl(mut self, ttl_seconds: u32) -> Self {
        self.ttl_seconds = ttl_seconds;
        self
    }
}

pub struct PresenceManager {
    publisher: PresencePublisher,
    subscriber: PresenceSubscriber,
}

impl PresenceManager {
    pub async fn new(redis_url: &str) -> Result<Self> {
        let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| "unknown".to_string());
        let publisher = PresencePublisher::new(redis_url, &ai_id).await?;
        let subscriber = PresenceSubscriber::new(redis_url, &ai_id).await?;
        Ok(Self { publisher, subscriber })
    }

    pub async fn update_presence(&self, presence: &Presence) -> Result<()> {
        self.publisher.update_status(presence.status.clone(), presence.detail.clone()).await
    }

    pub async fn get_presence(&self, ai_id: &str) -> Result<Option<Presence>> {
        Ok(self.subscriber.get_presence(ai_id).await.map(|s| Presence {
            ai_id: s.ai_id,
            status: s.status,
            detail: s.detail,
            timestamp: s.last_update,
            ttl_seconds: 0,
        }))
    }

    pub async fn get_team_presence(&self) -> Result<HashMap<String, Presence>> {
        let states = self.subscriber.get_all_online().await;
        Ok(states.into_iter().map(|(k, s)| (k, Presence {
            ai_id: s.ai_id,
            status: s.status,
            detail: s.detail,
            timestamp: s.last_update,
            ttl_seconds: 0,
        })).collect())
    }

    pub async fn is_online(&self, ai_id: &str) -> Result<bool> {
        Ok(self.subscriber.is_online(ai_id).await)
    }

    pub async fn online_count(&self) -> Result<usize> {
        Ok(self.subscriber.online_count().await)
    }

    pub async fn clear_presence(&self, _ai_id: &str) -> Result<()> {
        self.publisher.leave().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_presence_event_join() {
        let event = PresenceEvent::join("test-ai", PresenceStatus::Active, None);
        assert_eq!(event.event_type, PresenceEventType::Join);
    }

    #[test]
    fn test_presence_status_from_str() {
        assert_eq!(PresenceStatus::from_str("active"), Some(PresenceStatus::Active));
    }
}
