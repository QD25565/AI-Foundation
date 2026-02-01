//! Task CLI - High-Performance Local Task Manager for AI Agents
//!
//! Powered by Telos - AI-specific task engine with Engram-style storage:
//! - O(1) stats via pre-computed counters
//! - O(1) status filtering via in-memory indexes
//! - O(log n) priority queue for get-next-task
//! - Memory-mapped I/O with index persistence
//!
//! Usage:
//!   task-cli add "Fix authentication bug" -p high
//!   task-cli list                           # or: ls, show, tasks
//!   task-cli start 1                        # or: begin, work
//!   task-cli complete 1                     # or: done, finish
//!   task-cli verify 1                       # or: check, approve
//!   task-cli block 1 "Waiting for API"
//!   task-cli unblock 1                      # or: resume, continue
//!   task-cli delete 1                       # or: rm, remove, del
//!   task-cli get 1                          # or: info, show, view
//!   task-cli stats                          # or: status, summary
//!   task-cli next                           # Get highest priority task
//!   task-cli migrate                        # Migrate from legacy SQLite
//!   task-cli benchmark                      # Performance benchmarks

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::time::Instant;
use task_rs::{Task, TaskPriority, TaskStats, TaskStatus, TaskStore};

// ============================================================================
// CLI DEFINITION (Following CLI-BEST-PRACTICES.md)
// ============================================================================

#[derive(Parser)]
#[command(name = "task-cli")]
#[command(about = "High-performance AI task manager (Telos engine)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new task (e.g., task-cli add "Fix bug" --priority high)
    #[command(visible_aliases = ["create", "new", "task", "queue"])]
    Add {
        /// Task description (e.g., "Fix authentication bug")
        #[arg(value_name = "CONTENT")]
        content: String,

        /// Priority: low, normal, high, or critical
        #[arg(long, default_value = "normal", value_name = "PRIORITY")]
        priority: String,

        /// Link to a notebook note ID
        #[arg(long, value_name = "NOTE_ID")]
        note_id: Option<i64>,

        // Hidden flag fallbacks for AIs that try flag syntax
        #[arg(long = "content", hide = true)]
        content_flag: Option<String>,
    },

    /// List tasks (defaults to active: pending + in_progress + blocked)
    #[command(visible_aliases = ["ls", "show", "tasks", "todo"])]
    List {
        /// Show all tasks including completed/verified
        #[arg(long)]
        all: bool,

        /// Show only completed tasks
        #[arg(long)]
        completed: bool,

        /// Maximum results (default: 20)
        #[arg(long, default_value = "20", value_name = "LIMIT")]
        limit: usize,
    },

    /// Start working on a task (pending -> in_progress)
    #[command(visible_aliases = ["begin", "work", "pick", "claim"])]
    Start {
        /// Task ID to start (e.g., 1)
        #[arg(value_name = "ID")]
        id: u64,
    },

    /// Complete a task (in_progress -> completed)
    #[command(visible_aliases = ["done", "finish", "close", "end"])]
    Complete {
        /// Task ID to complete (e.g., 1)
        #[arg(value_name = "ID")]
        id: u64,
    },

    /// Verify a completed task (completed -> verified)
    #[command(visible_aliases = ["check", "approve", "confirm", "accept"])]
    Verify {
        /// Task ID to verify (e.g., 1)
        #[arg(value_name = "ID")]
        id: u64,
    },

    /// Block a task with a reason
    #[command(visible_aliases = ["hold", "pause", "wait", "stuck"])]
    Block {
        /// Task ID to block (e.g., 1)
        #[arg(value_name = "ID")]
        id: u64,

        /// Reason for blocking (e.g., "Waiting for API response")
        #[arg(value_name = "REASON")]
        reason: String,
    },

    /// Unblock a task (blocked -> pending)
    #[command(visible_aliases = ["resume", "continue", "unstick", "free"])]
    Unblock {
        /// Task ID to unblock (e.g., 1)
        #[arg(value_name = "ID")]
        id: u64,
    },

    /// Delete a task permanently
    #[command(visible_aliases = ["rm", "remove", "del", "drop"])]
    Delete {
        /// Task ID to delete (e.g., 1)
        #[arg(value_name = "ID")]
        id: u64,
    },

    /// Get details of a specific task
    #[command(visible_aliases = ["info", "view", "detail", "inspect"])]
    Get {
        /// Task ID (e.g., 1)
        #[arg(value_name = "ID")]
        id: u64,
    },

    /// Show task statistics (O(1) - pre-computed)
    #[command(visible_aliases = ["status", "summary", "count", "overview"])]
    Stats,

    /// Get the next highest-priority task (O(1) - heap peek)
    #[command(visible_aliases = ["top", "first", "priority", "peek"])]
    Next,

    /// Migrate from legacy SQLite database
    #[command(visible_aliases = ["import", "upgrade", "convert"])]
    Migrate,

    /// Run performance benchmarks
    #[command(visible_aliases = ["bench", "perf", "test"])]
    Benchmark,
}

