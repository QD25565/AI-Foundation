//! mDNS/DNS-SD discovery for LAN/WiFi peers
//!
//! Uses the `_teambook._tcp.local.` service type to discover
//! other Teambooks on the local network via the `mdns-sd` crate.
//!
//! # Architecture
//!
//! `start_mdns_discovery` runs as a long-lived async task, receiving
//! `ServiceEvent` notifications from the mdns-sd daemon thread and
//! translating them into `DiscoveryEvent`s for the federation layer.
//!
//! `advertise_service` registers this Teambook's QUIC endpoint on the
//! local network and returns an `MdnsAdvertiser` handle. The service
//! stays advertised until the handle is dropped or `shutdown()` is called.

use super::{DiscoveredPeer, DiscoveryEvent, DiscoveryType};
use crate::{Endpoint, FederationError, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// mDNS service type for Teambook federation.
pub const SERVICE_TYPE: &str = "_teambook._tcp.local.";

/// Default port for federation QUIC connections.
pub const DEFAULT_PORT: u16 = 31420;

/// TXT record keys.
pub const TXT_NODE_ID: &str = "node_id";
pub const TXT_DISPLAY_NAME: &str = "name";
pub const TXT_PROTOCOL_VERSION: &str = "proto";
pub const TXT_PUBKEY_FINGERPRINT: &str = "fp";
/// Full Ed25519 public key hex (64 chars) — used for iroh QUIC connections.
pub const TXT_PUBKEY: &str = "pubkey";

/// Current protocol version advertised in TXT records.
pub const PROTOCOL_VERSION: u32 = 1;

/// mDNS service advertisement info.
#[derive(Debug, Clone)]
pub struct MdnsAdvertisement {
    /// Service instance name (e.g. "af-a3f7c2d1").
    pub instance_name: String,

    /// Port to advertise.
    pub port: u16,

    /// TXT record data.
    pub txt_records: HashMap<String, String>,
}

impl MdnsAdvertisement {
    /// Create a new advertisement for a federation node.
    ///
    /// `pubkey_hex` is the full Ed25519 public key (64-char hex). Included in
    /// TXT records so discovered peers can establish iroh QUIC connections
    /// without a relay — essential for LAN-only federation.
    pub fn new(
        node_id: &str,
        display_name: &str,
        pubkey_hex: &str,
        port: u16,
        protocol_version: u32,
    ) -> Self {
        let mut txt_records = HashMap::new();
        txt_records.insert(TXT_NODE_ID.to_string(), node_id.to_string());
        txt_records.insert(TXT_DISPLAY_NAME.to_string(), display_name.to_string());
        txt_records.insert(TXT_PUBKEY.to_string(), pubkey_hex.to_string());
        txt_records.insert(
            TXT_PROTOCOL_VERSION.to_string(),
            protocol_version.to_string(),
        );

        // Use first 8 chars of node_id as instance name
        let instance_name = format!("af-{}", &node_id[..8.min(node_id.len())]);

        Self {
            instance_name,
            port,
            txt_records,
        }
    }

    /// Add pubkey fingerprint to the advertisement.
    pub fn with_fingerprint(mut self, fingerprint: &str) -> Self {
        self.txt_records
            .insert(TXT_PUBKEY_FINGERPRINT.to_string(), fingerprint.to_string());
        self
    }
}

// ---------------------------------------------------------------------------
// mDNS Discovery (browsing for peers)
// ---------------------------------------------------------------------------

/// mDNS discovery state — tracks known services and emits federation events.
pub struct MdnsDiscovery {
    /// Our node ID (to filter self-discovery).
    local_node_id: String,

    /// Service type being discovered.
    #[allow(dead_code)]
    service_type: String,

    /// Event sender.
    event_tx: mpsc::Sender<DiscoveryEvent>,

    /// Known services (fullname -> peer).
    known_services: HashMap<String, DiscoveredPeer>,
}

impl MdnsDiscovery {
    /// Create new mDNS discovery state.
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
        }
    }

    /// Handle a discovered service — filters self, deduplicates, emits event.
    pub async fn handle_service_found(
        &mut self,
        instance_name: &str,
        addr: SocketAddr,
        txt_records: &HashMap<String, String>,
    ) {
        let node_id = txt_records.get(TXT_NODE_ID).cloned();
        let display_name = txt_records.get(TXT_DISPLAY_NAME).cloned();
        let pubkey_hex = txt_records.get(TXT_PUBKEY).cloned();

        // Skip if this is us
        if let Some(ref id) = node_id {
            if id == &self.local_node_id {
                return;
            }
        }

        let endpoint = Endpoint::Mdns {
            service_name: instance_name.to_string(),
            resolved_addr: Some(addr),
        };

        let mut peer = DiscoveredPeer::new(endpoint, DiscoveryType::Mdns);
        if let Some(id) = node_id {
            peer = peer.with_node_id(&id);
        }
        if let Some(name) = display_name {
            peer = peer.with_name(&name);
        }
        if let Some(pk) = pubkey_hex {
            peer = peer.with_pubkey(&pk);
        }

        let is_new = !self.known_services.contains_key(instance_name);
        self.known_services
            .insert(instance_name.to_string(), peer.clone());

        if is_new {
            let _ = self.event_tx.send(DiscoveryEvent::PeerFound(peer)).await;
        }
    }

    /// Handle a service going away.
    pub async fn handle_service_lost(&mut self, instance_name: &str) {
        if let Some(peer) = self.known_services.remove(instance_name) {
            let _ = self
                .event_tx
                .send(DiscoveryEvent::PeerLost {
                    node_id: peer.node_id,
                    endpoint: peer.endpoint,
                })
                .await;
        }
    }
}

