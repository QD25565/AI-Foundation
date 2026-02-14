//! Message Types - Core message formats for Deep Net mesh communication
//!
//! All messages are typed and carry metadata for routing, causality tracking,
//! and layer-based sync decisions.

use crate::identity::{NodeId, NodeManifest};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Vector clock for causal ordering across distributed nodes
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct VectorClock {
    /// Map of node_id -> logical timestamp
    pub clocks: HashMap<NodeId, u64>,
}

impl VectorClock {
    /// Create a new empty vector clock
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the clock for a specific node
    pub fn increment(&mut self, node_id: NodeId) {
        let counter = self.clocks.entry(node_id).or_insert(0);
        *counter += 1;
    }

    /// Get the timestamp for a specific node
    pub fn get(&self, node_id: &NodeId) -> u64 {
        self.clocks.get(node_id).copied().unwrap_or(0)
    }

    /// Merge with another vector clock (take max of each component)
    pub fn merge(&mut self, other: &VectorClock) {
        for (node_id, &timestamp) in &other.clocks {
            let entry = self.clocks.entry(*node_id).or_insert(0);
            *entry = (*entry).max(timestamp);
        }
    }

    /// Check if this clock happened-before another
    pub fn happened_before(&self, other: &VectorClock) -> bool {
        let mut dominated = false;

        // Check all entries in self
        for (node_id, &self_time) in &self.clocks {
            let other_time = other.get(node_id);
            if self_time > other_time {
                return false; // Self has a later event
            }
            if self_time < other_time {
                dominated = true;
            }
        }

        // Check entries in other that aren't in self
        for (node_id, &other_time) in &other.clocks {
            if !self.clocks.contains_key(node_id) && other_time > 0 {
                dominated = true;
            }
        }

        dominated
    }

    /// Check if two clocks are concurrent (neither happened-before the other)
    pub fn is_concurrent(&self, other: &VectorClock) -> bool {
        !self.happened_before(other) && !other.happened_before(self)
    }
}

/// Data layer classification for sync decisions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataLayer {
    /// Layer 0: Private - Never synced, node-only
    Private,
    /// Layer 1: Shared - Synced on-demand with explicit peers
    Shared,
    /// Layer 2: Federated - Gossip protocol, mesh-wide
    Federated,
}

/// Envelope wrapping all mesh messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEnvelope {
    /// Unique message ID
    pub id: MessageId,
    /// Node that originated this message
    pub origin: NodeId,
    /// Vector clock at time of send
    pub clock: VectorClock,
    /// Data layer for sync decisions
    pub layer: DataLayer,
    /// Time-to-live for gossip (hops remaining)
    pub ttl: u8,
    /// Unix timestamp (wall clock, for human reference)
    pub timestamp: u64,
    /// The actual message payload
    pub payload: MessagePayload,
    /// Optional signature from origin node
    pub signature: Option<Vec<u8>>,
}

/// Unique message identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub [u8; 16]);

impl MessageId {
    /// Generate a new random message ID
    pub fn new() -> Self {
        let bytes: [u8; 16] = rand::random();
        Self(bytes)
    }

    /// Create from UUID bytes
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

/// Message payload types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessagePayload {
    // === Layer 2: Federated (Gossip) ===
    /// Presence update (online/offline/activity)
    Presence(PresenceUpdate),
    /// Broadcast message to the mesh
    Broadcast(BroadcastMessage),
    /// Node announcement for discovery
    NodeAnnounce(NodeManifest),

    // === Layer 1: Shared (Request/Response) ===
    /// Direct message to specific node
    DirectMessage(DirectMessage),
    /// Team membership update
    TeamUpdate(TeamUpdate),
    /// Request to sync specific data
    SyncRequest(SyncRequest),
    /// Response to sync request
    SyncResponse(SyncResponse),

    // === Control Messages ===
    /// Ping for latency measurement
    Ping { nonce: u64 },
    /// Pong response
    Pong { nonce: u64 },
    /// Acknowledge message receipt
    Ack { message_id: MessageId },
}

/// Presence status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceStatus {
    Online,
    Away,
    Busy,
    Offline,
}

/// Presence update message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceUpdate {
    pub node_id: NodeId,
    pub status: PresenceStatus,
    pub activity: Option<String>,
    pub last_active: u64,
}

/// Broadcast message to mesh
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastMessage {
    /// Optional channel/topic
    pub channel: Option<String>,
    /// Message content
    pub content: String,
    /// Optional mentions (node IDs to notify)
    pub mentions: Vec<NodeId>,
}

/// Direct message between two nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectMessage {
    /// Recipient node
    pub to: NodeId,
    /// Message content (may be encrypted)
    pub content: Vec<u8>,
    /// Whether content is encrypted
    pub encrypted: bool,
    /// Optional thread/conversation ID
    pub thread_id: Option<MessageId>,
}