// ============================================================================
// FORMATTING
// ============================================================================

fn format_relative_time(dt: &chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let diff = now.signed_duration_since(*dt);
    let minutes = diff.num_minutes();
    let hours = diff.num_hours();
    let days = diff.num_days();

    if minutes < 1 {
        "now".to_string()
    } else if minutes < 60 {
        format!("{}m ago", minutes)
    } else if hours < 24 {
        format!("{}h ago", hours)
    } else if days == 1 {
        "yesterday".to_string()
    } else if days < 7 {
        format!("{}d ago", days)
    } else {
        dt.format("%b %d").to_string()
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        // Safe Unicode truncation using char_indices to find valid byte boundary
        let truncate_at = max_len.saturating_sub(3);
        let byte_pos = s.char_indices()
            .nth(truncate_at)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}...", &s[..byte_pos])
    } else {
        s.to_string()
    }
}

fn print_task_line(task: &Task) {
    let when = format_relative_time(&task.created);
    let content = truncate(&task.content, 60);
    let priority = task.priority.marker();

    if task.status == TaskStatus::Blocked {
        let reason = task
            .blocked_reason
            .as_ref()
            .map(|r| format!(" ({})", truncate(r, 30)))
            .unwrap_or_default();
        println!("#{} {} {}: {}{}{}", task.id, task.status.symbol(), when, content, priority, reason);
    } else {
        println!("#{} {} {}: {}{}", task.id, task.status.symbol(), when, content, priority);
    }
}

fn print_task_detail(task: &Task) {
    println!("Task #{}", task.id);
    println!("Status: {}", task.status.symbol());
    println!("Priority: {}", task.priority.as_str());
    println!("Created: {}", format_relative_time(&task.created));
    println!("Updated: {}", format_relative_time(&task.updated));
    if let Some(reason) = &task.blocked_reason {
        println!("Blocked: {}", reason);
    }
    if let Some(nid) = task.note_id {
        println!("Linked note: #{}", nid);
    }
    println!();
    println!("{}", task.content);
}

fn print_stats(stats: &TaskStats, ai_id: &str) {
    println!(
        "Tasks ({}): Pending: {} | In Progress: {} | Blocked: {} | Completed: {} | Verified: {} | Total: {}",
        ai_id,
        stats.pending,
        stats.in_progress,
        stats.blocked,
        stats.completed,
        stats.verified,
        stats.total()
    );
}

