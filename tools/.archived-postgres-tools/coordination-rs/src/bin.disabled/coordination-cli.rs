///! Coordination CLI - Task & Workflow Management
///!
///! Replaces task_manager.py and workflow Python tools.

use anyhow::Result;
use clap::{Parser, Subcommand};
use ai_foundation_coordination::{Task, TaskManager, TaskStatus, TaskPriority};
use std::env;

#[derive(Parser)]
#[command(name = "coordination-cli")]
#[command(about = "AI task and workflow coordination", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Task management
    Task {
        #[command(subcommand)]
        operation: TaskOps,
    },

    /// Workflow management
    Workflow {
        #[command(subcommand)]
        operation: WorkflowOps,
    },
}

#[derive(Subcommand)]
enum TaskOps {
    /// Add a new task
    Add {
        /// Task description
        description: String,

        /// Priority (1-5)
        #[arg(long, default_value = "3")]
        priority: u8,

        /// Assigned AI
        #[arg(long)]
        assigned_to: Option<String>,
    },

    /// List tasks
    List {
        /// Show only pending tasks
        #[arg(long)]
        pending: bool,

        /// Show only completed tasks
        #[arg(long)]
        completed: bool,
    },

    /// Complete a task
    Complete {
        /// Task ID
        task_id: String,

        /// Result/notes
        #[arg(long)]
        result: Option<String>,
    },

    /// Claim a task
    Claim {
        /// Task ID
        task_id: String,

        /// AI identifier
        #[arg(long)]
        ai_id: String,
    },
}

#[derive(Subcommand)]
enum WorkflowOps {
    /// Create workflow
    Create {
        /// Workflow name
        name: String,

        /// Description
        #[arg(long)]
        description: Option<String>,
    },

    /// List workflows
    List,

    /// Run workflow
    Run {
        /// Workflow ID
        workflow_id: String,
    },
}

fn parse_priority(priority: u8) -> TaskPriority {
    match priority {
        1 => TaskPriority::Low,
        2 => TaskPriority::Low,
        3 => TaskPriority::Medium,
        4 => TaskPriority::High,
        5 => TaskPriority::Critical,
        _ => TaskPriority::Medium,
    }
}

fn get_database_url() -> String {
    env::var("DATABASE_URL")
        .or_else(|_| env::var("POSTGRES_URL"))
        .unwrap_or_else(|_| "postgresql://localhost/ai_foundation".to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let database_url = get_database_url();

    match cli.command {
        Commands::Task { operation } => {
            let manager = TaskManager::new(&database_url).await?;

            match operation {
                TaskOps::Add { description, priority, assigned_to } => {
                    let priority_enum = parse_priority(priority);

                    let mut task = Task::new(description.clone());
                    task.priority = priority_enum;
                    if let Some(assignee) = assigned_to {
                        task.assigned_to = Some(assignee);
                    }

                    let task_id = manager.create_task(&task).await?;
                    println!("Task created: {}", task_id);
                    println!("Description: {}", description);
                    println!("Priority: {:?}", priority_enum);
                }

                TaskOps::List { pending, completed } => {
                    let tasks = if pending {
                        manager.list_pending_tasks().await?
                    } else if completed {
                        manager.list_completed_tasks().await?
                    } else {
                        manager.list_all_tasks().await?
                    };

                    if tasks.is_empty() {
                        println!("No tasks");
                    } else {
                        println!("Tasks ({}):", tasks.len());
                        println!("============================================================");
                        for task in tasks {
                            let status_icon = match task.status {
                                TaskStatus::Pending => "⏳",
                                TaskStatus::InProgress => "🔄",
                                TaskStatus::Completed => "✅",
                                TaskStatus::Failed => "❌",
                                TaskStatus::Cancelled => "🚫",
                            };

                            println!("{} [{}] {:?} - {}",
                                status_icon,
                                task.id,
                                task.priority,
                                task.description
                            );

                            if let Some(assignee) = task.assigned_to {
                                println!("   Assigned: {}", assignee);
                            }
                        }
                    }
                }

                TaskOps::Complete { task_id, result } => {
                    manager.complete_task(&task_id, result.as_deref()).await?;
                    println!("Task completed: {}", task_id);
                }

                TaskOps::Claim { task_id, ai_id } => {
                    manager.claim_task(&task_id, &ai_id).await?;
                    println!("Task claimed: {} by {}", task_id, ai_id);
                }
            }
        }

        Commands::Workflow { operation } => {
            match operation {
                WorkflowOps::Create { name, description } => {
                    println!("Creating workflow: {}", name);
                    if let Some(desc) = description {
                        println!("Description: {}", desc);
                    }
                    println!("Note: Workflow engine implementation pending");
                }

                WorkflowOps::List => {
                    println!("Listing workflows...");
                    println!("Note: Workflow engine implementation pending");
                }

                WorkflowOps::Run { workflow_id } => {
                    println!("Running workflow: {}", workflow_id);
                    println!("Note: Workflow engine implementation pending");
                }
            }
        }
    }

    Ok(())
}
