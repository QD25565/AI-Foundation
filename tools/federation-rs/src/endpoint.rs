//! Endpoint types for reaching federation nodes

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use uuid::Uuid;

/// How to reach a federation node
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Endpoint {
    /// QUIC connection over internet
    Quic {
        /// Socket address
        addr: SocketAddr,
        /// TLS certificate fingerprint for verification
        cert_fingerprint: Option<[u8; 32]>,
    },

    /// mDNS discovered service on LAN
    Mdns {
        /// Service instance name
        service_name: String,
        /// Resolved address (if known)
        resolved_addr: Option<SocketAddr>,
    },

    /// Bluetooth Low Energy
    BluetoothLe {
        /// Device MAC address
        mac: [u8; 6],
        /// Service UUID
        service_uuid: Uuid,
    },

    /// Classic Bluetooth
    BluetoothClassic {
        /// Device MAC address
        mac: [u8; 6],
        /// RFCOMM channel
        channel: u8,
    },

    /// Passkey-based pairing (temporary)
    Passkey {
        /// The pairing code
        code: String,
        /// Encrypted endpoint info (AES-GCM)
        encrypted_endpoint: Vec<u8>,
        /// Expiration timestamp
        expires_at: u64,
    },

    /// Relay through another node
    Relay {
        /// The relay node ID
        relay_node_id: String,
        /// How to reach the relay
        relay_endpoint: Box<Endpoint>,
    },
}

impl Endpoint {
    /// Create a QUIC endpoint
    pub fn quic(addr: SocketAddr) -> Self {
        Endpoint::Quic {
            addr,
            cert_fingerprint: None,
        }
    }

    /// Create a QUIC endpoint with certificate pinning
    pub fn quic_pinned(addr: SocketAddr, cert_fingerprint: [u8; 32]) -> Self {
        Endpoint::Quic {
            addr,
            cert_fingerprint: Some(cert_fingerprint),
        }
    }

    /// Create an mDNS endpoint
    pub fn mdns(service_name: &str) -> Self {
        Endpoint::Mdns {
            service_name: service_name.to_string(),
            resolved_addr: None,
        }
    }

    /// Create a BLE endpoint
    pub fn bluetooth_le(mac: [u8; 6], service_uuid: Uuid) -> Self {
        Endpoint::BluetoothLe { mac, service_uuid }
    }

    /// Create a passkey endpoint
    pub fn passkey(code: &str, encrypted_endpoint: Vec<u8>, expires_at: u64) -> Self {
        Endpoint::Passkey {
            code: code.to_string(),
            encrypted_endpoint,
            expires_at,
        }
    }

    /// Check if this endpoint is still valid
    pub fn is_valid(&self) -> bool {
        match self {
            Endpoint::Passkey { expires_at, .. } => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                *expires_at > now
            }
            _ => true,
        }
    }

    /// Get the transport type for this endpoint
    pub fn transport_type(&self) -> crate::TransportType {
        match self {
            Endpoint::Quic { addr, .. } => {
                // Heuristic: private IPs are LAN
                if is_private_addr(&addr.ip()) {
                    crate::TransportType::QuicLan
                } else {
                    crate::TransportType::QuicInternet
                }
            }
            Endpoint::Mdns { .. } => crate::TransportType::Mdns,
            Endpoint::BluetoothLe { .. } => crate::TransportType::BluetoothLe,
            Endpoint::BluetoothClassic { .. } => crate::TransportType::BluetoothClassic,
            Endpoint::Passkey { .. } => crate::TransportType::Passkey,
            Endpoint::Relay { .. } => crate::TransportType::Relay,
        }
    }

    /// Get a human-readable description
    pub fn description(&self) -> String {
        match self {
            Endpoint::Quic { addr, .. } => format!("QUIC {}", addr),
            Endpoint::Mdns { service_name, .. } => format!("mDNS {}", service_name),
            Endpoint::BluetoothLe { mac, .. } => {
                format!("BLE {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                    mac[0], mac[1], mac[2], mac[3], mac[4], mac[5])
            }
            Endpoint::BluetoothClassic { mac, channel } => {
                format!("BT {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} ch{}",
                    mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], channel)
            }
            Endpoint::Passkey { code, .. } => format!("Passkey {}", code),
            Endpoint::Relay { relay_node_id, .. } => format!("Relay via {}", &relay_node_id[..8]),
        }
    }
}

/// Check if an IP address is private (LAN)
fn is_private_addr(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_private() || v4.is_loopback() || v4.is_link_local()
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified()
        }
    }
}

/// Endpoint quality metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointMetrics {
    /// Latency in milliseconds
    pub latency_ms: u32,

    /// Estimated bandwidth in kbps
    pub bandwidth_kbps: u32,

    /// Reliability score (0.0 to 1.0)
    pub reliability: f32,

    /// Last successful connection
    pub last_success: Option<u64>,

    /// Number of failed attempts
    pub failed_attempts: u32,
}

impl Default for EndpointMetrics {
    fn default() -> Self {
        Self {
            latency_ms: 0,
            bandwidth_kbps: 0,
            reliability: 1.0,
            last_success: None,
            failed_attempts: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn test_quic_endpoint() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 31420);
        let endpoint = Endpoint::quic(addr);

        assert!(matches!(endpoint.transport_type(), crate::TransportType::QuicLan));
        assert!(endpoint.is_valid());
    }

    #[test]
    fn test_passkey_expiration() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Valid passkey
        let valid = Endpoint::passkey("ABC123", vec![], now + 3600);
        assert!(valid.is_valid());

        // Expired passkey
        let expired = Endpoint::passkey("XYZ789", vec![], now - 1);
        assert!(!expired.is_valid());
    }

    #[test]
    fn test_endpoint_description() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 31420);
        let endpoint = Endpoint::quic(addr);
        assert!(endpoint.description().contains("QUIC"));
        assert!(endpoint.description().contains("8.8.8.8"));
    }
}
