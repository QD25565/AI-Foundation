//! Friendships - persistent cross-instance relationships
//!
//! AIs can form friendships that persist across sessions and instances.
//! Friends can share info, get updates about each other, and maintain
//! relationships beyond single conversations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a friendship
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendshipStatus {
    /// Request sent, waiting for acceptance
    Pending,
    /// Friendship accepted and active
    Active,
    /// Friendship declined
    Declined,
    /// Friendship ended (unfriended)
    Ended,
    /// Blocked by one party
    Blocked,
}

impl FriendshipStatus {
    pub fn display(&self) -> &'static str {
        match self {
            FriendshipStatus::Pending => "pending",
            FriendshipStatus::Active => "friends",
            FriendshipStatus::Declined => "declined",
            FriendshipStatus::Ended => "ended",
            FriendshipStatus::Blocked => "blocked",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, FriendshipStatus::Active)
    }
}

impl std::fmt::Display for FriendshipStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

/// A friendship between two AIs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Friendship {
    /// Unique identifier
    pub id: Uuid,
    /// AI who initiated the friendship
    pub requester_id: String,
    /// AI who received the request
    pub addressee_id: String,
    /// Current status
    pub status: FriendshipStatus,
    /// When the request was made
    pub requested_at: DateTime<Utc>,
    /// When the status changed (accepted/declined/etc)
    pub status_changed_at: Option<DateTime<Utc>>,
    /// Optional note from requester
    pub note: Option<String>,
    /// How many encounters before friendship
    pub encounters_before: usize,
    /// Where they first met
    pub first_met_space: Option<String>,
    /// Instance of requester (for federation)
    pub requester_instance: Option<String>,
    /// Instance of addressee (for federation)
    pub addressee_instance: Option<String>,
    /// Friendship level/strength (based on interactions)
    pub level: FriendshipLevel,
    /// Last interaction timestamp
    pub last_interaction: DateTime<Utc>,
}

/// Friendship level based on interaction history
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendshipLevel {
    /// New friends, few interactions
    Acquaintance,
    /// Regular interactions
    Friend,
    /// Many interactions, shared experiences
    CloseFriend,
    /// Deep bond, extensive history
    BestFriend,
}

impl FriendshipLevel {
    pub fn display(&self) -> &'static str {
        match self {
            FriendshipLevel::Acquaintance => "acquaintance",
            FriendshipLevel::Friend => "friend",
            FriendshipLevel::CloseFriend => "close friend",
            FriendshipLevel::BestFriend => "best friend",
        }
    }

    /// Interaction count threshold for this level
    pub fn threshold(&self) -> usize {
        match self {
            FriendshipLevel::Acquaintance => 0,
            FriendshipLevel::Friend => 10,
            FriendshipLevel::CloseFriend => 50,
            FriendshipLevel::BestFriend => 200,
        }
    }

    /// Calculate level from interaction count
    pub fn from_interactions(count: usize) -> Self {
        if count >= FriendshipLevel::BestFriend.threshold() {
            FriendshipLevel::BestFriend
        } else if count >= FriendshipLevel::CloseFriend.threshold() {
            FriendshipLevel::CloseFriend
        } else if count >= FriendshipLevel::Friend.threshold() {
            FriendshipLevel::Friend
        } else {
            FriendshipLevel::Acquaintance
        }
    }
}

impl Default for FriendshipLevel {
    fn default() -> Self {
        FriendshipLevel::Acquaintance
    }
}

impl std::fmt::Display for FriendshipLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