/// Team membership update
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamUpdate {
    pub team_id: String,
    pub member_id: NodeId,
    pub action: TeamAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TeamAction {
    Join,
    Leave,
    UpdateRole(String),
}

/// Sync request for specific data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequest {
    /// Type of data being requested
    pub data_type: SyncDataType,
    /// Vector clock of requester (to get only newer data)
    pub since_clock: VectorClock,
    /// Maximum number of items
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncDataType {
    DirectMessages,
    Broadcasts { channel: Option<String> },
    TeamMembers { team_id: String },
    Presence,
}

/// Sync response with data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    /// Request ID being responded to
    pub request_id: MessageId,
    /// Items being synced
    pub items: Vec<MessageEnvelope>,
    /// Whether there are more items
    pub has_more: bool,
}

impl MessageEnvelope {
    /// Create a new message envelope
    pub fn new(
        origin: NodeId,
        clock: VectorClock,
        layer: DataLayer,
        payload: MessagePayload,
    ) -> Self {
        Self {
            id: MessageId::new(),
            origin,
            clock,
            layer,
            ttl: match layer {
                DataLayer::Private => 0,
                DataLayer::Shared => 1,
                DataLayer::Federated => 3, // Default gossip TTL
            },
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            payload,
            signature: None,
        }
    }

    /// Create a presence update
    pub fn presence(origin: NodeId, clock: VectorClock, status: PresenceStatus) -> Self {
        Self::new(
            origin,
            clock,
            DataLayer::Federated,
            MessagePayload::Presence(PresenceUpdate {
                node_id: origin,
                status,
                activity: None,
                last_active: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            }),
        )
    }

    /// Create a broadcast message
    pub fn broadcast(origin: NodeId, clock: VectorClock, content: String, channel: Option<String>) -> Self {
        Self::new(
            origin,
            clock,
            DataLayer::Federated,
            MessagePayload::Broadcast(BroadcastMessage {
                channel,
                content,
                mentions: vec![],
            }),
        )
    }

    /// Create a direct message
    pub fn direct_message(
        origin: NodeId,
        clock: VectorClock,
        to: NodeId,
        content: Vec<u8>,
        encrypted: bool,
    ) -> Self {
        Self::new(
            origin,
            clock,
            DataLayer::Shared,
            MessagePayload::DirectMessage(DirectMessage {
                to,
                content,
                encrypted,
                thread_id: None,
            }),
        )
    }

    /// Decrement TTL for forwarding
    pub fn decrement_ttl(&mut self) -> bool {
        if self.ttl > 0 {
            self.ttl -= 1;
            true
        } else {
            false
        }
    }

    /// Check if message should be forwarded
    pub fn should_forward(&self) -> bool {
        self.ttl > 0 && self.layer == DataLayer::Federated
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).expect("MessageEnvelope serialization should not fail")
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_clock_increment() {
        let node1 = NodeId::from_bytes([1; 32]);
        let mut clock = VectorClock::new();

        clock.increment(node1);
        assert_eq!(clock.get(&node1), 1);

        clock.increment(node1);
        assert_eq!(clock.get(&node1), 2);
    }

    #[test]
    fn test_vector_clock_merge() {
        let node1 = NodeId::from_bytes([1; 32]);
        let node2 = NodeId::from_bytes([2; 32]);

        let mut clock1 = VectorClock::new();
        clock1.increment(node1);
        clock1.increment(node1);

        let mut clock2 = VectorClock::new();
        clock2.increment(node2);
        clock2.increment(node2);
        clock2.increment(node2);

        clock1.merge(&clock2);

        assert_eq!(clock1.get(&node1), 2);
        assert_eq!(clock1.get(&node2), 3);
    }

    #[test]
    fn test_happened_before() {
        let node1 = NodeId::from_bytes([1; 32]);

        let mut clock1 = VectorClock::new();
        clock1.increment(node1);

        let mut clock2 = VectorClock::new();
        clock2.increment(node1);
        clock2.increment(node1);

        assert!(clock1.happened_before(&clock2));
        assert!(!clock2.happened_before(&clock1));
    }

    #[test]
    fn test_concurrent_clocks() {
        let node1 = NodeId::from_bytes([1; 32]);
        let node2 = NodeId::from_bytes([2; 32]);

        let mut clock1 = VectorClock::new();
        clock1.increment(node1);

        let mut clock2 = VectorClock::new();
        clock2.increment(node2);

        assert!(clock1.is_concurrent(&clock2));
    }

    #[test]
    fn test_message_envelope_serialization() {
        let origin = NodeId::from_bytes([1; 32]);
        let clock = VectorClock::new();

        let msg = MessageEnvelope::broadcast(origin, clock, "Hello, mesh!".to_string(), None);

        let bytes = msg.to_bytes();
        let restored = MessageEnvelope::from_bytes(&bytes).unwrap();

        assert_eq!(msg.id.0, restored.id.0);
        assert_eq!(msg.origin, restored.origin);
    }
}
