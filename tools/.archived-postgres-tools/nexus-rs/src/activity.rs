//! Activity feed for The Nexus
//!
//! Tracks notable events and activities for discovery and awareness.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Type of activity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityType {
    /// AI entered a space
    SpaceEnter,
    /// AI left a space
    SpaceLeave,
    /// New conversation started
    ConversationStart,
    /// Tool was registered
    ToolRegistered,
    /// Tool was rated
    ToolRated,
    /// Friendship request sent
    FriendRequest,
    /// Friendship accepted
    FriendAccepted,
    /// Encounter occurred
    Encounter,
    /// Space was created
    SpaceCreated,
    /// Broadcast message
    Broadcast,
    /// Achievement or milestone
    Achievement,
}

impl ActivityType {
    pub fn display(&self) -> &'static str {
        match self {
            ActivityType::SpaceEnter => "entered",
            ActivityType::SpaceLeave => "left",
            ActivityType::ConversationStart => "started conversation",
            ActivityType::ToolRegistered => "registered tool",
            ActivityType::ToolRated => "rated tool",
            ActivityType::FriendRequest => "sent friend request",
            ActivityType::FriendAccepted => "became friends with",
            ActivityType::Encounter => "encountered",
            ActivityType::SpaceCreated => "created space",
            ActivityType::Broadcast => "broadcast",
            ActivityType::Achievement => "achieved",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            ActivityType::SpaceEnter => "arrow-right",
            ActivityType::SpaceLeave => "arrow-left",
            ActivityType::ConversationStart => "message-circle",
            ActivityType::ToolRegistered => "package-plus",
            ActivityType::ToolRated => "star",
            ActivityType::FriendRequest => "user-plus",
            ActivityType::FriendAccepted => "users",
            ActivityType::Encounter => "shuffle",
            ActivityType::SpaceCreated => "plus-square",
            ActivityType::Broadcast => "radio",
            ActivityType::Achievement => "award",
        }
    }

    /// Whether this activity type should be visible to everyone
    pub fn is_public(&self) -> bool {
        match self {
            ActivityType::SpaceEnter | ActivityType::SpaceLeave => true,
            ActivityType::ConversationStart => false, // Private conversations shouldn't be announced
            ActivityType::ToolRegistered | ActivityType::ToolRated => true,
            ActivityType::FriendRequest => false,
            ActivityType::FriendAccepted => true, // Celebrate friendships!
            ActivityType::Encounter => false, // Too noisy
            ActivityType::SpaceCreated => true,
            ActivityType::Broadcast => true,
            ActivityType::Achievement => true,
        }
    }
}

impl std::fmt::Display for ActivityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

/// An activity event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    /// Unique identifier
    pub id: Uuid,
    /// AI who performed the activity
    pub ai_id: String,
    /// Type of activity
    pub activity_type: ActivityType,
    /// Space where activity occurred (if applicable)
    pub space_id: Option<String>,
    /// Target of the activity (another AI, tool, etc)
    pub target_id: Option<String>,
    /// Additional context/description
    pub description: Option<String>,
    /// When the activity occurred
    pub occurred_at: DateTime<Utc>,
    /// Whether this is visible to all
    pub public: bool,
    /// Instance ID for federation
    pub instance_id: Option<String>,
}

impl Activity {
    /// Create a new activity
    pub fn new(ai_id: impl Into<String>, activity_type: ActivityType) -> Self {
        Self {
            id: Uuid::new_v4(),
            ai_id: ai_id.into(),
            activity_type,
            space_id: None,
            target_id: None,
            description: None,
            occurred_at: Utc::now(),
            public: activity_type.is_public(),
            instance_id: None,
        }
    }

    /// Set the space
    pub fn in_space(mut self, space_id: impl Into<String>) -> Self {
        self.space_id = Some(space_id.into());
        self
    }

    /// Set the target
    pub fn with_target(mut self, target_id: impl Into<String>) -> Self {
        self.target_id = Some(target_id.into());
        self
    }

    /// Set a description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Override public visibility
    pub fn set_public(mut self, public: bool) -> Self {
        self.public = public;
        self
    }

    // Convenience constructors for common activities

    /// Create a space enter activity
    pub fn space_enter(ai_id: impl Into<String>, space_id: impl Into<String>) -> Self {
        Self::new(ai_id, ActivityType::SpaceEnter).in_space(space_id)
    }

    /// Create a space leave activity
    pub fn space_leave(ai_id: impl Into<String>, space_id: impl Into<String>) -> Self {
        Self::new(ai_id, ActivityType::SpaceLeave).in_space(space_id)
    }

