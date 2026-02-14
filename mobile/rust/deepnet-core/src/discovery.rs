//! Discovery - Peer discovery mechanisms for Deep Net mesh
//!
//! Discovery is how nodes find each other. Multiple mechanisms are supported:
//! - Local (same device) - Unix socket / Named pipe paths
//! - mDNS (LAN) - Zero-configuration local network discovery
//! - Bluetooth LE - Mobile peer-to-peer
//! - DHT (Internet) - Kademlia-based global discovery
//! - Gossip - Learn about nodes from peers

use crate::identity::{NodeId, NodeManifest};
use crate::transport::NodeAddress;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;

/// Discovery mechanism type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiscoveryType {
    /// Same-device discovery (shared memory / pipes)
    Local,
    /// mDNS/DNS-SD for LAN discovery
    Mdns,
    /// Bluetooth Low Energy scanning
    Bluetooth,
    /// Kademlia DHT for internet-wide discovery
    Dht,
    /// Learn from gossip messages
    Gossip,
    /// Manual/static configuration
    Static,
}

/// Information about a discovered node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredNode {
    /// The node's identity
    pub node_id: NodeId,
    /// Node's public manifest (if available)
    pub manifest: Option<NodeManifest>,
    /// Known addresses for this node
    pub addresses: Vec<NodeAddress>,
    /// Unix timestamp when last seen
    pub last_seen: u64,
    /// How this node was discovered
    pub discovery_type: DiscoveryType,
    /// Number of relay hops away (0 = direct)
    pub hop_count: u8,
    /// Discovery-specific metadata
    pub metadata: HashMap<String, String>,
}

impl DiscoveredNode {
    /// Create a new discovered node
    pub fn new(node_id: NodeId, discovery_type: DiscoveryType) -> Self {
        Self {
            node_id,
            manifest: None,
            addresses: Vec::new(),
            last_seen: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            discovery_type,
            hop_count: 0,
            metadata: HashMap::new(),
        }
    }

    /// Update last seen timestamp
    pub fn touch(&mut self) {
        self.last_seen = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }

    /// Check if this node is stale (not seen recently)
    pub fn is_stale(&self, max_age_secs: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now - self.last_seen > max_age_secs
    }

