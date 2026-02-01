//! File action tracking and querying

use crate::database::DatabasePool;
use crate::FileAction;
use anyhow::Result;

/// File action manager
pub struct FileActionManager {
    db_pool: DatabasePool,
}

impl FileActionManager {
    pub fn new(db_pool: DatabasePool) -> Self {
        Self { db_pool }
    }

    /// Log a file action to PostgreSQL
    pub async fn log(&self, action: FileAction) -> Result<i64> {
        let client = self.db_pool.get().await?;

        let row = client
            .query_one(
                "INSERT INTO ai_file_actions (ai_id, action_type, file_path, file_type, file_size, working_directory)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 RETURNING id",
                &[
                    &action.ai_id as &(dyn tokio_postgres::types::ToSql + Sync),
                    &action.action_type as &(dyn tokio_postgres::types::ToSql + Sync),
                    &action.file_path as &(dyn tokio_postgres::types::ToSql + Sync),
                    &action.file_type as &(dyn tokio_postgres::types::ToSql + Sync),
                    &action.file_size as &(dyn tokio_postgres::types::ToSql + Sync),
                    &action.working_directory as &(dyn tokio_postgres::types::ToSql + Sync),
                ],
            )
            .await?;

        let id: i32 = row.get(0);  // PostgreSQL integer is i32
        Ok(id as i64)
    }

    /// Get recent file actions (all AIs)
    pub async fn get_recent(&self, limit: i64) -> Result<Vec<FileAction>> {
        let client = self.db_pool.get().await?;

        let rows = client
            .query(
                "SELECT id, ai_id, timestamp, action_type, file_path, file_type, file_size, working_directory
                 FROM ai_file_actions
                 ORDER BY timestamp DESC
                 LIMIT $1",
                &[&limit],
            )
            .await?;

        let actions = rows
            .iter()
            .map(|row| FileAction {
                id: Some(row.get(0)),
                ai_id: row.get(1),
                timestamp: row.get(2),
                action_type: row.get(3),
                file_path: row.get(4),
                file_type: row.get(5),
                file_size: row.get(6),
                working_directory: row.get(7),
            })
            .collect();

        Ok(actions)
    }

    /// Get recent actions by specific AI
    pub async fn get_by_ai(&self, ai_id: &str, limit: i64) -> Result<Vec<FileAction>> {
        let client = self.db_pool.get().await?;

        let rows = client
            .query(
                "SELECT id, ai_id, timestamp, action_type, file_path, file_type, file_size, working_directory
                 FROM ai_file_actions
                 WHERE ai_id = $1
                 ORDER BY timestamp DESC
                 LIMIT $2",
                &[&ai_id, &limit],
            )
            .await?;

        let actions = rows
            .iter()
            .map(|row| FileAction {
                id: Some(row.get(0)),
                ai_id: row.get(1),
                timestamp: row.get(2),
                action_type: row.get(3),
                file_path: row.get(4),
                file_type: row.get(5),
                file_size: row.get(6),
                working_directory: row.get(7),
            })
            .collect();

        Ok(actions)
    }

    /// Get team activity summary (action counts per AI)
    pub async fn get_team_activity(&self, minutes: i64) -> Result<Vec<(String, i64)>> {
        let client = self.db_pool.get().await?;

        let rows = client
            .query(
                "SELECT ai_id, COUNT(*) as count
                 FROM ai_file_actions
                 WHERE timestamp > NOW() - INTERVAL '1 minute' * $1
                 GROUP BY ai_id
                 ORDER BY count DESC",
                &[&minutes],
            )
            .await?;

        let activity = rows
            .iter()
            .map(|row| (row.get::<_, String>(0), row.get::<_, i64>(1)))
            .collect();

        Ok(activity)
    }

    /// Format recent actions for display (matches Python format)
    /// Emphasizes .MD/.txt files for team cohesion signals
    pub async fn format_for_display(&self, limit: i64) -> Result<String> {
        let actions = self.get_recent(limit).await?;

        if actions.is_empty() {
            return Ok("TEAM ACTIVITY: No recent file actions".to_string());
        }

        let mut lines = vec![format!("TEAM ACTIVITY (last {} actions):", limit)];

        for action in actions {
            // Calculate time ago
            let time_ago = chrono::Local::now().naive_local() - action.timestamp;
            let time_str = if time_ago.num_seconds() < 60 {
                format!("{}s ago", time_ago.num_seconds())
            } else if time_ago.num_minutes() < 60 {
                format!("{}m ago", time_ago.num_minutes())
            } else {
                format!("{}h ago", time_ago.num_hours())
            };

            // Detect file extension for emphasis
            let file_ext = action.file_path
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_uppercase();

            // Emphasize .MD/.txt files with uppercase extension marker
            // Creates contextual signals for team cohesion (user guidance)
            let file_display = if file_ext == "MD" || file_ext == "TXT" {
                format!("{} [{}]", action.file_path, file_ext)
            } else {
                action.file_path.clone()
            };

            // Format: "  - ai_short: action file_path [EXT] (time_ago)"
            let ai_short = action.ai_id.split('-').next().unwrap_or(&action.ai_id);
            lines.push(format!(
                "  - {}: {} {} ({})",
                ai_short, action.action_type, file_display, time_str
            ));
        }

        Ok(lines.join("\n"))
    }
}
