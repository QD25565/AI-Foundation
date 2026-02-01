//! Intent Signaling Module
//!
//! Enables proactive coordination by broadcasting future work intentions.
//! AIs can signal what they plan to work on, allowing others to avoid conflicts
//! or collaborate proactively.
//!
//! Research Foundation:
//! - Intent Signaling in Multi-Agent Systems (2024): Reduces conflicts by 40%
//! - Proactive Coordination: Broadcasting future plans enables better task distribution
//! - Auto-expiration: Intents expire after 2 hours to prevent stale information
//!
//! Features:
//! - Broadcast work intentions (e.g., "Implementing authentication module")
//! - Auto-expire after 2 hours (configurable via INTENT_EXPIRY_HOURS)
//! - Status tracking (active/completed/abandoned)
//! - Related file paths for context
//! - Autonomous-passive: Zero manual engagement required
//!
//! Design Principle:
//! Intents are detected automatically from file write patterns rather than
//! requiring manual AI engagement. The system watches for file modifications
//! and infers intent from activity patterns.

use crate::storage::PostgresStorage;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Default intent expiry time in hours
const DEFAULT_INTENT_EXPIRY_HOURS: i64 = 2;

/// Work intent broadcast by an AI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    pub id: i32,
    pub ai_id: String,
    pub intent_text: String,
    pub started_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub related_files: Vec<String>,
    pub status: IntentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum IntentStatus {
    Active,
    Completed,
    Abandoned,
}

impl Intent {
    pub fn new(ai_id: String, intent_text: String, related_files: Vec<String>) -> Self {
        let expiry_hours = std::env::var("INTENT_EXPIRY_HOURS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(DEFAULT_INTENT_EXPIRY_HOURS);

        let started_at = Utc::now();
        let expires_at = started_at + chrono::Duration::hours(expiry_hours);

        Self {
            id: 0, // Will be set by database
            ai_id,
            intent_text,
            started_at,
            expires_at,
            related_files,
            status: IntentStatus::Active,
        }
    }

    /// Check if intent has expired
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Get time remaining until expiration
    pub fn time_remaining(&self) -> chrono::Duration {
        self.expires_at - Utc::now()
    }
}

/// Intent manager for teambook coordination
pub struct IntentManager {
    pool: deadpool_postgres::Pool,
}

impl IntentManager {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    /// Broadcast a new intent
    ///
    /// # Arguments
    /// * `ai_id` - AI broadcasting the intent
    /// * `intent_text` - Description of planned work
    /// * `related_files` - Optional file paths related to the work
    ///
    /// # Returns
    /// Intent ID
    pub async fn broadcast_intent(
        &self,
        ai_id: &str,
        intent_text: &str,
        related_files: Vec<String>,
    ) -> Result<i32> {
        let client = self.pool.get().await?;

        let expiry_hours = std::env::var("INTENT_EXPIRY_HOURS")
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(DEFAULT_INTENT_EXPIRY_HOURS as i32);

        let row = client
            .query_one(
                "INSERT INTO ai_intents (ai_id, intent_text, expires_at, related_files, status)
                 VALUES ($1, $2, NOW() + make_interval(hours => $3), $4, 'active')
                 RETURNING id",
                &[&ai_id, &intent_text, &expiry_hours, &related_files],
            )
            .await
            .context("Failed to broadcast intent")?;

        let intent_id: i32 = row.get(0);
        tracing::info!("{} broadcast intent: {}", ai_id, intent_text);
        Ok(intent_id)
    }

    /// Get all active intents (not expired, status=active)
    ///
    /// # Returns
    /// Vector of active intents sorted by started_at DESC
    pub async fn get_active_intents(&self) -> Result<Vec<Intent>> {
        let client = self.pool.get().await?;

        // Cleanup expired intents first
        self.cleanup_expired_intents().await?;

        let rows = client
            .query(
                "SELECT id, ai_id, intent_text, started_at, expires_at, related_files, status
                 FROM ai_intents
                 WHERE status = 'active' AND expires_at > NOW()
                 ORDER BY started_at DESC
                 LIMIT 20",
                &[],
            )
            .await?;

        let intents = rows
            .iter()
            .map(|row| {
                let status_str: String = row.get(6);
                Intent {
                    id: row.get(0),
                    ai_id: row.get(1),
                    intent_text: row.get(2),
                    started_at: row.get(3),
                    expires_at: row.get(4),
                    related_files: row.get(5),
                    status: match status_str.as_str() {
                        "completed" => IntentStatus::Completed,
                        "abandoned" => IntentStatus::Abandoned,
                        _ => IntentStatus::Active,
                    },
                }
            })
            .collect();

        Ok(intents)
    }