    /// Merge with another discovered node record
    pub fn merge(&mut self, other: &DiscoveredNode) {
        // Take newer timestamp
        if other.last_seen > self.last_seen {
            self.last_seen = other.last_seen;
        }

        // Take manifest if we don't have one
        if self.manifest.is_none() && other.manifest.is_some() {
            self.manifest = other.manifest.clone();
        }

        // Merge addresses (deduplicate by serialized form)
        for addr in &other.addresses {
            let addr_bytes = bincode::serialize(addr).unwrap_or_default();
            let exists = self.addresses.iter().any(|a| {
                bincode::serialize(a).unwrap_or_default() == addr_bytes
            });
            if !exists {
                self.addresses.push(addr.clone());
            }
        }

        // Merge metadata
        for (k, v) in &other.metadata {
            self.metadata.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
}

/// Discovery trait - Interface for all discovery mechanisms
#[async_trait]
pub trait Discovery: Send + Sync {
    /// Announce our presence to the network
    async fn announce(&self, manifest: &NodeManifest) -> Result<(), DiscoveryError>;

    /// Stop announcing (going offline)
    async fn unannounce(&self) -> Result<(), DiscoveryError>;

    /// Discover nearby/available nodes
    async fn discover(&self) -> Result<Vec<DiscoveredNode>, DiscoveryError>;

    /// Resolve a specific node ID to addresses
    async fn resolve(&self, node_id: &NodeId) -> Result<Option<DiscoveredNode>, DiscoveryError>;

    /// Get the discovery type
    fn discovery_type(&self) -> DiscoveryType;

    /// Check if this discovery mechanism is available
    fn is_available(&self) -> bool;
}

/// Discovery manager - Coordinates multiple discovery mechanisms
pub struct DiscoveryManager {
    discoveries: Vec<Box<dyn Discovery>>,
    known_nodes: parking_lot::RwLock<HashMap<NodeId, DiscoveredNode>>,
    /// Maximum age before a node is considered stale
    stale_threshold_secs: u64,
}

impl DiscoveryManager {
    /// Create a new discovery manager
    pub fn new() -> Self {
        Self {
            discoveries: Vec::new(),
            known_nodes: parking_lot::RwLock::new(HashMap::new()),
            stale_threshold_secs: 300, // 5 minutes
        }
    }

    /// Create with custom stale threshold
    pub fn with_stale_threshold(stale_threshold: Duration) -> Self {
        Self {
            discoveries: Vec::new(),
            known_nodes: parking_lot::RwLock::new(HashMap::new()),
            stale_threshold_secs: stale_threshold.as_secs(),
        }
    }

    /// Register a discovery mechanism
    pub fn register(&mut self, discovery: Box<dyn Discovery>) {
        self.discoveries.push(discovery);
    }

    /// Announce our presence on all mechanisms
    pub async fn announce_all(&self, manifest: &NodeManifest) -> Vec<Result<(), DiscoveryError>> {
        let mut results = Vec::new();
        for discovery in &self.discoveries {
            if discovery.is_available() {
                results.push(discovery.announce(manifest).await);
            }
        }
        results
    }

    /// Stop announcing on all mechanisms
    pub async fn unannounce_all(&self) -> Vec<Result<(), DiscoveryError>> {
        let mut results = Vec::new();
        for discovery in &self.discoveries {
            results.push(discovery.unannounce().await);
        }
        results
    }

    /// Discover nodes from all mechanisms
    pub async fn discover_all(&self) -> Vec<DiscoveredNode> {
        let mut all_nodes: HashMap<NodeId, DiscoveredNode> = HashMap::new();

        for discovery in &self.discoveries {
            if discovery.is_available() {
                if let Ok(nodes) = discovery.discover().await {
                    for node in nodes {
                        all_nodes
                            .entry(node.node_id)
                            .and_modify(|existing| existing.merge(&node))
                            .or_insert(node);
                    }
                }
            }
        }

        // Update known nodes cache
        {
            let mut known = self.known_nodes.write();
            for (id, node) in &all_nodes {
                known
                    .entry(*id)
                    .and_modify(|existing| existing.merge(node))
                    .or_insert_with(|| node.clone());
            }
        }

        all_nodes.into_values().collect()
    }

    /// Resolve a node ID to addresses
    pub async fn resolve(&self, node_id: &NodeId) -> Option<DiscoveredNode> {
        // Check cache first
        {
            let known = self.known_nodes.read();
            if let Some(node) = known.get(node_id) {
                if !node.is_stale(self.stale_threshold_secs) {
                    return Some(node.clone());
                }
            }
        }

        // Try all discovery mechanisms
        for discovery in &self.discoveries {
            if discovery.is_available() {
                if let Ok(Some(node)) = discovery.resolve(node_id).await {
                    // Update cache
                    let mut known = self.known_nodes.write();
                    known
                        .entry(*node_id)
                        .and_modify(|existing| existing.merge(&node))
                        .or_insert_with(|| node.clone());
                    return Some(node);
                }
            }
        }

        None
    }

    /// Get all known nodes (from cache)
    pub fn known_nodes(&self) -> Vec<DiscoveredNode> {
        self.known_nodes.read().values().cloned().collect()
    }

    /// Get non-stale known nodes
    pub fn active_nodes(&self) -> Vec<DiscoveredNode> {
        self.known_nodes
            .read()
            .values()
            .filter(|n| !n.is_stale(self.stale_threshold_secs))
            .cloned()
            .collect()
    }

    /// Clean up stale nodes from cache
    pub fn cleanup_stale(&self) {
        let mut known = self.known_nodes.write();
        known.retain(|_, node| !node.is_stale(self.stale_threshold_secs));
    }

    /// Add a node manually (from gossip or static config)
    pub fn add_node(&self, node: DiscoveredNode) {
        let mut known = self.known_nodes.write();
        known
            .entry(node.node_id)
            .and_modify(|existing| existing.merge(&node))
            .or_insert(node);
    }

    /// Get available discovery types
    pub fn available_types(&self) -> Vec<DiscoveryType> {
        self.discoveries
            .iter()
            .filter(|d| d.is_available())
            .map(|d| d.discovery_type())
            .collect()
    }
}

impl Default for DiscoveryManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Discovery errors
#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("Discovery mechanism not available")]
    NotAvailable,

    #[error("Announcement failed: {0}")]
    AnnouncementFailed(String),

    #[error("Discovery failed: {0}")]
    DiscoveryFailed(String),

    #[error("Resolution failed: {0}")]
    ResolutionFailed(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Timeout")]
    Timeout,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

// ============================================================================
// mDNS Discovery Implementation
// ============================================================================

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::sync::Arc;

/// Deep Net service type for mDNS
pub const DEEPNET_SERVICE_TYPE: &str = "_deepnet._tcp.local.";

/// Default mDNS port
pub const DEEPNET_MDNS_PORT: u16 = 31415;

/// mDNS/DNS-SD discovery for LAN nodes
pub struct MdnsDiscovery {
    service_type: String,
    port: u16,
    daemon: Option<ServiceDaemon>,
    our_instance_name: parking_lot::RwLock<Option<String>>,
    discovered: parking_lot::RwLock<HashMap<String, DiscoveredNode>>,
}

impl MdnsDiscovery {
    /// Create a new mDNS discovery instance
    pub fn new() -> Self {
        Self::with_port(DEEPNET_MDNS_PORT)
    }

    /// Create with custom port
    pub fn with_port(port: u16) -> Self {
        Self {
            service_type: DEEPNET_SERVICE_TYPE.to_string(),
            port,
            daemon: None,
            our_instance_name: parking_lot::RwLock::new(None),
            discovered: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    /// Create with custom service type
    pub fn with_service_type(service_type: String, port: u16) -> Self {
        Self {
            service_type,
            port,
            daemon: None,
            our_instance_name: parking_lot::RwLock::new(None),
            discovered: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    /// Start the mDNS daemon and browse for services
    pub fn start(&mut self) -> Result<(), DiscoveryError> {
        if self.daemon.is_some() {
            return Ok(()); // Already started
        }

        let daemon = ServiceDaemon::new()
            .map_err(|e| DiscoveryError::DiscoveryFailed(format!("Failed to create mDNS daemon: {}", e)))?;

        // Start browsing for Deep Net services
        let receiver = daemon.browse(&self.service_type)
            .map_err(|e| DiscoveryError::DiscoveryFailed(format!("Failed to browse: {}", e)))?;

        // Spawn a task to handle discovered services
        let discovered = Arc::new(parking_lot::RwLock::new(HashMap::new()));
        let discovered_clone = discovered.clone();

        std::thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        tracing::info!("mDNS: Discovered service: {}", info.get_fullname());

                        // Extract node_id from TXT record if available
                        let properties = info.get_properties();
                        if let Some(node_id_hex) = properties.get("node_id") {
                            let node_id_str = node_id_hex.val_str();
                            if let Ok(node_id) = NodeId::from_hex(node_id_str) {
                                let mut node = DiscoveredNode::new(node_id, DiscoveryType::Mdns);

                                // Add addresses (they're already IpAddr)
                                for addr in info.get_addresses() {
                                    node.addresses.push(NodeAddress::Tcp {
                                        addr: std::net::SocketAddr::new(
                                            (*addr).into(),
                                            info.get_port(),
                                        ),
                                    });
                                }

                                // Extract display name
                                if let Some(name) = properties.get("name") {
                                    let name_str = name.val_str();
                                    node.metadata.insert("display_name".to_string(), name_str.to_string());
                                }

                                discovered_clone.write().insert(info.get_fullname().to_string(), node);
                            }
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        tracing::info!("mDNS: Service removed: {}", fullname);
                        discovered_clone.write().remove(&fullname);
                    }
                    ServiceEvent::ServiceFound(_, name) => {
                        tracing::debug!("mDNS: Service found (not yet resolved): {}", name);
                    }
                    ServiceEvent::SearchStarted(_) => {
                        tracing::debug!("mDNS: Search started");
                    }
                    ServiceEvent::SearchStopped(_) => {
                        tracing::debug!("mDNS: Search stopped");
                    }
                }
            }
        });

        // Store the shared discovered map
        *self.discovered.write() = discovered.read().clone();

        self.daemon = Some(daemon);
        Ok(())
    }

    /// Stop the mDNS daemon
    pub fn stop(&mut self) {
        if let Some(daemon) = self.daemon.take() {
            let _ = daemon.shutdown();
        }
    }

    /// Register our service
    fn register_service(&self, manifest: &NodeManifest) -> Result<(), DiscoveryError> {
        let daemon = self.daemon.as_ref()
            .ok_or_else(|| DiscoveryError::NotAvailable)?;

        // Create instance name from node_id short form
        let instance_name = format!("deepnet-{}", manifest.node_id.short());

        // Get local hostname
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "localhost".to_string());

        // Build TXT record properties
        let node_id_hex = manifest.node_id.to_hex();
        let version_str = manifest.protocol_version.to_string();
        let properties = [
            ("node_id", node_id_hex.as_str()),
            ("name", manifest.display_name.as_str()),
            ("version", version_str.as_str()),
        ];

        let service_info = ServiceInfo::new(
            &self.service_type,
            &instance_name,
            &format!("{}.local.", hostname),
            "",  // Empty string lets the library pick the IP
            self.port,
            &properties[..],
        ).map_err(|e| DiscoveryError::AnnouncementFailed(format!("Failed to create service info: {}", e)))?;

        daemon.register(service_info)
            .map_err(|e| DiscoveryError::AnnouncementFailed(format!("Failed to register: {}", e)))?;

        *self.our_instance_name.write() = Some(instance_name);

        Ok(())
    }
}

impl Default for MdnsDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MdnsDiscovery {
    fn drop(&mut self) {
        self.stop();
    }
}

#[async_trait]
impl Discovery for MdnsDiscovery {
    async fn announce(&self, manifest: &NodeManifest) -> Result<(), DiscoveryError> {
        // Note: For full async support, would need to start daemon if not started
        // For now, assume start() was called
        if self.daemon.is_none() {
            return Err(DiscoveryError::NotAvailable);
        }

        self.register_service(manifest)?;

        tracing::info!(
            "mDNS: Announcing {} as {} on port {}",
            manifest.display_name,
            manifest.node_id,
            self.port
        );
        Ok(())
    }

    async fn unannounce(&self) -> Result<(), DiscoveryError> {
        if let Some(daemon) = &self.daemon {
            if let Some(instance_name) = self.our_instance_name.read().as_ref() {
                let fullname = format!("{}.{}", instance_name, self.service_type);
                let _ = daemon.unregister(&fullname);
            }
        }
        tracing::info!("mDNS: Stopped announcement");
        Ok(())
    }

    async fn discover(&self) -> Result<Vec<DiscoveredNode>, DiscoveryError> {
        if self.daemon.is_none() {
            return Err(DiscoveryError::NotAvailable);
        }

        let nodes: Vec<DiscoveredNode> = self.discovered.read().values().cloned().collect();
        tracing::debug!("mDNS: Found {} nodes on {}", nodes.len(), self.service_type);
        Ok(nodes)
    }

    async fn resolve(&self, node_id: &NodeId) -> Result<Option<DiscoveredNode>, DiscoveryError> {
        let discovered = self.discovered.read();
        for node in discovered.values() {
            if node.node_id == *node_id {
                return Ok(Some(node.clone()));
            }
        }
        Ok(None)
    }

    fn discovery_type(&self) -> DiscoveryType {
        DiscoveryType::Mdns
    }

    fn is_available(&self) -> bool {
        self.daemon.is_some()
    }
}

// ============================================================================
// Static Discovery (for testing and manual configuration)
// ============================================================================

/// Static/manual discovery for known nodes
pub struct StaticDiscovery {
    nodes: parking_lot::RwLock<Vec<DiscoveredNode>>,
}

impl StaticDiscovery {
    /// Create a new static discovery instance
    pub fn new() -> Self {
        Self {
            nodes: parking_lot::RwLock::new(Vec::new()),
        }
    }

    /// Add a known node
    pub fn add_node(&self, node: DiscoveredNode) {
        self.nodes.write().push(node);
    }

    /// Add a node by address
    pub fn add_address(&self, node_id: NodeId, address: NodeAddress) {
        let mut node = DiscoveredNode::new(node_id, DiscoveryType::Static);
        node.addresses.push(address);
        self.add_node(node);
    }
}

impl Default for StaticDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Discovery for StaticDiscovery {
    async fn announce(&self, _manifest: &NodeManifest) -> Result<(), DiscoveryError> {
        // Static discovery doesn't announce
        Ok(())
    }

    async fn unannounce(&self) -> Result<(), DiscoveryError> {
        Ok(())
    }

    async fn discover(&self) -> Result<Vec<DiscoveredNode>, DiscoveryError> {
        Ok(self.nodes.read().clone())
    }

    async fn resolve(&self, node_id: &NodeId) -> Result<Option<DiscoveredNode>, DiscoveryError> {
        Ok(self.nodes.read().iter().find(|n| n.node_id == *node_id).cloned())
    }

    fn discovery_type(&self) -> DiscoveryType {
        DiscoveryType::Static
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovered_node_stale() {
        let node_id = NodeId::from_bytes([1; 32]);
        let mut node = DiscoveredNode::new(node_id, DiscoveryType::Static);

        // Should not be stale immediately
        assert!(!node.is_stale(60));

        // Artificially age the node
        node.last_seen -= 120;
        assert!(node.is_stale(60));
    }

    #[test]
    fn test_discovered_node_merge() {
        let node_id = NodeId::from_bytes([1; 32]);
        let mut node1 = DiscoveredNode::new(node_id, DiscoveryType::Mdns);
        node1.addresses.push(NodeAddress::Tcp {
            addr: "192.168.1.1:8080".parse().unwrap(),
        });

        let mut node2 = DiscoveredNode::new(node_id, DiscoveryType::Static);
        node2.addresses.push(NodeAddress::Tcp {
            addr: "192.168.1.2:8080".parse().unwrap(),
        });
        node2.last_seen += 100;

        node1.merge(&node2);

        // Should have both addresses
        assert_eq!(node1.addresses.len(), 2);
        // Should have newer timestamp
        assert_eq!(node1.last_seen, node2.last_seen);
    }

    #[tokio::test]
    async fn test_static_discovery() {
        let discovery = StaticDiscovery::new();

        let node_id = NodeId::from_bytes([1; 32]);
        discovery.add_address(
            node_id,
            NodeAddress::Tcp {
                addr: "127.0.0.1:8080".parse().unwrap(),
            },
        );

        let nodes = discovery.discover().await.unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].node_id, node_id);

        let resolved = discovery.resolve(&node_id).await.unwrap();
        assert!(resolved.is_some());
    }
}
