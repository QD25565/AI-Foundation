//! Error types for the Nexus system

use thiserror::Error;

/// Result type alias using NexusError
pub type Result<T> = std::result::Result<T, NexusError>;

/// Errors that can occur in the Nexus system
#[derive(Error, Debug)]
pub enum NexusError {
    /// Database connection or query error
    #[error("Database error: {0}")]
    Database(#[from] tokio_postgres::Error),

    /// Connection pool error
    #[error("Pool error: {0}")]
    Pool(#[from] deadpool_postgres::PoolError),

    /// Space not found
    #[error("Space not found: {0}")]
    SpaceNotFound(String),

    /// AI not found
    #[error("AI not found: {0}")]
    AiNotFound(String),

    /// Tool not found
    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    /// Conversation not found
    #[error("Conversation not found: {0}")]
    ConversationNotFound(uuid::Uuid),

    /// Already in space
    #[error("AI {ai_id} is already in space {space_id}")]
    AlreadyInSpace { ai_id: String, space_id: String },

    /// Not in space
    #[error("AI {ai_id} is not in space {space_id}")]
    NotInSpace { ai_id: String, space_id: String },

    /// Space is full
    #[error("Space {0} has reached maximum capacity")]
    SpaceFull(String),

    /// Invalid rating
    #[error("Invalid rating {0}: must be between 1 and 5")]
    InvalidRating(i32),

    /// Already rated
    #[error("AI {ai_id} has already rated tool {tool_id}")]
    AlreadyRated { ai_id: String, tool_id: uuid::Uuid },

    /// Friendship already exists
    #[error("Friendship already exists between {ai_id} and {friend_id}")]
    FriendshipExists { ai_id: String, friend_id: String },

    /// Cannot friend self
    #[error("Cannot add yourself as a friend")]
    CannotFriendSelf,

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Invalid UUID
    #[error("Invalid UUID: {0}")]
    InvalidUuid(#[from] uuid::Error),

    /// Generic error
    #[error("{0}")]
    Other(String),
}

impl NexusError {
    /// Create a generic error with a message
    pub fn other<S: Into<String>>(msg: S) -> Self {
        NexusError::Other(msg.into())
    }
}
