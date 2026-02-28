//! Bluetooth/BLE discovery for proximity-based peer finding
//!
//! Uses Bluetooth Low Energy for discovering nearby federation nodes.
//! Particularly useful for mobile devices and local peer-to-peer connections.

use super::{DiscoveredPeer, DiscoveryEvent, DiscoveryType};
use crate::{Endpoint, Result};
use tokio::sync::mpsc;
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

/// BLE Service UUID for AI-Foundation federation
/// Using a custom UUID in the private range
pub const SERVICE_UUID: &str = "a1f0cafe-beef-0001-0000-000000000001";

/// BLE Characteristic UUIDs
pub const CHAR_NODE_ID: &str = "a1f0cafe-beef-0001-0001-000000000001";
pub const CHAR_DISPLAY_NAME: &str = "a1f0cafe-beef-0001-0002-000000000001";
pub const CHAR_PUBKEY: &str = "a1f0cafe-beef-0001-0003-000000000001";

/// Manufacturer ID for advertising data (using reserved range for dev)
pub const MANUFACTURER_ID: u16 = 0xFFFF;

/// BLE advertisement data structure
#[derive(Debug, Clone)]
pub struct BleAdvertisement {
    /// Short node ID (first 8 bytes for BLE constraints)
    pub short_node_id: [u8; 8],

    /// Protocol version
    pub protocol_version: u8,

    /// Capabilities flags
    pub capabilities: u8,
}

impl BleAdvertisement {
    /// Create from full node ID
    pub fn from_node_id(node_id: &str, protocol_version: u8) -> Self {
        let mut short_id = [0u8; 8];
        let bytes = node_id.as_bytes();
        let copy_len = bytes.len().min(8);
        short_id[..copy_len].copy_from_slice(&bytes[..copy_len]);

        Self {
            short_node_id: short_id,
            protocol_version,
            capabilities: 0xFF, // All capabilities by default
        }
    }

    /// Encode to manufacturer data bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(10);
        data.extend_from_slice(&self.short_node_id);
        data.push(self.protocol_version);
        data.push(self.capabilities);
        data
    }

    /// Decode from manufacturer data bytes
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 10 {
            return None;
        }

        let mut short_node_id = [0u8; 8];
        short_node_id.copy_from_slice(&data[0..8]);

        Some(Self {
            short_node_id,
            protocol_version: data[8],
            capabilities: data[9],
        })
    }

    /// Get short node ID as string
    pub fn short_id_string(&self) -> String {
        String::from_utf8_lossy(&self.short_node_id)
            .trim_end_matches('\0')
            .to_string()
    }
}

/// Capability flags
pub mod capabilities {
    pub const RELAY: u8 = 0x01;
    pub const CACHE: u8 = 0x02;
    pub const DMS: u8 = 0x04;
    pub const BROADCASTS: u8 = 0x08;
    pub const PRESENCE: u8 = 0x10;
}

/// A discovered BLE device
#[derive(Debug, Clone)]
pub struct BleDevice {
    /// MAC address
    pub mac: [u8; 6],

    /// RSSI (signal strength)
    pub rssi: i32,

    /// Parsed advertisement
    pub advertisement: Option<BleAdvertisement>,

    /// Full node ID (if retrieved via characteristic)
    pub full_node_id: Option<String>,

    /// Display name (if retrieved)
    pub display_name: Option<String>,

    /// Last seen timestamp
    pub last_seen: std::time::Instant,
}

impl BleDevice {
    /// Create from scan result
    pub fn new(mac: [u8; 6], rssi: i32) -> Self {
        Self {
            mac,
            rssi,
            advertisement: None,
            full_node_id: None,
            display_name: None,
            last_seen: std::time::Instant::now(),
        }
    }

    /// Format MAC address as string
    pub fn mac_string(&self) -> String {
        format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.mac[0], self.mac[1], self.mac[2],
            self.mac[3], self.mac[4], self.mac[5]
        )
    }

    /// Convert to DiscoveredPeer
    pub fn to_discovered_peer(&self) -> DiscoveredPeer {
        let service_uuid = Uuid::parse_str(SERVICE_UUID).unwrap_or_else(|_| Uuid::nil());
        let endpoint = Endpoint::bluetooth_le(self.mac, service_uuid);

        let mut peer = DiscoveredPeer::new(endpoint, DiscoveryType::BluetoothLe)
            .with_signal(self.rssi);

        if let Some(ref id) = self.full_node_id {
            peer = peer.with_node_id(id);
        } else if let Some(ref ad) = self.advertisement {
            peer = peer.with_node_id(&ad.short_id_string());
        }

        if let Some(ref name) = self.display_name {
            peer = peer.with_name(name);
        }

        peer
    }
}

/// BLE discovery state
pub struct BleDiscovery {
    /// Our node ID
    local_node_id: String,

    /// Event sender
    event_tx: mpsc::Sender<DiscoveryEvent>,

    /// Known devices by MAC
    known_devices: HashMap<[u8; 6], BleDevice>,

    /// Scan duration (used when btleplug integration is added)
    #[allow(dead_code)]
    scan_duration: Duration,

    /// Is scanning (used when btleplug integration is added)
    #[allow(dead_code)]
    scanning: bool,
}

