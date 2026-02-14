//! Sync Protocol - Distributed synchronization without full replication
//!
//! Deep Net uses a layered sync model:
//! - Layer 0 (Private): Never synced
//! - Layer 1 (Shared): Pull-based, on-demand from explicit peers
//! - Layer 2 (Federated): Gossip protocol with TTL-limited propagation
//!
//! Key principles:
//! - No full replication: Nodes only sync what they need
//! - Vector clocks for causality: Know what's new without full history
//! - TTL-limited gossip: Broadcasts decay after N hops
//! - Pull-based DMs: Recipient requests, sender doesn't push

use crate::identity::NodeId;
use crate::message::{DataLayer, MessageEnvelope, MessageId, VectorClock};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Sync state for a single peer relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerSyncState {
    /// The peer node
    pub peer_id: NodeId,
    /// Our vector clock as of last sync with this peer
    pub our_clock: VectorClock,
    /// Their vector clock as of last sync
    pub their_clock: VectorClock,
    /// Messages we've sent but not confirmed
    pub pending_acks: HashSet<MessageId>,
    /// Unix timestamp of last sync
    pub last_sync: u64,
    /// Sync enabled for this peer
    pub enabled: bool,
}

impl PeerSyncState {
    /// Create a new peer sync state
    pub fn new(peer_id: NodeId) -> Self {
        Self {
            peer_id,
            our_clock: VectorClock::new(),
            their_clock: VectorClock::new(),
            pending_acks: HashSet::new(),
            last_sync: 0,
            enabled: true,
        }
    }

    /// Record that we synced
    pub fn record_sync(&mut self, their_clock: VectorClock) {
        self.their_clock = their_clock;
        self.last_sync = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
}

/// Sync manager - Coordinates synchronization with peers
pub struct SyncManager {
    /// Our node ID
    node_id: NodeId,
    /// Our current vector clock
    clock: RwLock<VectorClock>,
    /// Per-peer sync state
    peers: RwLock<HashMap<NodeId, PeerSyncState>>,
    /// Messages seen (for deduplication)
    seen_messages: RwLock<HashSet<MessageId>>,
    /// Outbound queue for each layer
    outbound_federated: RwLock<Vec<MessageEnvelope>>,
    outbound_shared: RwLock<HashMap<NodeId, Vec<MessageEnvelope>>>,
    /// Configuration
    config: SyncConfig,
}

/// Sync configuration
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Default TTL for federated messages
    pub default_ttl: u8,
    /// Maximum messages to keep in seen set
    pub max_seen_messages: usize,
    /// Maximum outbound queue size per peer
    pub max_outbound_queue: usize,
    /// How long to keep pending acks (seconds)
    pub ack_timeout_secs: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            default_ttl: 3,
            max_seen_messages: 10000,
            max_outbound_queue: 1000,
            ack_timeout_secs: 60,
        }
    }
}

impl SyncManager {
    /// Create a new sync manager
    pub fn new(node_id: NodeId) -> Self {
        Self::with_config(node_id, SyncConfig::default())
    }

    /// Create with custom config
    pub fn with_config(node_id: NodeId, config: SyncConfig) -> Self {
        Self {
            node_id,
            clock: RwLock::new(VectorClock::new()),
            peers: RwLock::new(HashMap::new()),
            seen_messages: RwLock::new(HashSet::new()),
            outbound_federated: RwLock::new(Vec::new()),
            outbound_shared: RwLock::new(HashMap::new()),
            config,
        }
    }

    /// Get current vector clock
    pub fn clock(&self) -> VectorClock {
        self.clock.read().clone()
    }

    /// Increment our clock (for sending)
    pub fn tick(&self) -> VectorClock {
        let mut clock = self.clock.write();
        clock.increment(self.node_id);
        clock.clone()
    }

    /// Merge received clock into ours
    pub fn merge_clock(&self, received: &VectorClock) {
        let mut clock = self.clock.write();
        clock.merge(received);
    }

    /// Process an incoming message
    pub fn process_incoming(&self, msg: &MessageEnvelope) -> SyncDecision {
        // Check if we've seen this message
        {
            let seen = self.seen_messages.read();
            if seen.contains(&msg.id) {
                return SyncDecision::AlreadySeen;
            }
        }

        // Mark as seen
        {
            let mut seen = self.seen_messages.write();
            seen.insert(msg.id);

            // Cleanup if too many
            if seen.len() > self.config.max_seen_messages {
                // Simple eviction: clear half (in production, use LRU)
                let to_remove: Vec<_> = seen.iter().take(seen.len() / 2).copied().collect();
                for id in to_remove {
                    seen.remove(&id);
                }
            }
        }

        // Merge their clock
        self.merge_clock(&msg.clock);

        // Determine if we should forward (Layer 2 only)
        let should_forward = msg.layer == DataLayer::Federated && msg.ttl > 0;

        SyncDecision::Process { should_forward }
    }