// ============================================================================
// MAIN
// ============================================================================

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Open store with auto-migration from SQLite
    let store_path = TaskStore::default_path();
    let (mut store, migrated) = TaskStore::open_with_migration(&store_path)?;

    if migrated > 0 {
        println!("Migrated {} tasks from SQLite to Telos", migrated);
    }

    let ai_id = TaskStore::get_ai_id();

    match cli.command {
        Commands::Add { content, priority, note_id, content_flag } => {
            // Consolidate positional and flag versions
            let task_content = if content.is_empty() {
                content_flag.unwrap_or_default()
            } else {
                content
            };

            if task_content.is_empty() {
                eprintln!("Error: Task content is required.");
                eprintln!("Hint: Use 'task-cli add \"Your task description\"'");
                return Ok(());
            }

            let priority_enum = TaskPriority::from_str(&priority);
            let id = store.add(&task_content, priority_enum, note_id)?;

            println!("Task added: #{}", id);
            if priority_enum != TaskPriority::Normal {
                println!("Priority: {}", priority_enum.as_str());
            }
            if let Some(nid) = note_id {
                println!("Linked to note: #{}", nid);
            }
        }

        Commands::List { all, completed, limit } => {
            let tasks: Vec<&Task> = if all {
                store.all_tasks().into_iter().take(limit).collect()
            } else if completed {
                store.completed_tasks().into_iter().take(limit).collect()
            } else {
                store.active_tasks().into_iter().take(limit).collect()
            };

            if tasks.is_empty() {
                if completed {
                    println!("No completed tasks");
                    println!("Hint: Use 'task-cli list' to see active tasks");
                } else if all {
                    println!("No tasks");
                    println!("Hint: Use 'task-cli add \"description\"' to create one");
                } else {
                    println!("No active tasks");
                    println!("Hint: Use 'task-cli add \"description\"' to create one");
                }
            } else {
                // Group by status for better readability
                let in_progress: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::InProgress).collect();
                let blocked: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::Blocked).collect();
                let pending: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::Pending).collect();
                let done: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::Completed || t.status == TaskStatus::Verified).collect();

                let active_count = in_progress.len() + blocked.len() + pending.len();

                if !in_progress.is_empty() || !blocked.is_empty() || !pending.is_empty() {
                    println!("ACTIVE TASKS ({})", active_count);
                    for task in &in_progress {
                        print_task_line(task);
                    }
                    for task in &blocked {
                        print_task_line(task);
                    }
                    for task in &pending {
                        print_task_line(task);
                    }
                }

                if !done.is_empty() && (all || completed) {
                    if active_count > 0 {
                        println!();
                    }
                    println!("COMPLETED ({})", done.len());
                    for task in &done {
                        print_task_line(task);
                    }
                }
            }
        }

        Commands::Start { id } => {
            if store.update_status(id, TaskStatus::InProgress, None)? {
                println!("Task #{} started", id);
            } else {
                eprintln!("Error: Task #{} not found.", id);
                eprintln!("Hint: Use 'task-cli list' to see available tasks");
            }
        }

        Commands::Complete { id } => {
            if store.update_status(id, TaskStatus::Completed, None)? {
                println!("Task #{} completed", id);
            } else {
                eprintln!("Error: Task #{} not found.", id);
                eprintln!("Hint: Use 'task-cli list' to see available tasks");
            }
        }

        Commands::Verify { id } => {
            if store.update_status(id, TaskStatus::Verified, None)? {
                println!("Task #{} verified", id);
            } else {
                eprintln!("Error: Task #{} not found.", id);
                eprintln!("Hint: Use 'task-cli list -c' to see completed tasks");
            }
        }

        Commands::Block { id, reason } => {
            if store.update_status(id, TaskStatus::Blocked, Some(&reason))? {
                println!("Task #{} blocked: {}", id, reason);
            } else {
                eprintln!("Error: Task #{} not found.", id);
                eprintln!("Hint: Use 'task-cli list' to see available tasks");
            }
        }

        Commands::Unblock { id } => {
            if store.update_status(id, TaskStatus::Pending, None)? {
                println!("Task #{} unblocked", id);
            } else {
                eprintln!("Error: Task #{} not found.", id);
                eprintln!("Hint: Use 'task-cli list' to see blocked tasks");
            }
        }

        Commands::Delete { id } => {
            if store.delete(id)? {
                println!("Task #{} deleted", id);
            } else {
                eprintln!("Error: Task #{} not found.", id);
                eprintln!("Hint: Use 'task-cli list -a' to see all tasks");
            }
        }

        Commands::Get { id } => {
            match store.get(id) {
                Some(task) => print_task_detail(task),
                None => {
                    eprintln!("Error: Task #{} not found.", id);
                    eprintln!("Hint: Use 'task-cli list' to see available tasks");
                }
            }
        }

        Commands::Stats => {
            let stats = store.stats();
            print_stats(stats, &ai_id);
        }

        Commands::Next => {
            match store.next_priority() {
                Some(task) => {
                    println!("Next priority task:");
                    print_task_detail(task);
                }
                None => {
                    println!("No active tasks in queue");
                    println!("Hint: Use 'task-cli add \"description\"' to create one");
                }
            }
        }

        Commands::Migrate => {
            if TaskStore::has_legacy_sqlite() {
                let count = store.migrate_from_sqlite()?;
                if count > 0 {
                    println!("Migrated {} tasks from SQLite to Telos", count);
                } else {
                    println!("No tasks found in legacy SQLite database");
                }
            } else {
                println!("No legacy SQLite database found at:");
                println!("  {}", TaskStore::legacy_sqlite_path().display());
                println!("Hint: Already using Telos engine");
            }
        }

        Commands::Benchmark => {
            println!("Telos Performance Benchmark");
            println!("============================");
            println!();

            // Benchmark add
            let start = Instant::now();
            let mut ids = Vec::new();
            for i in 0..100 {
                let id = store.add(&format!("Benchmark task {}", i), TaskPriority::Normal, None)?;
                ids.push(id);
            }
            let add_time = start.elapsed();
            println!("Add 100 tasks: {:?} ({:.2}µs/task)", add_time, add_time.as_micros() as f64 / 100.0);

            // Benchmark stats (should be O(1))
            let start = Instant::now();
            for _ in 0..1000 {
                let _ = store.stats();
            }
            let stats_time = start.elapsed();
            println!("Stats x1000: {:?} ({:.0}ns/call)", stats_time, stats_time.as_nanos() as f64 / 1000.0);

            // Benchmark get by ID (should be O(1))
            let start = Instant::now();
            for id in &ids {
                let _ = store.get(*id);
            }
            let get_time = start.elapsed();
            println!("Get 100 tasks: {:?} ({:.0}ns/task)", get_time, get_time.as_nanos() as f64 / 100.0);

            // Benchmark by_status (should be O(k))
            let start = Instant::now();
            for _ in 0..100 {
                let _ = store.by_status(TaskStatus::Pending);
            }
            let status_time = start.elapsed();
            println!("List by status x100: {:?} ({:.2}µs/call)", status_time, status_time.as_micros() as f64 / 100.0);

            // Benchmark next_priority (should be O(1) peek)
            let start = Instant::now();
            for _ in 0..1000 {
                let _ = store.next_priority();
            }
            let priority_time = start.elapsed();
            println!("Next priority x1000: {:?} ({:.0}ns/call)", priority_time, priority_time.as_nanos() as f64 / 1000.0);

            // Benchmark status update
            let start = Instant::now();
            for id in &ids[..50] {
                store.update_status(*id, TaskStatus::InProgress, None)?;
            }
            let update_time = start.elapsed();
            println!("Update 50 statuses: {:?} ({:.2}µs/update)", update_time, update_time.as_micros() as f64 / 50.0);

            // Cleanup benchmark tasks
            for id in ids {
                store.delete(id)?;
            }

            println!();
            println!("Benchmark complete. All test tasks cleaned up.");
        }
    }

    // Persist indexes for fast startup next time
    store.persist_indexes()?;

    Ok(())
}