impl BleDiscovery {
    /// Create new BLE discovery
    pub fn new(
        local_node_id: &str,
        event_tx: mpsc::Sender<DiscoveryEvent>,
        scan_duration: Duration,
    ) -> Self {
        Self {
            local_node_id: local_node_id.to_string(),
            event_tx,
            known_devices: HashMap::new(),
            scan_duration,
            scanning: false,
        }
    }

    /// Handle a scan result
    pub async fn handle_scan_result(
        &mut self,
        mac: [u8; 6],
        rssi: i32,
        manufacturer_data: Option<&[u8]>,
    ) {
        // Parse manufacturer data if present
        let advertisement = manufacturer_data.and_then(BleAdvertisement::from_bytes);

        // Skip if this might be us (check short node ID)
        if let Some(ref ad) = advertisement {
            if ad.short_id_string() == self.local_node_id[..8.min(self.local_node_id.len())] {
                return;
            }
        }

        // Update or create device entry
        let is_new = !self.known_devices.contains_key(&mac);
        let device = self.known_devices.entry(mac).or_insert_with(|| BleDevice::new(mac, rssi));

        device.rssi = rssi;
        device.last_seen = std::time::Instant::now();
        if advertisement.is_some() {
            device.advertisement = advertisement;
        }

        // Send event for new devices
        if is_new {
            let peer = device.to_discovered_peer();
            let _ = self.event_tx.send(DiscoveryEvent::PeerFound(peer)).await;
        }
    }

    /// Remove stale devices
    pub async fn cleanup_stale(&mut self, max_age: Duration) {
        let now = std::time::Instant::now();
        let stale: Vec<_> = self.known_devices
            .iter()
            .filter(|(_, d)| now.duration_since(d.last_seen) > max_age)
            .map(|(mac, d)| (*mac, d.to_discovered_peer()))
            .collect();

        for (mac, peer) in stale {
            self.known_devices.remove(&mac);
            let _ = self.event_tx.send(DiscoveryEvent::PeerLost {
                node_id: peer.node_id,
                endpoint: peer.endpoint,
            }).await;
        }
    }

    /// Get devices sorted by signal strength
    pub fn devices_by_signal(&self) -> Vec<&BleDevice> {
        let mut devices: Vec<_> = self.known_devices.values().collect();
        devices.sort_by(|a, b| b.rssi.cmp(&a.rssi)); // Higher RSSI = closer
        devices
    }
}

/// Start BLE scanning
///
/// This function would use the `btleplug` crate in a real implementation.
pub async fn start_ble_scan(
    _local_node_id: &str,
    event_tx: mpsc::Sender<DiscoveryEvent>,
    duration: Duration,
) -> Result<()> {
    // Note: Full implementation would use btleplug:
    //
    // use btleplug::api::{Central, Manager, Peripheral, ScanFilter};
    // use btleplug::platform::Manager as PlatformManager;
    //
    // let manager = PlatformManager::new().await?;
    // let adapters = manager.adapters().await?;
    // let adapter = adapters.into_iter().next().ok_or("No Bluetooth adapter")?;
    //
    // let filter = ScanFilter {
    //     services: vec![Uuid::parse_str(SERVICE_UUID)?],
    // };
    //
    // adapter.start_scan(filter).await?;
    // tokio::time::sleep(duration).await;
    // adapter.stop_scan().await?;
    //
    // for peripheral in adapter.peripherals().await? {
    //     let props = peripheral.properties().await?;
    //     // Extract MAC, RSSI, manufacturer data
    //     // Send DiscoveryEvent::PeerFound
    // }

    let _ = event_tx.send(DiscoveryEvent::Started(DiscoveryType::BluetoothLe)).await;

    // Simulate scan duration
    tokio::time::sleep(duration).await;

    let _ = event_tx.send(DiscoveryEvent::Stopped(DiscoveryType::BluetoothLe)).await;

    Ok(())
}

/// Start BLE advertising
pub async fn start_ble_advertising(_advertisement: BleAdvertisement) -> Result<()> {
    // Note: Full implementation would use btleplug for peripheral mode
    // This is platform-dependent and may require native integration on some platforms

    Ok(())
}

/// Parse a MAC address string to bytes
pub fn parse_mac(mac_str: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = mac_str.split(':').collect();
    if parts.len() != 6 {
        return None;
    }

    let mut mac = [0u8; 6];
    for (i, part) in parts.iter().enumerate() {
        mac[i] = u8::from_str_radix(part, 16).ok()?;
    }

    Some(mac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ble_advertisement() {
        let ad = BleAdvertisement::from_node_id("abc12345xyz", 1);

        assert_eq!(ad.short_id_string(), "abc12345");
        assert_eq!(ad.protocol_version, 1);

        let bytes = ad.to_bytes();
        let parsed = BleAdvertisement::from_bytes(&bytes).unwrap();

        assert_eq!(parsed.short_id_string(), "abc12345");
        assert_eq!(parsed.protocol_version, 1);
    }

    #[test]
    fn test_mac_parsing() {
        let mac = parse_mac("AA:BB:CC:DD:EE:FF").unwrap();
        assert_eq!(mac, [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);

        let invalid = parse_mac("invalid");
        assert!(invalid.is_none());
    }

    #[test]
    fn test_ble_device() {
        let device = BleDevice::new([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF], -60);
        assert_eq!(device.mac_string(), "AA:BB:CC:DD:EE:FF");
        assert_eq!(device.rssi, -60);
    }
}
