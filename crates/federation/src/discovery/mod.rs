//! Discovery mechanisms for finding federation peers
//!
//! Supports multiple discovery methods:
//! - mDNS for LAN/WiFi discovery
//! - Bluetooth LE for proximity discovery
//! - Passkey for manual pairing

pub mod mdns;
#[cfg(feature = "bluetooth")]
pub mod bluetooth;
pub mod passkey;

use crate::{Endpoint, Result, TransportType};
use std::time::Duration;
use tokio::sync::mpsc;

/// A discovered peer
#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    /// Node ID (if known)
    pub node_id: Option<String>,

    /// Display name (if advertised)
    pub display_name: Option<String>,

    /// Full Ed25519 public key hex (64 chars).
    /// Used by the node binary to establish iroh QUIC connections.
    pub pubkey_hex: Option<String>,

    /// How to reach this peer
    pub endpoint: Endpoint,

    /// Discovery method used
    pub discovery_type: DiscoveryType,

    /// Signal strength (for proximity sorting)
    pub signal_strength: Option<i32>,

    /// When discovered
    pub discovered_at: std::time::Instant,
}

impl DiscoveredPeer {
    /// Create a new discovered peer
    pub fn new(endpoint: Endpoint, discovery_type: DiscoveryType) -> Self {
        Self {
            node_id: None,
            display_name: None,
            pubkey_hex: None,
            endpoint,
            discovery_type,
            signal_strength: None,
            discovered_at: std::time::Instant::now(),
        }
    }

    /// Add node ID
    pub fn with_node_id(mut self, id: &str) -> Self {
        self.node_id = Some(id.to_string());
        self
    }

    /// Add display name
    pub fn with_name(mut self, name: &str) -> Self {
        self.display_name = Some(name.to_string());
        self
    }

    /// Add Ed25519 public key (hex-encoded, 64 chars)
    pub fn with_pubkey(mut self, pubkey_hex: &str) -> Self {
        self.pubkey_hex = Some(pubkey_hex.to_string());
        self
    }

    /// Add signal strength
    pub fn with_signal(mut self, rssi: i32) -> Self {
        self.signal_strength = Some(rssi);
        self
    }
}

/// How a peer was discovered
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryType {
    /// mDNS/DNS-SD on local network
    Mdns,
    /// Bluetooth Low Energy
    BluetoothLe,
    /// Classic Bluetooth
    BluetoothClassic,
    /// Manual passkey pairing
    Passkey,
    /// Direct address (no discovery)
    Direct,
}

impl From<DiscoveryType> for TransportType {
    fn from(dt: DiscoveryType) -> Self {
        match dt {
            DiscoveryType::Mdns => TransportType::Mdns,
            DiscoveryType::BluetoothLe => TransportType::BluetoothLe,
            DiscoveryType::BluetoothClassic => TransportType::BluetoothClassic,
            DiscoveryType::Passkey => TransportType::Passkey,
            DiscoveryType::Direct => TransportType::QuicInternet,
        }
    }
}

/// Discovery event
#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    /// New peer discovered
    PeerFound(DiscoveredPeer),

    /// Peer is no longer available
    PeerLost {
        node_id: Option<String>,
        endpoint: Endpoint,
    },

    /// Discovery error (non-fatal)
    Error(String),

    /// Discovery started
    Started(DiscoveryType),

    /// Discovery stopped
    Stopped(DiscoveryType),
}

/// Configuration for discovery
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Enable mDNS discovery
    pub enable_mdns: bool,

    /// Enable Bluetooth LE discovery
    pub enable_ble: bool,

    /// Enable classic Bluetooth discovery
    pub enable_bluetooth_classic: bool,

    /// mDNS service type
    pub mdns_service_type: String,

    /// BLE service UUID
    pub ble_service_uuid: String,

    /// How long to scan for Bluetooth devices
    pub bluetooth_scan_duration: Duration,

    /// How often to refresh discovery
    pub refresh_interval: Duration,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            enable_mdns: true,
            enable_ble: true,
            enable_bluetooth_classic: false, // Usually want BLE only
            mdns_service_type: "_teambook._tcp.local.".to_string(),
            ble_service_uuid: "a1f0-cafe-beef-0001".to_string(),
            bluetooth_scan_duration: Duration::from_secs(10),
            refresh_interval: Duration::from_secs(30),
        }
    }
}

/// Unified discovery manager
pub struct DiscoveryManager {
    /// Configuration
    config: DiscoveryConfig,

