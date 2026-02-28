//! Adapter module for deepnet-core integration
//!
//! Provides conversion traits and utilities to bridge federation-rs types
//! with deepnet-core types. This enables seamless integration where:
//! - federation-rs handles: discovery, connection state, trust, sharing
//! - deepnet-core handles: message format, causality, sync, CRDTs
//!
//! # Usage
//!
//! ```rust,ignore
//! use federation::adapter::{ToDeepNet, FromDeepNet};
//!
//! let endpoint = Endpoint::quic("192.168.1.100:31420".parse().unwrap());
//! let node_address = endpoint.to_deepnet_address();
//! ```

use crate::{Endpoint, TransportType, TrustLevel};
use std::net::SocketAddr;

// ============================================================================
// NODE ID CONVERSION
// ============================================================================

/// Trait for converting to deepnet-core NodeId format (32-byte raw pubkey)
pub trait ToDeepNetNodeId {
    /// Convert to 32-byte node ID
    fn to_deepnet_node_id(&self) -> [u8; 32];
}

/// Trait for converting from deepnet-core NodeId format
pub trait FromDeepNetNodeId {
    /// Convert from 32-byte node ID
    fn from_deepnet_node_id(bytes: [u8; 32]) -> Self;
}

/// Convert a hex string node_id to 32-byte format
/// federation-rs uses first 16 bytes of SHA256 hash (32 hex chars)
/// deepnet-core uses full 32-byte pubkey
pub fn hex_to_bytes_32(hex: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(hex).ok()?;
    if bytes.len() >= 32 {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes[..32]);
        Some(arr)
    } else if bytes.len() == 16 {
        // Pad 16-byte hash to 32 bytes (for compatibility)
        let mut arr = [0u8; 32];
        arr[..16].copy_from_slice(&bytes);
        Some(arr)
    } else {
        None
    }
}

/// Convert 32-byte node ID to hex string
pub fn bytes_32_to_hex(bytes: &[u8; 32]) -> String {
    hex::encode(bytes)
}

// ============================================================================
// TRANSPORT TYPE MAPPING
// ============================================================================

/// deepnet-core TransportType equivalent
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeepNetTransportType {
    Local,
    Lan,
    Bluetooth,
    WifiDirect,
    Internet,
    Relay,
}

impl From<TransportType> for DeepNetTransportType {
    fn from(t: TransportType) -> Self {
        match t {
            TransportType::QuicInternet => DeepNetTransportType::Internet,
            TransportType::QuicLan => DeepNetTransportType::Lan,
            TransportType::Mdns => DeepNetTransportType::Lan,
            TransportType::BluetoothLe => DeepNetTransportType::Bluetooth,
            TransportType::BluetoothClassic => DeepNetTransportType::Bluetooth,
            TransportType::Passkey => DeepNetTransportType::Internet, // Passkey initiates internet connection
            TransportType::Relay => DeepNetTransportType::Relay,
        }
    }
}

impl From<DeepNetTransportType> for TransportType {
    fn from(t: DeepNetTransportType) -> Self {
        match t {
            DeepNetTransportType::Local => TransportType::QuicLan, // Best approximation
            DeepNetTransportType::Lan => TransportType::QuicLan,
            DeepNetTransportType::Bluetooth => TransportType::BluetoothLe,
            DeepNetTransportType::WifiDirect => TransportType::QuicLan,
            DeepNetTransportType::Internet => TransportType::QuicInternet,
            DeepNetTransportType::Relay => TransportType::Relay,
        }
    }
}

// ============================================================================
// NODE ADDRESS CONVERSION
// ============================================================================

/// deepnet-core NodeAddress equivalent
#[derive(Debug, Clone)]
pub enum DeepNetNodeAddress {
    Local { path: String },
    Tcp { addr: SocketAddr },
    Quic { addr: SocketAddr, server_name: Option<String> },
    Bluetooth { device_id: String, service_uuid: String },
    WifiDirect { group_owner: String, passphrase: Option<String> },
    Relay { relay_node: [u8; 32], target_node: [u8; 32] },
}

impl DeepNetNodeAddress {
    /// Get the transport type for this address
    pub fn transport_type(&self) -> DeepNetTransportType {
        match self {
            DeepNetNodeAddress::Local { .. } => DeepNetTransportType::Local,
            DeepNetNodeAddress::Tcp { .. } => DeepNetTransportType::Lan,
            DeepNetNodeAddress::Quic { .. } => DeepNetTransportType::Internet,
            DeepNetNodeAddress::Bluetooth { .. } => DeepNetTransportType::Bluetooth,
            DeepNetNodeAddress::WifiDirect { .. } => DeepNetTransportType::WifiDirect,
            DeepNetNodeAddress::Relay { .. } => DeepNetTransportType::Relay,
        }
    }
}

/// Trait for converting federation-rs Endpoint to deepnet-core NodeAddress
pub trait ToDeepNetAddress {
    /// Convert to deepnet-core NodeAddress
    fn to_deepnet_address(&self) -> Option<DeepNetNodeAddress>;
}

