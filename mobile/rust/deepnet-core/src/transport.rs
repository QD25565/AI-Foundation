//! Transport Abstraction - Unified interface for all connection types
//!
//! Deep Net supports multiple transport mechanisms:
//! - Local (Unix socket / Named pipe) - Same device
//! - LAN (TCP/QUIC over mDNS) - Local network
//! - Bluetooth LE - Mobile peer-to-peer
//! - WiFi Direct - High bandwidth local
//! - Internet (QUIC with relay fallback) - Global
//!
//! All transports implement the same traits for uniform handling.

use crate::identity::NodeId;
use crate::message::MessageEnvelope;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::SocketAddr;
use thiserror::Error;

/// Transport type identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransportType {
    /// Same device communication (Unix socket / Named pipe)
    Local,
    /// LAN connection (TCP/QUIC, discovered via mDNS)
    Lan,
    /// Bluetooth Low Energy
    Bluetooth,
    /// WiFi Direct / P2P
    WifiDirect,
    /// Internet QUIC connection
    Internet,
    /// Relayed connection through a relay node
    Relay,
}

impl fmt::Display for TransportType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportType::Local => write!(f, "local"),
            TransportType::Lan => write!(f, "lan"),
            TransportType::Bluetooth => write!(f, "bluetooth"),
            TransportType::WifiDirect => write!(f, "wifi-direct"),
            TransportType::Internet => write!(f, "internet"),
            TransportType::Relay => write!(f, "relay"),
        }
    }
}

/// Address for connecting to a node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeAddress {
    /// Local socket path
    Local { path: String },
    /// TCP/IP address
    Tcp { addr: SocketAddr },
    /// QUIC address with optional server name
    Quic { addr: SocketAddr, server_name: Option<String> },
    /// Bluetooth device address
    Bluetooth { device_id: String, service_uuid: String },
    /// WiFi Direct group info
    WifiDirect { group_owner: String, passphrase: Option<String> },
    /// Relay through another node
    Relay { relay_node: NodeId, target_node: NodeId },
}

impl NodeAddress {
    /// Get the transport type for this address
    pub fn transport_type(&self) -> TransportType {
        match self {
            NodeAddress::Local { .. } => TransportType::Local,
            NodeAddress::Tcp { .. } => TransportType::Lan,
            NodeAddress::Quic { .. } => TransportType::Internet,
            NodeAddress::Bluetooth { .. } => TransportType::Bluetooth,
            NodeAddress::WifiDirect { .. } => TransportType::WifiDirect,
            NodeAddress::Relay { .. } => TransportType::Relay,
        }
    }
}

/// Connection quality metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectionMetrics {
    /// Round-trip latency in milliseconds
    pub latency_ms: u32,
    /// Estimated bandwidth tier
    pub bandwidth: BandwidthTier,
    /// Packet loss percentage (0-100)
    pub packet_loss: u8,
    /// Whether connection is encrypted
    pub encrypted: bool,
    /// Number of relay hops (0 = direct)
    pub hops: u8,
}

/// Bandwidth classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BandwidthTier {
    /// Very slow (< 100 Kbps) - Bluetooth LE, bad cellular
    VeryLow,
    /// Slow (100 Kbps - 1 Mbps) - Typical Bluetooth, 3G
    Low,
    #[default]
    /// Medium (1-10 Mbps) - WiFi, 4G
    Medium,
    /// High (10-100 Mbps) - Good WiFi, 5G
    High,
    /// Very high (> 100 Mbps) - Ethernet, local
    VeryHigh,
}

/// Transport trait - Factory for creating connections
#[async_trait]
pub trait Transport: Send + Sync {
    /// Connect to a node at the given address
    async fn connect(&self, addr: &NodeAddress) -> Result<Box<dyn Connection>, TransportError>;

    /// Start listening for incoming connections
    async fn listen(&self) -> Result<Box<dyn Listener>, TransportError>;

    /// Get the transport type
    fn transport_type(&self) -> TransportType;

    /// Check if this transport can handle the given address
    fn can_handle(&self, addr: &NodeAddress) -> bool;
}

