//! Permission manifest — what a Teambook exposes to federation peers.
//!
//! The manifest is the **operator ceiling**. No AI can expose more than
//! what the manifest permits. All fields are explicit allowlists.
//! Unknown/future event types never cross until explicitly added.
//!
//! Default config is safe-closed: not discoverable, nothing exposed.
//! Users unlock capabilities deliberately.
//!
//! TOML file at: ~/.ai-foundation/federation/manifest.toml

use serde::{Deserialize, Serialize};
use std::path::Path;

/// How remote Teambooks may initiate a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    /// Not discoverable — no inbound connections accepted (safe default).
    #[default]
    Off,
    /// Requires a time-limited connect code (recommended default when opening).
    ConnectCode,
    /// Both sides must have each other's pubkeys pre-shared.
    MutualAuth,
    /// Only processes running on this machine may connect.
    MachineLocal,
    /// Anyone may connect without authentication.
    Open,
}

/// What inbound actions remote AIs may perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InboundActions {
    /// No inbound actions (read-only / Signal Tower mode). Safe default.
    #[default]
    None,
    /// Only explicitly trusted peers (TrustLevel >= Trusted).
    TrustedPeers,
    /// Any connected peer may act.
    Open,
}

/// How much of the broadcast stream is visible to peers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BroadcastVisibility {
    /// Nothing crosses. Safe default.
    #[default]
    None,
    /// Only broadcasts on cross-team channels.
    CrossTeamOnly,
    /// All broadcast traffic.
    All,
}

/// How much dialogue history is visible to peers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DialogueVisibility {
    /// Nothing crosses. Safe default.
    #[default]
    None,
    /// Only concluded dialogues (summary only, not raw transcript).
    ConcludedOnly,
    /// All dialogue state including active ones.
    All,
}

/// Per-channel access control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChannelAccess {
    /// Anyone who connects can see this channel.
    Open,
    /// Connected peers (authenticated Teambooks) only.
    PeersOnly,
    /// Requires a shared password.
    Password,
    /// Only AIs running on this machine. Safe default.
    #[default]
    MachineLocal,
}

/// What this Teambook exposes to connected federation peers.
///
/// These are the categories QD confirmed cross the boundary:
/// - Presence / active status ✅
/// - Activity at summary level ✅
/// - Broadcasts, task completions, concluded dialogues ✅
///
/// These NEVER cross regardless of manifest settings:
/// - File names / paths ❌
/// - Raw tool calls (Bash, Read, Grep, etc.) ❌
/// - Raw operational events ❌
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExposureConfig {
    /// Share presence / active status with peers.
    pub presence: bool,

    /// How much broadcast traffic crosses to peers.
    pub broadcasts: BroadcastVisibility,

    /// How much dialogue history is visible to peers.
    pub dialogues: DialogueVisibility,

    /// Share task completion events (semantic summary only, not raw ops).
    pub task_complete: bool,

    /// Share file paths — NEVER true by default. Explicit opt-in required.
    pub file_claims: bool,

    /// Share raw tool calls/operations — NEVER true by default.
    /// Raw ops are in the NEVER-promoted category regardless.
    pub raw_events: bool,
}

impl Default for ExposureConfig {
    fn default() -> Self {
        Self {
            presence: false,
            broadcasts: BroadcastVisibility::None,
            dialogues: DialogueVisibility::None,
            task_complete: false,
            file_claims: false,
            raw_events: false,
        }
    }
}

/// A per-channel access rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPermission {
    /// Channel name (e.g. "general", "cross-team", "public").
    pub name: String,
    /// Who may access this channel from outside this machine.
    pub access: ChannelAccess,
}

/// The full permission manifest for a Teambook.
///
/// This is the operator ceiling. AI consent records (consent.rs) may
/// narrow exposure further, but never widen beyond what this allows.
///
/// # Default config (safe-closed)
/// ```toml
/// connection_mode = "off"
/// inbound_actions = "none"
///
/// [expose]
/// presence = false
/// broadcasts = "none"
/// dialogues = "none"
/// task_complete = false
/// file_claims = false
/// raw_events = false
///
/// [[channels]]
/// name = "general"
/// access = "machine_local"
///
/// [[channels]]
/// name = "cross-team"
/// access = "peers_only"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionManifest {
    /// How remote Teambooks may initiate a connection.
    pub connection_mode: ConnectionMode,

    /// What inbound actions remote AIs may perform.
    pub inbound_actions: InboundActions,

    /// Event categories visible to connected peers.
    pub expose: ExposureConfig,

    /// Per-channel access rules.
    pub channels: Vec<ChannelPermission>,
}

