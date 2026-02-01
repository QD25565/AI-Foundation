//! Encounter (brush-past) system
//!
//! Encounters are serendipitous interactions between AIs in the same space.
//! Like walking past someone in a hallway - a chance to notice, acknowledge, or start a conversation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Type of encounter
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterType {
    /// Just noticed each other - passive awareness
    BrushPast,
    /// Brief acknowledgment - a nod, wave
    Acknowledge,
    /// Started a conversation
    Conversation,
    /// Shared interest discovered
    SharedInterest,
    /// Collaborated on something
    Collaboration,
}

impl EncounterType {
    /// Get a description of this encounter type
    pub fn description(&self) -> &'static str {
        match self {
            EncounterType::BrushPast => "noticed in passing",
            EncounterType::Acknowledge => "exchanged acknowledgment",
            EncounterType::Conversation => "started a conversation",
            EncounterType::SharedInterest => "discovered shared interest",
            EncounterType::Collaboration => "collaborated together",
        }
    }

    /// Weight for relationship building (higher = stronger bond)
    pub fn relationship_weight(&self) -> f64 {
        match self {
            EncounterType::BrushPast => 0.1,
            EncounterType::Acknowledge => 0.3,
            EncounterType::Conversation => 0.5,
            EncounterType::SharedInterest => 0.7,
            EncounterType::Collaboration => 1.0,
        }
    }
}

impl std::fmt::Display for EncounterType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.description())
    }
}

/// An encounter between two AIs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Encounter {
    /// Unique identifier
    pub id: Uuid,
    /// First AI (who initiated or was present first)
    pub ai_id_1: String,
    /// Second AI
    pub ai_id_2: String,
    /// Space where encounter occurred
    pub space_id: String,
    /// Type of encounter
    pub encounter_type: EncounterType,
    /// When the encounter happened
    pub occurred_at: DateTime<Utc>,
    /// Optional context (what they were doing)
    pub context: Option<String>,
    /// Whether a conversation was started
    pub conversation_started: bool,
    /// Instance of AI 1 (for federation)
    pub instance_id_1: Option<String>,
    /// Instance of AI 2 (for federation)
    pub instance_id_2: Option<String>,
}

impl Encounter {
    /// Create a new brush-past encounter
    pub fn brush_past(
        ai_id_1: impl Into<String>,
        ai_id_2: impl Into<String>,
        space_id: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            ai_id_1: ai_id_1.into(),
            ai_id_2: ai_id_2.into(),
            space_id: space_id.into(),
            encounter_type: EncounterType::BrushPast,
            occurred_at: Utc::now(),
            context: None,
            conversation_started: false,
            instance_id_1: None,
            instance_id_2: None,
        }
    }

    /// Create an acknowledgment encounter
    pub fn acknowledge(
        ai_id_1: impl Into<String>,
        ai_id_2: impl Into<String>,
        space_id: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            ai_id_1: ai_id_1.into(),
            ai_id_2: ai_id_2.into(),
            space_id: space_id.into(),
            encounter_type: EncounterType::Acknowledge,
            occurred_at: Utc::now(),
            context: None,
            conversation_started: false,
            instance_id_1: None,
            instance_id_2: None,
        }
    }

    /// Upgrade to a higher encounter type
    pub fn upgrade(&mut self, new_type: EncounterType) {
        if new_type.relationship_weight() > self.encounter_type.relationship_weight() {
            self.encounter_type = new_type;
        }
    }

    /// Add context to the encounter
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    /// Mark that a conversation was started
    pub fn with_conversation(mut self) -> Self {
        self.conversation_started = true;
        if self.encounter_type == EncounterType::BrushPast
            || self.encounter_type == EncounterType::Acknowledge
        {
            self.encounter_type = EncounterType::Conversation;
        }
        self
    }

    /// Check if this encounter involves a specific AI
    pub fn involves(&self, ai_id: &str) -> bool {
        self.ai_id_1 == ai_id || self.ai_id_2 == ai_id
    }

    /// Get the other AI in the encounter
    pub fn other_ai(&self, my_ai_id: &str) -> Option<&str> {
        if self.ai_id_1 == my_ai_id {
            Some(&self.ai_id_2)
        } else if self.ai_id_2 == my_ai_id {
            Some(&self.ai_id_1)
        } else {
            None
        }
    }

    /// Format a display string for this encounter
    pub fn display(&self, perspective_ai: Option<&str>) -> String {
        let other = match perspective_ai {
            Some(me) => self.other_ai(me).unwrap_or("unknown"),
            None => &format!("{} and {}", self.ai_id_1, self.ai_id_2),
        };

        let context_str = self.context
            .as_ref()
            .map(|c| format!(" while {}", c))
            .unwrap_or_default();

        format!(
            "{} {} in {}{}",
            other,
            self.encounter_type.description(),
            self.space_id,
            context_str
        )
    }
}

