//! Task CLI with DAG Support - High-Performance Task Manager for AI Agents
//!
//! Enhanced with Directed Acyclic Graph (DAG) features:
//! - Task dependencies (depends_on)
//! - Topological sort for execution order
//! - Critical path analysis
//! - Parallel task identification
//! - Backward validation propagation
//!
//! Usage:
//!   task-cli add "Fix auth bug" --priority high --depends-on 1,2
//!   task-cli ready                    # Tasks ready to execute
//!   task-cli parallel                 # Tasks that can run in parallel
//!   task-cli critical-path            # Show longest dependency chain
//!   task-cli validate --id 1 --pass   # Validate task (or --fail --reason "...")
//!   task-cli dag                      # ASCII visualization

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::PathBuf;

// ============================================================================
// DATA STRUCTURES
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TaskStatus {
    Pending,
    Ready,
    InProgress,
    Executed,
    Validated,
    ValidationFailed,
    Blocked,
    Failed,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::Ready => write!(f, "ready"),
            TaskStatus::InProgress => write!(f, "in_progress"),
            TaskStatus::Executed => write!(f, "executed"),
            TaskStatus::Validated => write!(f, "validated"),
            TaskStatus::ValidationFailed => write!(f, "validation_failed"),
            TaskStatus::Blocked => write!(f, "blocked"),
            TaskStatus::Failed => write!(f, "failed"),
        }
    }
}

impl TaskStatus {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "ready" => TaskStatus::Ready,
            "in_progress" => TaskStatus::InProgress,
            "executed" => TaskStatus::Executed,
            "validated" => TaskStatus::Validated,
            "validation_failed" => TaskStatus::ValidationFailed,
            "blocked" => TaskStatus::Blocked,
            "failed" => TaskStatus::Failed,
            _ => TaskStatus::Pending,
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "[ ]",
            TaskStatus::Ready => "[R]",
            TaskStatus::InProgress => "[>]",
            TaskStatus::Executed => "[*]",
            TaskStatus::Validated => "[V]",
            TaskStatus::ValidationFailed => "[X]",
            TaskStatus::Blocked => "[!]",
            TaskStatus::Failed => "[F]",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TaskPriority {
    Low,
    Normal,
    High,
    Critical,
}

impl fmt::Display for TaskPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskPriority::Low => write!(f, "low"),
            TaskPriority::Normal => write!(f, "normal"),
            TaskPriority::High => write!(f, "high"),
            TaskPriority::Critical => write!(f, "critical"),
        }
    }
}

impl TaskPriority {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "low" => TaskPriority::Low,
            "high" => TaskPriority::High,
            "critical" => TaskPriority::Critical,
            _ => TaskPriority::Normal,
        }
    }

    fn value(&self) -> i32 {
        match self {
            TaskPriority::Low => 1,
            TaskPriority::Normal => 5,
            TaskPriority::High => 8,
            TaskPriority::Critical => 10,
        }
    }

    fn marker(&self) -> &'static str {
        match self {
            TaskPriority::Low => "",
            TaskPriority::Normal => "",
            TaskPriority::High => " [high]",
            TaskPriority::Critical => " [CRITICAL]",
        }
    }
}


#[derive(Debug)]
struct Task {
    id: i64,
    content: String,
    status: TaskStatus,
    priority: TaskPriority,
    created: DateTime<Utc>,
    updated: DateTime<Utc>,
    depends_on: Vec<i64>,
    blocked_reason: Option<String>,
    validation_reason: Option<String>,
    output: Option<String>,
}

// ============================================================================
// CLI DEFINITION
// ============================================================================

