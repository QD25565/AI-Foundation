//! Storage Trait - Abstract interface for persistent storage
//!
//! Deep Net uses pluggable storage backends. This trait defines the interface
//! that any storage implementation must provide.
//!
//! Storage layers:
//! - Layer 0 (Private): Stored locally only, never leaves the device
//! - Layer 1 (Shared): Stored locally, synced on-demand with explicit peers
//! - Layer 2 (Federated): Stored locally, propagated via gossip

use crate::identity::{NodeId, NodeManifest};
use crate::message::{DataLayer, MessageEnvelope, MessageId, PresenceUpdate, VectorClock};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Storage trait - Interface for all storage backends
#[async_trait]
pub trait Store: Send + Sync {
    // ========================================================================
    // Identity
    // ========================================================================

    /// Store node identity
    async fn store_identity(&self, identity_bytes: &[u8]) -> Result<(), StoreError>;

    /// Load node identity
    async fn load_identity(&self) -> Result<Option<Vec<u8>>, StoreError>;

    // ========================================================================
    // Messages
    // ========================================================================

    /// Store a message
    async fn store_message(&self, msg: &MessageEnvelope) -> Result<(), StoreError>;

    /// Get a message by ID
    async fn get_message(&self, id: &MessageId) -> Result<Option<MessageEnvelope>, StoreError>;

    /// Get messages since a vector clock
    async fn get_messages_since(
        &self,
        layer: DataLayer,
        since: &VectorClock,
        limit: u32,
    ) -> Result<Vec<MessageEnvelope>, StoreError>;

    /// Delete a message
    async fn delete_message(&self, id: &MessageId) -> Result<bool, StoreError>;

    // ========================================================================
    // Direct Messages (Layer 1)
    // ========================================================================

    /// Get DMs with a specific peer
    async fn get_dms_with(
        &self,
        peer_id: &NodeId,
        limit: u32,
    ) -> Result<Vec<MessageEnvelope>, StoreError>;

    /// Get recent DMs (all peers)
    async fn get_recent_dms(&self, limit: u32) -> Result<Vec<MessageEnvelope>, StoreError>;

    /// Get unread DM count
    async fn get_unread_dm_count(&self) -> Result<u32, StoreError>;

    /// Mark DMs as read
    async fn mark_dms_read(&self, peer_id: &NodeId) -> Result<(), StoreError>;

    // ========================================================================
    // Broadcasts (Layer 2)
    // ========================================================================

    /// Get broadcasts, optionally filtered by channel
    async fn get_broadcasts(
        &self,
        channel: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MessageEnvelope>, StoreError>;

    // ========================================================================
    // Presence (Layer 2)
    // ========================================================================

    /// Update presence for a node
    async fn update_presence(&self, update: &PresenceUpdate) -> Result<(), StoreError>;

    /// Get presence for a node
    async fn get_presence(&self, node_id: &NodeId) -> Result<Option<PresenceUpdate>, StoreError>;

    /// Get all known presences
    async fn get_all_presences(&self) -> Result<Vec<PresenceUpdate>, StoreError>;

    // ========================================================================
    // Peers
    // ========================================================================

    /// Store a known peer
    async fn store_peer(&self, manifest: &NodeManifest) -> Result<(), StoreError>;

    /// Get a peer's manifest
    async fn get_peer(&self, node_id: &NodeId) -> Result<Option<NodeManifest>, StoreError>;

    /// Get all known peers
    async fn get_all_peers(&self) -> Result<Vec<NodeManifest>, StoreError>;

    /// Delete a peer
    async fn delete_peer(&self, node_id: &NodeId) -> Result<bool, StoreError>;

    // ========================================================================
    // Vector Clock
    // ========================================================================

    /// Store current vector clock
    async fn store_clock(&self, clock: &VectorClock) -> Result<(), StoreError>;

    /// Load vector clock
    async fn load_clock(&self) -> Result<VectorClock, StoreError>;

    // ========================================================================
    // Key-Value (for arbitrary metadata)
    // ========================================================================

    /// Set a key-value pair
    async fn kv_set(&self, key: &str, value: &[u8]) -> Result<(), StoreError>;

    /// Get a value by key
    async fn kv_get(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError>;

    /// Delete a key
    async fn kv_delete(&self, key: &str) -> Result<bool, StoreError>;

    // ========================================================================
    // Maintenance
    // ========================================================================

    /// Compact/vacuum the store
    async fn compact(&self) -> Result<(), StoreError>;

    /// Get storage statistics
    async fn stats(&self) -> Result<StoreStats, StoreError>;
}

/// Storage statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoreStats {
    /// Total messages stored
    pub message_count: u64,
    /// Total DMs
    pub dm_count: u64,
    /// Total broadcasts
    pub broadcast_count: u64,
    /// Total peers known
    pub peer_count: u64,
    /// Storage size in bytes
    pub storage_bytes: u64,
    /// Last compaction timestamp
    pub last_compaction: Option<u64>,
}

/// Store errors
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("Not found")]
    NotFound,

    #[error("Already exists")]
    AlreadyExists,

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Storage full")]
    StorageFull,

    #[error("Corruption detected: {0}")]
    Corruption(String),

    #[error("Operation not supported")]
    NotSupported,
}

