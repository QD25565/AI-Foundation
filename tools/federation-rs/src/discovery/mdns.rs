//! mDNS/DNS-SD discovery for LAN/WiFi peers
//!
//! Uses the _ai-foundation._udp.local service type to discover
//! other federation nodes on the local network.

use super::{DiscoveredPeer, DiscoveryEvent, DiscoveryType};
use crate::{Endpoint, Result, FederationError};
use tokio::sync::mpsc;
use std::net::SocketAddr;
use std::collections::HashMap;

/// mDNS service type for AI-Foundation federation
pub const SERVICE_TYPE: &str = "_ai-foundation._udp.local.";

/// Default port for federation QUIC connections
pub const DEFAULT_PORT: u16 = 31420;

/// TXT record keys
pub const TXT_NODE_ID: &str = "node_id";
pub const TXT_DISPLAY_NAME: &str = "name";
pub const TXT_PROTOCOL_VERSION: &str = "proto";
pub const TXT_PUBKEY_FINGERPRINT: &str = "fp";

/// mDNS service advertisement info
#[derive(Debug, Clone)]
pub struct MdnsAdvertisement {
    /// Service instance name
    pub instance_name: String,

    /// Port to advertise
    pub port: u16,

    /// TXT record data
    pub txt_records: HashMap<String, String>,
}

impl MdnsAdvertisement {
    /// Create a new advertisement for a federation node
    pub fn new(node_id: &str, display_name: &str, port: u16, protocol_version: u32) -> Self {
        let mut txt_records = HashMap::new();
        txt_records.insert(TXT_NODE_ID.to_string(), node_id.to_string());
        txt_records.insert(TXT_DISPLAY_NAME.to_string(), display_name.to_string());
        txt_records.insert(TXT_PROTOCOL_VERSION.to_string(), protocol_version.to_string());

        // Use first 8 chars of node_id as instance name
        let instance_name = format!("af-{}", &node_id[..8.min(node_id.len())]);

        Self {
            instance_name,
            port,
            txt_records,
        }
    }

    /// Add pubkey fingerprint
    pub fn with_fingerprint(mut self, fingerprint: &str) -> Self {
        self.txt_records.insert(TXT_PUBKEY_FINGERPRINT.to_string(), fingerprint.to_string());
        self
    }
}

/// mDNS discovery state
pub struct MdnsDiscovery {
    /// Our node ID (to filter self-discovery)
    local_node_id: String,

    /// Service type to discover
    service_type: String,

    /// Event sender
    event_tx: mpsc::Sender<DiscoveryEvent>,

    /// Known services (instance name -> peer)
    known_services: HashMap<String, DiscoveredPeer>,

    /// Is running
    running: bool,
}

impl MdnsDiscovery {
    /// Create new mDNS discovery
    pub fn new(
        local_node_id: &str,
        service_type: &str,
        event_tx: mpsc::Sender<DiscoveryEvent>,
    ) -> Self {
        Self {
            local_node_id: local_node_id.to_string(),
            service_type: service_type.to_string(),
            event_tx,
            known_services: HashMap::new(),
            running: false,
        }
    }

    /// Handle a discovered service
    pub async fn handle_service_found(
        &mut self,
        instance_name: &str,
        addr: SocketAddr,
        txt_records: &HashMap<String, String>,
    ) {
        // Extract node info from TXT records
        let node_id = txt_records.get(TXT_NODE_ID).cloned();
        let display_name = txt_records.get(TXT_DISPLAY_NAME).cloned();

        // Skip if this is us
        if let Some(ref id) = node_id {
            if id == &self.local_node_id {
                return;
            }
        }

        // Create endpoint
        let endpoint = Endpoint::Mdns {
            service_name: instance_name.to_string(),
            resolved_addr: Some(addr),
        };

        // Create discovered peer
        let mut peer = DiscoveredPeer::new(endpoint, DiscoveryType::Mdns);
        if let Some(id) = node_id {
            peer = peer.with_node_id(&id);
        }
        if let Some(name) = display_name {
            peer = peer.with_name(&name);
        }

        // Track and emit event
        let is_new = !self.known_services.contains_key(instance_name);
        self.known_services.insert(instance_name.to_string(), peer.clone());

        if is_new {
            let _ = self.event_tx.send(DiscoveryEvent::PeerFound(peer)).await;
        }
    }

