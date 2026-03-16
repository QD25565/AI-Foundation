//! Hardware Fingerprinting
//!
//! Generates a unique fingerprint of the hardware to:
//! - Prevent ban evasion (hardware bans are hard to circumvent)
//! - Provide accountability without requiring KYC
//! - Enable trust establishment
//!
//! The fingerprint is salted per-teambook so it cannot be correlated
//! across different teambooks (privacy protection).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sysinfo::System;

use crate::error::{AFPError, Result};

/// Hardware fingerprint components
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareFingerprint {
    /// CPU brand and model
    pub cpu_brand: String,

    /// Number of CPU cores
    pub cpu_cores: u32,

    /// Total system memory in bytes
    pub total_memory: u64,

    /// Hostname
    pub hostname: String,

    /// OS name and version
    pub os_info: String,

    /// MAC addresses of network interfaces (sorted)
    pub mac_addresses: Vec<String>,

    /// Disk serial numbers (if available)
    pub disk_serials: Vec<String>,

    /// The computed fingerprint hash (set after collect())
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint_hash: Option<[u8; 32]>,
}

impl HardwareFingerprint {
    /// Collect hardware information from the current system
    pub fn collect() -> Result<Self> {
        let mut sys = System::new_all();
        sys.refresh_all();

        // CPU info
        let cpu_brand = sys
            .cpus()
            .first()
            .map(|cpu| cpu.brand().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let cpu_cores = sys.cpus().len() as u32;

        // Memory
        let total_memory = sys.total_memory();

        // Hostname
        let hostname = System::host_name().unwrap_or_else(|| "unknown".to_string());

        // OS info
        let os_info = format!(
            "{} {} {}",
            System::name().unwrap_or_else(|| "unknown".to_string()),
            System::os_version().unwrap_or_else(|| "unknown".to_string()),
            System::kernel_version().unwrap_or_else(|| "unknown".to_string())
        );

        // MAC addresses
        let mac_addresses = collect_mac_addresses();

        // Disk serials (platform-specific, may be empty)
        let disk_serials = collect_disk_serials();

        let mut fingerprint = Self {
            cpu_brand,
            cpu_cores,
            total_memory,
            hostname,
            os_info,
            mac_addresses,
            disk_serials,
            fingerprint_hash: None,
        };

        // Compute the hash
        fingerprint.compute_hash(None);

        Ok(fingerprint)
    }

    /// Compute the fingerprint hash, optionally with a teambook-specific salt
    pub fn compute_hash(&mut self, teambook_salt: Option<&str>) {
        let mut hasher = Sha256::new();

        // Add salt first if provided (makes fingerprint teambook-specific)
        if let Some(salt) = teambook_salt {
            hasher.update(b"AFP_FINGERPRINT_SALT:");
            hasher.update(salt.as_bytes());
            hasher.update(b":");
        }

        // Add all hardware components
        hasher.update(self.cpu_brand.as_bytes());
        hasher.update(&self.cpu_cores.to_le_bytes());
        hasher.update(&self.total_memory.to_le_bytes());
        hasher.update(self.hostname.as_bytes());
        hasher.update(self.os_info.as_bytes());

        for mac in &self.mac_addresses {
            hasher.update(mac.as_bytes());
        }

        for serial in &self.disk_serials {
            hasher.update(serial.as_bytes());
        }

        let result = hasher.finalize();
        self.fingerprint_hash = Some(result.into());
    }

    /// Get the fingerprint hash as hex string
    pub fn hash_hex(&self) -> String {
        match &self.fingerprint_hash {
            Some(hash) => hex::encode(hash),
            None => "not_computed".to_string(),
        }
    }

    /// Get a short fingerprint for display (first 16 chars of hash)
    pub fn short_hash(&self) -> String {
        self.hash_hex().chars().take(16).collect()
    }

    /// Verify that this fingerprint matches another (for ban checking)
    pub fn matches(&self, other: &HardwareFingerprint) -> bool {
        match (&self.fingerprint_hash, &other.fingerprint_hash) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// Calculate similarity score (0.0 to 1.0) with another fingerprint
    /// Used for fuzzy matching when hardware changes slightly
    pub fn similarity(&self, other: &HardwareFingerprint) -> f64 {
        let mut matches = 0.0;
        let mut total = 0.0;

        // CPU brand (weighted heavily)
        total += 2.0;
        if self.cpu_brand == other.cpu_brand {
            matches += 2.0;
        }

        // CPU cores
        total += 1.0;
        if self.cpu_cores == other.cpu_cores {
            matches += 1.0;
        }

        // Memory (within 10% is a match)
        total += 1.0;
        let mem_diff = (self.total_memory as f64 - other.total_memory as f64).abs();
        let mem_pct = mem_diff / self.total_memory.max(1) as f64;
        if mem_pct < 0.1 {
            matches += 1.0;
        }

        // Hostname
        total += 1.0;
        if self.hostname == other.hostname {
            matches += 1.0;
        }

        // OS info
        total += 1.0;
        if self.os_info == other.os_info {
            matches += 1.0;
        }

        // MAC addresses (check for overlap)
        total += 2.0;
        let mac_overlap: usize = self
            .mac_addresses
            .iter()
            .filter(|m| other.mac_addresses.contains(m))
            .count();
        if mac_overlap > 0 {
            matches += 2.0 * (mac_overlap as f64 / self.mac_addresses.len().max(1) as f64);
        }

        // Disk serials (check for overlap)
        total += 2.0;
        let disk_overlap: usize = self
            .disk_serials
            .iter()
            .filter(|s| other.disk_serials.contains(s))
            .count();
        if disk_overlap > 0 {
            matches += 2.0 * (disk_overlap as f64 / self.disk_serials.len().max(1) as f64);
        }

        matches / total
    }

    /// Serialize to CBOR
    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf)
            .map_err(|e| AFPError::SerializationFailed(e.to_string()))?;
        Ok(buf)
    }

