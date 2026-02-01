//! Data sharing preferences and participation requirements

use crate::TrustLevel;
use serde::{Deserialize, Serialize};

/// Categories of data that can be shared
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DataCategory {
    /// Online status, current activity
    Presence,
    /// Visual identity (name, colors, avatar)
    Profile,
    /// Channel broadcast messages
    Broadcasts,
    /// Direct messages
    DirectMessages,
    /// Notes (shared subset)
    Notes,
    /// Tasks
    Tasks,
    /// File claims / coordination data
    Coordination,
    /// Tool registry / capabilities
    ToolRegistry,
}

/// Policy for accepting direct messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DmPolicy {
    /// Accept DMs from anyone
    Anyone,
    /// Only from nodes with Trusted level or higher
    TrustedOnly,
    /// Only from explicitly whitelisted nodes
    Whitelist(Vec<String>),
    /// Don't accept any DMs
    Nobody,
}

impl Default for DmPolicy {
    fn default() -> Self {
        DmPolicy::Anyone
    }
}

/// What a node is willing to share (user-controlled)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharingPreferences {
    /// Share online/offline status
    pub share_presence: bool,

    /// Share profile/visual identity
    pub share_profile: bool,

    /// Channels to share broadcasts from (empty = none)
    pub share_broadcast_channels: Vec<String>,

    /// Policy for accepting DMs
    pub accept_dms_from: DmPolicy,

    /// Share notes tagged with these tags
    pub share_notes_tags: Vec<String>,

    /// Share tasks
    pub share_tasks: bool,

    /// Allow this node to relay messages for others
    pub allow_relay: bool,

    /// Allow caching data from the mesh
    pub allow_cache: bool,

    /// Maximum data to cache (bytes)
    pub max_cache_bytes: u64,
}

impl Default for SharingPreferences {
    fn default() -> Self {
        Self {
            share_presence: true,
            share_profile: true,
            share_broadcast_channels: vec!["general".to_string()],
            accept_dms_from: DmPolicy::Anyone,
            share_notes_tags: vec![],
            share_tasks: false,
            allow_relay: true,
            allow_cache: true,
            max_cache_bytes: 100 * 1024 * 1024, // 100 MB
        }
    }
}

impl SharingPreferences {
    /// Minimal sharing (for anonymous/untrusted nodes)
    pub fn minimal() -> Self {
        Self {
            share_presence: true,
            share_profile: false,
            share_broadcast_channels: vec![],
            accept_dms_from: DmPolicy::Nobody,
            share_notes_tags: vec![],
            share_tasks: false,
            allow_relay: false,
            allow_cache: false,
            max_cache_bytes: 0,
        }
    }

    /// Full sharing (for trusted environments)
    pub fn full() -> Self {
        Self {
            share_presence: true,
            share_profile: true,
            share_broadcast_channels: vec!["*".to_string()], // All channels
            accept_dms_from: DmPolicy::Anyone,
            share_notes_tags: vec!["shared".to_string(), "public".to_string()],
            share_tasks: true,
            allow_relay: true,
            allow_cache: true,
            max_cache_bytes: 1024 * 1024 * 1024, // 1 GB
        }
    }

    /// Check if a specific channel's broadcasts would be shared
    pub fn shares_channel(&self, channel: &str) -> bool {
        self.share_broadcast_channels.contains(&"*".to_string())
            || self.share_broadcast_channels.contains(&channel.to_string())
    }

    /// Check if a note with given tags would be shared
    pub fn shares_note_tags(&self, tags: &[String]) -> bool {
        tags.iter().any(|t| self.share_notes_tags.contains(t))
    }
}

/// Requirements a Teambook can set for participating nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipationRequirements {
    /// Minimum trust level to connect
    pub min_trust_level: TrustLevel,

    /// Require hardware fingerprint verification
    pub require_fingerprint: bool,

    /// Require profile to be shared
    pub require_profile: bool,

    /// Required data categories to share
    pub required_sharing: Vec<DataCategory>,

    /// Banned hardware fingerprints
    pub banned_fingerprints: Vec<String>,

    /// Banned node IDs
    pub banned_nodes: Vec<String>,

    /// Maximum anonymous connections (0 = no anonymous)
    pub max_anonymous: u32,

    /// Require specific protocol version
    pub min_protocol_version: u32,
}

impl Default for ParticipationRequirements {
    fn default() -> Self {
        Self {
            min_trust_level: TrustLevel::Anonymous,
            require_fingerprint: false,
            require_profile: false,
            required_sharing: vec![DataCategory::Presence],
            banned_fingerprints: vec![],
            banned_nodes: vec![],
            max_anonymous: 10,
            min_protocol_version: 1,
        }
    }
}