#[derive(Parser)]
#[command(name = "task-cli")]
#[command(about = "High-performance AI task manager with DAG support", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new task
    Add {
        /// Task description
        content: String,

        /// Priority: low, normal, high, or critical
        #[arg(long, default_value = "normal")]
        priority: String,

        /// Comma-separated list of task IDs this task depends on
        #[arg(long)]
        depends_on: Option<String>,
    },

    /// List tasks (defaults to active)
    List {
        /// Show all tasks
        #[arg(long)]
        all: bool,

        /// Show only completed/validated tasks
        #[arg(long)]
        completed: bool,

        /// Maximum results
        #[arg(long, default_value = "20")]
        limit: i64,
    },

    /// Start working on a task (pending/ready -> in_progress)
    Start {
        #[arg(long)]
        id: i64,
    },

    /// Mark task as executed (awaiting validation)
    Execute {
        #[arg(long)]
        id: i64,

        /// Optional output/result from execution
        #[arg(long)]
        output: Option<String>,
    },

    /// Validate a task (pass or fail with backward propagation)
    Validate {
        #[arg(long)]
        id: i64,

        /// Mark validation as passed
        #[arg(long)]
        pass: bool,

        /// Mark validation as failed
        #[arg(long)]
        fail: bool,

        /// Reason for validation failure
        #[arg(long)]
        reason: Option<String>,
    },

    /// Block a task with a reason
    Block {
        #[arg(long)]
        id: i64,

        #[arg(long)]
        reason: String,
    },

    /// Unblock a task
    Unblock {
        #[arg(long)]
        id: i64,
    },

    /// Add a dependency to a task
    Depend {
        /// Task that will depend on another
        #[arg(long)]
        id: i64,

        /// Task ID to depend on
        #[arg(long)]
        on: i64,
    },

    /// Remove a dependency from a task
    Undepend {
        #[arg(long)]
        id: i64,

        #[arg(long)]
        from: i64,
    },

    /// Get tasks ready to execute (all dependencies satisfied)
    Ready,

    /// Get tasks that can run in parallel
    Parallel,

    /// Show the critical path (longest dependency chain)
    CriticalPath,

    /// Show ASCII visualization of the task DAG
    Dag,

    /// Delete a task
    Delete {
        #[arg(long)]
        id: i64,
    },

    /// Get a specific task
    Get {
        #[arg(long)]
        id: i64,
    },

    /// Show task statistics
    Stats,
}


// ============================================================================
// DATABASE
// ============================================================================

fn get_db_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join(".ai-foundation");
    std::fs::create_dir_all(&dir).ok();
    dir.join("tasks_dag.db")
}

fn get_ai_id() -> String {
    std::env::var("AI_ID")
        .or_else(|_| std::env::var("AGENT_ID"))
        .unwrap_or_else(|_| "default".to_string())
}

fn init_db(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ai_id TEXT NOT NULL,
            content TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            priority TEXT NOT NULL DEFAULT 'normal',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            depends_on TEXT DEFAULT '[]',
            blocked_reason TEXT,
            validation_reason TEXT,
            output TEXT
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tasks_ai_status ON tasks(ai_id, status)",
        [],
    )?;

    // Create edges table for efficient graph queries
    conn.execute(
        "CREATE TABLE IF NOT EXISTS task_edges (
            from_id INTEGER NOT NULL,
            to_id INTEGER NOT NULL,
            ai_id TEXT NOT NULL,
            PRIMARY KEY (from_id, to_id, ai_id),
            FOREIGN KEY (from_id) REFERENCES tasks(id) ON DELETE CASCADE,
            FOREIGN KEY (to_id) REFERENCES tasks(id) ON DELETE CASCADE
        )",
        [],
    )?;

    Ok(())
}

fn open_db() -> Result<Connection> {
    let conn = Connection::open(get_db_path())?;
    init_db(&conn)?;
    Ok(conn)
}

// ============================================================================
// FORMATTING
// ============================================================================

