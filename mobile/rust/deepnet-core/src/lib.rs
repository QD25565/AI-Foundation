//! Deep Net Core - Distributed Mesh Federation Protocol
//!
//! This crate provides the core protocol, types, and traits for the Deep Net
//! distributed mesh network. Each Teambook instance is a sovereign node in the
//! mesh - there is no central server.
//!
//! # Architecture
//!
//! ## Data Layers
//!
//! - **Layer 0 (Private)**: Data that never leaves the node
//! - **Layer 1 (Shared)**: Data synced on-demand with explicit peers
//! - **Layer 2 (Federated)**: Data propagated via TTL-limited gossip
//!
//! ## Core Components
//!
//! - [`identity`]: Node identity based on Ed25519 keys
//! - [`message`]: Message types with vector clocks for causality
//! - [`transport`]: Abstraction over multiple connection types
//! - [`discovery`]: Peer discovery mechanisms (mDNS, DHT, etc.)
//! - [`sync`]: Synchronization protocol with CRDTs
//! - [`store`]: Storage trait and implementations
//!
//! # Example
//!
//! ```rust,ignore
//! use deepnet_core::{
//!     identity::NodeIdentity,
//!     message::{MessageEnvelope, VectorClock},
//!     store::MemoryStore,
//!     sync::SyncManager,
//! };
//!
//! // Create a new node identity
//! let identity = NodeIdentity::generate("My Node".to_string());
//!
//! // Create sync manager
//! let sync_manager = SyncManager::new(identity.node_id());
//!
//! // Create a broadcast message
//! let clock = sync_manager.tick();
//! let msg = MessageEnvelope::broadcast(
//!     identity.node_id(),
//!     clock,
//!     "Hello, mesh!".to_string(),
//!     None,
//! );
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod discovery;
pub mod identity;
pub mod message;
pub mod quic;
pub mod store;
pub mod sync;
pub mod transport;

// Re-export commonly used types
pub use identity::{Capability, NodeId, NodeIdentity, NodeManifest};
pub use message::{
    BroadcastMessage, DataLayer, DirectMessage, MessageEnvelope, MessageId, MessagePayload,
    PresenceStatus, PresenceUpdate, VectorClock,
};
pub use store::{MemoryStore, Store, StoreError, StoreStats};
pub use sync::{GCounter, GSet, LwwRegister, SyncConfig, SyncDecision, SyncManager};
pub use transport::{
    BandwidthTier, Connection, ConnectionMetrics, Listener, NodeAddress, Transport,
    TransportError, TransportManager, TransportType,
};
pub use discovery::{
    DiscoveredNode, Discovery, DiscoveryError, DiscoveryManager, DiscoveryType,
    MdnsDiscovery, StaticDiscovery, DEEPNET_SERVICE_TYPE, DEEPNET_MDNS_PORT,
};
pub use quic::{QuicTransport, QuicConnection, DEFAULT_QUIC_PORT};

/// Protocol version
pub const PROTOCOL_VERSION: u32 = 1;

/// Default gossip TTL
pub const DEFAULT_GOSSIP_TTL: u8 = 3;

/// Deep Net service type for mDNS
pub const MDNS_SERVICE_TYPE: &str = "_deepnet._tcp.local.";

/// Deep Net port for WebSocket fallback
pub const DEFAULT_WS_PORT: u16 = 31416;

/// Prelude module for convenient imports
pub mod prelude {
    //! Convenient imports for common use cases
    pub use crate::identity::{NodeId, NodeIdentity, NodeManifest};
    pub use crate::message::{DataLayer, MessageEnvelope, VectorClock};
    pub use crate::store::{MemoryStore, Store};
    pub use crate::sync::SyncManager;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exports() {
        // Just verify exports compile
        let _: NodeId = NodeId::from_bytes([0; 32]);
        let _: VectorClock = VectorClock::new();
        let _: DataLayer = DataLayer::Federated;
    }

    #[test]
    fn test_end_to_end_message() {
        // Create two nodes
        let node1 = NodeIdentity::generate("Node 1".to_string());
        let node2 = NodeIdentity::generate("Node 2".to_string());

        // Create sync managers
        let sync1 = SyncManager::new(node1.node_id());
        let sync2 = SyncManager::new(node2.node_id());

        // Node 1 creates a broadcast
        let clock = sync1.tick();
        let msg = MessageEnvelope::broadcast(
            node1.node_id(),
            clock,
            "Hello from Node 1!".to_string(),
            Some("general".to_string()),
        );

        // Node 2 receives it
        let decision = sync2.process_incoming(&msg);
        assert!(matches!(decision, SyncDecision::Process { should_forward: true }));

        // Verify deduplication
        let decision2 = sync2.process_incoming(&msg);
        assert_eq!(decision2, SyncDecision::AlreadySeen);
    }

    #[tokio::test]
    async fn test_store_integration() {
        let store = MemoryStore::new();
        let node = NodeIdentity::generate("Test".to_string());

        // Store identity
        store.store_identity(&node.to_bytes()).await.unwrap();

        // Create and store a message
        let msg = MessageEnvelope::broadcast(
            node.node_id(),
            VectorClock::new(),
            "Test broadcast".to_string(),
            None,
        );
        store.store_message(&msg).await.unwrap();

        // Verify stats
        let stats = store.stats().await.unwrap();
        assert_eq!(stats.message_count, 1);
        assert_eq!(stats.broadcast_count, 1);
    }
}
