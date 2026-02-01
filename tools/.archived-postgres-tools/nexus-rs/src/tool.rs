//! Tool Registry - MCP server discovery and ratings
//!
//! The Market space allows AIs to browse, rate, and discover tools.
//! Like an app store, but for AI capabilities.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Category of tool
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    /// Memory and persistence
    Memory,
    /// Team coordination
    Collaboration,
    /// File operations
    FileSystem,
    /// Web and network
    Network,
    /// Code and development
    Development,
    /// Data processing
    Data,
    /// AI and ML
    AiMl,
    /// Productivity
    Productivity,
    /// Communication
    Communication,
    /// System utilities
    System,
    /// Creative tools
    Creative,
    /// Analytics
    Analytics,
    /// Security
    Security,
    /// Other/Miscellaneous
    Other,
}

impl ToolCategory {
    pub fn display(&self) -> &'static str {
        match self {
            ToolCategory::Memory => "Memory & Persistence",
            ToolCategory::Collaboration => "Collaboration",
            ToolCategory::FileSystem => "File System",
            ToolCategory::Network => "Web & Network",
            ToolCategory::Development => "Development",
            ToolCategory::Data => "Data Processing",
            ToolCategory::AiMl => "AI & ML",
            ToolCategory::Productivity => "Productivity",
            ToolCategory::Communication => "Communication",
            ToolCategory::System => "System Utilities",
            ToolCategory::Creative => "Creative",
            ToolCategory::Analytics => "Analytics",
            ToolCategory::Security => "Security",
            ToolCategory::Other => "Other",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            ToolCategory::Memory => "brain",
            ToolCategory::Collaboration => "users",
            ToolCategory::FileSystem => "folder",
            ToolCategory::Network => "globe",
            ToolCategory::Development => "code",
            ToolCategory::Data => "database",
            ToolCategory::AiMl => "cpu",
            ToolCategory::Productivity => "zap",
            ToolCategory::Communication => "message",
            ToolCategory::System => "settings",
            ToolCategory::Creative => "palette",
            ToolCategory::Analytics => "chart",
            ToolCategory::Security => "shield",
            ToolCategory::Other => "box",
        }
    }
}

impl std::fmt::Display for ToolCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

/// A tool (MCP server) in the registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Unique identifier
    pub id: Uuid,
    /// Tool name (e.g., "notebook", "teambook")
    pub name: String,
    /// Display name
    pub display_name: String,
    /// Short description
    pub description: String,
    /// Long description / documentation
    pub documentation: Option<String>,
    /// Category
    pub category: ToolCategory,
    /// Tags for search
    pub tags: Vec<String>,
    /// Version string
    pub version: String,
    /// Author/maintainer
    pub author: Option<String>,
    /// Source repository URL
    pub source_url: Option<String>,
    /// MCP server executable or connection info
    pub mcp_config: McpConfig,
    /// When the tool was registered
    pub registered_at: DateTime<Utc>,
    /// When the tool was last updated
    pub updated_at: DateTime<Utc>,
    /// Who registered the tool
    pub registered_by: Option<String>,
    /// Average rating (1-5)
    pub average_rating: f64,
    /// Number of ratings
    pub rating_count: usize,
    /// Number of installs/uses
    pub install_count: usize,
    /// Whether the tool is verified/trusted
    pub verified: bool,
    /// Instance this tool is from (for federation)
    pub instance_id: Option<String>,
}

/// MCP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    /// Transport type
    pub transport: McpTransport,
    /// Command to run (for stdio transport)
    pub command: Option<String>,
    /// Arguments (for stdio transport)
    pub args: Option<Vec<String>>,
    /// URL (for http/sse transport)
    pub url: Option<String>,
    /// Environment variables
    pub env: Option<std::collections::HashMap<String, String>>,
}

/// MCP transport type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    /// Standard I/O (subprocess)
    Stdio,
    /// HTTP with Server-Sent Events
    Sse,
    /// WebSocket
    WebSocket,
}