    /// Prepare a message for sending
    pub fn prepare_outbound(&self, mut msg: MessageEnvelope) -> MessageEnvelope {
        // Set our clock
        msg.clock = self.tick();
        msg.origin = self.node_id;

        // Set TTL if not set
        if msg.ttl == 0 && msg.layer == DataLayer::Federated {
            msg.ttl = self.config.default_ttl;
        }

        // Queue based on layer
        match msg.layer {
            DataLayer::Federated => {
                let mut queue = self.outbound_federated.write();
                if queue.len() < self.config.max_outbound_queue {
                    queue.push(msg.clone());
                }
            }
            DataLayer::Shared => {
                // For shared messages, queue to specific peer
                if let crate::message::MessagePayload::DirectMessage(ref dm) = msg.payload {
                    let mut queues = self.outbound_shared.write();
                    let queue = queues.entry(dm.to).or_insert_with(Vec::new);
                    if queue.len() < self.config.max_outbound_queue {
                        queue.push(msg.clone());
                    }
                }
            }
            DataLayer::Private => {
                // Private messages never sync
            }
        }

        msg
    }

    /// Get federated messages to gossip
    pub fn get_gossip_batch(&self, limit: usize) -> Vec<MessageEnvelope> {
        let mut queue = self.outbound_federated.write();
        let drain_count = limit.min(queue.len());
        let batch: Vec<_> = queue.drain(..drain_count).collect();
        batch
    }

    /// Get messages to sync with a specific peer
    pub fn get_peer_sync_batch(&self, peer_id: &NodeId, limit: usize) -> Vec<MessageEnvelope> {
        let mut queues = self.outbound_shared.write();
        if let Some(queue) = queues.get_mut(peer_id) {
            let batch: Vec<_> = queue.drain(..limit.min(queue.len())).collect();
            batch
        } else {
            Vec::new()
        }
    }

    /// Record that a peer acknowledged messages
    pub fn record_acks(&self, peer_id: &NodeId, message_ids: &[MessageId]) {
        let mut peers = self.peers.write();
        if let Some(state) = peers.get_mut(peer_id) {
            for id in message_ids {
                state.pending_acks.remove(id);
            }
        }
    }

    /// Add a peer for sync
    pub fn add_peer(&self, peer_id: NodeId) {
        let mut peers = self.peers.write();
        peers.entry(peer_id).or_insert_with(|| PeerSyncState::new(peer_id));
    }

    /// Remove a peer
    pub fn remove_peer(&self, peer_id: &NodeId) {
        self.peers.write().remove(peer_id);
        self.outbound_shared.write().remove(peer_id);
    }

    /// Get all peer IDs
    pub fn peer_ids(&self) -> Vec<NodeId> {
        self.peers.read().keys().copied().collect()
    }

    /// Check what's new since a given clock
    pub fn whats_new_since(&self, since: &VectorClock) -> WhatsNew {
        let current = self.clock.read();

        let mut new_from = Vec::new();
        for (node_id, &current_time) in &current.clocks {
            let since_time = since.get(node_id);
            if current_time > since_time {
                new_from.push((*node_id, since_time + 1, current_time));
            }
        }

        WhatsNew {
            ranges: new_from,
            current_clock: current.clone(),
        }
    }
}

/// Result of processing an incoming message
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncDecision {
    /// Message already seen, ignore
    AlreadySeen,
    /// Process the message
    Process {
        /// Whether to forward (gossip) this message
        should_forward: bool,
    },
}

/// Information about what's new since a clock
#[derive(Debug, Clone)]
pub struct WhatsNew {
    /// Ranges of new messages per node: (node_id, from_seq, to_seq)
    pub ranges: Vec<(NodeId, u64, u64)>,
    /// Current clock state
    pub current_clock: VectorClock,
}

impl WhatsNew {
    /// Check if there's anything new
    pub fn has_updates(&self) -> bool {
        !self.ranges.is_empty()
    }

    /// Total new message count (upper bound)
    pub fn total_new(&self) -> u64 {
        self.ranges.iter().map(|(_, from, to)| to - from + 1).sum()
    }
}

/// Sync errors
#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Peer not found: {0}")]
    PeerNotFound(NodeId),

    #[error("Sync rejected: {0}")]
    Rejected(String),

    #[error("Clock mismatch")]
    ClockMismatch,

    #[error("Message too old")]
    MessageTooOld,

    #[error("Rate limited")]
    RateLimited,
}