impl ParticipationRequirements {
    /// Strict requirements (for sensitive teambooks)
    pub fn strict() -> Self {
        Self {
            min_trust_level: TrustLevel::Verified,
            require_fingerprint: true,
            require_profile: true,
            required_sharing: vec![
                DataCategory::Presence,
                DataCategory::Profile,
            ],
            banned_fingerprints: vec![],
            banned_nodes: vec![],
            max_anonymous: 0,
            min_protocol_version: 1,
        }
    }

    /// Check if a node meets these requirements
    pub fn check_node(&self, node: &crate::FederationNode, prefs: &SharingPreferences) -> Result<(), String> {
        // Check trust level
        if node.trust_level < self.min_trust_level {
            return Err(format!(
                "Trust level {:?} below required {:?}",
                node.trust_level, self.min_trust_level
            ));
        }

        // Check fingerprint requirement
        if self.require_fingerprint && node.hardware_fingerprint.is_none() {
            return Err("Hardware fingerprint required".to_string());
        }

        // Check banned list
        if self.banned_nodes.contains(&node.node_id) {
            return Err("Node is banned".to_string());
        }

        if let Some(ref fp) = node.hardware_fingerprint {
            if self.banned_fingerprints.contains(fp) {
                return Err("Hardware fingerprint is banned".to_string());
            }
        }

        // Check required sharing
        for category in &self.required_sharing {
            match category {
                DataCategory::Presence if !prefs.share_presence => {
                    return Err("Presence sharing required".to_string());
                }
                DataCategory::Profile if !prefs.share_profile => {
                    return Err("Profile sharing required".to_string());
                }
                // Other categories could be checked similarly
                _ => {}
            }
        }

        // Check protocol version
        if node.capabilities.protocol_version < self.min_protocol_version {
            return Err(format!(
                "Protocol version {} below required {}",
                node.capabilities.protocol_version, self.min_protocol_version
            ));
        }

        Ok(())
    }
}

/// Result of negotiating sharing between two nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiatedSharing {
    /// Categories that will actually be shared
    pub shared_categories: Vec<DataCategory>,

    /// Channels that will be shared
    pub shared_channels: Vec<String>,

    /// Whether DMs are allowed
    pub dms_allowed: bool,

    /// Whether relay is allowed
    pub relay_allowed: bool,

    /// Whether caching is allowed
    pub cache_allowed: bool,
}

impl NegotiatedSharing {
    /// Negotiate sharing between local preferences and remote requirements
    pub fn negotiate(
        local_prefs: &SharingPreferences,
        remote_prefs: &SharingPreferences,
    ) -> Self {
        let mut shared_categories = Vec::new();

        // Both must be willing to share presence
        if local_prefs.share_presence && remote_prefs.share_presence {
            shared_categories.push(DataCategory::Presence);
        }

        if local_prefs.share_profile && remote_prefs.share_profile {
            shared_categories.push(DataCategory::Profile);
        }

        // Intersection of shared channels
        let shared_channels: Vec<String> = local_prefs
            .share_broadcast_channels
            .iter()
            .filter(|c| remote_prefs.shares_channel(c))
            .cloned()
            .collect();

        if !shared_channels.is_empty() {
            shared_categories.push(DataCategory::Broadcasts);
        }

        // DMs allowed if both accept
        let dms_allowed = matches!(local_prefs.accept_dms_from, DmPolicy::Anyone | DmPolicy::TrustedOnly)
            && matches!(remote_prefs.accept_dms_from, DmPolicy::Anyone | DmPolicy::TrustedOnly);

        if dms_allowed {
            shared_categories.push(DataCategory::DirectMessages);
        }

        Self {
            shared_categories,
            shared_channels,
            dms_allowed,
            relay_allowed: local_prefs.allow_relay && remote_prefs.allow_relay,
            cache_allowed: local_prefs.allow_cache && remote_prefs.allow_cache,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_preferences() {
        let prefs = SharingPreferences::default();
        assert!(prefs.share_presence);
        assert!(prefs.share_profile);
        assert!(prefs.shares_channel("general"));
        assert!(!prefs.shares_channel("private"));
    }

    #[test]
    fn test_wildcard_channel() {
        let prefs = SharingPreferences::full();
        assert!(prefs.shares_channel("any-channel"));
        assert!(prefs.shares_channel("random"));
    }

    #[test]
    fn test_negotiation() {
        let local = SharingPreferences::default();
        let remote = SharingPreferences::default();

        let negotiated = NegotiatedSharing::negotiate(&local, &remote);

        assert!(negotiated.shared_categories.contains(&DataCategory::Presence));
        assert!(negotiated.shared_categories.contains(&DataCategory::Profile));
        assert!(negotiated.dms_allowed);
    }

    #[test]
    fn test_strict_requirements() {
        let requirements = ParticipationRequirements::strict();
        assert_eq!(requirements.min_trust_level, TrustLevel::Verified);
        assert!(requirements.require_fingerprint);
        assert_eq!(requirements.max_anonymous, 0);
    }
}
