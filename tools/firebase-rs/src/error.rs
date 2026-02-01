//! Error types for Firebase CLI
//!
//! Following AI-Foundation philosophy: Fail loudly, no silent errors

use thiserror::Error;

/// Result type for Firebase operations
pub type Result<T> = std::result::Result<T, FirebaseError>;

/// Firebase CLI errors - explicit and informative
#[derive(Error, Debug)]
pub enum FirebaseError {
    #[error("Authentication failed: {0}")]
    AuthError(String),

    #[error("Service account file not found: {0}")]
    ServiceAccountNotFound(String),

    #[error("Invalid service account format: {0}")]
    InvalidServiceAccount(String),

    #[error("Token generation failed: {0}")]
    TokenError(String),

    #[error("API request failed: {status} - {message}")]
    ApiError { status: u16, message: String },

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("JSON parsing error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("JWT error: {0}")]
    JwtError(#[from] jsonwebtoken::errors::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Project ID not configured. Set FIREBASE_PROJECT_ID or use --project")]
    ProjectIdMissing,

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Rate limited. Retry after {0} seconds")]
    RateLimited(u64),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
}

impl FirebaseError {
    /// Create API error from response status and body
    pub fn from_response(status: u16, body: &str) -> Self {
        // Try to parse Firebase error format
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
            if let Some(error) = json.get("error") {
                let message = error
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or(body);
                return FirebaseError::ApiError {
                    status,
                    message: message.to_string(),
                };
            }
        }

        FirebaseError::ApiError {
            status,
            message: body.to_string(),
        }
    }

    /// Check if error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            FirebaseError::NetworkError(_)
                | FirebaseError::RateLimited(_)
                | FirebaseError::ApiError { status: 500..=599, .. }
        )
    }
}