// ============================================================================
// CRDT Types for Conflict-Free Sync
// ============================================================================

/// G-Counter: Grow-only counter (for message counts, etc.)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GCounter {
    counts: HashMap<NodeId, u64>,
}

impl GCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment for a node
    pub fn increment(&mut self, node_id: NodeId) {
        *self.counts.entry(node_id).or_insert(0) += 1;
    }

    /// Get total value
    pub fn value(&self) -> u64 {
        self.counts.values().sum()
    }

    /// Merge with another counter
    pub fn merge(&mut self, other: &GCounter) {
        for (node_id, &count) in &other.counts {
            let entry = self.counts.entry(*node_id).or_insert(0);
            *entry = (*entry).max(count);
        }
    }
}

/// G-Set: Grow-only set (for membership, tags, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GSet<T: Eq + std::hash::Hash + Clone> {
    elements: HashSet<T>,
}

impl<T: Eq + std::hash::Hash + Clone> Default for GSet<T> {
    fn default() -> Self {
        Self {
            elements: HashSet::new(),
        }
    }
}

impl<T: Eq + std::hash::Hash + Clone> GSet<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an element
    pub fn add(&mut self, element: T) {
        self.elements.insert(element);
    }

    /// Check if element exists
    pub fn contains(&self, element: &T) -> bool {
        self.elements.contains(element)
    }

    /// Get all elements
    pub fn elements(&self) -> impl Iterator<Item = &T> {
        self.elements.iter()
    }

    /// Merge with another set
    pub fn merge(&mut self, other: &GSet<T>) {
        for element in &other.elements {
            self.elements.insert(element.clone());
        }
    }

    /// Get size
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
}

/// LWW-Register: Last-writer-wins register (for single values)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LwwRegister<T: Clone> {
    value: T,
    timestamp: u64,
    node_id: NodeId,
}

impl<T: Clone> LwwRegister<T> {
    /// Create a new register
    pub fn new(value: T, node_id: NodeId) -> Self {
        Self {
            value,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_micros() as u64,
            node_id,
        }
    }

    /// Get the value
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Set a new value
    pub fn set(&mut self, value: T, node_id: NodeId) {
        self.value = value;
        self.timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        self.node_id = node_id;
    }

    /// Merge with another register (last writer wins)
    pub fn merge(&mut self, other: &LwwRegister<T>) {
        if other.timestamp > self.timestamp
            || (other.timestamp == self.timestamp && other.node_id.0 > self.node_id.0)
        {
            self.value = other.value.clone();
            self.timestamp = other.timestamp;
            self.node_id = other.node_id;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_manager_tick() {
        let node_id = NodeId::from_bytes([1; 32]);
        let manager = SyncManager::new(node_id);

        let clock1 = manager.tick();
        assert_eq!(clock1.get(&node_id), 1);

        let clock2 = manager.tick();
        assert_eq!(clock2.get(&node_id), 2);
    }

    #[test]
    fn test_sync_decision_dedup() {
        let node_id = NodeId::from_bytes([1; 32]);
        let manager = SyncManager::new(node_id);

        let msg = MessageEnvelope::broadcast(
            node_id,
            VectorClock::new(),
            "test".to_string(),
            None,
        );

        // First time should process
        let decision1 = manager.process_incoming(&msg);
        assert!(matches!(decision1, SyncDecision::Process { .. }));

        // Second time should be already seen
        let decision2 = manager.process_incoming(&msg);
        assert_eq!(decision2, SyncDecision::AlreadySeen);
    }

    #[test]
    fn test_gcounter() {
        let node1 = NodeId::from_bytes([1; 32]);
        let node2 = NodeId::from_bytes([2; 32]);

        let mut counter1 = GCounter::new();
        counter1.increment(node1);
        counter1.increment(node1);

        let mut counter2 = GCounter::new();
        counter2.increment(node2);
        counter2.increment(node2);
        counter2.increment(node2);

        counter1.merge(&counter2);
        assert_eq!(counter1.value(), 5);
    }

    #[test]
    fn test_lww_register() {
        let node1 = NodeId::from_bytes([1; 32]);
        let node2 = NodeId::from_bytes([2; 32]);

        let mut reg1 = LwwRegister::new("value1".to_string(), node1);
        std::thread::sleep(std::time::Duration::from_millis(1));
        let reg2 = LwwRegister::new("value2".to_string(), node2);

        reg1.merge(&reg2);
        assert_eq!(reg1.value(), "value2");
    }
}
