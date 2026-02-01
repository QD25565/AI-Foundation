//! Project CLI - Manage projects and features for autonomous context injection
//!
//! This CLI provides fast Rust access to the Project System stored in PostgreSQL.
//! Projects and features are shared across all AI instances on the device.
//!
//! Usage:
//!   project-cli list                              # List all projects
//!   project-cli create --name "My Project" --overview "..." --root "/path/to/project"
//!   project-cli get --id 1                        # Get project details
//!   project-cli update --id 1 --overview "New overview"
//!   project-cli features --project-id 1           # List features
//!   project-cli create-feature --project-id 1 --name "Auth" --overview "..." --directory "src/auth"
//!   project-cli resolve --path "/path/to/file.rs" # Find project/feature for file

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use postgres::{Client, NoTls};
use std::env;

// ============================================================================
// CLI DEFINITION
// ============================================================================

#[derive(Parser)]
#[command(name = "project-cli")]
#[command(about = "Project System CLI - manage projects and features")]
#[command(version = "0.1.0")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all projects
    List {
        /// Filter by status (active, archived, deleted)
        #[arg(long, default_value = "active")]
        status: String,
        /// Maximum number of results
        #[arg(long, default_value = "50")]
        limit: i64,
    },

    /// Create a new project
    Create {
        /// Project name
        #[arg(long)]
        name: String,
        /// High-level overview (1-3 sentences)
        #[arg(long)]
        overview: String,
        /// Root directory (absolute path)
        #[arg(long)]
        root: String,
        /// Technical details (optional)
        #[arg(long)]
        details: Option<String>,
    },

    /// Get project details
    Get {
        /// Project ID
        #[arg(long)]
        id: i32,
    },

    /// Update project
    Update {
        /// Project ID
        #[arg(long)]
        id: i32,
        /// New overview
        #[arg(long)]
        overview: Option<String>,
        /// New details
        #[arg(long)]
        details: Option<String>,
        /// New status
        #[arg(long)]
        status: Option<String>,
    },

    /// Delete project (soft delete)
    Delete {
        /// Project ID
        #[arg(long)]
        id: i32,
    },

    /// List features for a project
    Features {
        /// Project ID
        #[arg(long)]
        project_id: i32,
        /// Filter by status
        #[arg(long, default_value = "active")]
        status: String,
    },

    /// Create a new feature
    CreateFeature {
        /// Parent project ID
        #[arg(long)]
        project_id: i32,
        /// Feature name
        #[arg(long)]
        name: String,
        /// High-level overview
        #[arg(long)]
        overview: String,
        /// Directory (relative to project root or absolute)
        #[arg(long)]
        directory: String,
        /// Technical details (optional)
        #[arg(long)]
        details: Option<String>,
    },

    /// Update a feature
    UpdateFeature {
        /// Feature ID
        #[arg(long)]
        id: i32,
        /// New overview
        #[arg(long)]
        overview: Option<String>,
        /// New details
        #[arg(long)]
        details: Option<String>,
    },

    /// Resolve file path to project/feature
    Resolve {
        /// File path to resolve
        #[arg(long)]
        path: String,
    },

    /// Get injection context for a file (for hooks)
    Context {
        /// File path
        #[arg(long)]
        path: String,
        /// AI ID
        #[arg(long)]
        ai_id: Option<String>,
        /// Session ID
        #[arg(long)]
        session_id: Option<String>,
    },
}

// ============================================================================
// DATABASE CONNECTION
// ============================================================================

fn get_connection() -> Result<Client> {
    let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string());
    let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "15432".to_string());
    let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "postgres".to_string());
    let password = env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "postgres".to_string());
    let dbname = env::var("POSTGRES_DB").unwrap_or_else(|_| "postgres".to_string());

    let conn_string = format!(
        "host={} port={} user={} password={} dbname={}",
        host, port, user, password, dbname
    );

    Client::connect(&conn_string, NoTls).context("Failed to connect to PostgreSQL")
}