    /// Our node ID (for filtering self-discovery)
    local_node_id: String,

    /// Event sender
    event_tx: mpsc::Sender<DiscoveryEvent>,

    /// Event receiver
    event_rx: mpsc::Receiver<DiscoveryEvent>,

    /// Currently known peers
    known_peers: Vec<DiscoveredPeer>,

    /// Is discovery running?
    running: bool,
}

impl DiscoveryManager {
    /// Create a new discovery manager
    pub fn new(local_node_id: &str, config: DiscoveryConfig) -> Self {
        let (event_tx, event_rx) = mpsc::channel(100);

        Self {
            config,
            local_node_id: local_node_id.to_string(),
            event_tx,
            event_rx,
            known_peers: Vec::new(),
            running: false,
        }
    }

    /// Get the event sender for discovery backends
    pub fn event_sender(&self) -> mpsc::Sender<DiscoveryEvent> {
        self.event_tx.clone()
    }

    /// Start discovery on all configured methods
    pub async fn start(&mut self) -> Result<()> {
        if self.running {
            return Ok(());
        }

        self.running = true;

        // Start mDNS if enabled
        if self.config.enable_mdns {
            let tx = self.event_tx.clone();
            let service_type = self.config.mdns_service_type.clone();
            let node_id = self.local_node_id.clone();

            tokio::spawn(async move {
                if let Err(e) = mdns::start_mdns_discovery(&service_type, &node_id, tx).await {
                    eprintln!("mDNS discovery error: {}", e);
                }
            });
        }

        // BLE discovery would be started similarly
        // For now, we log that it's configured but not yet implemented
        if self.config.enable_ble {
            let _ = self.event_tx.send(DiscoveryEvent::Started(DiscoveryType::BluetoothLe)).await;
        }

        Ok(())
    }

    /// Stop all discovery
    pub async fn stop(&mut self) -> Result<()> {
        self.running = false;
        // Send stop events
        let _ = self.event_tx.send(DiscoveryEvent::Stopped(DiscoveryType::Mdns)).await;
        let _ = self.event_tx.send(DiscoveryEvent::Stopped(DiscoveryType::BluetoothLe)).await;
        Ok(())
    }

    /// Wait for next discovery event (event-driven, blocks until event received)
    pub async fn next_event(&mut self) -> Option<DiscoveryEvent> {
        self.event_rx.recv().await
    }

    /// Get currently known peers
    pub fn known_peers(&self) -> &[DiscoveredPeer] {
        &self.known_peers
    }

    /// Add a peer from an event
    pub fn add_peer(&mut self, peer: DiscoveredPeer) {
        // Don't add ourselves
        if let Some(ref id) = peer.node_id {
            if id == &self.local_node_id {
                return;
            }
        }

        // Check if we already know this peer (by endpoint)
        let exists = self.known_peers.iter().any(|p| p.endpoint == peer.endpoint);
        if !exists {
            self.known_peers.push(peer);
        }
    }

    /// Remove a peer
    pub fn remove_peer(&mut self, endpoint: &Endpoint) {
        self.known_peers.retain(|p| &p.endpoint != endpoint);
    }

    /// Get peers sorted by signal strength (best first)
    pub fn peers_by_signal(&self) -> Vec<&DiscoveredPeer> {
        let mut peers: Vec<_> = self.known_peers.iter().collect();
        peers.sort_by(|a, b| {
            match (&b.signal_strength, &a.signal_strength) {
                (Some(b_sig), Some(a_sig)) => b_sig.cmp(a_sig),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });
        peers
    }

    /// Clear stale peers (not seen recently)
    pub fn cleanup_stale(&mut self, max_age: Duration) {
        let now = std::time::Instant::now();
        self.known_peers.retain(|p| {
            now.duration_since(p.discovered_at) < max_age
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovered_peer() {
        let endpoint = Endpoint::mdns("test-service");
        let peer = DiscoveredPeer::new(endpoint, DiscoveryType::Mdns)
            .with_node_id("node-123")
            .with_name("Test Node")
            .with_signal(-50);

        assert_eq!(peer.node_id, Some("node-123".to_string()));
        assert_eq!(peer.display_name, Some("Test Node".to_string()));
        assert_eq!(peer.signal_strength, Some(-50));
    }

    #[test]
    fn test_discovery_config_default() {
        let config = DiscoveryConfig::default();
        assert!(config.enable_mdns);
        assert!(config.enable_ble);
        assert!(!config.enable_bluetooth_classic);
    }
}
