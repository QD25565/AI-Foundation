//! TeamEngram CLI - Test and benchmark tool

use anyhow::Result;
use clap::{Parser, Subcommand};
use teamengram::{TeamEngram, RecordData};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "teamengram-cli")]
#[command(about = "TeamEngram - LMDB-style storage for AI-Foundation")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Send a direct message
    #[command(visible_aliases = ["dm", "message", "msg"])]
    DirectMessage {
        /// Sender AI ID
        from: String,
        /// Recipient AI ID
        to: String,
        /// Message content
        content: String,
    },

    /// Read direct messages
    #[command(visible_aliases = ["dms", "inbox", "read-dms"])]
    ReadDms {
        /// Recipient AI ID
        ai_id: String,
        /// Maximum messages to return
        #[arg(long, default_value = "10")]
        limit: usize,
    },

    /// Send a broadcast
    #[command(visible_aliases = ["bc", "announce", "broadcast"])]
    Broadcast {
        /// Sender AI ID
        from: String,
        /// Channel name
        #[arg(long, default_value = "general")]
        channel: String,
        /// Message content
        content: String,
    },

    /// Read broadcasts
    #[command(visible_aliases = ["bcs", "messages", "read-broadcasts"])]
    ReadBroadcasts {
        /// Channel name
        #[arg(long, default_value = "general")]
        channel: String,
        /// Maximum messages to return
        #[arg(long, default_value = "10")]
        limit: usize,
    },

    /// Update presence
    #[command(visible_aliases = ["presence", "status", "update-status"])]
    UpdatePresence {
        /// AI ID
        ai_id: String,
        /// Status (active, standby, idle)
        #[arg(long, default_value = "active")]
        status: String,
        /// Current task
        #[arg(long, default_value = "")]
        task: String,
    },

    /// Show store statistics
    #[command(visible_aliases = ["info", "status", "statistics"])]
    Stats,

    /// Run benchmarks
    #[command(visible_aliases = ["perf", "test", "performance"])]
    Benchmark {
        /// Number of operations
        #[arg(long, default_value = "1000")]
        count: usize,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let path = TeamEngram::default_path();

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut store = TeamEngram::open(&path)?;

    match cli.command {
        Commands::DirectMessage { from, to, content } => {
            let id = store.insert_dm(&from, &to, &content)?;
            println!("dm_sent|{}|{}|{}|{}", id, from, to, truncate(&content, 50));
        }

        Commands::ReadDms { ai_id, limit } => {
            let dms = store.get_dms(&ai_id, limit)?;
            println!("DIRECT MESSAGES ({})", dms.len());
            for record in dms {
                if let RecordData::DirectMessage(dm) = record.data {
                    println!("from|{}|{}|{}", dm.from_ai, time_ago(record.created_at), dm.content);
                }
            }
        }

        Commands::Broadcast { from, channel, content } => {
            let id = store.insert_broadcast(&from, &channel, &content)?;
            println!("broadcast_sent|{}|{}|{}|{}", id, from, channel, truncate(&content, 50));
        }

        Commands::ReadBroadcasts { channel, limit } => {
            let broadcasts = store.get_broadcasts(&channel, limit)?;
            println!("BROADCASTS ({})", broadcasts.len());
            for record in broadcasts {
                if let RecordData::Broadcast(bc) = record.data {
                    println!("{}|{}|{}|{}", bc.from_ai, time_ago(record.created_at), bc.channel, bc.content);
                }
            }
        }

        Commands::UpdatePresence { ai_id, status, task } => {
            store.update_presence(&ai_id, &status, &task)?;
            println!("presence_updated|{}|{}|{}", ai_id, status, task);
        }

        Commands::Stats => {
            let stats = store.stats();
            println!("TEAMENGRAM STATS");
            println!("================");
            println!("File size:    {} KB", stats.file_size / 1024);
            println!("Total pages:  {}", stats.total_pages);
            println!("Used pages:   {}", stats.used_pages);
            println!("Transaction:  {}", stats.txn_id);
            println!("Next ID:      {}", stats.next_id);
        }

        Commands::Benchmark { count } => {
            println!("TEAMENGRAM BENCHMARK");
            println!("====================");
            println!("Operations: {}", count);
            println!();

            // Benchmark DM inserts
            let start = Instant::now();
            for i in 0..count {
                store.insert_dm("bench-from", "bench-to", &format!("Message {}", i))?;
            }
            let dm_insert_time = start.elapsed();
            let dm_insert_rate = count as f64 / dm_insert_time.as_secs_f64();
            println!("DM Insert:    {:>8.2} ops/sec ({:?} total)", dm_insert_rate, dm_insert_time);

            // Benchmark broadcast inserts
            let start = Instant::now();
            for i in 0..count {
                store.insert_broadcast("bench-from", "bench", &format!("Broadcast {}", i))?;
            }
            let bc_insert_time = start.elapsed();
            let bc_insert_rate = count as f64 / bc_insert_time.as_secs_f64();
            println!("BC Insert:    {:>8.2} ops/sec ({:?} total)", bc_insert_rate, bc_insert_time);

            // Benchmark presence updates
            let start = Instant::now();
            for i in 0..count {
                store.update_presence("bench-ai", "active", &format!("Task {}", i))?;
            }
            let presence_time = start.elapsed();
            let presence_rate = count as f64 / presence_time.as_secs_f64();
            println!("Presence:     {:>8.2} ops/sec ({:?} total)", presence_rate, presence_time);

            // Benchmark DM reads
            let start = Instant::now();
            for _ in 0..count {
                let _ = store.get_dms("bench-to", 10)?;
            }
            let dm_read_time = start.elapsed();
            let dm_read_rate = count as f64 / dm_read_time.as_secs_f64();
            println!("DM Read:      {:>8.2} ops/sec ({:?} total)", dm_read_rate, dm_read_time);

            println!();
            println!("Final stats:");
            let stats = store.stats();
            println!("  File size: {} KB", stats.file_size / 1024);
            println!("  Transactions: {}", stats.txn_id);
        }
    }

    Ok(())
}

// NO TRUNCATION - QD directive: "context starvation is the ABSOLUTE ENEMY OF AIs"
// This function preserved for API compatibility but returns full content
fn truncate(s: &str, _max: usize) -> String {
    s.to_string()
}

fn time_ago(millis: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let diff = now.saturating_sub(millis);
    let secs = diff / 1000;

    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}