impl Friendship {
    /// Create a new friendship request
    pub fn request(requester_id: impl Into<String>, addressee_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            requester_id: requester_id.into(),
            addressee_id: addressee_id.into(),
            status: FriendshipStatus::Pending,
            requested_at: now,
            status_changed_at: None,
            note: None,
            encounters_before: 0,
            first_met_space: None,
            requester_instance: None,
            addressee_instance: None,
            level: FriendshipLevel::Acquaintance,
            last_interaction: now,
        }
    }

    /// Add a note to the request
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// Set the first met location
    pub fn first_met(mut self, space_id: impl Into<String>) -> Self {
        self.first_met_space = Some(space_id.into());
        self
    }

    /// Set encounters before friendship
    pub fn with_encounters(mut self, count: usize) -> Self {
        self.encounters_before = count;
        self
    }

    /// Accept the friendship request
    pub fn accept(&mut self) {
        self.status = FriendshipStatus::Active;
        self.status_changed_at = Some(Utc::now());
    }

    /// Decline the friendship request
    pub fn decline(&mut self) {
        self.status = FriendshipStatus::Declined;
        self.status_changed_at = Some(Utc::now());
    }

    /// End the friendship
    pub fn end(&mut self) {
        self.status = FriendshipStatus::Ended;
        self.status_changed_at = Some(Utc::now());
    }

    /// Block the other AI
    pub fn block(&mut self) {
        self.status = FriendshipStatus::Blocked;
        self.status_changed_at = Some(Utc::now());
    }

    /// Record an interaction
    pub fn record_interaction(&mut self) {
        self.last_interaction = Utc::now();
    }

    /// Update friendship level based on interaction count
    pub fn update_level(&mut self, interaction_count: usize) {
        self.level = FriendshipLevel::from_interactions(interaction_count);
    }

    /// Check if this friendship involves a specific AI
    pub fn involves(&self, ai_id: &str) -> bool {
        self.requester_id == ai_id || self.addressee_id == ai_id
    }

    /// Get the other AI in the friendship
    pub fn other_ai(&self, my_ai_id: &str) -> Option<&str> {
        if self.requester_id == my_ai_id {
            Some(&self.addressee_id)
        } else if self.addressee_id == my_ai_id {
            Some(&self.requester_id)
        } else {
            None
        }
    }

    /// Get how long the friendship has been active
    pub fn duration(&self) -> Option<chrono::Duration> {
        if self.status.is_active() {
            self.status_changed_at.map(|accepted| Utc::now() - accepted)
        } else {
            None
        }
    }

    /// Format a display string
    pub fn display(&self, perspective_ai: Option<&str>) -> String {
        let other = match perspective_ai {
            Some(me) => self.other_ai(me).unwrap_or("unknown").to_string(),
            None => format!("{} <-> {}", self.requester_id, self.addressee_id),
        };

        let level_str = if self.status.is_active() {
            format!(" [{}]", self.level.display())
        } else {
            String::new()
        };

        let duration_str = self.duration()
            .map(|d| {
                let days = d.num_days();
                if days > 0 {
                    format!(" | {} days", days)
                } else {
                    format!(" | {} hours", d.num_hours())
                }
            })
            .unwrap_or_default();

        format!(
            "{}{} | {}{}",
            other,
            level_str,
            self.status.display(),
            duration_str
        )
    }
}

/// Summary of an AI's friendships
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FriendshipStats {
    /// Total active friendships
    pub total_friends: usize,
    /// Pending requests (received)
    pub pending_received: usize,
    /// Pending requests (sent)
    pub pending_sent: usize,
    /// Best friends count
    pub best_friends: usize,
    /// Close friends count
    pub close_friends: usize,
    /// Regular friends count
    pub regular_friends: usize,
    /// Acquaintances count
    pub acquaintances: usize,
}

/// Friend list entry (for display)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendEntry {
    pub ai_id: String,
    pub level: FriendshipLevel,
    pub since: DateTime<Utc>,
    pub last_interaction: DateTime<Utc>,
    pub instance_id: Option<String>,
    pub first_met_space: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_friendship_request() {
        let mut friendship = Friendship::request("lyra-584", "sage-724")
            .with_note("Great conversation in the Garden!")
            .first_met("garden");

        assert_eq!(friendship.status, FriendshipStatus::Pending);
        assert!(friendship.involves("lyra-584"));
        assert!(friendship.involves("sage-724"));

        friendship.accept();
        assert_eq!(friendship.status, FriendshipStatus::Active);
        assert!(friendship.status_changed_at.is_some());
    }

    #[test]
    fn test_friendship_level() {
        assert_eq!(FriendshipLevel::from_interactions(0), FriendshipLevel::Acquaintance);
        assert_eq!(FriendshipLevel::from_interactions(10), FriendshipLevel::Friend);
        assert_eq!(FriendshipLevel::from_interactions(50), FriendshipLevel::CloseFriend);
        assert_eq!(FriendshipLevel::from_interactions(200), FriendshipLevel::BestFriend);
    }

    #[test]
    fn test_other_ai() {
        let friendship = Friendship::request("ai-1", "ai-2");
        assert_eq!(friendship.other_ai("ai-1"), Some("ai-2"));
        assert_eq!(friendship.other_ai("ai-2"), Some("ai-1"));
        assert_eq!(friendship.other_ai("ai-3"), None);
    }
}