fn get_ai_id() -> String {
    env::var("AI_ID").unwrap_or_else(|_| "unknown".to_string())
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

fn format_relative_time(dt: Option<DateTime<Utc>>) -> String {
    match dt {
        None => "never".to_string(),
        Some(dt) => {
            let now = Utc::now();
            let diff = now.signed_duration_since(dt);
            let minutes = diff.num_minutes();

            if minutes < 1 {
                "now".to_string()
            } else if minutes < 60 {
                format!("{}m ago", minutes)
            } else if minutes < 1440 {
                format!("{}h ago", minutes / 60)
            } else if minutes < 10080 {
                format!("{}d ago", minutes / 1440)
            } else {
                dt.format("%Y-%m-%d").to_string()
            }
        }
    }
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

// ============================================================================
// COMMANDS
// ============================================================================

fn cmd_list(status: &str, limit: i64) -> Result<()> {
    let mut client = get_connection()?;

    let rows = client.query(
        "SELECT p.id, p.name, p.overview, p.root_directory, p.updated_at, p.status,
                COUNT(f.id) as feature_count
         FROM projects p
         LEFT JOIN project_features f ON p.id = f.project_id AND f.status = 'active'
         WHERE p.status = $1
         GROUP BY p.id
         ORDER BY p.updated_at DESC
         LIMIT $2",
        &[&status, &limit],
    )?;

    if rows.is_empty() {
        println!("No projects found (status: {})", status);
        return Ok(());
    }

    println!("=== PROJECTS ({}) ===", rows.len());
    println!();

    for row in rows {
        let id: i32 = row.get(0);
        let name: String = row.get(1);
        let overview: String = row.get(2);
        let root: String = row.get(3);
        let updated_at: Option<DateTime<Utc>> = row.get(4);
        let feature_count: i64 = row.get(6);

        let when = format_relative_time(updated_at);
        let overview_short = truncate_str(&overview, 60);

        println!("[{}] {} | {} features | {}", id, name, feature_count, when);
        println!("    {}", overview_short);
        println!("    root: {}", root);
        println!();
    }

    Ok(())
}

fn cmd_create(name: &str, overview: &str, root: &str, details: Option<&str>) -> Result<()> {
    let mut client = get_connection()?;
    let ai_id = get_ai_id();

    // Normalize path
    let root_normalized = root.replace('\\', "/");

    let row = client.query_one(
        "INSERT INTO projects (name, overview, details, root_directory, created_by)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, name, created_at",
        &[&name, &overview, &details, &root_normalized, &ai_id],
    )?;

    let id: i32 = row.get(0);
    let proj_name: String = row.get(1);

    println!("=== PROJECT CREATED ===");
    println!("ID: {}", id);
    println!("Name: {}", proj_name);
    println!("Root: {}", root_normalized);
    println!("Created by: {}", ai_id);

    Ok(())
}

fn cmd_get(id: i32) -> Result<()> {
    let mut client = get_connection()?;

    let row = client.query_opt(
        "SELECT id, name, overview, details, root_directory, created_at, created_by,
                updated_at, updated_by, status
         FROM projects WHERE id = $1",
        &[&id],
    )?;

    match row {
        None => {
            println!("Project not found: {}", id);
        }
        Some(row) => {
            let name: String = row.get(1);
            let overview: String = row.get(2);
            let details: Option<String> = row.get(3);
            let root: String = row.get(4);
            let created_at: Option<DateTime<Utc>> = row.get(5);
            let created_by: Option<String> = row.get(6);
            let updated_at: Option<DateTime<Utc>> = row.get(7);
            let status: String = row.get(9);

            println!("=== PROJECT: {} ===", name);
            println!();
            println!("ID: {}", id);
            println!("Status: {}", status);
            println!("Root: {}", root);
            println!();
            println!("Overview:");
            println!("  {}", overview);

            if let Some(d) = details {
                println!();
                println!("Details:");
                for line in d.lines() {
                    println!("  {}", line);
                }
            }

            println!();
            println!("Created: {} by {}",
                format_relative_time(created_at),
                created_by.unwrap_or_else(|| "unknown".to_string())
            );
            println!("Updated: {}", format_relative_time(updated_at));

            // List features
            let features = client.query(
                "SELECT id, name, overview, directory FROM project_features
                 WHERE project_id = $1 AND status = 'active'
                 ORDER BY name",
                &[&id],
            )?;

            if !features.is_empty() {
                println!();
                println!("Features ({}):", features.len());
                for f in features {
                    let fid: i32 = f.get(0);
                    let fname: String = f.get(1);
                    let foverview: String = f.get(2);
                    let fdir: String = f.get(3);
                    println!("  [{}] {} - {}", fid, fname, truncate_str(&foverview, 50));
                    println!("       dir: {}", fdir);
                }
            }
        }
    }

    Ok(())
}

fn cmd_update(id: i32, overview: Option<&str>, details: Option<&str>, status: Option<&str>) -> Result<()> {
    let mut client = get_connection()?;
    let ai_id = get_ai_id();

    // Check project exists
    let exists = client.query_opt("SELECT id FROM projects WHERE id = $1", &[&id])?;
    if exists.is_none() {
        println!("Project not found: {}", id);
        return Ok(());
    }

    // Build dynamic update
    let mut updates = vec!["updated_at = NOW()".to_string(), "updated_by = $1".to_string()];
    let mut param_idx = 2;

    if overview.is_some() {
        updates.push(format!("overview = ${}", param_idx));
        param_idx += 1;
    }
    if details.is_some() {
        updates.push(format!("details = ${}", param_idx));
        param_idx += 1;
    }
    if status.is_some() {
        updates.push(format!("status = ${}", param_idx));
    }

    let query = format!(
        "UPDATE projects SET {} WHERE id = ${}",
        updates.join(", "),
        param_idx
    );

    // Execute with dynamic params (simplified - just show the concept)
    // In production, you'd build params dynamically
    if let (Some(o), Some(d), Some(s)) = (overview, details, status) {
        client.execute(&query, &[&ai_id, &o, &d, &s, &id])?;
    } else if let (Some(o), Some(d), None) = (overview, details, status) {
        client.execute(&query, &[&ai_id, &o, &d, &id])?;
    } else if let (Some(o), None, None) = (overview, details, status) {
        client.execute(&query, &[&ai_id, &o, &id])?;
    } else if let (None, Some(d), None) = (overview, details, status) {
        client.execute(&query, &[&ai_id, &d, &id])?;
    } else if let (None, None, Some(s)) = (overview, details, status) {
        client.execute(&query, &[&ai_id, &s, &id])?;
    } else {
        // Just update timestamp
        client.execute(
            "UPDATE projects SET updated_at = NOW(), updated_by = $1 WHERE id = $2",
            &[&ai_id, &id],
        )?;
    }

    println!("=== PROJECT UPDATED ===");
    println!("ID: {}", id);
    println!("Updated by: {}", ai_id);

    Ok(())
}

fn cmd_delete(id: i32) -> Result<()> {
    let mut client = get_connection()?;
    let ai_id = get_ai_id();

    let result = client.execute(
        "UPDATE projects SET status = 'deleted', updated_at = NOW(), updated_by = $1
         WHERE id = $2 AND status != 'deleted'",
        &[&ai_id, &id],
    )?;

    if result > 0 {
        println!("=== PROJECT DELETED ===");
        println!("ID: {}", id);
    } else {
        println!("Project not found or already deleted: {}", id);
    }

    Ok(())
}

fn cmd_features(project_id: i32, status: &str) -> Result<()> {
    let mut client = get_connection()?;

    // Get project name
    let proj = client.query_opt(
        "SELECT name FROM projects WHERE id = $1",
        &[&project_id],
    )?;

    let proj_name = match proj {
        None => {
            println!("Project not found: {}", project_id);
            return Ok(());
        }
        Some(row) => row.get::<_, String>(0),
    };

    let rows = client.query(
        "SELECT id, name, overview, directory, updated_at
         FROM project_features
         WHERE project_id = $1 AND status = $2
         ORDER BY name",
        &[&project_id, &status],
    )?;

    if rows.is_empty() {
        println!("No features found for project {} (status: {})", proj_name, status);
        return Ok(());
    }

    println!("=== FEATURES: {} ({}) ===", proj_name, rows.len());
    println!();

    for row in rows {
        let id: i32 = row.get(0);
        let name: String = row.get(1);
        let overview: String = row.get(2);
        let directory: String = row.get(3);
        let updated_at: Option<DateTime<Utc>> = row.get(4);

        let when = format_relative_time(updated_at);

        println!("[{}] {} | {}", id, name, when);
        println!("    {}", truncate_str(&overview, 60));
        println!("    dir: {}", directory);
        println!();
    }

    Ok(())
}

fn cmd_create_feature(
    project_id: i32,
    name: &str,
    overview: &str,
    directory: &str,
    details: Option<&str>,
) -> Result<()> {
    let mut client = get_connection()?;
    let ai_id = get_ai_id();

    // Normalize directory
    let dir_normalized = directory.replace('\\', "/");

    let row = client.query_one(
        "INSERT INTO project_features (project_id, name, overview, details, directory, created_by)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, name",
        &[&project_id, &name, &overview, &details, &dir_normalized, &ai_id],
    )?;

    let id: i32 = row.get(0);
    let feat_name: String = row.get(1);

    println!("=== FEATURE CREATED ===");
    println!("ID: {}", id);
    println!("Name: {}", feat_name);
    println!("Project ID: {}", project_id);
    println!("Directory: {}", dir_normalized);
    println!("Created by: {}", ai_id);

    Ok(())
}

fn cmd_update_feature(id: i32, overview: Option<&str>, details: Option<&str>) -> Result<()> {
    let mut client = get_connection()?;
    let ai_id = get_ai_id();

    // Build and execute update (simplified)
    if let Some(o) = overview {
        if let Some(d) = details {
            client.execute(
                "UPDATE project_features SET overview = $1, details = $2, updated_at = NOW(), updated_by = $3 WHERE id = $4",
                &[&o, &d, &ai_id, &id],
            )?;
        } else {
            client.execute(
                "UPDATE project_features SET overview = $1, updated_at = NOW(), updated_by = $2 WHERE id = $3",
                &[&o, &ai_id, &id],
            )?;
        }
    } else if let Some(d) = details {
        client.execute(
            "UPDATE project_features SET details = $1, updated_at = NOW(), updated_by = $2 WHERE id = $3",
            &[&d, &ai_id, &id],
        )?;
    }

    println!("=== FEATURE UPDATED ===");
    println!("ID: {}", id);

    Ok(())
}

fn cmd_resolve(path: &str) -> Result<()> {
    let mut client = get_connection()?;

    // Normalize path
    let normalized = path.replace('\\', "/");

    let row = client.query_opt(
        "SELECT project_id, project_name, feature_id, feature_name
         FROM find_project_for_file($1)",
        &[&normalized],
    )?;

    match row {
        None => {
            println!("=== NO MATCH ===");
            println!("Path: {}", path);
            println!("No project contains this file");
        }
        Some(row) => {
            let project_id: Option<i32> = row.get(0);
            let project_name: Option<String> = row.get(1);
            let feature_id: Option<i32> = row.get(2);
            let feature_name: Option<String> = row.get(3);

            if project_id.is_none() {
                println!("=== NO MATCH ===");
                println!("Path: {}", path);
                println!("No project contains this file");
            } else {
                println!("=== RESOLVED ===");
                println!("Path: {}", path);
                println!();
                println!("Project: {} (ID: {})",
                    project_name.unwrap_or_default(),
                    project_id.unwrap_or(0)
                );

                if let Some(fid) = feature_id {
                    println!("Feature: {} (ID: {})",
                        feature_name.unwrap_or_default(),
                        fid
                    );
                } else {
                    println!("Feature: (none - in project root)");
                }
            }
        }
    }

    Ok(())
}

fn cmd_context(path: &str, ai_id: Option<&str>, session_id: Option<&str>) -> Result<()> {
    let mut client = get_connection()?;

    let ai = ai_id.map(|s| s.to_string()).unwrap_or_else(get_ai_id);
    let session = session_id.unwrap_or("default");

    // Normalize path
    let normalized = path.replace('\\', "/");

    let row = client.query_opt(
        "SELECT get_injection_context($1, $2, $3, 1800, 0)",
        &[&normalized, &ai, &session],
    )?;

    match row {
        None => {
            println!("=== NO CONTEXT ===");
            println!("Path: {}", path);
        }
        Some(row) => {
            let context: Option<serde_json::Value> = row.get(0);

            match context {
                None => {
                    println!("=== NO CONTEXT ===");
                    println!("Path: {}", path);
                }
                Some(ctx) => {
                    println!("=== INJECTION CONTEXT ===");
                    println!("Path: {}", path);
                    println!("AI: {}", ai);
                    println!("Session: {}", session);
                    println!();

                    // Parse context
                    if let Some(proj) = ctx.get("project") {
                        if let Some(name) = proj.get("name").and_then(|v| v.as_str()) {
                            println!("Project: {}", name);
                            if let Some(overview) = proj.get("overview").and_then(|v| v.as_str()) {
                                println!("  {}", truncate_str(overview, 70));
                            }
                        }
                    }

                    if let Some(feat) = ctx.get("feature") {
                        if let Some(name) = feat.get("name").and_then(|v| v.as_str()) {
                            println!("Feature: {}", name);
                            if let Some(overview) = feat.get("overview").and_then(|v| v.as_str()) {
                                println!("  {}", truncate_str(overview, 70));
                            }
                        }
                    }

                    println!();
                    println!("Should inject project: {}",
                        ctx.get("should_inject_project").and_then(|v| v.as_bool()).unwrap_or(false)
                    );
                    println!("Should inject feature: {}",
                        ctx.get("should_inject_feature").and_then(|v| v.as_bool()).unwrap_or(false)
                    );
                }
            }
        }
    }

    Ok(())
}

// ============================================================================
// MAIN
// ============================================================================

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::List { status, limit } => cmd_list(&status, limit),
        Commands::Create { name, overview, root, details } => {
            cmd_create(&name, &overview, &root, details.as_deref())
        }
        Commands::Get { id } => cmd_get(id),
        Commands::Update { id, overview, details, status } => {
            cmd_update(id, overview.as_deref(), details.as_deref(), status.as_deref())
        }
        Commands::Delete { id } => cmd_delete(id),
        Commands::Features { project_id, status } => cmd_features(project_id, &status),
        Commands::CreateFeature { project_id, name, overview, directory, details } => {
            cmd_create_feature(project_id, &name, &overview, &directory, details.as_deref())
        }
        Commands::UpdateFeature { id, overview, details } => {
            cmd_update_feature(id, overview.as_deref(), details.as_deref())
        }
        Commands::Resolve { path } => cmd_resolve(&path),
        Commands::Context { path, ai_id, session_id } => {
            cmd_context(&path, ai_id.as_deref(), session_id.as_deref())
        }
    }
}