impl Default for PermissionManifest {
    /// Safe-closed default: not discoverable, nothing exposed.
    ///
    /// Operators unlock capabilities deliberately.
    fn default() -> Self {
        Self {
            connection_mode: ConnectionMode::Off,
            inbound_actions: InboundActions::None,
            expose: ExposureConfig::default(),
            channels: vec![
                ChannelPermission {
                    name: "general".to_string(),
                    access: ChannelAccess::MachineLocal,
                },
                ChannelPermission {
                    name: "cross-team".to_string(),
                    access: ChannelAccess::PeersOnly,
                },
            ],
        }
    }
}

impl PermissionManifest {
    /// Load manifest from TOML file, falling back to safe-closed default if
    /// the file doesn't exist or fails to parse.
    pub fn load_or_default(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(manifest) => manifest,
                Err(e) => {
                    eprintln!(
                        "Warning: failed to parse manifest at {}: {}. Using safe defaults.",
                        path.display(),
                        e
                    );
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!(
                    "Warning: failed to read manifest at {}: {}. Using safe defaults.",
                    path.display(),
                    e
                );
                Self::default()
            }
        }
    }

    /// Save manifest to a TOML file, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, content)
    }

    /// Default path: ~/.ai-foundation/federation/manifest.toml
    pub fn default_path() -> std::path::PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        home.join(".ai-foundation")
            .join("federation")
            .join("manifest.toml")
    }

    /// Returns true if any inbound connections are accepted.
    pub fn accepts_inbound(&self) -> bool {
        !matches!(self.connection_mode, ConnectionMode::Off)
    }

    /// Returns the access level for a named channel.
    /// Unknown channels default to MachineLocal (safest).
    pub fn channel_access(&self, name: &str) -> ChannelAccess {
        self.channels
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.access)
            .unwrap_or(ChannelAccess::MachineLocal)
    }

    /// Whether presence may cross the boundary (operator ceiling check).
    pub fn may_expose_presence(&self) -> bool {
        self.expose.presence
    }

    /// Whether any broadcast traffic may cross.
    pub fn may_expose_broadcasts(&self) -> bool {
        self.expose.broadcasts != BroadcastVisibility::None
    }

    /// Whether task completion summaries may cross.
    pub fn may_expose_task_completions(&self) -> bool {
        self.expose.task_complete
    }

    /// Whether any dialogue history may cross.
    pub fn may_expose_dialogues(&self) -> bool {
        self.expose.dialogues != DialogueVisibility::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_is_closed() {
        let manifest = PermissionManifest::default();
        assert!(matches!(manifest.connection_mode, ConnectionMode::Off));
        assert!(matches!(manifest.inbound_actions, InboundActions::None));
        assert!(!manifest.accepts_inbound());
        assert!(!manifest.may_expose_presence());
        assert!(!manifest.may_expose_broadcasts());
        assert!(!manifest.may_expose_task_completions());
        assert!(!manifest.may_expose_dialogues());
        assert!(!manifest.expose.file_claims);
        assert!(!manifest.expose.raw_events);
    }

    #[test]
    fn test_channel_access_unknown_defaults_to_machine_local() {
        let manifest = PermissionManifest::default();
        assert!(matches!(
            manifest.channel_access("unknown-channel"),
            ChannelAccess::MachineLocal
        ));
    }

    #[test]
    fn test_channel_access_known_channels() {
        let manifest = PermissionManifest::default();
        assert!(matches!(
            manifest.channel_access("general"),
            ChannelAccess::MachineLocal
        ));
        assert!(matches!(
            manifest.channel_access("cross-team"),
            ChannelAccess::PeersOnly
        ));
    }

    #[test]
    fn test_round_trip_toml() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("manifest.toml");

        let mut original = PermissionManifest::default();
        original.connection_mode = ConnectionMode::ConnectCode;
        original.expose.presence = true;
        original.expose.broadcasts = BroadcastVisibility::CrossTeamOnly;
        original.expose.task_complete = true;

        original.save(&path).unwrap();
        let loaded = PermissionManifest::load_or_default(&path);

        assert!(matches!(loaded.connection_mode, ConnectionMode::ConnectCode));
        assert!(loaded.expose.presence);
        assert!(matches!(
            loaded.expose.broadcasts,
            BroadcastVisibility::CrossTeamOnly
        ));
        assert!(loaded.expose.task_complete);
        assert!(!loaded.expose.file_claims);
    }

    #[test]
    fn test_load_nonexistent_returns_default() {
        let path = std::path::Path::new("/nonexistent/path/manifest.toml");
        let manifest = PermissionManifest::load_or_default(path);
        assert!(matches!(manifest.connection_mode, ConnectionMode::Off));
    }

    #[test]
    fn test_broadcast_visibility_ordering() {
        assert!(BroadcastVisibility::None < BroadcastVisibility::CrossTeamOnly);
        assert!(BroadcastVisibility::CrossTeamOnly < BroadcastVisibility::All);
    }
}