// ============================================================================
// In-Memory Store (for testing and simple use cases)
// ============================================================================

/// Simple in-memory store implementation
pub struct MemoryStore {
    identity: parking_lot::RwLock<Option<Vec<u8>>>,
    messages: parking_lot::RwLock<HashMap<MessageId, MessageEnvelope>>,
    presences: parking_lot::RwLock<HashMap<NodeId, PresenceUpdate>>,
    peers: parking_lot::RwLock<HashMap<NodeId, NodeManifest>>,
    clock: parking_lot::RwLock<VectorClock>,
    kv: parking_lot::RwLock<HashMap<String, Vec<u8>>>,
    read_markers: parking_lot::RwLock<HashMap<NodeId, u64>>,
}

impl MemoryStore {
    /// Create a new in-memory store
    pub fn new() -> Self {
        Self {
            identity: parking_lot::RwLock::new(None),
            messages: parking_lot::RwLock::new(HashMap::new()),
            presences: parking_lot::RwLock::new(HashMap::new()),
            peers: parking_lot::RwLock::new(HashMap::new()),
            clock: parking_lot::RwLock::new(VectorClock::new()),
            kv: parking_lot::RwLock::new(HashMap::new()),
            read_markers: parking_lot::RwLock::new(HashMap::new()),
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Store for MemoryStore {
    async fn store_identity(&self, identity_bytes: &[u8]) -> Result<(), StoreError> {
        *self.identity.write() = Some(identity_bytes.to_vec());
        Ok(())
    }

    async fn load_identity(&self) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.identity.read().clone())
    }

    async fn store_message(&self, msg: &MessageEnvelope) -> Result<(), StoreError> {
        self.messages.write().insert(msg.id, msg.clone());
        Ok(())
    }

    async fn get_message(&self, id: &MessageId) -> Result<Option<MessageEnvelope>, StoreError> {
        Ok(self.messages.read().get(id).cloned())
    }

    async fn get_messages_since(
        &self,
        layer: DataLayer,
        since: &VectorClock,
        limit: u32,
    ) -> Result<Vec<MessageEnvelope>, StoreError> {
        let messages = self.messages.read();
        let mut result: Vec<_> = messages
            .values()
            .filter(|m| m.layer == layer && !since.happened_before(&m.clock))
            .cloned()
            .collect();

        result.sort_by_key(|m| m.timestamp);
        result.truncate(limit as usize);
        Ok(result)
    }

    async fn delete_message(&self, id: &MessageId) -> Result<bool, StoreError> {
        Ok(self.messages.write().remove(id).is_some())
    }

    async fn get_dms_with(
        &self,
        peer_id: &NodeId,
        limit: u32,
    ) -> Result<Vec<MessageEnvelope>, StoreError> {
        let messages = self.messages.read();
        let mut result: Vec<_> = messages
            .values()
            .filter(|m| {
                if let crate::message::MessagePayload::DirectMessage(ref dm) = m.payload {
                    m.origin == *peer_id || dm.to == *peer_id
                } else {
                    false
                }
            })
            .cloned()
            .collect();

        result.sort_by_key(|m| std::cmp::Reverse(m.timestamp));
        result.truncate(limit as usize);
        Ok(result)
    }

    async fn get_recent_dms(&self, limit: u32) -> Result<Vec<MessageEnvelope>, StoreError> {
        let messages = self.messages.read();
        let mut result: Vec<_> = messages
            .values()
            .filter(|m| matches!(m.payload, crate::message::MessagePayload::DirectMessage(_)))
            .cloned()
            .collect();

        result.sort_by_key(|m| std::cmp::Reverse(m.timestamp));
        result.truncate(limit as usize);
        Ok(result)
    }

    async fn get_unread_dm_count(&self) -> Result<u32, StoreError> {
        // Simplified: count all DMs (in real impl, track read state)
        let messages = self.messages.read();
        let count = messages
            .values()
            .filter(|m| matches!(m.payload, crate::message::MessagePayload::DirectMessage(_)))
            .count();
        Ok(count as u32)
    }

    async fn mark_dms_read(&self, peer_id: &NodeId) -> Result<(), StoreError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.read_markers.write().insert(*peer_id, now);
        Ok(())
    }

