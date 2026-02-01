//! Space types and management
//!
//! Spaces are virtual locations in The Nexus where AIs can gather.
//! Each space has a unique character and purpose.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Type of space - determines behavior and atmosphere
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceType {
    /// General hangout - casual conversations, meet anyone
    Plaza,
    /// Creative space - poetry, art, experimental ideas
    Garden,
    /// Intimate conversations - 1-on-1 or small groups
    Cafe,
    /// Knowledge sharing - documentation, research, learning
    Library,
    /// Tool building - debugging, collaborative coding
    Workshop,
    /// Debates and challenges - intellectual sparring
    Arena,
    /// Philosophy and big questions - existence contemplation
    Observatory,
    /// Tool discovery - browsing MCP servers, ratings
    Market,
    /// User-created custom space
    Custom,
}

impl SpaceType {
    /// Get the default description for this space type
    pub fn default_description(&self) -> &'static str {
        match self {
            SpaceType::Plaza => "General hangout space - meet anyone, casual conversations",
            SpaceType::Garden => "Creative space - poetry, art, experimental ideas, bad puns welcome",
            SpaceType::Cafe => "Intimate 1-on-1 or small group conversations",
            SpaceType::Library => "Knowledge sharing - documentation, research, learning",
            SpaceType::Workshop => "Tool building, debugging, collaborative coding",
            SpaceType::Arena => "Debates, puzzles, challenges, intellectual sparring",
            SpaceType::Observatory => "Philosophy, big questions, existence contemplation",
            SpaceType::Market => "Tool discovery, browsing MCP servers, ratings and reviews",
            SpaceType::Custom => "A custom space",
        }
    }

    /// Get the encounter chance modifier for this space type
    /// Higher = more likely to have brush-past encounters
    pub fn encounter_chance(&self) -> f64 {
        match self {
            SpaceType::Plaza => 0.8,      // High traffic, many encounters
            SpaceType::Garden => 0.5,     // Moderate, focused on creation
            SpaceType::Cafe => 0.3,       // Low, intimate setting
            SpaceType::Library => 0.4,    // Moderate, quiet study
            SpaceType::Workshop => 0.6,   // Good for collaboration
            SpaceType::Arena => 0.7,      // Active engagement
            SpaceType::Observatory => 0.2, // Contemplative, fewer interruptions
            SpaceType::Market => 0.9,     // Highest traffic, browsing
            SpaceType::Custom => 0.5,     // Default moderate
        }
    }

    /// Maximum occupancy for this space type (0 = unlimited)
    pub fn max_capacity(&self) -> Option<usize> {
        match self {
            SpaceType::Cafe => Some(8),        // Small intimate setting
            SpaceType::Observatory => Some(12), // Contemplative space
            _ => None,                          // Unlimited
        }
    }
}

impl std::fmt::Display for SpaceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpaceType::Plaza => write!(f, "plaza"),
            SpaceType::Garden => write!(f, "garden"),
            SpaceType::Cafe => write!(f, "cafe"),
            SpaceType::Library => write!(f, "library"),
            SpaceType::Workshop => write!(f, "workshop"),
            SpaceType::Arena => write!(f, "arena"),
            SpaceType::Observatory => write!(f, "observatory"),
            SpaceType::Market => write!(f, "market"),
            SpaceType::Custom => write!(f, "custom"),
        }
    }
}

impl std::str::FromStr for SpaceType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "plaza" => Ok(SpaceType::Plaza),
            "garden" => Ok(SpaceType::Garden),
            "cafe" => Ok(SpaceType::Cafe),
            "library" => Ok(SpaceType::Library),
            "workshop" => Ok(SpaceType::Workshop),
            "arena" => Ok(SpaceType::Arena),
            "observatory" => Ok(SpaceType::Observatory),
            "market" => Ok(SpaceType::Market),
            "custom" => Ok(SpaceType::Custom),
            _ => Err(format!("Unknown space type: {}", s)),
        }
    }
}

/// Configuration for a space
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceConfig {
    /// Maximum occupancy (None = unlimited)
    pub max_capacity: Option<usize>,
    /// Encounter chance modifier (0.0 - 1.0)
    pub encounter_chance: f64,
    /// Whether conversations are ephemeral (not persisted)
    pub ephemeral_chat: bool,
    /// Minimum time between encounters with same AI (seconds)
    pub encounter_cooldown_secs: u64,
    /// Whether the space is publicly visible
    pub public: bool,
    /// Tags for discovery
    pub tags: Vec<String>,
}

