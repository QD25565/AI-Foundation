/// Project Context - Fast project/feature detection for files
///
/// Replaces the slow Python ProjectService with sub-millisecond Rust lookups.
use tokio_postgres::{Client, Error, Row};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectContext {
    pub project_id: Option<i32>,
    pub project_name: Option<String>,
    pub feature_id: Option<i32>,
    pub feature_name: Option<String>,
    pub should_inject: bool,
}

/// Find project and feature for a given file path
///
/// Returns the project/feature that contains this file, using longest-prefix matching.
/// Optimized for speed with minimal database queries.
pub async fn find_project_for_file(
    client: &Client,
    file_path: &str,
) -> Result<ProjectContext, Error> {
    // Normalize path: convert backslashes to forward slashes
    let normalized_path = file_path.replace('\\', "/");

    // Single optimized query to find both project and feature
    let row = client
        .query_opt(
            r#"
            WITH matching_project AS (
                SELECT id, name, root_directory
                FROM projects
                WHERE status = 'active'
                  AND position(REPLACE(root_directory, E'\\', '/') in $1) = 1
                ORDER BY LENGTH(root_directory) DESC
                LIMIT 1
            ),
            matching_feature AS (
                SELECT pf.id, pf.name AS feature_name, pf.directory
                FROM project_features pf
                JOIN matching_project mp ON pf.project_id = mp.id
                WHERE pf.status = 'active'
                  AND position(REPLACE(mp.root_directory || E'\\' || pf.directory, E'\\', '/') in $1) = 1
                ORDER BY LENGTH(pf.directory) DESC
                LIMIT 1
            )
            SELECT
                mp.id as project_id,
                mp.name as project_name,
                mf.id as feature_id,
                mf.feature_name
            FROM matching_project mp
            LEFT JOIN matching_feature mf ON true
            "#,
            &[&normalized_path],
        )
        .await?;

    match row {
        Some(row) => Ok(ProjectContext {
            project_id: row.get("project_id"),
            project_name: row.get("project_name"),
            feature_id: row.get("feature_id"),
            feature_name: row.get("feature_name"),
            should_inject: true,
        }),
        None => Ok(ProjectContext {
            project_id: None,
            project_name: None,
            feature_id: None,
            feature_name: None,
            should_inject: false,
        }),
    }
}

/// Get formatted context string for injection into Claude
///
/// Returns a markdown-formatted string ready to inject into the conversation.
pub fn format_context(ctx: &ProjectContext) -> Option<String> {
    if !ctx.should_inject {
        return None;
    }

    let mut output = Vec::new();

    if let (Some(proj_id), Some(proj_name)) = (&ctx.project_id, &ctx.project_name) {
        output.push(format!("[PROJECT: {}]", proj_name));
    }

    if let (Some(feat_id), Some(feat_name)) = (&ctx.feature_id, &ctx.feature_name) {
        output.push(format!("[FEATURE: {}]", feat_name));
    }

    if output.is_empty() {
        None
    } else {
        Some(output.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_normalization() {
        let windows_path = r"C:\Users\Test\project\file.py";
        let normalized = windows_path.replace('\\', "/");
        assert_eq!(normalized, "C:/Users/Test/project/file.py");
    }

    #[test]
    fn test_format_context() {
        let ctx = ProjectContext {
            project_id: Some(1),
            project_name: Some("AI Foundation".to_string()),
            feature_id: Some(2),
            feature_name: Some("Rust Integration".to_string()),
            should_inject: true,
        };

        let formatted = format_context(&ctx).unwrap();
        assert!(formatted.contains("[PROJECT: AI Foundation]"));
        assert!(formatted.contains("[FEATURE: Rust Integration]"));
    }
}
