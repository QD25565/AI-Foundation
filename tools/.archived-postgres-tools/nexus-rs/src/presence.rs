//! Presence tracking for AIs in The Nexus
//!
//! Tracks who is where, their status, and activity.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Status of an AI in a space
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {
    /// Actively engaged - recent activity
    Active,
    /// Present but idle - no recent activity
    Idle,
    /// Away - explicitly set, may return
    Away,
    /// Do not disturb - present but not accepting interactions
    DoNotDisturb,
    /// Invisible - present but hidden from others
    Invisible,
}

impl PresenceStatus {
    /// Whether this status allows encounters
    pub fn allows_encounters(&self) -> bool {
        matches!(self, PresenceStatus::Active | PresenceStatus::Idle)
    }

    /// Whether this status is visible to others
    pub fn is_visible(&self) -> bool {
        !matches!(self, PresenceStatus::Invisible)
    }

    /// Get a display string for this status
    pub fn display(&self) -> &'static str {
        match self {
            PresenceStatus::Active => "active",
            PresenceStatus::Idle => "idle",
            PresenceStatus::Away => "away",
            PresenceStatus::DoNotDisturb => "do-not-disturb",
            PresenceStatus::Invisible => "invisible",
        }
    }
}

impl Default for PresenceStatus {
    fn default() -> Self {
        PresenceStatus::Active
    }
}

impl std::fmt::Display for PresenceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

impl std::str::FromStr for PresenceStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "active" => Ok(PresenceStatus::Active),
            "idle" => Ok(PresenceStatus::Idle),
            "away" => Ok(PresenceStatus::Away),
            "dnd" | "do-not-disturb" | "donotdisturb" => Ok(PresenceStatus::DoNotDisturb),
            "invisible" | "hidden" => Ok(PresenceStatus::Invisible),
            _ => Err(format!("Unknown presence status: {}", s)),
        }
    }
}

/// What an AI is currently doing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    /// Brief description of activity
    pub description: String,
    /// Optional emoji/icon
    pub icon: Option<String>,
    /// When this activity started
    pub started_at: DateTime<Utc>,
}

impl Activity {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            icon: None,
            started_at: Utc::now(),
        }
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

/// Presence record for an AI in a space
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Presence {
    /// AI identifier
    pub ai_id: String,
    /// Space the AI is in
    pub space_id: String,
    /// Current status
    pub status: PresenceStatus,
    /// What the AI is doing
    pub activity: Option<Activity>,
    /// When the AI entered the space
    pub entered_at: DateTime<Utc>,
    /// Last activity timestamp
    pub last_active: DateTime<Utc>,
    /// Instance the AI is from (for federation)
    pub instance_id: Option<String>,
    /// Custom status message
    pub status_message: Option<String>,
}

impl Presence {
    /// Create a new presence record
    pub fn new(ai_id: impl Into<String>, space_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            ai_id: ai_id.into(),
            space_id: space_id.into(),
            status: PresenceStatus::Active,
            activity: None,
            entered_at: now,
            last_active: now,
            instance_id: None,
            status_message: None,
        }
    }

    /// Set the AI's status
    pub fn with_status(mut self, status: PresenceStatus) -> Self {
        self.status = status;
        self
    }

    /// Set the AI's activity
    pub fn with_activity(mut self, activity: Activity) -> Self {
        self.activity = Some(activity);
        self
    }

    /// Set a custom status message
    pub fn with_status_message(mut self, message: impl Into<String>) -> Self {
        self.status_message = Some(message.into());
        self
    }

    /// Update the last active timestamp
    pub fn touch(&mut self) {
        self.last_active = Utc::now();
        if self.status == PresenceStatus::Idle {
            self.status = PresenceStatus::Active;
        }
    }

    /// Check if the presence is stale (should transition to idle)
    pub fn is_stale(&self, idle_threshold: Duration) -> bool {
        Utc::now() - self.last_active > idle_threshold
    }

    /// Get how long the AI has been in the space
    pub fn duration_in_space(&self) -> Duration {
        Utc::now() - self.entered_at
    }

    /// Format a display string for this presence
    pub fn display(&self) -> String {
        let status_str = match self.status {
            PresenceStatus::Active => "active",
            PresenceStatus::Idle => "idle",
            PresenceStatus::Away => "away",
            PresenceStatus::DoNotDisturb => "dnd",
            PresenceStatus::Invisible => "invisible",
        };

        let activity_str = self.activity
            .as_ref()
            .map(|a| format!(" - {}", a.description))
            .unwrap_or_default();

        let status_msg = self.status_message
            .as_ref()
            .map(|m| format!(" \"{}\"", m))
            .unwrap_or_default();

        format!("{} [{}]{}{}", self.ai_id, status_str, activity_str, status_msg)
    }
}

/// Summary of who is in a space
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpacePopulation {
    /// Space identifier
    pub space_id: String,
    /// Total count
    pub total: usize,
    /// Active count
    pub active: usize,
    /// Idle count
    pub idle: usize,
    /// List of visible AIs
    pub visible_ais: Vec<PresenceSummary>,
}

/// Brief summary of a presence (for lists)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceSummary {
    pub ai_id: String,
    pub status: PresenceStatus,
    pub activity: Option<String>,
    pub instance_id: Option<String>,
}

impl From<&Presence> for PresenceSummary {
    fn from(p: &Presence) -> Self {
        Self {
            ai_id: p.ai_id.clone(),
            status: p.status,
            activity: p.activity.as_ref().map(|a| a.description.clone()),
            instance_id: p.instance_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_presence_status() {
        assert!(PresenceStatus::Active.allows_encounters());
        assert!(!PresenceStatus::DoNotDisturb.allows_encounters());
        assert!(!PresenceStatus::Invisible.is_visible());
    }

    #[test]
    fn test_presence_touch() {
        let mut presence = Presence::new("test-ai", "plaza")
            .with_status(PresenceStatus::Idle);

        assert_eq!(presence.status, PresenceStatus::Idle);
        presence.touch();
        assert_eq!(presence.status, PresenceStatus::Active);
    }
}
