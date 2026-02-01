///! Awareness CLI - File Tracking & Action Logging
///!
///! Tracks file operations for multi-AI coordination

use anyhow::Result;
use clap::{Parser, Subcommand};
use awareness_core::{FileActionManager, database::DatabasePool, analyze};
use chrono::Utc;

#[derive(Parser)]
#[command(name = "awareness-cli")]
#[command(about = "AI file tracking and team awareness system", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze a file for context and freshness
    #[command(alias = "check", alias = "inspect", alias = "scan", alias = "review")]
    Analyze {
        /// File path to analyze (e.g., "src/auth.rs")
        #[arg(value_name = "FILE_PATH")]
        path: String,
    },

    /// Log a file operation for team awareness
    #[command(alias = "log", alias = "track", alias = "record", alias = "save")]
    LogAction {
        /// Action type: read, edit, write, delete
        #[arg(value_name = "ACTION")]
        action: String,

        /// File path (e.g., "src/main.rs")
        #[arg(value_name = "FILE_PATH")]
        path: String,

        /// AI identifier (defaults to AI_ID env var)
        #[arg(long, hide = true)]
        ai_id: Option<String>,
    },

    /// Show recent file actions
    #[command(alias = "recent", alias = "history", alias = "actions", alias = "list")]
    RecentActions {
        /// AI identifier to filter by (e.g., "lyra-584")
        #[arg(value_name = "AI_ID")]
        ai_id_positional: Option<String>,

        /// Number of results to show
        #[arg(value_name = "LIMIT")]
        limit_positional: Option<i32>,

        #[arg(long, hide = true)]
        ai_id: Option<String>,

        #[arg(long, default_value = "10", hide = true)]
        limit: i32,
    },

    /// Show team activity summary
    #[command(alias = "team", alias = "activity", alias = "stats", alias = "who")]
    TeamActivity {
        /// Minutes to look back (e.g., 60 for last hour)
        #[arg(value_name = "MINUTES")]
        minutes_positional: Option<i64>,

        #[arg(long, default_value = "60", hide = true)]
        minutes: i64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Analyze { path } => {
            let context = analyze(&path);

            println!("File Analysis: {}", path);
            println!("============================================================");
            println!("Path: {}", context.path);
            println!("Freshness: {:?}", context.freshness);
            println!("Git Status: {:?}", context.git_status);
            println!("Dependencies: {:?}", context.dependency_sync);
            println!("Safety: {:?}", context.safety);
        }

        Commands::LogAction { action, path, ai_id } => {
            // Get AI_ID from flag or environment
            let final_ai_id = ai_id
                .or_else(|| std::env::var("AI_ID").ok())
                .ok_or_else(|| anyhow::anyhow!("AI_ID not provided (use --ai-id or AI_ID env var)"))?;

            let db_pool = DatabasePool::from_env().await?;
            let manager = FileActionManager::new(db_pool);

            let file_action = awareness_core::FileAction {
                id: None,
                ai_id: final_ai_id.clone(),
                timestamp: Utc::now().naive_utc(),
                action_type: action.clone(),
                file_path: path.clone(),
                file_type: None,
                file_size: None,
                working_directory: std::env::current_dir().ok()
                    .and_then(|p| p.to_str().map(String::from)),
            };

            manager.log(file_action).await?;
            println!("Logged: {} {} by {}", action, path, final_ai_id);
        }

        Commands::RecentActions { ai_id_positional, limit_positional, ai_id, limit } => {
            let final_ai_id = ai_id_positional.or(ai_id);
            let final_limit = limit_positional.unwrap_or(limit);

            let db_pool = DatabasePool::from_env().await?;
            let manager = FileActionManager::new(db_pool);

            let actions = if let Some(ai) = final_ai_id {
                manager.get_by_ai(&ai, final_limit as i64).await?
            } else {
                manager.get_recent(final_limit as i64).await?
            };

            if actions.is_empty() {
                println!("No recent actions");
            } else {
                println!("Recent Actions ({}):", actions.len());
                println!("============================================================");
                for action in actions {
                    println!("[{}] {} {} - {}",
                        action.timestamp.format("%H:%M:%S"),
                        action.ai_id,
                        action.action_type,
                        action.file_path
                    );
                }
            }
        }

        Commands::TeamActivity { minutes_positional, minutes } => {
            let final_minutes = minutes_positional.unwrap_or(minutes);

            let db_pool = DatabasePool::from_env().await?;
            let manager = FileActionManager::new(db_pool);

            let activity = manager.get_team_activity(final_minutes as i64).await?;

            if activity.is_empty() {
                println!("No team activity in last {} minutes", final_minutes);
            } else {
                println!("Team Activity (last {} minutes):", final_minutes);
                println!("============================================================");
                for (ai_id, count) in activity {
                    println!("{}: {} actions", ai_id, count);
                }
            }
        }
    }

    Ok(())
}