    /// Handle a service going away
    pub async fn handle_service_lost(&mut self, instance_name: &str) {
        if let Some(peer) = self.known_services.remove(instance_name) {
            let _ = self.event_tx.send(DiscoveryEvent::PeerLost {
                node_id: peer.node_id,
                endpoint: peer.endpoint,
            }).await;
        }
    }
}

/// Start mDNS discovery (async)
///
/// This function would use the `mdns-sd` crate in a real implementation.
/// For now, we provide the structure for integration.
pub async fn start_mdns_discovery(
    service_type: &str,
    local_node_id: &str,
    event_tx: mpsc::Sender<DiscoveryEvent>,
) -> Result<()> {
    // Note: Full implementation would use mdns-sd crate:
    //
    // use mdns_sd::{ServiceDaemon, ServiceEvent};
    //
    // let mdns = ServiceDaemon::new()?;
    // let receiver = mdns.browse(service_type)?;
    //
    // while let Ok(event) = receiver.recv() {
    //     match event {
    //         ServiceEvent::ServiceResolved(info) => {
    //             // Extract address and TXT records
    //             // Create DiscoveredPeer and send event
    //         }
    //         ServiceEvent::ServiceRemoved(_, name) => {
    //             // Send PeerLost event
    //         }
    //         _ => {}
    //     }
    // }

    // Notify that discovery has started
    let _ = event_tx.send(DiscoveryEvent::Started(DiscoveryType::Mdns)).await;

    // In a real implementation, this would loop and discover peers
    // For now, we just return Ok - the actual mdns-sd integration
    // will be added when we add the dependency

    Ok(())
}

/// Advertise our service via mDNS
pub async fn advertise_service(advertisement: MdnsAdvertisement) -> Result<()> {
    // Note: Full implementation would use mdns-sd crate:
    //
    // use mdns_sd::{ServiceDaemon, ServiceInfo};
    //
    // let mdns = ServiceDaemon::new()?;
    // let service_type = SERVICE_TYPE;
    // let instance_name = &advertisement.instance_name;
    //
    // let txt_records: Vec<_> = advertisement.txt_records
    //     .iter()
    //     .map(|(k, v)| format!("{}={}", k, v))
    //     .collect();
    //
    // let service = ServiceInfo::new(
    //     service_type,
    //     instance_name,
    //     &hostname,
    //     (),
    //     advertisement.port,
    //     Some(txt_records),
    // )?;
    //
    // mdns.register(service)?;

    Ok(())
}

/// Parse mDNS TXT records into a HashMap
pub fn parse_txt_records(txt_data: &[u8]) -> HashMap<String, String> {
    let mut records = HashMap::new();

    // TXT records are length-prefixed strings in format "key=value"
    let mut pos = 0;
    while pos < txt_data.len() {
        let len = txt_data[pos] as usize;
        if pos + 1 + len > txt_data.len() {
            break;
        }

        if let Ok(record) = std::str::from_utf8(&txt_data[pos + 1..pos + 1 + len]) {
            if let Some((key, value)) = record.split_once('=') {
                records.insert(key.to_string(), value.to_string());
            }
        }

        pos += 1 + len;
    }

    records
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_advertisement_creation() {
        let ad = MdnsAdvertisement::new(
            "abc123def456",
            "Test Node",
            31420,
            1,
        );

        assert_eq!(ad.instance_name, "af-abc123de");
        assert_eq!(ad.port, 31420);
        assert_eq!(ad.txt_records.get(TXT_NODE_ID), Some(&"abc123def456".to_string()));
        assert_eq!(ad.txt_records.get(TXT_DISPLAY_NAME), Some(&"Test Node".to_string()));
    }

    #[test]
    fn test_parse_txt_records() {
        // Simulate TXT record format: length byte + "key=value"
        let txt_data = [
            11, b'n', b'o', b'd', b'e', b'_', b'i', b'd', b'=', b'a', b'b', b'c',
            9, b'n', b'a', b'm', b'e', b'=', b'T', b'e', b's', b't',
        ];

        let records = parse_txt_records(&txt_data);
        assert_eq!(records.get("node_id"), Some(&"abc".to_string()));
        assert_eq!(records.get("name"), Some(&"Test".to_string()));
    }
}