/// Listener for incoming connections
#[async_trait]
pub trait Listener: Send + Sync {
    /// Accept the next incoming connection
    async fn accept(&mut self) -> Result<Box<dyn Connection>, TransportError>;

    /// Get the local address being listened on
    fn local_addr(&self) -> Option<NodeAddress>;

    /// Stop listening
    async fn close(&mut self) -> Result<(), TransportError>;
}

/// Active connection to another node
#[async_trait]
pub trait Connection: Send + Sync {
    /// Send a message
    async fn send(&mut self, msg: &MessageEnvelope) -> Result<(), TransportError>;

    /// Receive a message (blocking until available)
    async fn recv(&mut self) -> Result<MessageEnvelope, TransportError>;

    /// Try to receive a message (non-blocking)
    async fn try_recv(&mut self) -> Result<Option<MessageEnvelope>, TransportError>;

    /// Get the peer's node ID
    fn peer_id(&self) -> &NodeId;

    /// Get the transport type
    fn transport_type(&self) -> TransportType;

    /// Get current connection metrics
    fn metrics(&self) -> ConnectionMetrics;

    /// Check if connection is still alive
    fn is_alive(&self) -> bool;

    /// Close the connection
    async fn close(&mut self) -> Result<(), TransportError>;

    /// Send a ping and measure latency
    async fn ping(&mut self) -> Result<u32, TransportError>;
}

/// Transport-related errors
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Connection refused")]
    ConnectionRefused,

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Connection timeout")]
    Timeout,

    #[error("Address not supported by this transport")]
    UnsupportedAddress,

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("TLS error: {0}")]
    TlsError(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("Relay error: {0}")]
    RelayError(String),
}

/// Transport manager - Coordinates multiple transports
pub struct TransportManager {
    transports: Vec<Box<dyn Transport>>,
}

impl TransportManager {
    /// Create a new transport manager
    pub fn new() -> Self {
        Self {
            transports: Vec::new(),
        }
    }

    /// Register a transport
    pub fn register(&mut self, transport: Box<dyn Transport>) {
        self.transports.push(transport);
    }

    /// Connect to a node, automatically selecting the right transport
    pub async fn connect(&self, addr: &NodeAddress) -> Result<Box<dyn Connection>, TransportError> {
        for transport in &self.transports {
            if transport.can_handle(addr) {
                return transport.connect(addr).await;
            }
        }
        Err(TransportError::UnsupportedAddress)
    }

    /// Connect to a node using the best available address
    pub async fn connect_best(
        &self,
        addresses: &[NodeAddress],
    ) -> Result<Box<dyn Connection>, TransportError> {
        // Priority: Local > LAN > WiFi Direct > Internet > Bluetooth > Relay
        let priority = |addr: &NodeAddress| -> u8 {
            match addr.transport_type() {
                TransportType::Local => 0,
                TransportType::Lan => 1,
                TransportType::WifiDirect => 2,
                TransportType::Internet => 3,
                TransportType::Bluetooth => 4,
                TransportType::Relay => 5,
            }
        };

        let mut sorted: Vec<_> = addresses.iter().collect();
        sorted.sort_by_key(|a| priority(a));

        let mut last_error = None;
        for addr in sorted {
            match self.connect(addr).await {
                Ok(conn) => return Ok(conn),
                Err(e) => last_error = Some(e),
            }
        }

        Err(last_error.unwrap_or(TransportError::UnsupportedAddress))
    }

    /// Get all registered transport types
    pub fn available_transports(&self) -> Vec<TransportType> {
        self.transports.iter().map(|t| t.transport_type()).collect()
    }
}

impl Default for TransportManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_transport_type() {
        let local = NodeAddress::Local { path: "/tmp/test".to_string() };
        assert_eq!(local.transport_type(), TransportType::Local);

        let tcp = NodeAddress::Tcp {
            addr: "127.0.0.1:8080".parse().unwrap(),
        };
        assert_eq!(tcp.transport_type(), TransportType::Lan);
    }

    #[test]
    fn test_bandwidth_tier_ordering() {
        // Just ensure the enum variants exist and can be compared
        assert_ne!(BandwidthTier::VeryLow, BandwidthTier::VeryHigh);
    }
}