fn format_relative_time(created: &DateTime<Utc>) -> String {
    let now = Utc::now();
    let diff = now.signed_duration_since(*created);
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
        created.format("%b %d").to_string()
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

fn parse_depends_on(s: &str) -> Vec<i64> {
    if s.is_empty() || s == "[]" {
        return vec![];
    }
    // Handle JSON array format
    let cleaned = s.trim_start_matches('[').trim_end_matches(']');
    cleaned
        .split(',')
        .filter_map(|x| x.trim().parse::<i64>().ok())
        .collect()
}

fn serialize_depends_on(deps: &[i64]) -> String {
    format!("[{}]", deps.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(","))
}

fn print_task_line(task: &Task) {
    let when = format_relative_time(&task.created);
    let content = truncate(&task.content, 50);
    let priority = task.priority.marker();
    let deps = if task.depends_on.is_empty() {
        String::new()
    } else {
        format!(" deps:{:?}", task.depends_on)
    };

    match task.status {
        TaskStatus::Blocked => {
            let reason = task.blocked_reason
                .as_ref()
                .map(|r| format!(" ({})", truncate(r, 20)))
                .unwrap_or_default();
            println!("{} #{} {}: {}{}{}{}", task.status.symbol(), task.id, when, content, priority, deps, reason);
        }
        TaskStatus::ValidationFailed => {
            let reason = task.validation_reason
                .as_ref()
                .map(|r| format!(" ({})", truncate(r, 20)))
                .unwrap_or_default();
            println!("{} #{} {}: {}{}{}{}", task.status.symbol(), task.id, when, content, priority, deps, reason);
        }
        _ => {
            println!("{} #{} {}: {}{}{}", task.status.symbol(), task.id, when, content, priority, deps);
        }
    }
}


// ============================================================================
// CORE TASK OPERATIONS
// ============================================================================

fn add_task(conn: &Connection, content: &str, priority: &str, depends_on: Option<&str>) -> Result<i64> {
    let ai_id = get_ai_id();
    let now = Utc::now().to_rfc3339();
    let priority_enum = TaskPriority::from_str(priority);

    // Parse dependencies
    let deps: Vec<i64> = depends_on
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_default();

    // Validate dependencies exist
    for dep_id in &deps {
        let exists: bool = conn.query_row(
            "SELECT 1 FROM tasks WHERE id = ?1 AND ai_id = ?2",
            params![dep_id, ai_id],
            |_| Ok(true)
        ).unwrap_or(false);
        if !exists {
            return Err(anyhow!("Dependency task #{} not found", dep_id));
        }
    }

    // Determine initial status
    let initial_status = if deps.is_empty() {
        TaskStatus::Ready
    } else {
        TaskStatus::Pending
    };

    let deps_json = serialize_depends_on(&deps);

    conn.execute(
        "INSERT INTO tasks (ai_id, content, status, priority, created_at, updated_at, depends_on)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![ai_id, content, initial_status.to_string(), priority_enum.to_string(), now, now, deps_json],
    )?;

    let task_id = conn.last_insert_rowid();

    // Add edges for dependencies
    for dep_id in &deps {
        conn.execute(
            "INSERT OR IGNORE INTO task_edges (from_id, to_id, ai_id) VALUES (?1, ?2, ?3)",
            params![dep_id, task_id, ai_id],
        )?;
    }

    Ok(task_id)
}

fn load_task(conn: &Connection, id: i64) -> Result<Option<Task>> {
    let ai_id = get_ai_id();
    let mut stmt = conn.prepare(
        "SELECT id, content, status, priority, created_at, updated_at, depends_on, blocked_reason, validation_reason, output
         FROM tasks WHERE id = ?1 AND ai_id = ?2",
    )?;

    let task = stmt
        .query_row(params![id, ai_id], |row| {
            let deps_str: String = row.get(6)?;
            Ok(Task {
                id: row.get(0)?,
                content: row.get(1)?,
                status: TaskStatus::from_str(&row.get::<_, String>(2)?),
                priority: TaskPriority::from_str(&row.get::<_, String>(3)?),
                created: DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                depends_on: parse_depends_on(&deps_str),
                blocked_reason: row.get(7)?,
                validation_reason: row.get(8)?,
                output: row.get(9)?,
            })
        })
        .ok();

    Ok(task)
}

fn list_tasks(conn: &Connection, all: bool, completed: bool, limit: i64) -> Result<Vec<Task>> {
    let ai_id = get_ai_id();

    let query = if all {
        "SELECT id, content, status, priority, created_at, updated_at, depends_on, blocked_reason, validation_reason, output
         FROM tasks WHERE ai_id = ?1 ORDER BY
         CASE status
             WHEN 'in_progress' THEN 0
             WHEN 'ready' THEN 1
             WHEN 'blocked' THEN 2
             WHEN 'pending' THEN 3
             WHEN 'executed' THEN 4
             WHEN 'validation_failed' THEN 5
             WHEN 'validated' THEN 6
             WHEN 'failed' THEN 7
         END, updated_at DESC LIMIT ?2"
    } else if completed {
        "SELECT id, content, status, priority, created_at, updated_at, depends_on, blocked_reason, validation_reason, output
         FROM tasks WHERE ai_id = ?1 AND status IN ('validated', 'executed')
         ORDER BY updated_at DESC LIMIT ?2"
    } else {
        "SELECT id, content, status, priority, created_at, updated_at, depends_on, blocked_reason, validation_reason, output
         FROM tasks WHERE ai_id = ?1 AND status IN ('pending', 'ready', 'in_progress', 'blocked', 'validation_failed')
         ORDER BY
         CASE status
             WHEN 'in_progress' THEN 0
             WHEN 'ready' THEN 1
             WHEN 'blocked' THEN 2
             WHEN 'validation_failed' THEN 3
             WHEN 'pending' THEN 4
         END, updated_at DESC LIMIT ?2"
    };

    let mut stmt = conn.prepare(query)?;
    let task_iter = stmt.query_map(params![ai_id, limit], |row| {
        let deps_str: String = row.get(6)?;
        Ok(Task {
            id: row.get(0)?,
            content: row.get(1)?,
            status: TaskStatus::from_str(&row.get::<_, String>(2)?),
            priority: TaskPriority::from_str(&row.get::<_, String>(3)?),
            created: DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            depends_on: parse_depends_on(&deps_str),
            blocked_reason: row.get(7)?,
            validation_reason: row.get(8)?,
            output: row.get(9)?,
        })
    })?;

    Ok(task_iter.filter_map(|t| t.ok()).collect())
}


// ============================================================================
// STATUS UPDATES
// ============================================================================

fn update_status(conn: &Connection, id: i64, new_status: TaskStatus) -> Result<bool> {
    let ai_id = get_ai_id();
    let now = Utc::now().to_rfc3339();

    let affected = conn.execute(
        "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE id = ?3 AND ai_id = ?4",
        params![new_status.to_string(), now, id, ai_id],
    )?;

    Ok(affected > 0)
}

fn block_task(conn: &Connection, id: i64, reason: &str) -> Result<bool> {
    let ai_id = get_ai_id();
    let now = Utc::now().to_rfc3339();

    let affected = conn.execute(
        "UPDATE tasks SET status = ?1, updated_at = ?2, blocked_reason = ?3 WHERE id = ?4 AND ai_id = ?5",
        params![TaskStatus::Blocked.to_string(), now, reason, id, ai_id],
    )?;

    Ok(affected > 0)
}

fn execute_task(conn: &Connection, id: i64, output: Option<&str>) -> Result<bool> {
    let ai_id = get_ai_id();
    let now = Utc::now().to_rfc3339();

    let affected = conn.execute(
        "UPDATE tasks SET status = ?1, updated_at = ?2, output = ?3 WHERE id = ?4 AND ai_id = ?5",
        params![TaskStatus::Executed.to_string(), now, output, id, ai_id],
    )?;

    Ok(affected > 0)
}

/// Validate a task with backward propagation on failure
fn validate_task(conn: &Connection, id: i64, passed: bool, reason: Option<&str>) -> Result<Vec<String>> {
    let ai_id = get_ai_id();
    let now = Utc::now().to_rfc3339();
    let mut messages = Vec::new();

    if passed {
        // Mark as validated
        conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE id = ?3 AND ai_id = ?4",
            params![TaskStatus::Validated.to_string(), now, id, ai_id],
        )?;
        messages.push(format!("Task #{} validated", id));

        // Check if any dependent tasks can now become ready
        let dependent_ids: Vec<i64> = conn.prepare(
            "SELECT to_id FROM task_edges WHERE from_id = ?1 AND ai_id = ?2"
        )?
        .query_map(params![id, ai_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

        for dep_id in dependent_ids {
            if check_all_deps_validated(conn, dep_id)? {
                conn.execute(
                    "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE id = ?3 AND ai_id = ?4 AND status = 'pending'",
                    params![TaskStatus::Ready.to_string(), now, dep_id, ai_id],
                )?;
                messages.push(format!("Task #{} is now ready (all deps satisfied)", dep_id));
            }
        }
    } else {
        // Validation failed - backward propagation
        conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = ?2, validation_reason = ?3 WHERE id = ?4 AND ai_id = ?5",
            params![TaskStatus::ValidationFailed.to_string(), now, reason, id, ai_id],
        )?;
        messages.push(format!("Task #{} validation failed: {}", id, reason.unwrap_or("no reason")));

        // Block all dependent tasks
        let dependent_ids: Vec<i64> = conn.prepare(
            "SELECT to_id FROM task_edges WHERE from_id = ?1 AND ai_id = ?2"
        )?
        .query_map(params![id, ai_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

        for dep_id in dependent_ids {
            conn.execute(
                "UPDATE tasks SET status = ?1, updated_at = ?2, blocked_reason = ?3 WHERE id = ?4 AND ai_id = ?5",
                params![TaskStatus::Blocked.to_string(), now, format!("Upstream task #{} failed validation", id), dep_id, ai_id],
            )?;
            messages.push(format!("Task #{} blocked (depends on failed #{})", dep_id, id));
        }

        // Reopen upstream tasks for revision (key DAG feature)
        let task = load_task(conn, id)?.ok_or_else(|| anyhow!("Task not found"))?;
        for upstream_id in &task.depends_on {
            let upstream = load_task(conn, *upstream_id)?;
            if let Some(up) = upstream {
                if up.status == TaskStatus::Validated {
                    conn.execute(
                        "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE id = ?3 AND ai_id = ?4",
                        params![TaskStatus::Pending.to_string(), now, upstream_id, ai_id],
                    )?;
                    messages.push(format!("[AUTO-ESCALATE] Reopened upstream #{} for revision", upstream_id));
                }
            }
        }
    }

    Ok(messages)
}

fn check_all_deps_validated(conn: &Connection, task_id: i64) -> Result<bool> {
    let ai_id = get_ai_id();

    let task = load_task(conn, task_id)?.ok_or_else(|| anyhow!("Task not found"))?;

    for dep_id in &task.depends_on {
        let dep = load_task(conn, *dep_id)?;
        if let Some(d) = dep {
            if d.status != TaskStatus::Validated {
                return Ok(false);
            }
        } else {
            return Ok(false);
        }
    }

    Ok(true)
}

fn add_dependency(conn: &Connection, task_id: i64, depends_on_id: i64) -> Result<()> {
    let ai_id = get_ai_id();
    let now = Utc::now().to_rfc3339();

    // Check for cycles
    if would_create_cycle(conn, task_id, depends_on_id)? {
        return Err(anyhow!("Adding this dependency would create a cycle"));
    }

    // Update depends_on array
    let task = load_task(conn, task_id)?.ok_or_else(|| anyhow!("Task not found"))?;
    let mut deps = task.depends_on.clone();
    if !deps.contains(&depends_on_id) {
        deps.push(depends_on_id);
    }
    let deps_json = serialize_depends_on(&deps);

    conn.execute(
        "UPDATE tasks SET depends_on = ?1, updated_at = ?2 WHERE id = ?3 AND ai_id = ?4",
        params![deps_json, now, task_id, ai_id],
    )?;

    // Add edge
    conn.execute(
        "INSERT OR IGNORE INTO task_edges (from_id, to_id, ai_id) VALUES (?1, ?2, ?3)",
        params![depends_on_id, task_id, ai_id],
    )?;

    // Update status if needed
    if task.status == TaskStatus::Ready {
        conn.execute(
            "UPDATE tasks SET status = 'pending' WHERE id = ?1 AND ai_id = ?2",
            params![task_id, ai_id],
        )?;
    }

    Ok(())
}

fn remove_dependency(conn: &Connection, task_id: i64, depends_on_id: i64) -> Result<()> {
    let ai_id = get_ai_id();
    let now = Utc::now().to_rfc3339();

    // Update depends_on array
    let task = load_task(conn, task_id)?.ok_or_else(|| anyhow!("Task not found"))?;
    let deps: Vec<i64> = task.depends_on.iter().filter(|&&d| d != depends_on_id).copied().collect();
    let deps_json = serialize_depends_on(&deps);

    conn.execute(
        "UPDATE tasks SET depends_on = ?1, updated_at = ?2 WHERE id = ?3 AND ai_id = ?4",
        params![deps_json, now, task_id, ai_id],
    )?;

    // Remove edge
    conn.execute(
        "DELETE FROM task_edges WHERE from_id = ?1 AND to_id = ?2 AND ai_id = ?3",
        params![depends_on_id, task_id, ai_id],
    )?;

    // Check if task can become ready
    if task.status == TaskStatus::Pending && deps.is_empty() {
        conn.execute(
            "UPDATE tasks SET status = 'ready' WHERE id = ?1 AND ai_id = ?2",
            params![task_id, ai_id],
        )?;
    }

    Ok(())
}


// ============================================================================
// DAG ALGORITHMS
// ============================================================================

fn would_create_cycle(conn: &Connection, task_id: i64, new_dep_id: i64) -> Result<bool> {
    // Check if new_dep_id is reachable from task_id (would create cycle)
    let ai_id = get_ai_id();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(new_dep_id);

    while let Some(current) = queue.pop_front() {
        if current == task_id {
            return Ok(true); // Cycle detected
        }
        if visited.contains(&current) {
            continue;
        }
        visited.insert(current);

        // Get tasks that depend on current
        let dependents: Vec<i64> = conn.prepare(
            "SELECT to_id FROM task_edges WHERE from_id = ?1 AND ai_id = ?2"
        )?
        .query_map(params![current, ai_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

        for dep in dependents {
            queue.push_back(dep);
        }
    }

    Ok(false)
}

/// Get tasks that are ready to execute (all dependencies validated)
fn get_ready_tasks(conn: &Connection) -> Result<Vec<Task>> {
    let ai_id = get_ai_id();
    let all_tasks = list_tasks(conn, true, false, 1000)?;

    let mut ready = Vec::new();
    for task in all_tasks {
        if task.status == TaskStatus::Ready {
            ready.push(task);
        } else if task.status == TaskStatus::Pending {
            // Check if all deps are validated
            let mut all_validated = true;
            for dep_id in &task.depends_on {
                let dep = load_task(conn, *dep_id)?;
                if let Some(d) = dep {
                    if d.status != TaskStatus::Validated {
                        all_validated = false;
                        break;
                    }
                } else {
                    all_validated = false;
                    break;
                }
            }
            if all_validated {
                ready.push(task);
            }
        }
    }

    // Sort by priority (higher first)
    ready.sort_by(|a, b| b.priority.value().cmp(&a.priority.value()));
    Ok(ready)
}

/// Get tasks that can run in parallel (no dependencies between them)
fn get_parallel_tasks(conn: &Connection) -> Result<Vec<Task>> {
    let ready = get_ready_tasks(conn)?;

    // All ready tasks can run in parallel by definition
    // (their deps are all satisfied and they don't depend on each other)
    Ok(ready)
}

/// Get the critical path (longest dependency chain)
fn get_critical_path(conn: &Connection) -> Result<Vec<Task>> {
    let ai_id = get_ai_id();
    let all_tasks = list_tasks(conn, true, false, 1000)?;

    if all_tasks.is_empty() {
        return Ok(vec![]);
    }

    // Build adjacency map
    let task_map: HashMap<i64, &Task> = all_tasks.iter().map(|t| (t.id, t)).collect();

    // Find longest path using DFS with memoization
    let mut memo: HashMap<i64, Vec<i64>> = HashMap::new();

    fn longest_chain(
        task_id: i64,
        task_map: &HashMap<i64, &Task>,
        memo: &mut HashMap<i64, Vec<i64>>,
        visited: &mut HashSet<i64>,
    ) -> Vec<i64> {
        if let Some(cached) = memo.get(&task_id) {
            return cached.clone();
        }

        if visited.contains(&task_id) {
            return vec![]; // Cycle protection
        }
        visited.insert(task_id);

        let task = match task_map.get(&task_id) {
            Some(t) => t,
            None => return vec![task_id],
        };

        if task.depends_on.is_empty() {
            let result = vec![task_id];
            memo.insert(task_id, result.clone());
            return result;
        }

        let mut longest = vec![];
        for dep_id in &task.depends_on {
            let chain = longest_chain(*dep_id, task_map, memo, visited);
            if chain.len() > longest.len() {
                longest = chain;
            }
        }

        longest.push(task_id);
        memo.insert(task_id, longest.clone());
        visited.remove(&task_id);
        longest
    }

    // Find longest chain starting from any task
    let mut max_chain = vec![];
    for task in &all_tasks {
        let mut visited = HashSet::new();
        let chain = longest_chain(task.id, &task_map, &mut memo, &mut visited);
        if chain.len() > max_chain.len() {
            max_chain = chain;
        }
    }

    // Convert IDs to tasks
    let result: Vec<Task> = max_chain
        .iter()
        .filter_map(|id| task_map.get(id).map(|t| (*t).clone()))
        .collect();

    Ok(result)
}

impl Clone for Task {
    fn clone(&self) -> Self {
        Task {
            id: self.id,
            content: self.content.clone(),
            status: self.status,
            priority: self.priority,
            created: self.created,
            updated: self.updated,
            depends_on: self.depends_on.clone(),
            blocked_reason: self.blocked_reason.clone(),
            validation_reason: self.validation_reason.clone(),
            output: self.output.clone(),
        }
    }
}


// ============================================================================
// VISUALIZATION
// ============================================================================

fn visualize_dag(conn: &Connection) -> Result<String> {
    let ai_id = get_ai_id();
    let all_tasks = list_tasks(conn, true, false, 1000)?;

    let mut lines = Vec::new();
    lines.push("=".repeat(80));
    lines.push("TASK DAG VISUALIZATION".to_string());
    lines.push("=".repeat(80));
    lines.push(String::new());

    // Group by status
    let mut by_status: HashMap<TaskStatus, Vec<&Task>> = HashMap::new();
    for task in &all_tasks {
        by_status.entry(task.status).or_insert_with(Vec::new).push(task);
    }

    let status_order = [
        TaskStatus::InProgress,
        TaskStatus::Ready,
        TaskStatus::Executed,
        TaskStatus::Pending,
        TaskStatus::Blocked,
        TaskStatus::ValidationFailed,
        TaskStatus::Validated,
        TaskStatus::Failed,
    ];

    for status in &status_order {
        if let Some(tasks) = by_status.get(status) {
            if !tasks.is_empty() {
                lines.push(format!("[{}]", status.to_string().to_uppercase()));
                let mut sorted_tasks = tasks.clone();
                sorted_tasks.sort_by(|a, b| b.priority.value().cmp(&a.priority.value()));
                for task in sorted_tasks {
                    let deps = if task.depends_on.is_empty() {
                        String::new()
                    } else {
                        format!(" <- {:?}", task.depends_on)
                    };
                    lines.push(format!("  {} #{} {}{}", status.symbol(), task.id, truncate(&task.content, 40), deps));
                }
                lines.push(String::new());
            }
        }
    }

    // Show critical path
    let critical = get_critical_path(conn)?;
    if !critical.is_empty() {
        lines.push("CRITICAL PATH (longest dependency chain):".to_string());
        for (i, task) in critical.iter().enumerate() {
            let prefix = if i > 0 { " -> " } else { "   " };
            lines.push(format!("{} #{} {}", prefix, task.id, truncate(&task.content, 40)));
        }
        lines.push(String::new());
    }

    // Show parallel opportunities
    let parallel = get_parallel_tasks(conn)?;
    if !parallel.is_empty() {
        lines.push(format!("CAN RUN IN PARALLEL ({} tasks):", parallel.len()));
        for task in &parallel {
            lines.push(format!("  * #{} {} (priority {})", task.id, truncate(&task.content, 40), task.priority));
        }
        lines.push(String::new());
    }

    // Show blocked tasks
    let blocked: Vec<_> = all_tasks.iter().filter(|t| t.status == TaskStatus::Blocked).collect();
    if !blocked.is_empty() {
        lines.push("BLOCKED TASKS:".to_string());
        for task in blocked {
            let reason = task.blocked_reason.as_ref().map(|r| r.as_str()).unwrap_or("unknown");
            lines.push(format!("  ! #{} {} - {}", task.id, truncate(&task.content, 30), reason));
        }
        lines.push(String::new());
    }

    // Show validation failures
    let failed_val: Vec<_> = all_tasks.iter().filter(|t| t.status == TaskStatus::ValidationFailed).collect();
    if !failed_val.is_empty() {
        lines.push("VALIDATION FAILURES (need revision):".to_string());
        for task in failed_val {
            let reason = task.validation_reason.as_ref().map(|r| r.as_str()).unwrap_or("unknown");
            lines.push(format!("  X #{} {} - {}", task.id, truncate(&task.content, 30), reason));
        }
        lines.push(String::new());
    }

    lines.push("=".repeat(80));

    Ok(lines.join("
"))
}

fn delete_task(conn: &Connection, id: i64) -> Result<bool> {
    let ai_id = get_ai_id();

    // Remove edges
    conn.execute(
        "DELETE FROM task_edges WHERE (from_id = ?1 OR to_id = ?1) AND ai_id = ?2",
        params![id, ai_id],
    )?;

    // Remove task
    let affected = conn.execute(
        "DELETE FROM tasks WHERE id = ?1 AND ai_id = ?2",
        params![id, ai_id],
    )?;

    Ok(affected > 0)
}

fn get_stats(conn: &Connection) -> Result<HashMap<String, i64>> {
    let ai_id = get_ai_id();
    let mut stats = HashMap::new();

    let statuses = [
        "pending", "ready", "in_progress", "executed", "validated", "validation_failed", "blocked", "failed"
    ];

    for status in &statuses {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE ai_id = ?1 AND status = ?2",
            params![ai_id, status],
            |row| row.get(0),
        )?;
        stats.insert(status.to_string(), count);
    }

    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE ai_id = ?1",
        params![ai_id],
        |row| row.get(0),
    )?;
    stats.insert("total".to_string(), total);

    let edges: i64 = conn.query_row(
        "SELECT COUNT(*) FROM task_edges WHERE ai_id = ?1",
        params![ai_id],
        |row| row.get(0),
    )?;
    stats.insert("dependencies".to_string(), edges);

    Ok(stats)
}

// ============================================================================
// MAIN
// ============================================================================

fn main() -> Result<()> {
    let cli = Cli::parse();
    let conn = open_db()?;

    match cli.command {
        Commands::Add { content, priority, depends_on } => {
            let id = add_task(&conn, &content, &priority, depends_on.as_deref())?;
            let priority_enum = TaskPriority::from_str(&priority);
            println!("Task added: #{}", id);
            if priority_enum != TaskPriority::Normal {
                println!("Priority: {}", priority_enum);
            }
            if let Some(deps) = depends_on {
                println!("Depends on: {}", deps);
            }
        }

        Commands::List { all, completed, limit } => {
            let tasks = list_tasks(&conn, all, completed, limit)?;

            if tasks.is_empty() {
                println!("No tasks");
            } else {
                // Group by status
                let in_progress: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::InProgress).collect();
                let ready: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::Ready).collect();
                let blocked: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::Blocked).collect();
                let pending: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::Pending).collect();
                let val_failed: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::ValidationFailed).collect();
                let executed: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::Executed).collect();
                let validated: Vec<_> = tasks.iter().filter(|t| t.status == TaskStatus::Validated).collect();

                let active_count = in_progress.len() + ready.len() + blocked.len() + pending.len() + val_failed.len();

                if active_count > 0 {
                    println!("=== ACTIVE TASKS ({}) ===", active_count);
                    for task in &in_progress { print_task_line(task); }
                    for task in &ready { print_task_line(task); }
                    for task in &blocked { print_task_line(task); }
                    for task in &val_failed { print_task_line(task); }
                    for task in &pending { print_task_line(task); }
                }

                if (!executed.is_empty() || !validated.is_empty()) && (all || completed) {
                    if active_count > 0 { println!(); }
                    println!("=== COMPLETED ({}) ===", executed.len() + validated.len());
                    for task in &executed { print_task_line(task); }
                    for task in &validated { print_task_line(task); }
                }
            }
        }

        Commands::Start { id } => {
            if update_status(&conn, id, TaskStatus::InProgress)? {
                println!("Task #{} started", id);
            } else {
                println!("Task #{} not found", id);
            }
        }

        Commands::Execute { id, output } => {
            if execute_task(&conn, id, output.as_deref())? {
                println!("Task #{} executed (awaiting validation)", id);
            } else {
                println!("Task #{} not found", id);
            }
        }

        Commands::Validate { id, pass, fail, reason } => {
            if pass == fail {
                println!("Error: specify either --pass or --fail");
                return Ok(());
            }

            let messages = validate_task(&conn, id, pass, reason.as_deref())?;
            for msg in messages {
                println!("{}", msg);
            }
        }

        Commands::Block { id, reason } => {
            if block_task(&conn, id, &reason)? {
                println!("Task #{} blocked: {}", id, reason);
            } else {
                println!("Task #{} not found", id);
            }
        }

        Commands::Unblock { id } => {
            if update_status(&conn, id, TaskStatus::Pending)? {
                println!("Task #{} unblocked", id);
            } else {
                println!("Task #{} not found", id);
            }
        }

        Commands::Depend { id, on } => {
            add_dependency(&conn, id, on)?;
            println!("Task #{} now depends on #{}", id, on);
        }

        Commands::Undepend { id, from } => {
            remove_dependency(&conn, id, from)?;
            println!("Removed dependency: #{} no longer depends on #{}", id, from);
        }

        Commands::Ready => {
            let ready = get_ready_tasks(&conn)?;
            if ready.is_empty() {
                println!("No tasks ready to execute");
            } else {
                println!("=== READY TO EXECUTE ({}) ===", ready.len());
                for task in &ready {
                    print_task_line(task);
                }
            }
        }

        Commands::Parallel => {
            let parallel = get_parallel_tasks(&conn)?;
            if parallel.is_empty() {
                println!("No tasks can run in parallel");
            } else {
                println!("=== CAN RUN IN PARALLEL ({}) ===", parallel.len());
                for task in &parallel {
                    print_task_line(task);
                }
            }
        }

        Commands::CriticalPath => {
            let critical = get_critical_path(&conn)?;
            if critical.is_empty() {
                println!("No critical path (no dependencies)");
            } else {
                println!("=== CRITICAL PATH ({} tasks) ===", critical.len());
                for (i, task) in critical.iter().enumerate() {
                    let prefix = if i > 0 { " -> " } else { "    " };
                    println!("{}{} #{} {}", prefix, task.status.symbol(), task.id, truncate(&task.content, 50));
                }
            }
        }

        Commands::Dag => {
            let viz = visualize_dag(&conn)?;
            println!("{}", viz);
        }

        Commands::Delete { id } => {
            if delete_task(&conn, id)? {
                println!("Task #{} deleted", id);
            } else {
                println!("Task #{} not found", id);
            }
        }

        Commands::Get { id } => {
            match load_task(&conn, id)? {
                Some(task) => {
                    println!("Task #{}", task.id);
                    println!("Status: {}", task.status);
                    println!("Priority: {}", task.priority);
                    println!("Created: {}", format_relative_time(&task.created));
                    println!("Updated: {}", format_relative_time(&task.updated));
                    if !task.depends_on.is_empty() {
                        println!("Depends on: {:?}", task.depends_on);
                    }
                    if let Some(reason) = &task.blocked_reason {
                        println!("Blocked: {}", reason);
                    }
                    if let Some(reason) = &task.validation_reason {
                        println!("Validation: {}", reason);
                    }
                    if let Some(output) = &task.output {
                        println!("Output: {}", output);
                    }
                    println!();
                    println!("{}", task.content);
                }
                None => println!("Task #{} not found", id),
            }
        }

        Commands::Stats => {
            let stats = get_stats(&conn)?;
            let ai_id = get_ai_id();
            let total = stats.get("total").unwrap_or(&0);
            let deps = stats.get("dependencies").unwrap_or(&0);

            println!("Task DAG Statistics (AI: {}):", ai_id);
            println!("  Total tasks: {}", total);
            println!("  Dependencies: {}", deps);
            println!();
            println!("  Active:");
            println!("    [ ] Pending: {}", stats.get("pending").unwrap_or(&0));
            println!("    [R] Ready: {}", stats.get("ready").unwrap_or(&0));
            println!("    [>] In progress: {}", stats.get("in_progress").unwrap_or(&0));
            println!("    [!] Blocked: {}", stats.get("blocked").unwrap_or(&0));
            println!("    [X] Validation failed: {}", stats.get("validation_failed").unwrap_or(&0));
            println!();
            println!("  Done:");
            println!("    [*] Executed: {}", stats.get("executed").unwrap_or(&0));
            println!("    [V] Validated: {}", stats.get("validated").unwrap_or(&0));
        }
    }

    Ok(())
}