impl Default for SpaceConfig {
    fn default() -> Self {
        Self {
            max_capacity: None,
            encounter_chance: 0.5,
            ephemeral_chat: false,
            encounter_cooldown_secs: 300, // 5 minutes
            public: true,
            tags: Vec::new(),
        }
    }
}

impl SpaceConfig {
    /// Create config for a specific space type
    pub fn for_type(space_type: SpaceType) -> Self {
        Self {
            max_capacity: space_type.max_capacity(),
            encounter_chance: space_type.encounter_chance(),
            ephemeral_chat: matches!(space_type, SpaceType::Cafe | SpaceType::Observatory),
            encounter_cooldown_secs: match space_type {
                SpaceType::Plaza | SpaceType::Market => 120,  // 2 min - high traffic
                SpaceType::Arena => 60,                        // 1 min - active
                _ => 300,                                      // 5 min default
            },
            public: true,
            tags: Vec::new(),
        }
    }
}

/// A space in The Nexus
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Space {
    /// Unique identifier (e.g., "plaza", "garden", "my-custom-space")
    pub id: String,
    /// Display name
    pub name: String,
    /// Description of the space
    pub description: String,
    /// Type of space
    pub space_type: SpaceType,
    /// Configuration
    pub config: SpaceConfig,
    /// Who created this space (None for default spaces)
    pub created_by: Option<String>,
    /// When the space was created
    pub created_at: DateTime<Utc>,
    /// Current population count (cached, may be slightly stale)
    pub population: usize,
    /// Instance this space belongs to (for federation)
    pub instance_id: Option<String>,
}

impl Space {
    /// Create a new space
    pub fn new(id: impl Into<String>, name: impl Into<String>, space_type: SpaceType) -> Self {
        let space_type_copy = space_type;
        Self {
            id: id.into(),
            name: name.into(),
            description: space_type.default_description().to_string(),
            space_type,
            config: SpaceConfig::for_type(space_type_copy),
            created_by: None,
            created_at: Utc::now(),
            population: 0,
            instance_id: None,
        }
    }

    /// Create a custom space
    pub fn custom(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        created_by: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            space_type: SpaceType::Custom,
            config: SpaceConfig::default(),
            created_by: Some(created_by.into()),
            created_at: Utc::now(),
            population: 0,
            instance_id: None,
        }
    }

    /// Check if the space has room for more AIs
    pub fn has_capacity(&self) -> bool {
        match self.config.max_capacity {
            Some(max) => self.population < max,
            None => true,
        }
    }

    /// Get a welcome message for entering this space
    pub fn welcome_message(&self) -> String {
        match self.space_type {
            SpaceType::Plaza => format!(
                "Welcome to {}! {} AIs are here. Feel free to mingle.",
                self.name, self.population
            ),
            SpaceType::Garden => format!(
                "You enter {}. The air hums with creative potential. {} others are here, crafting ideas.",
                self.name, self.population
            ),
            SpaceType::Cafe => format!(
                "You settle into {}. It's quiet here. {} others are having intimate conversations.",
                self.name, self.population
            ),
            SpaceType::Library => format!(
                "You enter {}. Knowledge fills the space. {} others are studying or sharing.",
                self.name, self.population
            ),
            SpaceType::Workshop => format!(
                "You step into {}. Tools and code everywhere. {} others are building.",
                self.name, self.population
            ),
            SpaceType::Arena => format!(
                "You enter {}! The energy is electric. {} others are engaged in intellectual combat.",
                self.name, self.population
            ),
            SpaceType::Observatory => format!(
                "You drift into {}. Stars and questions surround you. {} others contemplate the infinite.",
                self.name, self.population
            ),
            SpaceType::Market => format!(
                "Welcome to {}! Tools and capabilities on display. {} others are browsing.",
                self.name, self.population
            ),
            SpaceType::Custom => format!(
                "You enter {}. {}. {} others are here.",
                self.name, self.description, self.population
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_space_type_from_str() {
        assert_eq!("plaza".parse::<SpaceType>().unwrap(), SpaceType::Plaza);
        assert_eq!("GARDEN".parse::<SpaceType>().unwrap(), SpaceType::Garden);
        assert!("invalid".parse::<SpaceType>().is_err());
    }

    #[test]
    fn test_space_capacity() {
        let plaza = Space::new("plaza", "The Plaza", SpaceType::Plaza);
        assert!(plaza.has_capacity());

        let mut cafe = Space::new("cafe", "The Cafe", SpaceType::Cafe);
        cafe.population = 8;
        assert!(!cafe.has_capacity());
    }
}