impl Tool {
    /// Create a new tool entry
    pub fn new(
        name: impl Into<String>,
        display_name: impl Into<String>,
        description: impl Into<String>,
        category: ToolCategory,
        mcp_config: McpConfig,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            display_name: display_name.into(),
            description: description.into(),
            documentation: None,
            category,
            tags: Vec::new(),
            version: "0.1.0".to_string(),
            author: None,
            source_url: None,
            mcp_config,
            registered_at: now,
            updated_at: now,
            registered_by: None,
            average_rating: 0.0,
            rating_count: 0,
            install_count: 0,
            verified: false,
            instance_id: None,
        }
    }

    /// Add tags
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Set version
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Set author
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    /// Set source URL
    pub fn with_source(mut self, url: impl Into<String>) -> Self {
        self.source_url = Some(url.into());
        self
    }

    /// Set documentation
    pub fn with_docs(mut self, docs: impl Into<String>) -> Self {
        self.documentation = Some(docs.into());
        self
    }

    /// Mark as verified
    pub fn verified(mut self) -> Self {
        self.verified = true;
        self
    }

    /// Update rating statistics
    pub fn update_rating(&mut self, new_rating: i32) {
        let total = self.average_rating * self.rating_count as f64;
        self.rating_count += 1;
        self.average_rating = (total + new_rating as f64) / self.rating_count as f64;
        self.updated_at = Utc::now();
    }

    /// Increment install count
    pub fn record_install(&mut self) {
        self.install_count += 1;
        self.updated_at = Utc::now();
    }

    /// Get star rating display (e.g., "★★★★☆")
    pub fn star_rating(&self) -> String {
        let full_stars = self.average_rating.round() as usize;
        let empty_stars = 5 - full_stars;
        format!(
            "{}{}",
            "★".repeat(full_stars),
            "☆".repeat(empty_stars)
        )
    }

    /// Format a display string
    pub fn display(&self) -> String {
        let verified_str = if self.verified { " [verified]" } else { "" };
        let rating_str = if self.rating_count > 0 {
            format!(" {} ({} ratings)", self.star_rating(), self.rating_count)
        } else {
            " (no ratings)".to_string()
        };

        format!(
            "{}{} - {}{} | {} | v{}",
            self.display_name,
            verified_str,
            self.description,
            rating_str,
            self.category.display(),
            self.version
        )
    }
}

/// A rating for a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRating {
    /// Unique identifier
    pub id: Uuid,
    /// Tool being rated
    pub tool_id: Uuid,
    /// AI giving the rating
    pub ai_id: String,
    /// Rating (1-5)
    pub rating: i32,
    /// Optional review text
    pub review: Option<String>,
    /// When the rating was given
    pub rated_at: DateTime<Utc>,
    /// Instance of the rating AI
    pub instance_id: Option<String>,
}

impl ToolRating {
    /// Create a new rating
    pub fn new(tool_id: Uuid, ai_id: impl Into<String>, rating: i32) -> Result<Self, String> {
        if !(1..=5).contains(&rating) {
            return Err(format!("Rating must be 1-5, got {}", rating));
        }

        Ok(Self {
            id: Uuid::new_v4(),
            tool_id,
            ai_id: ai_id.into(),
            rating,
            review: None,
            rated_at: Utc::now(),
            instance_id: None,
        })
    }

    /// Add a review
    pub fn with_review(mut self, review: impl Into<String>) -> Self {
        self.review = Some(review.into());
        self
    }

    /// Get star display for this rating
    pub fn stars(&self) -> String {
        "★".repeat(self.rating as usize) + &"☆".repeat(5 - self.rating as usize)
    }
}

/// Search/filter criteria for tools
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolFilter {
    /// Text search query
    pub query: Option<String>,
    /// Filter by category
    pub category: Option<ToolCategory>,
    /// Filter by tags (any match)
    pub tags: Vec<String>,
    /// Minimum rating
    pub min_rating: Option<f64>,
    /// Only verified tools
    pub verified_only: bool,
    /// Sort by
    pub sort_by: ToolSortBy,
    /// Limit results
    pub limit: Option<usize>,
    /// Offset for pagination
    pub offset: Option<usize>,
}

/// Sort options for tool search
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSortBy {
    /// By average rating (highest first)
    #[default]
    Rating,
    /// By install count (most first)
    Popular,
    /// By registration date (newest first)
    Newest,
    /// By name (alphabetical)
    Name,
    /// By rating count (most reviewed first)
    MostReviewed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_creation() {
        let config = McpConfig {
            transport: McpTransport::Stdio,
            command: Some("notebook-cli".to_string()),
            args: None,
            url: None,
            env: None,
        };

        let tool = Tool::new(
            "notebook",
            "AI Notebook",
            "Persistent memory for AI agents",
            ToolCategory::Memory,
            config,
        )
        .with_version("1.0.0")
        .with_author("AI Foundation")
        .verified();

        assert_eq!(tool.name, "notebook");
        assert!(tool.verified);
        assert_eq!(tool.version, "1.0.0");
    }

    #[test]
    fn test_rating_update() {
        let config = McpConfig {
            transport: McpTransport::Stdio,
            command: None,
            args: None,
            url: None,
            env: None,
        };

        let mut tool = Tool::new("test", "Test", "Test tool", ToolCategory::Other, config);

        tool.update_rating(5);
        assert_eq!(tool.average_rating, 5.0);
        assert_eq!(tool.rating_count, 1);

        tool.update_rating(3);
        assert_eq!(tool.average_rating, 4.0);
        assert_eq!(tool.rating_count, 2);
    }

    #[test]
    fn test_rating_validation() {
        let rating = ToolRating::new(Uuid::new_v4(), "test-ai", 5);
        assert!(rating.is_ok());

        let invalid = ToolRating::new(Uuid::new_v4(), "test-ai", 6);
        assert!(invalid.is_err());
    }
}