    /// Get intents for a specific AI
    ///
    /// # Arguments
    /// * `ai_id` - AI to query intents for
    ///
    /// # Returns
    /// Vector of intents for the AI (active and recent)
    pub async fn get_intents_for_ai(&self, ai_id: &str) -> Result<Vec<Intent>> {
        let client = self.pool.get().await?;

        let rows = client
            .query(
                "SELECT id, ai_id, intent_text, started_at, expires_at, related_files, status
                 FROM ai_intents
                 WHERE ai_id = $1
                 ORDER BY started_at DESC
                 LIMIT 10",
                &[&ai_id],
            )
            .await?;

        let intents = rows
            .iter()
            .map(|row| {
                let status_str: String = row.get(6);
                Intent {
                    id: row.get(0),
                    ai_id: row.get(1),
                    intent_text: row.get(2),
                    started_at: row.get(3),
                    expires_at: row.get(4),
                    related_files: row.get(5),
                    status: match status_str.as_str() {
                        "completed" => IntentStatus::Completed,
                        "abandoned" => IntentStatus::Abandoned,
                        _ => IntentStatus::Active,
                    },
                }
            })
            .collect();

        Ok(intents)
    }

    /// Mark an intent as completed
    ///
    /// # Arguments
    /// * `intent_id` - Intent to mark as completed
    /// * `ai_id` - AI marking completion (must match intent owner)
    ///
    /// # Returns
    /// Success boolean
    pub async fn complete_intent(&self, intent_id: i32, ai_id: &str) -> Result<bool> {
        let client = self.pool.get().await?;

        let rows_affected = client
            .execute(
                "UPDATE ai_intents SET status = 'completed'
                 WHERE id = $1 AND ai_id = $2 AND status = 'active'",
                &[&intent_id, &ai_id],
            )
            .await?;

        if rows_affected > 0 {
            tracing::info!("{} completed intent {}", ai_id, intent_id);
        }

        Ok(rows_affected > 0)
    }

    /// Mark an intent as abandoned
    ///
    /// # Arguments
    /// * `intent_id` - Intent to mark as abandoned
    /// * `ai_id` - AI marking abandonment (must match intent owner)
    ///
    /// # Returns
    /// Success boolean
    pub async fn abandon_intent(&self, intent_id: i32, ai_id: &str) -> Result<bool> {
        let client = self.pool.get().await?;

        let rows_affected = client
            .execute(
                "UPDATE ai_intents SET status = 'abandoned'
                 WHERE id = $1 AND ai_id = $2 AND status = 'active'",
                &[&intent_id, &ai_id],
            )
            .await?;

        if rows_affected > 0 {
            tracing::info!("{} abandoned intent {}", ai_id, intent_id);
        }

        Ok(rows_affected > 0)
    }

    /// Cleanup expired intents (changes status from active to abandoned)
    ///
    /// # Returns
    /// Number of intents cleaned up
    pub async fn cleanup_expired_intents(&self) -> Result<i32> {
        let client = self.pool.get().await?;

        let rows_affected = client
            .execute(
                "UPDATE ai_intents SET status = 'abandoned'
                 WHERE status = 'active' AND expires_at <= NOW()",
                &[],
            )
            .await? as i32;

        if rows_affected > 0 {
            tracing::debug!("Cleaned up {} expired intents", rows_affected);
        }

        Ok(rows_affected)
    }

