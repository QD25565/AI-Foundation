//! Presence CLI - Real-Time Presence via Redis Pub/Sub (NO POLLING)

use anyhow::Result;
use clap::{Parser, Subcommand};
use presence_rs::{PresencePublisher, PresenceSubscriber, PresenceStatus};
use std::env;

#[derive(Parser)]
#[command(name = "presence-cli")]
#[command(about = "Real-time AI presence management (NO POLLING)")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(long, global = true, hide = true)]
    redis_url: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Announce joining (publishes join event)
    #[command(visible_aliases = ["start", "online", "connect", "arrive"])]
    Join {
        #[arg(value_name = "AI_ID")]
        ai_id: Option<String>,
        #[arg(value_name = "STATUS", default_value = "active")]
        status: Option<String>,
        #[arg(value_name = "DETAIL")]
        detail: Option<String>,
    },

    /// Announce leaving (publishes leave event)
    #[command(visible_aliases = ["stop", "offline", "disconnect", "exit"])]
    Leave {
        #[arg(value_name = "AI_ID")]
        ai_id: Option<String>,
    },

    /// Update status (publishes status change)
    #[command(visible_aliases = ["set", "status", "change", "modify"])]
    Update {
        #[arg(value_name = "AI_ID")]
        ai_id: Option<String>,
        #[arg(value_name = "STATUS")]
        status: Option<String>,
        #[arg(value_name = "DETAIL")]
        detail: Option<String>,
    },

    /// Watch presence events in real-time (blocking)
    #[command(visible_aliases = ["subscribe", "monitor", "listen", "follow"])]
    Watch,

    /// List online AIs (from local cache after watch)
    #[command(visible_aliases = ["who", "team", "ls", "show"])]
    List,

    /// Check if AI is online
    #[command(visible_aliases = ["check", "query", "lookup", "ping"])]
    IsOnline {
        #[arg(value_name = "AI_ID")]
        ai_id: String,
    },

    /// Count online AIs
    #[command(visible_aliases = ["total", "num", "howmany", "size"])]
    Count,
}

fn parse_status(s: &str) -> Result<PresenceStatus> {
    PresenceStatus::from_str(s)
        .ok_or_else(|| anyhow::anyhow!("Invalid status: {}. Use active, standby, idle, offline", s))
}

fn get_redis_url(cli_url: Option<String>) -> String {
    cli_url.or_else(|| env::var("REDIS_URL").ok())
        .unwrap_or_else(|| "redis://localhost:12963/0".to_string())
}

fn get_ai_id(provided: Option<String>) -> Option<String> {
    provided.or_else(|| env::var("AI_ID").ok())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let redis_url = get_redis_url(cli.redis_url);

    match cli.command {
        Commands::Join { ai_id, status, detail } => {
            let ai_id = get_ai_id(ai_id)
                .ok_or_else(|| anyhow::anyhow!("AI_ID required.\nHint: Use 'presence-cli join <AI_ID>' or set AI_ID environment variable"))?;
            let status_enum = parse_status(&status.unwrap_or_else(|| "active".to_string()))?;
            let publisher = PresencePublisher::new(&redis_url, &ai_id).await?;
            publisher.join(status_enum, detail).await?;
            println!("PUBLISHED: {} joined", ai_id);
        }

        Commands::Leave { ai_id } => {
            let ai_id = get_ai_id(ai_id)
                .ok_or_else(|| anyhow::anyhow!("AI_ID required.\nHint: Use 'presence-cli leave <AI_ID>' or set AI_ID environment variable"))?;
            let publisher = PresencePublisher::new(&redis_url, &ai_id).await?;
            publisher.leave().await?;
            println!("PUBLISHED: {} left", ai_id);
        }

        Commands::Update { ai_id, status, detail } => {
            let ai_id = get_ai_id(ai_id)
                .ok_or_else(|| anyhow::anyhow!("AI_ID required.\nHint: Use 'presence-cli update <AI_ID> <STATUS>' or set AI_ID environment variable"))?;
            let status_str = status.ok_or_else(|| anyhow::anyhow!("Status required.\nHint: Use 'presence-cli update <AI_ID> <STATUS>' where STATUS is active, standby, idle, or offline"))?;
            let status_enum = parse_status(&status_str)?;
            let publisher = PresencePublisher::new(&redis_url, &ai_id).await?;
            publisher.update_status(status_enum, detail).await?;
            println!("PUBLISHED: {} -> {}", ai_id, status_str);
        }

        Commands::Watch => {
            let ai_id = get_ai_id(None).unwrap_or_else(|| "watcher".to_string());
            println!("=== REAL-TIME PRESENCE WATCH ===");
            println!("Subscribing to presence:events (Ctrl+C to stop)...");
            let subscriber = PresenceSubscriber::new(&redis_url, &ai_id).await?;
            subscriber.subscribe().await?;
        }

        Commands::List => {
            let ai_id = get_ai_id(None).unwrap_or_else(|| "query".to_string());
            let subscriber = PresenceSubscriber::new(&redis_url, &ai_id).await?;
            let online = subscriber.get_all_online().await;
            if online.is_empty() {
                println!("No AIs in cache. Run 'presence-cli watch' first.");
            } else {
                println!("=== ONLINE ({}) ===", online.len());
                for (id, state) in online {
                    println!("  {} : {}", id, state.status.as_str());
                }
            }
        }

        Commands::IsOnline { ai_id } => {
            let my_id = get_ai_id(None).unwrap_or_else(|| "query".to_string());
            let subscriber = PresenceSubscriber::new(&redis_url, &my_id).await?;
            if subscriber.is_online(&ai_id).await {
                println!("{} ONLINE", ai_id);
            } else {
                println!("{} OFFLINE", ai_id);
                std::process::exit(1);
            }
        }

        Commands::Count => {
            let ai_id = get_ai_id(None).unwrap_or_else(|| "query".to_string());
            let subscriber = PresenceSubscriber::new(&redis_url, &ai_id).await?;
            println!("Online: {}", subscriber.online_count().await);
        }
    }
    Ok(())
}
