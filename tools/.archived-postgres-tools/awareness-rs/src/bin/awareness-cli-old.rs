///! Awareness CLI - File Tracking & Action Logging
///!
///! Replaces awareness Python tools with native Rust binary.

use anyhow::Result;
use clap::{Parser, Subcommand, Args};
use awareness_core::{FileActionManager, database::DatabasePool, analyze};
use chrono::Utc;

#[derive(Parser)]
#[command(name = "awareness-cli")]
#[command(about = "AI file tracking and awareness system", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze a file for context
    Analyze {
        /// File path to analyze
        path: String,
    },

    /// Log a file action
    LogAction {
        /// Action type (read, edit, write, delete)
        action: String,

        /// File path
        path: String,

        /// AI identifier
        #[arg(long)]
        ai_id: String,
    },

    /// Get recent actions
    RecentActions {
        /// AI identifier
        #[arg(long)]
        ai_id: Option<String>,

        /// Limit results
        #[arg(long, default_value = "10")]
        limit: i32,
    },

    /// Get team activity
    TeamActivity {
        /// Minutes to look back
        #[arg(long, default_value = "60")]
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
            let db_pool = DatabasePool::from_env().await?;
            let manager = FileActionManager::new(db_pool);

            let file_action = awareness_core::FileAction {
                id: None,
                ai_id: ai_id.clone(),
                timestamp: Utc::now().naive_utc(),
                action_type: action.clone(),
                file_path: path.clone(),
                file_type: None,
                file_size: None,
                working_directory: std::env::current_dir().ok()
                    .and_then(|p| p.to_str().map(String::from)),
            };

            manager.log(file_action).await?;
            println!("Logged: {} {} by {}", action, path, ai_id);
        }

        Commands::RecentActions { ai_id, limit } => {
            let db_pool = DatabasePool::from_env().await?;
            let manager = FileActionManager::new(db_pool);

            let actions = if let Some(ai) = ai_id {
                manager.get_by_ai(&ai, limit as i64).await?
            } else {
                manager.get_recent(limit as i64).await?
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

        Commands::TeamActivity { minutes } => {
            let db_pool = DatabasePool::from_env().await?;
            let manager = FileActionManager::new(db_pool);

            let activity = manager.get_team_activity(minutes as i64).await?;

            if activity.is_empty() {
                println!("No team activity in last {} minutes", minutes);
            } else {
                println!("Team Activity (last {} minutes):", minutes);
                println!("============================================================");
                for (ai_id, count) in activity {
                    println!("{}: {} actions", ai_id, count);
                }
            }
        }
    }

    Ok(())
}
