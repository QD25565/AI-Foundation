//! TURN Relay Configuration

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::Path;

/// TURN server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnConfig {
    /// Bind address
    pub bind: SocketAddr,

    /// External/public IP (for NAT scenarios)
    pub external_ip: Option<String>,

    /// Shared secret for credential verification
    pub secret: Option<String>,

    /// Authentication realm
    pub realm: String,

    /// Enable TCP transport
    #[serde(default)]
    pub enable_tcp: bool,

    /// Minimum relay port
    #[serde(default = "default_min_port")]
    pub min_port: u16,

    /// Maximum relay port
    #[serde(default = "default_max_port")]
    pub max_port: u16,
}

fn default_min_port() -> u16 {
    49152
}

fn default_max_port() -> u16 {
    65535
}

impl Default for TurnConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:3478".parse().unwrap(),
            external_ip: None,
            secret: None,
            realm: "ai-foundation.local".to_string(),
            enable_tcp: false,
            min_port: default_min_port(),
            max_port: default_max_port(),
        }
    }
}

impl TurnConfig {
    /// Load configuration from TOML file
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: TurnConfig = toml::from_str(&content)?;
        Ok(config)
    }

    /// Save configuration to TOML file
    pub fn to_file(&self, path: &Path) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

/// Example configuration file content
pub fn example_config() -> &'static str {
    r#"# AI-Foundation TURN Relay Configuration

# Bind address for UDP/TCP
bind = "0.0.0.0:3478"

# External/public IP address (set this if behind NAT)
# external_ip = "203.0.113.50"

# Shared secret for HMAC credential verification
# Get this from the Discovery Registry configuration
secret = "your-shared-secret-here"

# Authentication realm
realm = "ai-foundation.local"

# Enable TCP transport (in addition to UDP)
enable_tcp = false

# Port range for relay allocations
min_port = 49152
max_port = 65535
"#
}