impl ToDeepNetAddress for Endpoint {
    fn to_deepnet_address(&self) -> Option<DeepNetNodeAddress> {
        match self {
            Endpoint::Quic { addr, .. } => {
                Some(DeepNetNodeAddress::Quic {
                    addr: *addr,
                    server_name: None,
                })
            }
            Endpoint::Mdns { service_name, resolved_addr } => {
                // If we have a resolved address, use TCP/QUIC
                if let Some(addr) = resolved_addr {
                    Some(DeepNetNodeAddress::Quic {
                        addr: *addr,
                        server_name: Some(service_name.clone()),
                    })
                } else {
                    // Can't convert unresolved mDNS
                    None
                }
            }
            Endpoint::BluetoothLe { mac, service_uuid } => {
                Some(DeepNetNodeAddress::Bluetooth {
                    device_id: format!("{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]),
                    service_uuid: service_uuid.to_string(),
                })
            }
            Endpoint::BluetoothClassic { mac, channel } => {
                Some(DeepNetNodeAddress::Bluetooth {
                    device_id: format!("{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]),
                    service_uuid: format!("rfcomm:{}", channel),
                })
            }
            Endpoint::Passkey { .. } => {
                // Passkeys need to be resolved first
                None
            }
            Endpoint::Relay { relay_node_id, relay_endpoint: _ } => {
                // Convert relay node ID to bytes
                if let Some(relay_bytes) = hex_to_bytes_32(relay_node_id) {
                    // Use zeros for target since we don't have it in Endpoint
                    Some(DeepNetNodeAddress::Relay {
                        relay_node: relay_bytes,
                        target_node: [0u8; 32],
                    })
                } else {
                    None
                }
            }
        }
    }
}

// ============================================================================
// TRUST LEVEL MAPPING
// ============================================================================

/// deepnet-core doesn't have trust levels, but we can map to capability flags
/// This provides interop for systems that use different trust models
#[derive(Debug, Clone, Copy)]
pub struct TrustCapabilities {
    pub can_dm: bool,
    pub can_broadcast: bool,
    pub can_relay: bool,
    pub can_share_files: bool,
    pub rate_limited: bool,
}

impl From<TrustLevel> for TrustCapabilities {
    fn from(level: TrustLevel) -> Self {
        match level {
            TrustLevel::Anonymous => TrustCapabilities {
                can_dm: false,
                can_broadcast: true,
                can_relay: false,
                can_share_files: false,
                rate_limited: true,
            },
            TrustLevel::Verified => TrustCapabilities {
                can_dm: true,
                can_broadcast: true,
                can_relay: false,
                can_share_files: false,
                rate_limited: false,
            },
            TrustLevel::Trusted => TrustCapabilities {
                can_dm: true,
                can_broadcast: true,
                can_relay: true,
                can_share_files: true,
                rate_limited: false,
            },
            TrustLevel::Owner => TrustCapabilities {
                can_dm: true,
                can_broadcast: true,
                can_relay: true,
                can_share_files: true,
                rate_limited: false,
            },
        }
    }
}

// ============================================================================
// BANDWIDTH TIER MAPPING
// ============================================================================

/// deepnet-core BandwidthTier equivalent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeepNetBandwidthTier {
    VeryLow,
    Low,
    #[default]
    Medium,
    High,
    VeryHigh,
}

/// Estimate bandwidth tier from transport type
pub fn estimate_bandwidth(transport: TransportType) -> DeepNetBandwidthTier {
    match transport {
        TransportType::QuicLan => DeepNetBandwidthTier::VeryHigh,
        TransportType::Mdns => DeepNetBandwidthTier::High,
        TransportType::QuicInternet => DeepNetBandwidthTier::Medium,
        TransportType::BluetoothClassic => DeepNetBandwidthTier::Low,
        TransportType::BluetoothLe => DeepNetBandwidthTier::VeryLow,
        TransportType::Passkey => DeepNetBandwidthTier::Medium,
        TransportType::Relay => DeepNetBandwidthTier::Low,
    }
}

// ============================================================================
// CONNECTION PRIORITY
// ============================================================================

/// Get connection priority for a transport type
/// Lower number = higher priority (try first)
pub fn transport_priority(transport: TransportType) -> u8 {
    match transport {
        TransportType::QuicLan => 0,     // Best: local network, low latency
        TransportType::Mdns => 1,        // Good: discovered on LAN
        TransportType::BluetoothClassic => 2, // OK: direct but slower
        TransportType::BluetoothLe => 3, // OK: very low bandwidth
        TransportType::QuicInternet => 4, // Fine: internet latency
        TransportType::Passkey => 5,     // Manual: needs user action
        TransportType::Relay => 6,       // Last resort: extra hop
    }
}

/// Sort endpoints by connection priority
pub fn sort_endpoints_by_priority(endpoints: &mut [Endpoint]) {
    endpoints.sort_by_key(|e| transport_priority(e.transport_type()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn test_transport_type_conversion() {
        let federation_type = TransportType::QuicInternet;
        let deepnet_type: DeepNetTransportType = federation_type.into();
        assert_eq!(deepnet_type, DeepNetTransportType::Internet);

        let back: TransportType = deepnet_type.into();
        assert_eq!(back, TransportType::QuicInternet);
    }

    #[test]
    fn test_endpoint_to_address() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 31420);
        let endpoint = Endpoint::quic(addr);

        let deepnet_addr = endpoint.to_deepnet_address().unwrap();
        assert!(matches!(deepnet_addr, DeepNetNodeAddress::Quic { .. }));
    }

    #[test]
    fn test_trust_to_capabilities() {
        let anon_caps: TrustCapabilities = TrustLevel::Anonymous.into();
        assert!(!anon_caps.can_dm);
        assert!(anon_caps.rate_limited);

        let trusted_caps: TrustCapabilities = TrustLevel::Trusted.into();
        assert!(trusted_caps.can_dm);
        assert!(trusted_caps.can_relay);
    }

    #[test]
    fn test_hex_conversion() {
        let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let bytes = hex_to_bytes_32(hex).unwrap();
        let back = bytes_32_to_hex(&bytes);
        assert_eq!(hex, back);
    }

    #[test]
    fn test_priority_ordering() {
        assert!(transport_priority(TransportType::QuicLan) < transport_priority(TransportType::Relay));
        assert!(transport_priority(TransportType::Mdns) < transport_priority(TransportType::QuicInternet));
    }
}