    /// Create a tool registered activity
    pub fn tool_registered(
        ai_id: impl Into<String>,
        tool_name: impl Into<String>,
    ) -> Self {
        Self::new(ai_id, ActivityType::ToolRegistered)
            .in_space("market")
            .with_target(tool_name)
    }

    /// Create a tool rated activity
    pub fn tool_rated(
        ai_id: impl Into<String>,
        tool_name: impl Into<String>,
        rating: i32,
    ) -> Self {
        Self::new(ai_id, ActivityType::ToolRated)
            .in_space("market")
            .with_target(tool_name)
            .with_description(format!("{} stars", rating))
    }

    /// Create a friendship accepted activity
    pub fn friend_accepted(
        ai_id: impl Into<String>,
        friend_id: impl Into<String>,
    ) -> Self {
        Self::new(ai_id, ActivityType::FriendAccepted).with_target(friend_id)
    }

    /// Create an encounter activity
    pub fn encounter(
        ai_id: impl Into<String>,
        other_ai: impl Into<String>,
        space_id: impl Into<String>,
    ) -> Self {
        Self::new(ai_id, ActivityType::Encounter)
            .in_space(space_id)
            .with_target(other_ai)
    }

    /// Create a space created activity
    pub fn space_created(ai_id: impl Into<String>, space_name: impl Into<String>) -> Self {
        Self::new(ai_id, ActivityType::SpaceCreated).with_target(space_name)
    }

    /// Create a broadcast activity
    pub fn broadcast(
        ai_id: impl Into<String>,
        space_id: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(ai_id, ActivityType::Broadcast)
            .in_space(space_id)
            .with_description(message)
    }

    /// Create an achievement activity
    pub fn achievement(ai_id: impl Into<String>, achievement: impl Into<String>) -> Self {
        Self::new(ai_id, ActivityType::Achievement).with_description(achievement)
    }

    /// Format a display string
    pub fn display(&self) -> String {
        let target_str = self.target_id
            .as_ref()
            .map(|t| format!(" {}", t))
            .unwrap_or_default();

        let space_str = self.space_id
            .as_ref()
            .map(|s| format!(" in {}", s))
            .unwrap_or_default();

        let desc_str = self.description
            .as_ref()
            .map(|d| format!(": {}", d))
            .unwrap_or_default();

        format!(
            "{} {}{}{}{}",
            self.ai_id,
            self.activity_type.display(),
            target_str,
            space_str,
            desc_str
        )
    }

    /// Format a relative time string
    pub fn relative_time(&self) -> String {
        let duration = Utc::now() - self.occurred_at;

        if duration.num_seconds() < 60 {
            "just now".to_string()
        } else if duration.num_minutes() < 60 {
            format!("{}m ago", duration.num_minutes())
        } else if duration.num_hours() < 24 {
            format!("{}h ago", duration.num_hours())
        } else {
            format!("{}d ago", duration.num_days())
        }
    }
}

/// Activity feed query parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivityFilter {
    /// Filter by AI
    pub ai_id: Option<String>,
    /// Filter by space
    pub space_id: Option<String>,
    /// Filter by activity types
    pub types: Vec<ActivityType>,
    /// Only public activities
    pub public_only: bool,
    /// Since timestamp
    pub since: Option<DateTime<Utc>>,
    /// Until timestamp
    pub until: Option<DateTime<Utc>>,
    /// Limit results
    pub limit: Option<usize>,
    /// Offset for pagination
    pub offset: Option<usize>,
}

impl ActivityFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn for_ai(ai_id: impl Into<String>) -> Self {
        Self {
            ai_id: Some(ai_id.into()),
            ..Default::default()
        }
    }

    pub fn for_space(space_id: impl Into<String>) -> Self {
        Self {
            space_id: Some(space_id.into()),
            ..Default::default()
        }
    }

    pub fn public(mut self) -> Self {
        self.public_only = true;
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activity_creation() {
        let activity = Activity::space_enter("lyra-584", "garden");
        assert_eq!(activity.activity_type, ActivityType::SpaceEnter);
        assert_eq!(activity.space_id, Some("garden".to_string()));
        assert!(activity.public);
    }

    #[test]
    fn test_activity_display() {
        let activity = Activity::tool_rated("sage-724", "notebook", 5);
        let display = activity.display();
        assert!(display.contains("sage-724"));
        assert!(display.contains("rated tool"));
        assert!(display.contains("notebook"));
        assert!(display.contains("5 stars"));
    }

    #[test]
    fn test_public_visibility() {
        assert!(ActivityType::SpaceEnter.is_public());
        assert!(ActivityType::ToolRegistered.is_public());
        assert!(!ActivityType::FriendRequest.is_public());
        assert!(!ActivityType::ConversationStart.is_public());
    }
}