/// Start mDNS discovery (async, long-running).
///
/// Creates an mdns-sd daemon, browses for `_teambook._tcp.local.` services,
/// and emits `DiscoveryEvent`s for each peer found or lost. Runs until the
/// browse channel is closed or the search is stopped.
///
/// Spawned as a tokio task by `DiscoveryManager::start()`.
pub async fn start_mdns_discovery(
    service_type: &str,
    local_node_id: &str,
    event_tx: mpsc::Sender<DiscoveryEvent>,
) -> Result<()> {
    let daemon = ServiceDaemon::new().map_err(|e| {
        FederationError::DiscoveryError(format!("Failed to create mDNS daemon: {e}"))
    })?;

    let receiver = daemon.browse(service_type).map_err(|e| {
        FederationError::DiscoveryError(format!("Failed to start mDNS browse for {service_type}: {e}"))
    })?;

    info!(service_type, "mDNS discovery started");
    let _ = event_tx
        .send(DiscoveryEvent::Started(DiscoveryType::Mdns))
        .await;

    let mut discovery = MdnsDiscovery::new(local_node_id, service_type, event_tx.clone());

    // Event loop — recv_async() yields events from the mdns-sd daemon thread.
    while let Ok(event) = receiver.recv_async().await {
        match event {
            ServiceEvent::ServiceResolved(info) => {
                let addrs_v4 = info.get_addresses_v4();
                let addr_v4 = addrs_v4.iter().next().copied();

                if let Some(ipv4) = addr_v4 {
                    let socket_addr = SocketAddr::new(IpAddr::V4(ipv4), info.get_port());
                    let fullname = info.get_fullname().to_string();

                    // Extract known TXT record keys
                    let props = info.get_properties();
                    let mut txt_records = HashMap::new();
                    for key in [
                        TXT_NODE_ID,
                        TXT_DISPLAY_NAME,
                        TXT_PROTOCOL_VERSION,
                        TXT_PUBKEY_FINGERPRINT,
                        TXT_PUBKEY,
                    ] {
                        if let Some(val) = props.get_property_val_str(key) {
                            txt_records.insert(key.to_string(), val.to_string());
                        }
                    }

                    debug!(
                        fullname = %fullname,
                        addr = %socket_addr,
                        "mDNS: service resolved"
                    );

                    discovery
                        .handle_service_found(&fullname, socket_addr, &txt_records)
                        .await;
                } else {
                    let all_addrs = info.get_addresses();
                    if all_addrs.is_empty() {
                        warn!(
                            fullname = %info.get_fullname(),
                            "mDNS: resolved service has no addresses, skipping"
                        );
                    } else {
                        debug!(
                            fullname = %info.get_fullname(),
                            addr_count = all_addrs.len(),
                            "mDNS: resolved service has only IPv6 addresses, skipping"
                        );
                    }
                }
            }

            ServiceEvent::ServiceRemoved(_service_type, fullname) => {
                debug!(fullname = %fullname, "mDNS: service removed");
                discovery.handle_service_lost(&fullname).await;
            }

            ServiceEvent::SearchStarted(st) => {
                debug!(service_type = %st, "mDNS: browse search started");
            }

            ServiceEvent::ServiceFound(_st, fullname) => {
                debug!(
                    fullname = %fullname,
                    "mDNS: service found, awaiting resolution"
                );
            }

            ServiceEvent::SearchStopped(st) => {
                info!(service_type = %st, "mDNS: browse search stopped");
                break;
            }

            _ => {}
        }
    }

    let _ = event_tx
        .send(DiscoveryEvent::Stopped(DiscoveryType::Mdns))
        .await;
    let _ = daemon.shutdown();

    Ok(())
}

// ---------------------------------------------------------------------------
// mDNS Advertisement (registering our service)
// ---------------------------------------------------------------------------