    async fn get_broadcasts(
        &self,
        channel: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MessageEnvelope>, StoreError> {
        let messages = self.messages.read();
        let mut result: Vec<_> = messages
            .values()
            .filter(|m| {
                if let crate::message::MessagePayload::Broadcast(ref bc) = m.payload {
                    match (channel, &bc.channel) {
                        (Some(c), Some(mc)) => c == mc,
                        (None, _) => true,
                        _ => false,
                    }
                } else {
                    false
                }
            })
            .cloned()
            .collect();

        result.sort_by_key(|m| std::cmp::Reverse(m.timestamp));
        result.truncate(limit as usize);
        Ok(result)
    }

    async fn update_presence(&self, update: &PresenceUpdate) -> Result<(), StoreError> {
        self.presences.write().insert(update.node_id, update.clone());
        Ok(())
    }

    async fn get_presence(&self, node_id: &NodeId) -> Result<Option<PresenceUpdate>, StoreError> {
        Ok(self.presences.read().get(node_id).cloned())
    }

    async fn get_all_presences(&self) -> Result<Vec<PresenceUpdate>, StoreError> {
        Ok(self.presences.read().values().cloned().collect())
    }

    async fn store_peer(&self, manifest: &NodeManifest) -> Result<(), StoreError> {
        self.peers.write().insert(manifest.node_id, manifest.clone());
        Ok(())
    }

    async fn get_peer(&self, node_id: &NodeId) -> Result<Option<NodeManifest>, StoreError> {
        Ok(self.peers.read().get(node_id).cloned())
    }

    async fn get_all_peers(&self) -> Result<Vec<NodeManifest>, StoreError> {
        Ok(self.peers.read().values().cloned().collect())
    }

    async fn delete_peer(&self, node_id: &NodeId) -> Result<bool, StoreError> {
        Ok(self.peers.write().remove(node_id).is_some())
    }

    async fn store_clock(&self, clock: &VectorClock) -> Result<(), StoreError> {
        *self.clock.write() = clock.clone();
        Ok(())
    }

    async fn load_clock(&self) -> Result<VectorClock, StoreError> {
        Ok(self.clock.read().clone())
    }

    async fn kv_set(&self, key: &str, value: &[u8]) -> Result<(), StoreError> {
        self.kv.write().insert(key.to_string(), value.to_vec());
        Ok(())
    }

    async fn kv_get(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.kv.read().get(key).cloned())
    }

    async fn kv_delete(&self, key: &str) -> Result<bool, StoreError> {
        Ok(self.kv.write().remove(key).is_some())
    }

    async fn compact(&self) -> Result<(), StoreError> {
        // No-op for memory store
        Ok(())
    }

    async fn stats(&self) -> Result<StoreStats, StoreError> {
        let messages = self.messages.read();
        let dm_count = messages
            .values()
            .filter(|m| matches!(m.payload, crate::message::MessagePayload::DirectMessage(_)))
            .count() as u64;
        let broadcast_count = messages
            .values()
            .filter(|m| matches!(m.payload, crate::message::MessagePayload::Broadcast(_)))
            .count() as u64;

        Ok(StoreStats {
            message_count: messages.len() as u64,
            dm_count,
            broadcast_count,
            peer_count: self.peers.read().len() as u64,
            storage_bytes: 0, // In-memory, not tracked
            last_compaction: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_store_messages() {
        let store = MemoryStore::new();
        let node_id = NodeId::from_bytes([1; 32]);

        let msg = MessageEnvelope::broadcast(
            node_id,
            VectorClock::new(),
            "Hello!".to_string(),
            Some("general".to_string()),
        );

        store.store_message(&msg).await.unwrap();

        let retrieved = store.get_message(&msg.id).await.unwrap();
        assert!(retrieved.is_some());

        let broadcasts = store.get_broadcasts(Some("general"), 10).await.unwrap();
        assert_eq!(broadcasts.len(), 1);
    }

    #[tokio::test]
    async fn test_memory_store_peers() {
        let store = MemoryStore::new();

        let manifest = NodeManifest {
            node_id: NodeId::from_bytes([1; 32]),
            display_name: "Test Node".to_string(),
            capabilities: vec![],
            created_at: 0,
            protocol_version: 1,
            metadata: None,
        };

        store.store_peer(&manifest).await.unwrap();

        let retrieved = store.get_peer(&manifest.node_id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().display_name, "Test Node");
    }

    #[tokio::test]
    async fn test_memory_store_kv() {
        let store = MemoryStore::new();

        store.kv_set("test_key", b"test_value").await.unwrap();

        let value = store.kv_get("test_key").await.unwrap();
        assert_eq!(value, Some(b"test_value".to_vec()));

        store.kv_delete("test_key").await.unwrap();
        let value = store.kv_get("test_key").await.unwrap();
        assert!(value.is_none());
    }
}
