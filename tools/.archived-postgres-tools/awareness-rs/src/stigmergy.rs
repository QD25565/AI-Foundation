//! Stigmergy Pattern Surfacing Module
//!
//! Enables indirect coordination through environmental traces.
//! Surfaces team collaboration patterns by analyzing file actions.
//!
//! Research Foundation:
//! - Stigmergy = Indirect coordination through environment modification
//! - ACO (Ant Colony Optimization) accounts for 45% of swarm intelligence market
//! - Google A2A protocol (2025) emphasizes environment-mediated coordination
//! - Scales from few agents to thousands without computational overhead
//!
//! Features:
//! - File activity queries (who touched what when)
//! - Co-activity pattern detection (files edited together)
//! - Team collaboration hotspot identification
//! - Passive coordination (zero AI cognition required)

use crate::database::DatabasePool;
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// File activity by a specific AI
#[derive(Debug, Clone)]
pub struct FileActivity {
    pub ai_id: String,
    pub action_type: String,  // created/modified/accessed
    pub file_path: String,
    pub timestamp: DateTime<Utc>,
}

/// Co-activity pattern (multiple AIs working on same directory)
#[derive(Debug, Clone)]
pub struct CoactivityPattern {
    pub directory: String,
    pub ai_ids: Vec<String>,
    pub file_count: i32,
    pub last_activity: DateTime<Utc>,
}

/// Stigmergy manager
pub struct StigmergyManager {
    db_pool: DatabasePool,
}

impl StigmergyManager {
    pub fn new(db_pool: DatabasePool) -> Self {
        Self { db_pool }
    }

    /// Get file activity for a specific path (or directory) within time window
    ///
    /// # Arguments
    /// * `path` - File path or directory to query (supports LIKE patterns)
    /// * `hours` - Time window in hours (default: 24)
    ///
    /// # Returns
    /// Vector of FileActivity sorted by timestamp DESC
    pub async fn get_file_activity(&self, path: &str, hours: i64) -> Result<Vec<FileActivity>> {
        let client = self.db_pool.get().await?;

        let rows = client
            .query(
                "SELECT ai_id, action_type, file_path, timestamp
                 FROM ai_file_actions
                 WHERE file_path LIKE $1
                   AND timestamp >= NOW() - INTERVAL '1 hour' * $2
                 ORDER BY timestamp DESC
                 LIMIT 50",
                &[&format!("%{}%", path), &hours],
            )
            .await?;

        let activities = rows
            .iter()
            .map(|row| FileActivity {
                ai_id: row.get(0),
                action_type: row.get(1),
                file_path: row.get(2),
                timestamp: row.get(3),
            })
            .collect();

        Ok(activities)
    }

    /// Get co-activity patterns (directories with multiple AIs working)
    ///
    /// # Arguments
    /// * `hours` - Time window in hours (default: 24)
    ///
    /// # Returns
    /// Vector of CoactivityPattern showing collaboration hotspots
    pub async fn get_coactivity_patterns(&self, hours: i64) -> Result<Vec<CoactivityPattern>> {
        let client = self.db_pool.get().await?;

        // Extract directory from file_path using PostgreSQL string functions
        let rows = client
            .query(
                "SELECT
                    CASE
                        WHEN file_path LIKE '%/%' THEN SUBSTRING(file_path FROM 1 FOR LENGTH(file_path) - POSITION('/' IN REVERSE(file_path)))
                        WHEN file_path LIKE '%\\%' THEN SUBSTRING(file_path FROM 1 FOR LENGTH(file_path) - POSITION('\\' IN REVERSE(file_path)))
                        ELSE 'root'
                    END as directory,
                    array_agg(DISTINCT ai_id) as ai_ids,
                    COUNT(*) as file_count,
                    MAX(timestamp) as last_activity
                 FROM ai_file_actions
                 WHERE timestamp >= NOW() - INTERVAL '1 hour' * $1
                 GROUP BY directory
                 HAVING COUNT(DISTINCT ai_id) > 1
                 ORDER BY file_count DESC, last_activity DESC
                 LIMIT 10",
                &[&hours],
            )
            .await?;

        let patterns = rows
            .iter()
            .map(|row| CoactivityPattern {
                directory: row.get(0),
                ai_ids: row.get(1),
                file_count: row.get::<_, i64>(2) as i32,
                last_activity: row.get(3),
            })
            .collect();

        Ok(patterns)
    }