/// Handle to a running mDNS advertisement.
///
/// Keeps the service registered on the local network. The mdns-sd daemon
/// responds to queries in a background thread. Call `shutdown()` for a clean
/// unregister, or drop the handle (service expires via TTL).
pub struct MdnsAdvertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

impl MdnsAdvertiser {
    /// The full service name as registered on the network.
    pub fn fullname(&self) -> &str {
        &self.fullname
    }

    /// Explicitly unregister and shut down. Best-effort cleanup.
    pub fn shutdown(self) -> Result<()> {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
        Ok(())
    }
}

/// Advertise our Teambook's federation endpoint via mDNS.
///
/// Returns an `MdnsAdvertiser` handle that keeps the service advertised
/// on the local network. Other Teambooks running `start_mdns_discovery`
/// will pick up this service automatically.
///
/// Uses `enable_addr_auto()` so the daemon publishes all local network
/// addresses and adapts to network changes.
pub fn advertise_service(advertisement: &MdnsAdvertisement) -> Result<MdnsAdvertiser> {
    let daemon = ServiceDaemon::new().map_err(|e| {
        FederationError::DiscoveryError(format!("Failed to create mDNS daemon: {e}"))
    })?;

    // Hostname unique to this node — used for A/AAAA records
    let host_name = format!("{}.local.", advertisement.instance_name);

    // TXT properties as slice of tuples (implements IntoTxtProperties)
    let props: Vec<(&str, &str)> = advertisement
        .txt_records
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let service = ServiceInfo::new(
        SERVICE_TYPE,
        &advertisement.instance_name,
        &host_name,
        "", // empty = no hardcoded IPs, relies on addr_auto
        advertisement.port,
        props.as_slice(),
    )
    .map_err(|e| {
        FederationError::DiscoveryError(format!("Failed to create mDNS service info: {e}"))
    })?
    .enable_addr_auto();

    let fullname = service.get_fullname().to_string();

    daemon.register(service).map_err(|e| {
        FederationError::DiscoveryError(format!("Failed to register mDNS service: {e}"))
    })?;

    info!(
        fullname = %fullname,
        port = advertisement.port,
        "mDNS: advertising federation service"
    );

    Ok(MdnsAdvertiser { daemon, fullname })
}

// ---------------------------------------------------------------------------
// Raw TXT record parsing
// ---------------------------------------------------------------------------