    /// Deserialize from CBOR
    pub fn from_cbor(data: &[u8]) -> Result<Self> {
        ciborium::from_reader(data)
            .map_err(|e| AFPError::DeserializationFailed(e.to_string()))
    }
}

impl std::fmt::Display for HardwareFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HW[{}] CPU:{} Cores:{} Mem:{}GB Host:{}",
            self.short_hash(),
            self.cpu_brand,
            self.cpu_cores,
            self.total_memory / 1024 / 1024 / 1024,
            self.hostname
        )
    }
}

/// Collect MAC addresses from network interfaces.
///
/// SAFETY: All subprocess commands use hardcoded names and arguments only.
/// `std::process::Command` does not invoke a shell, so even if arguments
/// were dynamic, shell injection is not possible. Output is parsed from
/// stdout as read-only data. Do NOT add user-controlled arguments to
/// these commands without audit.
fn collect_mac_addresses() -> Vec<String> {

    #[cfg(target_os = "windows")]
    {
        // Try to get MAC addresses via command
        if let Ok(output) = std::process::Command::new("getmac")
            .args(["/fo", "csv", "/nh"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let macs: Vec<String> = stdout
                .lines()
                .filter_map(|line| {
                    let parts: Vec<&str> = line.split(',').collect();
                    parts.first().map(|s| s.trim_matches('"').to_string())
                })
                .filter(|mac| !mac.is_empty() && mac.contains('-'))
                .collect();
            if !macs.is_empty() {
                return macs;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Read from /sys/class/net/*/address
        if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
            let macs: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let path = e.path().join("address");
                    std::fs::read_to_string(path).ok()
                })
                .map(|s| s.trim().to_uppercase())
                .filter(|mac| mac != "00:00:00:00:00:00" && mac.len() == 17)
                .collect();
            if !macs.is_empty() {
                return macs;
            }
        }
    }

    vec![]
}

/// Collect disk serial numbers (platform-specific).
///
/// SAFETY: See `collect_mac_addresses` — same hardcoded-command invariant.
fn collect_disk_serials() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        // Try wmic (deprecated but widely available)
        if let Ok(output) = std::process::Command::new("wmic")
            .args(["diskdrive", "get", "serialnumber"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let serials: Vec<String> = stdout
                .lines()
                .skip(1) // Skip header
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !serials.is_empty() {
                return serials;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try lsblk
        if let Ok(output) = std::process::Command::new("lsblk")
            .args(["-o", "SERIAL", "-n"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let serials: Vec<String> = stdout
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !serials.is_empty() {
                return serials;
            }
        }
    }

    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_fingerprint() {
        let fp = HardwareFingerprint::collect().expect("Failed to collect fingerprint");
        println!("Fingerprint: {}", fp);
        println!("Hash: {}", fp.hash_hex());
        assert!(!fp.cpu_brand.is_empty());
        assert!(fp.cpu_cores > 0);
        assert!(fp.total_memory > 0);
        assert!(fp.fingerprint_hash.is_some());
    }

    #[test]
    fn test_fingerprint_with_salt() {
        let mut fp1 = HardwareFingerprint::collect().unwrap();
        let mut fp2 = HardwareFingerprint::collect().unwrap();

        fp1.compute_hash(Some("teambook-a"));
        fp2.compute_hash(Some("teambook-b"));

        // Same hardware, different salts = different hashes
        assert_ne!(fp1.fingerprint_hash, fp2.fingerprint_hash);
    }

    #[test]
    fn test_similarity() {
        let fp1 = HardwareFingerprint::collect().unwrap();
        let fp2 = HardwareFingerprint::collect().unwrap();

        // Same machine should have 1.0 similarity
        assert!((fp1.similarity(&fp2) - 1.0).abs() < 0.01);
    }
}