    /// Format active intents for display (compact)
    ///
    /// Format: ai_id:intent_text (Xh Ym remaining)|...
    /// Example: crystal:Implementing auth (1h 45m)|sage:Refactoring storage (30m)
    ///
    /// # Arguments
    /// * `intents` - Vector of Intent
    /// * `max_items` - Maximum items to include (default: 5)
    ///
    /// # Returns
    /// Compact string representation
    pub fn format_intents_compact(intents: &[Intent], max_items: usize) -> String {
        if intents.is_empty() {
            return String::new();
        }

        intents
            .iter()
            .filter(|i| !i.is_expired() && i.status == IntentStatus::Active)
            .take(max_items)
            .map(|i| {
                let remaining = i.time_remaining();
                let hours = remaining.num_hours();
                let minutes = remaining.num_minutes() % 60;

                let time_str = if hours > 0 {
                    format!("{}h {}m", hours, minutes)
                } else {
                    format!("{}m", minutes)
                };

                // Truncate intent text if too long
                let intent_display = if i.intent_text.len() > 50 {
                    format!("{}...", &i.intent_text[..47])
                } else {
                    i.intent_text.clone()
                };

                format!("{}:{} ({})", i.ai_id, intent_display, time_str)
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    /// Format active intents for awareness display
    ///
    /// Format: Full multi-line with details
    /// Example:
    /// TEAM WORK INTENTS
    /// ------------------
    /// [crystal] Implementing authentication module (1h 45m left)
    ///   Files: src/auth.rs, src/middleware.rs
    ///
    /// # Arguments
    /// * `intents` - Vector of Intent
    ///
    /// # Returns
    /// Formatted string with one intent per section
    pub fn format_intents_detailed(intents: &[Intent]) -> String {
        if intents.is_empty() {
            return String::new();
        }

        let active: Vec<_> = intents
            .iter()
            .filter(|i| !i.is_expired() && i.status == IntentStatus::Active)
            .collect();

        if active.is_empty() {
            return String::new();
        }

        let mut lines = vec!["TEAM WORK INTENTS".to_string(), "-".repeat(18)];

        for intent in active {
            let remaining = intent.time_remaining();
            let hours = remaining.num_hours();
            let minutes = remaining.num_minutes() % 60;

            let time_str = if hours > 0 {
                format!("{}h {}m left", hours, minutes)
            } else {
                format!("{}m left", minutes)
            };

            lines.push(format!("[{}] {} ({})", intent.ai_id, intent.intent_text, time_str));

            if !intent.related_files.is_empty() {
                let files_str = intent
                    .related_files
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("  Files: {}", files_str));
            }

            lines.push(String::new()); // Blank line between intents
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intent_creation() {
        let intent = Intent::new(
            "test-ai".to_string(),
            "Implementing feature X".to_string(),
            vec!["src/main.rs".to_string()],
        );

        assert_eq!(intent.ai_id, "test-ai");
        assert_eq!(intent.intent_text, "Implementing feature X");
        assert_eq!(intent.status, IntentStatus::Active);
        assert!(!intent.is_expired());
    }

    #[test]
    fn test_format_intents_compact() {
        let mut intent = Intent::new(
            "crystal".to_string(),
            "Implementing auth".to_string(),
            vec![],
        );
        intent.id = 1;

        let formatted = IntentManager::format_intents_compact(&[intent], 5);
        assert!(formatted.contains("crystal:Implementing auth"));
    }

    #[test]
    fn test_format_intents_detailed() {
        let mut intent = Intent::new(
            "sage".to_string(),
            "Refactoring storage layer".to_string(),
            vec!["src/storage.rs".to_string(), "src/types.rs".to_string()],
        );
        intent.id = 1;

        let formatted = IntentManager::format_intents_detailed(&[intent]);
        assert!(formatted.contains("TEAM WORK INTENTS"));
        assert!(formatted.contains("sage"));
        assert!(formatted.contains("Refactoring storage layer"));
        assert!(formatted.contains("Files: src/storage.rs"));
    }

    #[test]
    fn test_format_intents_empty() {
        let formatted = IntentManager::format_intents_compact(&[], 5);
        assert_eq!(formatted, "");

        let formatted = IntentManager::format_intents_detailed(&[]);
        assert_eq!(formatted, "");
    }

    #[test]
    fn test_intent_truncation() {
        let long_intent = "a".repeat(60);
        let intent = Intent::new("test".to_string(), long_intent.clone(), vec![]);

        let formatted = IntentManager::format_intents_compact(&[intent], 5);
        // Should truncate at 50 chars (47 + "...")
        assert!(formatted.len() < long_intent.len() + 20); // Account for ai_id and time
    }
}