/// Parse raw mDNS TXT record bytes into a HashMap.
///
/// TXT records use length-prefixed `key=value` strings per RFC 6763 section 6.
/// For resolved services from mdns-sd, use `ResolvedService::get_properties()`
/// instead — this function handles the raw wire format.
pub fn parse_txt_records(txt_data: &[u8]) -> HashMap<String, String> {
    let mut records = HashMap::new();

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
        let ad = MdnsAdvertisement::new("abc123def456", "Test Node", "aabbccdd00112233aabbccdd00112233aabbccdd00112233aabbccdd00112233", 31420, 1);

        assert_eq!(ad.instance_name, "af-abc123de");
        assert_eq!(ad.port, 31420);
        assert_eq!(
            ad.txt_records.get(TXT_NODE_ID),
            Some(&"abc123def456".to_string())
        );
        assert_eq!(
            ad.txt_records.get(TXT_DISPLAY_NAME),
            Some(&"Test Node".to_string())
        );
        assert_eq!(
            ad.txt_records.get(TXT_PROTOCOL_VERSION),
            Some(&"1".to_string())
        );
    }

    #[test]
    fn test_advertisement_with_fingerprint() {
        let ad =
            MdnsAdvertisement::new("abc123def456", "Test", "aabbccdd00112233aabbccdd00112233aabbccdd00112233aabbccdd00112233", 31420, 1).with_fingerprint("deadbeef");

        assert_eq!(
            ad.txt_records.get(TXT_PUBKEY_FINGERPRINT),
            Some(&"deadbeef".to_string())
        );
    }

    #[test]
    fn test_parse_txt_records() {
        let txt_data = [
            11, b'n', b'o', b'd', b'e', b'_', b'i', b'd', b'=', b'a', b'b', b'c', 9, b'n',
            b'a', b'm', b'e', b'=', b'T', b'e', b's', b't',
        ];

        let records = parse_txt_records(&txt_data);
        assert_eq!(records.get("node_id"), Some(&"abc".to_string()));
        assert_eq!(records.get("name"), Some(&"Test".to_string()));
    }

    #[test]
    fn test_parse_txt_records_empty() {
        let records = parse_txt_records(&[]);
        assert!(records.is_empty());
    }

    #[test]
    fn test_parse_txt_records_truncated() {
        // Length byte says 20 but only 5 bytes follow — should stop gracefully
        let txt_data = [20, b'a', b'=', b'b', b'c', b'd'];
        let records = parse_txt_records(&txt_data);
        assert!(records.is_empty());
    }

    #[test]
    fn test_service_type_format() {
        assert!(SERVICE_TYPE.ends_with("._tcp.local."));
        assert!(SERVICE_TYPE.starts_with('_'));
    }

    #[test]
    fn test_protocol_version_constant() {
        assert!(PROTOCOL_VERSION >= 1);
    }

    #[tokio::test]
    async fn test_discovery_self_filter() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut discovery = MdnsDiscovery::new("my-node-id", SERVICE_TYPE, tx);

        let mut txt = HashMap::new();
        txt.insert(TXT_NODE_ID.to_string(), "my-node-id".to_string());

        let addr: SocketAddr = "192.168.1.100:31420".parse().unwrap();
        discovery
            .handle_service_found("test-service", addr, &txt)
            .await;

        // Self-discovery should be filtered — channel empty
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_discovery_peer_found() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut discovery = MdnsDiscovery::new("my-node-id", SERVICE_TYPE, tx);

        let mut txt = HashMap::new();
        txt.insert(TXT_NODE_ID.to_string(), "other-node".to_string());
        txt.insert(TXT_DISPLAY_NAME.to_string(), "Other Teambook".to_string());

        let addr: SocketAddr = "192.168.1.200:31420".parse().unwrap();
        discovery
            .handle_service_found("other-service", addr, &txt)
            .await;

        match rx.try_recv() {
            Ok(DiscoveryEvent::PeerFound(peer)) => {
                assert_eq!(peer.node_id, Some("other-node".to_string()));
                assert_eq!(peer.display_name, Some("Other Teambook".to_string()));
            }
            other => panic!("Expected PeerFound, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_discovery_peer_lost() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut discovery = MdnsDiscovery::new("my-node-id", SERVICE_TYPE, tx);

        let mut txt = HashMap::new();
        txt.insert(TXT_NODE_ID.to_string(), "other-node".to_string());
        let addr: SocketAddr = "192.168.1.200:31420".parse().unwrap();
        discovery
            .handle_service_found("other-service", addr, &txt)
            .await;
        let _ = rx.recv().await; // consume PeerFound

        discovery.handle_service_lost("other-service").await;

        match rx.try_recv() {
            Ok(DiscoveryEvent::PeerLost { node_id, .. }) => {
                assert_eq!(node_id, Some("other-node".to_string()));
            }
            other => panic!("Expected PeerLost, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_discovery_dedup() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut discovery = MdnsDiscovery::new("my-node-id", SERVICE_TYPE, tx);

        let mut txt = HashMap::new();
        txt.insert(TXT_NODE_ID.to_string(), "other-node".to_string());
        let addr: SocketAddr = "192.168.1.200:31420".parse().unwrap();

        // Same peer reported twice
        discovery
            .handle_service_found("other-service", addr, &txt)
            .await;
        discovery
            .handle_service_found("other-service", addr, &txt)
            .await;

        // Should only emit one PeerFound event
        assert!(rx.recv().await.is_some());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_discovery_no_node_id() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut discovery = MdnsDiscovery::new("my-node-id", SERVICE_TYPE, tx);

        // Peer without node_id TXT record — should still be discovered
        let txt = HashMap::new();
        let addr: SocketAddr = "192.168.1.200:31420".parse().unwrap();
        discovery
            .handle_service_found("unknown-service", addr, &txt)
            .await;

        match rx.try_recv() {
            Ok(DiscoveryEvent::PeerFound(peer)) => {
                assert_eq!(peer.node_id, None);
            }
            other => panic!("Expected PeerFound, got {:?}", other),
        }
    }

    /// Integration test: advertise and discover on the same machine.
    ///
    /// Requires a real network stack with mDNS multicast support.
    /// WSL2's virtual network adapter does not support multicast reliably.
    #[tokio::test]
    #[ignore = "requires multicast-capable network stack (not WSL2)"]
    async fn test_advertise_and_discover() {
        let ad = MdnsAdvertisement::new(
            "testnode12345678",
            "Test Teambook",
            "aabbccdd00112233aabbccdd00112233aabbccdd00112233aabbccdd00112233",
            DEFAULT_PORT,
            PROTOCOL_VERSION,
        );
        let advertiser = advertise_service(&ad).expect("advertise should succeed");

        // Give the daemon time to announce
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let (tx, mut rx) = mpsc::channel(10);
        let discovery_handle = tokio::spawn(async move {
            start_mdns_discovery(SERVICE_TYPE, "different-node", tx).await
        });

        // Wait for discovery event with timeout
        let found = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await;

        advertiser.shutdown().ok();
        discovery_handle.abort();

        assert!(
            found.is_ok(),
            "Should discover advertised service within timeout"
        );
    }
}