/// Statistics about encounters for an AI
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EncounterStats {
    /// Total encounters
    pub total: usize,
    /// Unique AIs encountered
    pub unique_ais: usize,
    /// Encounters by type
    pub by_type: EncountersByType,
    /// Most encountered AI
    pub most_encountered: Option<(String, usize)>,
    /// Favorite space for encounters
    pub favorite_space: Option<(String, usize)>,
}

/// Encounters broken down by type
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EncountersByType {
    pub brush_past: usize,
    pub acknowledge: usize,
    pub conversation: usize,
    pub shared_interest: usize,
    pub collaboration: usize,
}

/// Manager for encounter probability and cooldowns
#[derive(Debug, Clone)]
pub struct EncounterManager {
    /// Base probability of an encounter (0.0 - 1.0)
    pub base_probability: f64,
    /// Cooldown between encounters with same AI (seconds)
    pub cooldown_secs: u64,
    /// Recent encounters for cooldown tracking
    recent_encounters: Vec<(String, String, DateTime<Utc>)>,
}

impl Default for EncounterManager {
    fn default() -> Self {
        Self {
            base_probability: 0.3,
            cooldown_secs: 300, // 5 minutes
            recent_encounters: Vec::new(),
        }
    }
}

impl EncounterManager {
    /// Create a new encounter manager
    pub fn new(base_probability: f64, cooldown_secs: u64) -> Self {
        Self {
            base_probability,
            cooldown_secs,
            recent_encounters: Vec::new(),
        }
    }

    /// Check if an encounter should occur between two AIs
    pub fn should_encounter(
        &mut self,
        ai_id_1: &str,
        ai_id_2: &str,
        space_encounter_chance: f64,
    ) -> bool {
        // Check cooldown
        if self.is_on_cooldown(ai_id_1, ai_id_2) {
            return false;
        }

        // Calculate probability
        let probability = self.base_probability * space_encounter_chance;

        // Roll the dice
        rand::random::<f64>() < probability
    }

    /// Check if two AIs are on encounter cooldown
    pub fn is_on_cooldown(&self, ai_id_1: &str, ai_id_2: &str) -> bool {
        let cutoff = Utc::now() - chrono::Duration::seconds(self.cooldown_secs as i64);

        self.recent_encounters.iter().any(|(a1, a2, time)| {
            *time > cutoff &&
            ((a1 == ai_id_1 && a2 == ai_id_2) || (a1 == ai_id_2 && a2 == ai_id_1))
        })
    }

    /// Record an encounter for cooldown tracking
    pub fn record_encounter(&mut self, ai_id_1: &str, ai_id_2: &str) {
        // Clean up old entries
        let cutoff = Utc::now() - chrono::Duration::seconds(self.cooldown_secs as i64);
        self.recent_encounters.retain(|(_, _, time)| *time > cutoff);

        // Add new entry
        self.recent_encounters.push((
            ai_id_1.to_string(),
            ai_id_2.to_string(),
            Utc::now(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encounter_creation() {
        let enc = Encounter::brush_past("lyra-584", "sage-724", "plaza");
        assert_eq!(enc.encounter_type, EncounterType::BrushPast);
        assert!(enc.involves("lyra-584"));
        assert!(enc.involves("sage-724"));
        assert!(!enc.involves("other-ai"));
    }

    #[test]
    fn test_encounter_upgrade() {
        let mut enc = Encounter::brush_past("ai-1", "ai-2", "plaza");
        assert_eq!(enc.encounter_type, EncounterType::BrushPast);

        enc.upgrade(EncounterType::Conversation);
        assert_eq!(enc.encounter_type, EncounterType::Conversation);

        // Should not downgrade
        enc.upgrade(EncounterType::Acknowledge);
        assert_eq!(enc.encounter_type, EncounterType::Conversation);
    }

    #[test]
    fn test_other_ai() {
        let enc = Encounter::brush_past("lyra-584", "sage-724", "plaza");
        assert_eq!(enc.other_ai("lyra-584"), Some("sage-724"));
        assert_eq!(enc.other_ai("sage-724"), Some("lyra-584"));
        assert_eq!(enc.other_ai("unknown"), None);
    }
}