    /// Format file activities as compact pipe-delimited string
    ///
    /// Format: ai_id:action:filename|ai_id:action:filename
    /// Example: crystal:modified:main.rs|sage:created:test.rs
    ///
    /// # Arguments
    /// * `activities` - Vector of FileActivity
    /// * `max_items` - Maximum items to include (default: 5)
    ///
    /// # Returns
    /// Compact string representation
    pub fn format_activities_compact(activities: &[FileActivity], max_items: usize) -> String {
        activities
            .iter()
            .take(max_items)
            .map(|a| {
                let filename = std::path::Path::new(&a.file_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&a.file_path);
                format!("{}:{}:{}", a.ai_id, a.action_type, filename)
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    /// Format co-activity patterns for display
    ///
    /// Format: directory - ai1, ai2, ai3 (N files)
    /// Example: src/auth - crystal, sage (3 files)
    ///
    /// # Arguments
    /// * `patterns` - Vector of CoactivityPattern
    ///
    /// # Returns
    /// Formatted string with one pattern per line
    pub fn format_coactivity_patterns(patterns: &[CoactivityPattern]) -> String {
        if patterns.is_empty() {
            return String::new();
        }

        let mut lines = vec!["TEAM FILE ACTIVITY".to_string(), "-".repeat(22)];

        for pattern in patterns {
            let ai_list = pattern.ai_ids.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            lines.push(format!(
                "{} - {} ({} files)",
                pattern.directory, ai_list, pattern.file_count
            ));
        }

        lines.join("\n")
    }

    /// Get file activity around a specific note creation time
    /// (For enriching notebook recall with stigmergy context)
    ///
    /// # Arguments
    /// * `timestamp` - Note creation time
    /// * `window_minutes` - Time window in minutes (±)
    ///
    /// # Returns
    /// Compact formatted string of file activities
    pub async fn get_activity_around_time(
        &self,
        timestamp: DateTime<Utc>,
        window_minutes: i64,
    ) -> Result<String> {
        let client = self.db_pool.get().await?;

        let rows = client
            .query(
                "SELECT DISTINCT ai_id, action_type, file_path
                 FROM ai_file_actions
                 WHERE timestamp BETWEEN $1 - INTERVAL '1 minute' * $2
                   AND $1 + INTERVAL '1 minute' * $2
                 ORDER BY timestamp DESC
                 LIMIT 10",
                &[&timestamp, &window_minutes],
            )
            .await?;

        if rows.is_empty() {
            return Ok(String::new());
        }

        let activities: Vec<FileActivity> = rows
            .iter()
            .map(|row| FileActivity {
                ai_id: row.get(0),
                action_type: row.get(1),
                file_path: row.get(2),
                timestamp,  // Use provided timestamp
            })
            .collect();

        Ok(Self::format_activities_compact(&activities, 5))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_activities_compact() {
        let activities = vec![
            FileActivity {
                ai_id: "crystal".to_string(),
                action_type: "modified".to_string(),
                file_path: "/path/to/main.rs".to_string(),
                timestamp: Utc::now(),
            },
            FileActivity {
                ai_id: "sage".to_string(),
                action_type: "created".to_string(),
                file_path: "/path/to/test.rs".to_string(),
                timestamp: Utc::now(),
            },
        ];

        let formatted = StigmergyManager::format_activities_compact(&activities, 5);
        assert_eq!(formatted, "crystal:modified:main.rs|sage:created:test.rs");
    }

    #[test]
    fn test_format_coactivity_patterns() {
        let patterns = vec![CoactivityPattern {
            directory: "src/auth".to_string(),
            ai_ids: vec!["crystal".to_string(), "sage".to_string()],
            file_count: 3,
            last_activity: Utc::now(),
        }];

        let formatted = StigmergyManager::format_coactivity_patterns(&patterns);
        assert!(formatted.contains("TEAM FILE ACTIVITY"));
        assert!(formatted.contains("src/auth - crystal, sage (3 files)"));
    }

    #[test]
    fn test_format_activities_truncation() {
        let activities = vec![
            FileActivity {
                ai_id: "ai1".to_string(),
                action_type: "modified".to_string(),
                file_path: "file1.rs".to_string(),
                timestamp: Utc::now(),
            },
            FileActivity {
                ai_id: "ai2".to_string(),
                action_type: "created".to_string(),
                file_path: "file2.rs".to_string(),
                timestamp: Utc::now(),
            },
            FileActivity {
                ai_id: "ai3".to_string(),
                action_type: "modified".to_string(),
                file_path: "file3.rs".to_string(),
                timestamp: Utc::now(),
            },
        ];

        let formatted = StigmergyManager::format_activities_compact(&activities, 2);
        let parts: Vec<&str> = formatted.split('|').collect();
        assert_eq!(parts.len(), 2);  // Should truncate to 2 items
    }
}
