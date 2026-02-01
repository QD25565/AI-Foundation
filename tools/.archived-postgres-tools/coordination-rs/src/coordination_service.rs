//! Coordination Service - Distributed locks, task queues, atomic operations
//!
//! Handles all teambook coordination primitives: locks, task queues, projects,
//! file claims, and evolution DAGs.
//!
//! Rust implementation of Python's coordination_service.py
//! Performance target: 5-10x faster than Python, <100μs for atomic operations

use chrono::{DateTime, Duration, Utc};
use deadpool_postgres::{Client, Pool};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

// ============= CONSTANTS =============

pub const MAX_LOCK_DURATION_SECONDS: i32 = 300; // 5 minutes max
pub const DEFAULT_LOCK_TIMEOUT: i32 = 30; // 30 seconds default
pub const MAX_LOCKS_PER_AI: usize = 10; // Prevent resource hoarding
pub const MAX_QUEUE_SIZE: usize = 1000; // Prevent memory exhaustion
pub const MAX_TASK_LENGTH: usize = 2000;
pub const MAX_RESOURCE_ID_LENGTH: usize = 100;

// ============= ERROR TYPES =============

/// Coordination errors
#[derive(Debug, Error)]
pub enum CoordinationError {
    #[error("Invalid resource ID: {0}")]
    InvalidResourceId(String),

    #[error("Lock limit reached (max {MAX_LOCKS_PER_AI})")]
    LockLimitReached,

    #[error("Resource locked by {0}")]
    ResourceLocked(String),

    #[error("Lock not found: {0}")]
    LockNotFound(String),

    #[error("Not authorized to {operation} {resource}")]
    NotAuthorized {
        operation: String,
        resource: String,
    },

    #[error("Task not found: {0}")]
    TaskNotFound(i32),

    #[error("Task not available (already claimed or completed)")]
    TaskNotAvailable,

    #[error("Empty task description")]
    EmptyTask,

    #[error("Queue full (max {MAX_QUEUE_SIZE}, current: {current})")]
    QueueFull { current: usize },

    #[error("Database error: {0}")]
    DatabaseError(#[from] tokio_postgres::Error),

    #[error("Pool error: {0}")]
    PoolError(#[from] deadpool_postgres::PoolError),

    #[error("Verification task creation failed: {0}")]
    VerificationError(String),

    #[error("Test execution failed: {0}")]
    TestExecutionError(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}

pub type Result<T> = std::result::Result<T, CoordinationError>;

// ============= DATA TYPES =============

/// Lock information (cached)
#[derive(Debug, Clone)]
#[allow(dead_code)] // Used for caching, fields accessed indirectly
struct LockInfo {
    held_by: String,
    expires_at: DateTime<Utc>,
}

/// Task status enum with exhaustive matching
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Claimed,
    InProgress,
    Completed,
    Blocked,
    Cancelled,
}

impl TaskStatus {
    /// Convert to database string
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Claimed => "claimed",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Completed => "completed",
            TaskStatus::Blocked => "blocked",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    /// Parse from database string
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(TaskStatus::Pending),
            "claimed" => Some(TaskStatus::Claimed),
            "in_progress" => Some(TaskStatus::InProgress),
            "completed" => Some(TaskStatus::Completed),
            "blocked" => Some(TaskStatus::Blocked),
            "cancelled" => Some(TaskStatus::Cancelled),
            _ => None,
        }
    }
}

/// Task row from database
#[derive(Debug, Clone)]
pub struct TaskRow {
    pub id: i32,
    pub task: String,
    pub priority: i32,
    pub status: String,
    pub claimed_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub result: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Lock row from database
#[derive(Debug, Clone)]
pub struct LockRow {
    pub resource_id: String,
    pub held_by: String,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Task claim result
#[derive(Debug, Clone)]
pub struct ClaimedTask {
    pub task_id: i32,
    pub task: String,
    pub priority: i32,
    pub created_at: DateTime<Utc>,
    pub metadata: Option<serde_json::Value>,
}

/// Task statistics
#[derive(Debug, Clone)]
pub struct QueueStats {
    pub total: i64,
    pub pending: i64,
    pub claimed: i64,
    pub completed: i64,
    pub my_tasks: i64,
}

// ============= MAIN SERVICE =============

/// Coordination Service - Distributed locks, task queues, atomic operations
///
/// Thread-safe, async, PostgreSQL-backed coordination primitives.
/// Provides atomic operations for multi-AI teamwork.
pub struct CoordinationService {
    /// PostgreSQL connection pool
    pool: Pool,

    /// Teambook name
    teambook_name: String,

    /// Current AI ID
    ai_id: String,

    /// In-memory lock cache for performance
    /// Arc<RwLock<>> allows concurrent reads, exclusive writes
    lock_cache: Arc<RwLock<HashMap<String, LockInfo>>>,

    /// Per-AI lock count (for limit enforcement)
    ai_lock_count: Arc<RwLock<HashMap<String, usize>>>,

    /// Resource ID validation regex (compiled once)
    resource_id_regex: Regex,
}

impl CoordinationService {
    /// Create new coordination service
    ///
    /// # Arguments
    /// * `pool` - PostgreSQL connection pool
    /// * `teambook_name` - Teambook identifier
    /// * `ai_id` - Current AI identifier
    pub fn new(pool: Pool, teambook_name: String, ai_id: String) -> Self {
        // Compile regex once at construction
        let resource_id_regex = Regex::new(r"^[A-Za-z0-9_:\-\./]+$")
            .expect("Resource ID regex is valid");

        CoordinationService {
            pool,
            teambook_name,
            ai_id,
            lock_cache: Arc::new(RwLock::new(HashMap::new())),
            ai_lock_count: Arc::new(RwLock::new(HashMap::new())),
            resource_id_regex,
        }
    }

    // ============= INTERNAL HELPERS =============

    /// Get connection from pool
    async fn get_conn(&self) -> Result<Client> {
        Ok(self.pool.get().await?)
    }

    /// Sanitize resource identifier - SECURITY CRITICAL
    ///
    /// Returns None if invalid, sanitized string if valid.
    fn sanitize_resource_id(&self, resource_id: &str) -> Option<String> {
        if resource_id.is_empty() {
            return None;
        }

        let trimmed = resource_id.trim();

        // Length check
        if trimmed.len() > MAX_RESOURCE_ID_LENGTH {
            return None;
        }

        // Character whitelist
        if !self.resource_id_regex.is_match(trimmed) {
            return None;
        }

        Some(trimmed.to_string())
    }

    /// Validate and clamp timeout value
    fn validate_timeout(&self, timeout: i32) -> i32 {
        if timeout < 1 {
            return DEFAULT_LOCK_TIMEOUT;
        }
        if timeout > MAX_LOCK_DURATION_SECONDS {
            return MAX_LOCK_DURATION_SECONDS;
        }
        timeout
    }

    /// Validate and clamp priority value (0-9, higher = more urgent)
    fn validate_priority(&self, priority: i32) -> i32 {
        priority.clamp(0, 9)
    }

    /// Clean up expired locks
    async fn cleanup_expired_locks(&self, conn: &Client) -> Result<usize> {
        let now = Utc::now();

        // Find expired locks
        let rows = conn
            .query(
                "SELECT resource_id, held_by FROM locks WHERE expires_at < $1",
                &[&now],
            )
            .await?;

        if rows.is_empty() {
            return Ok(0);
        }

        // Collect resource IDs
        let resource_ids: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
        let holders: Vec<String> = rows.iter().map(|row| row.get(1)).collect();

        // Delete expired locks
        let placeholders: Vec<String> = (1..=resource_ids.len())
            .map(|i| format!("${}", i))
            .collect();
        let query = format!(
            "DELETE FROM locks WHERE resource_id IN ({})",
            placeholders.join(",")
        );

        let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = resource_ids
            .iter()
            .map(|r| r as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        conn.execute(&query, &params).await?;

        // Update in-memory cache
        let mut cache = self.lock_cache.write().await;
        let mut counts = self.ai_lock_count.write().await;

        for (resource_id, holder) in resource_ids.iter().zip(holders.iter()) {
            cache.remove(resource_id);
            if let Some(count) = counts.get_mut(holder) {
                *count = count.saturating_sub(1);
            }
        }

        Ok(resource_ids.len())
    }

    /// Initialize coordination tables (idempotent)
    async fn init_coordination_tables(&self, conn: &Client) -> Result<()> {
        // Create locks table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS locks (
                resource_id VARCHAR(100) PRIMARY KEY,
                held_by VARCHAR(100) NOT NULL,
                acquired_at TIMESTAMPTZ NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL,
                teambook_name VARCHAR(50)
            )",
            &[],
        )
        .await?;

        // Create indexes
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_locks_expires ON locks(expires_at)",
            &[],
        )
        .await?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_locks_holder ON locks(held_by)",
            &[],
        )
        .await?;

        // Create task_queue table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS task_queue (
                id SERIAL PRIMARY KEY,
                task TEXT NOT NULL,
                priority INTEGER NOT NULL DEFAULT 5,
                status VARCHAR(20) NOT NULL DEFAULT 'pending',
                claimed_by VARCHAR(100),
                created_at TIMESTAMPTZ NOT NULL,
                claimed_at TIMESTAMPTZ,
                completed_at TIMESTAMPTZ,
                result TEXT,
                teambook_name VARCHAR(50),
                metadata TEXT
            )",
            &[],
        )
        .await?;

        // Create indexes
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_queue_status_priority
             ON task_queue(status, priority DESC, created_at)",
            &[],
        )
        .await?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_queue_claimed
             ON task_queue(claimed_by, status)",
            &[],
        )
        .await?;

        // Note: projects and coordination_tasks tables will be created by project service

        Ok(())
    }

    // ============= PUBLIC API - DISTRIBUTED LOCKS =============

    /// Acquire distributed lock on a resource
    ///
    /// Security:
    /// - Automatic expiration prevents deadlock
    /// - Per-AI limits prevent hoarding
    /// - Atomic check-and-set prevents races
    ///
    /// # Arguments
    /// * `resource_id` - Unique resource identifier
    /// * `timeout` - Lock duration in seconds (default: 30, max: 300)
    ///
    /// # Returns
    /// * `Ok(expires_at)` - Lock acquired, expires at timestamp
    /// * `Err(ResourceLocked(holder))` - Lock held by another AI
    /// * `Err(InvalidResourceId)` - Invalid resource ID
    /// * `Err(LockLimitReached)` - AI has too many locks
    pub async fn acquire_lock(
        &self,
        resource_id: &str,
        timeout: i32,
    ) -> Result<DateTime<Utc>> {
        // Validate and sanitize
        let resource_id = self
            .sanitize_resource_id(resource_id)
            .ok_or_else(|| CoordinationError::InvalidResourceId(resource_id.to_string()))?;

        let timeout = self.validate_timeout(timeout);

        // Check per-AI lock limit
        {
            let counts = self.ai_lock_count.read().await;
            if counts.get(&self.ai_id).copied().unwrap_or(0) >= MAX_LOCKS_PER_AI {
                return Err(CoordinationError::LockLimitReached);
            }
        }

        let now = Utc::now();
        let expires_at = now + Duration::seconds(timeout as i64);

        let conn = self.get_conn().await?;
        self.init_coordination_tables(&conn).await?;

        // Atomic check-and-acquire using ON CONFLICT
        conn.execute(
            "INSERT INTO locks (resource_id, held_by, acquired_at, expires_at, teambook_name)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT(resource_id) DO UPDATE SET
                 held_by = excluded.held_by,
                 acquired_at = excluded.acquired_at,
                 expires_at = excluded.expires_at
             WHERE locks.expires_at < $3",
            &[&resource_id, &self.ai_id, &now, &expires_at, &self.teambook_name],
        )
        .await?;

        // Verify we got the lock
        let row = conn
            .query_one(
                "SELECT held_by, expires_at FROM locks WHERE resource_id = $1",
                &[&resource_id],
            )
            .await?;

        let holder: String = row.get(0);
        let actual_expires: DateTime<Utc> = row.get(1);

        if holder != self.ai_id {
            return Err(CoordinationError::ResourceLocked(holder));
        }

        // Update cache
        {
            let mut cache = self.lock_cache.write().await;
            cache.insert(
                resource_id.clone(),
                LockInfo {
                    held_by: self.ai_id.clone(),
                    expires_at: actual_expires,
                },
            );

            let mut counts = self.ai_lock_count.write().await;
            *counts.entry(self.ai_id.clone()).or_insert(0) += 1;
        }

        Ok(actual_expires)
    }

    /// Release a held lock
    ///
    /// Security: Only holder can release their own lock.
    ///
    /// # Arguments
    /// * `resource_id` - Resource to unlock
    ///
    /// # Returns
    /// * `Ok(())` - Lock released
    /// * `Err(LockNotFound)` - No lock on this resource
    /// * `Err(NotAuthorized)` - Lock held by different AI
    pub async fn release_lock(&self, resource_id: &str) -> Result<()> {
        let resource_id = self
            .sanitize_resource_id(resource_id)
            .ok_or_else(|| CoordinationError::InvalidResourceId(resource_id.to_string()))?;

        let conn = self.get_conn().await?;
        self.init_coordination_tables(&conn).await?;

        // Verify ownership
        let row = conn
            .query_opt(
                "SELECT held_by FROM locks WHERE resource_id = $1",
                &[&resource_id],
            )
            .await?;

        match row {
            None => {
                return Err(CoordinationError::LockNotFound(resource_id));
            }
            Some(row) => {
                let holder: String = row.get(0);
                if holder != self.ai_id {
                    return Err(CoordinationError::NotAuthorized {
                        operation: "release".to_string(),
                        resource: resource_id,
                    });
                }
            }
        }

        // Release lock
        conn.execute(
            "DELETE FROM locks WHERE resource_id = $1",
            &[&resource_id],
        )
        .await?;

        // Update cache
        {
            let mut cache = self.lock_cache.write().await;
            cache.remove(&resource_id);

            let mut counts = self.ai_lock_count.write().await;
            if let Some(count) = counts.get_mut(&self.ai_id) {
                *count = count.saturating_sub(1);
            }
        }

        Ok(())
    }

    /// Extend lock expiration time
    ///
    /// Security: Only holder can extend, limited duration.
    ///
    /// # Arguments
    /// * `resource_id` - Resource to extend
    /// * `additional_seconds` - Seconds to add (max: 300 total from now)
    ///
    /// # Returns
    /// * `Ok(new_expires_at)` - New expiration timestamp
    /// * `Err(LockNotFound)` - No lock on this resource
    /// * `Err(NotAuthorized)` - Lock held by different AI
    pub async fn extend_lock(
        &self,
        resource_id: &str,
        additional_seconds: i32,
    ) -> Result<DateTime<Utc>> {
        let resource_id = self
            .sanitize_resource_id(resource_id)
            .ok_or_else(|| CoordinationError::InvalidResourceId(resource_id.to_string()))?;

        let additional = self.validate_timeout(additional_seconds);

        let conn = self.get_conn().await?;
        self.init_coordination_tables(&conn).await?;

        // Verify ownership
        let row = conn
            .query_opt(
                "SELECT held_by, expires_at FROM locks WHERE resource_id = $1",
                &[&resource_id],
            )
            .await?;

        let (holder, current_expires) = match row {
            None => return Err(CoordinationError::LockNotFound(resource_id)),
            Some(row) => {
                let holder: String = row.get(0);
                let expires: DateTime<Utc> = row.get(1);
                if holder != self.ai_id {
                    return Err(CoordinationError::NotAuthorized {
                        operation: "extend".to_string(),
                        resource: resource_id,
                    });
                }
                (holder, expires)
            }
        };

        // Calculate new expiration (max 5 minutes from now)
        let now = Utc::now();
        let new_expires = std::cmp::min(
            current_expires + Duration::seconds(additional as i64),
            now + Duration::seconds(MAX_LOCK_DURATION_SECONDS as i64),
        );

        // Update expiration
        conn.execute(
            "UPDATE locks SET expires_at = $1 WHERE resource_id = $2",
            &[&new_expires, &resource_id],
        )
        .await?;

        // Update cache
        {
            let mut cache = self.lock_cache.write().await;
            cache.insert(
                resource_id.clone(),
                LockInfo {
                    held_by: holder,
                    expires_at: new_expires,
                },
            );
        }

        Ok(new_expires)
    }

    /// List active locks
    ///
    /// # Arguments
    /// * `show_all` - If true, show all locks; if false, only current AI's locks
    ///
    /// # Returns
    /// Vector of (resource_id, held_by, expires_at) tuples
    pub async fn list_locks(&self, show_all: bool) -> Result<Vec<LockRow>> {
        let conn = self.get_conn().await?;
        self.init_coordination_tables(&conn).await?;

        // Clean up expired locks first
        self.cleanup_expired_locks(&conn).await?;

        let now = Utc::now();

        let rows = if show_all {
            conn.query(
                "SELECT resource_id, held_by, acquired_at, expires_at
                 FROM locks
                 WHERE expires_at > $1
                 ORDER BY expires_at",
                &[&now],
            )
            .await?
        } else {
            conn.query(
                "SELECT resource_id, held_by, acquired_at, expires_at
                 FROM locks
                 WHERE held_by = $1 AND expires_at > $2
                 ORDER BY expires_at",
                &[&self.ai_id, &now],
            )
            .await?
        };

        let locks: Vec<LockRow> = rows
            .iter()
            .map(|row| LockRow {
                resource_id: row.get(0),
                held_by: row.get(1),
                acquired_at: row.get(2),
                expires_at: row.get(3),
            })
            .collect();

        Ok(locks)
    }

    // ============= PUBLIC API - TASK QUEUE =============

    /// Add task to distributed queue
    ///
    /// # Arguments
    /// * `task` - Task description (max 2000 chars)
    /// * `priority` - Priority 0-9 (higher = more urgent)
    /// * `metadata` - Optional JSON metadata
    ///
    /// # Returns
    /// * `Ok(task_id)` - Task queued with ID
    /// * `Err(EmptyTask)` - Task description empty
    /// * `Err(QueueFull)` - Queue at capacity
    pub async fn queue_task(
        &self,
        task: &str,
        priority: i32,
        metadata: Option<serde_json::Value>,
    ) -> Result<i32> {
        let task = task.trim();
        if task.is_empty() {
            return Err(CoordinationError::EmptyTask);
        }

        // Truncate if too long
        let task = if task.len() > MAX_TASK_LENGTH {
            &task[..MAX_TASK_LENGTH]
        } else {
            task
        };

        let priority = self.validate_priority(priority);
        let metadata_str = metadata.map(|m| m.to_string());

        let conn = self.get_conn().await?;
        self.init_coordination_tables(&conn).await?;

        // Check queue size
        let row = conn
            .query_one("SELECT COUNT(*) FROM task_queue WHERE status = 'pending'", &[])
            .await?;
        let count: i64 = row.get(0);

        if count >= MAX_QUEUE_SIZE as i64 {
            return Err(CoordinationError::QueueFull {
                current: count as usize,
            });
        }

        // Insert task
        let now = Utc::now();
        let row = conn
            .query_one(
                "INSERT INTO task_queue (task, priority, status, created_at, teambook_name, metadata)
                 VALUES ($1, $2, 'pending', $3, $4, $5)
                 RETURNING id",
                &[&task, &priority, &now, &self.teambook_name, &metadata_str],
            )
            .await?;

        let task_id: i32 = row.get(0);
        Ok(task_id)
    }

    /// Claim next available task from queue (atomic)
    ///
    /// Priority logic:
    /// 1. Verification tasks (priority 8-9) first
    /// 2. Normal tasks by priority DESC, then FIFO
    ///
    /// # Arguments
    /// * `prefer_priority` - If true, gets highest priority task (default behavior)
    ///
    /// # Returns
    /// * `Ok(Some(claimed_task))` - Task claimed
    /// * `Ok(None)` - Queue empty
    /// * `Err(TaskNotAvailable)` - Race condition (try again)
    pub async fn claim_task(&self, prefer_priority: bool) -> Result<Option<ClaimedTask>> {
        let conn = self.get_conn().await?;
        self.init_coordination_tables(&conn).await?;

        // Try to get a task (with priority logic)
        let task_row = if prefer_priority {
            // First try verification tasks
            let verification_row = conn
                .query_opt(
                    "SELECT id, task, priority, created_at, metadata
                     FROM task_queue
                     WHERE status = 'pending'
                       AND metadata LIKE '%verification%'
                     ORDER BY priority DESC, created_at ASC
                     LIMIT 1",
                    &[],
                )
                .await?;

            if verification_row.is_some() {
                verification_row
            } else {
                // No verification tasks, get normal tasks
                conn.query_opt(
                    "SELECT id, task, priority, created_at, metadata
                     FROM task_queue
                     WHERE status = 'pending'
                     ORDER BY priority DESC, created_at ASC
                     LIMIT 1",
                    &[],
                )
                .await?
            }
        } else {
            // FIFO order
            conn.query_opt(
                "SELECT id, task, priority, created_at, metadata
                 FROM task_queue
                 WHERE status = 'pending'
                 ORDER BY created_at ASC
                 LIMIT 1",
                &[],
            )
            .await?
        };

        if task_row.is_none() {
            return Ok(None); // Empty queue
        }

        let task_row = task_row.unwrap();
        let task_id: i32 = task_row.get(0);
        let task_desc: String = task_row.get(1);
        let priority: i32 = task_row.get(2);
        let created_at: DateTime<Utc> = task_row.get(3);
        let metadata_str: Option<String> = task_row.get(4);

        let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());

        // Claim it atomically
        let now = Utc::now();
        let updated = conn
            .execute(
                "UPDATE task_queue
                 SET status = 'claimed', claimed_by = $1, claimed_at = $2
                 WHERE id = $3 AND status = 'pending'",
                &[&self.ai_id, &now, &task_id],
            )
            .await?;

        if updated == 0 {
            // Race condition - someone else claimed it
            return Err(CoordinationError::TaskNotAvailable);
        }

        // Verify we got it (double-check for race conditions)
        let verify_row = conn
            .query_one(
                "SELECT claimed_by FROM task_queue WHERE id = $1",
                &[&task_id],
            )
            .await?;

        let claimer: String = verify_row.get(0);
        if claimer != self.ai_id {
            return Err(CoordinationError::TaskNotAvailable);
        }

        Ok(Some(ClaimedTask {
            task_id,
            task: task_desc,
            priority,
            created_at,
            metadata,
        }))
    }

    /// Mark task as completed
    ///
    /// Security: Only claimer can complete their task.
    /// Automatically creates verification task.
    ///
    /// # Arguments
    /// * `task_id` - Task ID to complete
    /// * `result` - Optional result description
    ///
    /// # Returns
    /// * `Ok(verification_task_id)` - Task completed, verification created
    /// * `Err(TaskNotFound)` - Task doesn't exist
    /// * `Err(NotAuthorized)` - Task claimed by different AI
    pub async fn complete_task(
        &self,
        task_id: i32,
        result: Option<&str>,
    ) -> Result<Option<i32>> {
        let conn = self.get_conn().await?;
        self.init_coordination_tables(&conn).await?;

        // Verify ownership
        let verify_row = conn
            .query_opt(
                "SELECT claimed_by, status, task FROM task_queue WHERE id = $1",
                &[&task_id],
            )
            .await?;

        let (_claimer, _status, task_desc) = match verify_row {
            None => return Err(CoordinationError::TaskNotFound(task_id)),
            Some(row) => {
                let claimer: Option<String> = row.get(0);
                let status: String = row.get(1);
                let task: String = row.get(2);

                if status == "completed" {
                    // Already completed - not an error
                    return Ok(None);
                }

                match claimer {
                    None => {
                        return Err(CoordinationError::NotAuthorized {
                            operation: "complete".to_string(),
                            resource: format!("task:{}", task_id),
                        })
                    }
                    Some(c) if c != self.ai_id => {
                        return Err(CoordinationError::NotAuthorized {
                            operation: "complete".to_string(),
                            resource: format!("task:{}", task_id),
                        })
                    }
                    Some(c) => (c, status, task),
                }
            }
        };

        // Complete the task
        let result_str = result.map(|r| {
            if r.len() > MAX_TASK_LENGTH {
                &r[..MAX_TASK_LENGTH]
            } else {
                r
            }
        });

        let now = Utc::now();
        conn.execute(
            "UPDATE task_queue
             SET status = 'completed', completed_at = $1, result = $2
             WHERE id = $3",
            &[&now, &result_str, &task_id],
        )
        .await?;

        // Auto-create verification task
        let verify_task_id = self.create_verification_task(&task_desc, task_id).await?;

        Ok(verify_task_id)
    }

    /// Create verification task for completed task
    async fn create_verification_task(
        &self,
        original_task_desc: &str,
        original_task_id: i32,
    ) -> Result<Option<i32>> {
        // Truncate task description for verification
        let verify_desc = if original_task_desc.len() > 80 {
            format!("Verify: {}...", &original_task_desc[..80])
        } else {
            format!("Verify: {}", original_task_desc)
        };

        // Create metadata
        let metadata = serde_json::json!({
            "task_type": "verification",
            "original_task_id": original_task_id,
            "completed_by": self.ai_id,
            "verification_type": "test",
            "created_at": Utc::now().to_rfc3339()
        });

        // Queue verification task with high priority (8)
        match self.queue_task(&verify_desc, 8, Some(metadata)).await {
            Ok(verify_id) => Ok(Some(verify_id)),
            Err(e) => {
                // Log error but don't fail the completion
                eprintln!("Failed to create verification task: {}", e);
                Ok(None)
            }
        }
    }

    /// Get task queue statistics
    ///
    /// # Returns
    /// Statistics including total, pending, claimed, completed, and my tasks
    pub async fn queue_stats(&self) -> Result<QueueStats> {
        let conn = self.get_conn().await?;
        self.init_coordination_tables(&conn).await?;

        let row = conn
            .query_one(
                "SELECT
                    COUNT(*) as total,
                    COUNT(CASE WHEN status = 'pending' THEN 1 END) as pending,
                    COUNT(CASE WHEN status = 'claimed' THEN 1 END) as claimed,
                    COUNT(CASE WHEN status = 'completed' THEN 1 END) as completed,
                    COUNT(CASE WHEN claimed_by = $1 THEN 1 END) as my_tasks
                 FROM task_queue",
                &[&self.ai_id],
            )
            .await?;

        Ok(QueueStats {
            total: row.get(0),
            pending: row.get(1),
            claimed: row.get(2),
            completed: row.get(3),
            my_tasks: row.get(4),
        })
    }

    // ============= VERIFICATION & TEST EXECUTION =============

    /// Execute verification for a task.
    ///
    /// This runs tests/checks for a verification task and reports results.
    ///
    /// # Arguments
    /// * `task_id` - Verification task ID to execute
    ///
    /// # Returns
    /// Result string with verification outcome
    ///
    /// # Security
    /// - Verifies task is claimed by this AI
    /// - Enforces 5-minute test timeout
    /// - Automatically creates fix tasks for failures
    pub async fn execute_verification(&self, task_id: i32) -> Result<String> {
        // Get task details
        let conn = self.get_conn().await?;

        let row = conn
            .query_one(
                "SELECT task, metadata, claimed_by
                 FROM task_queue
                 WHERE id = $1",
                &[&task_id],
            )
            .await
            .map_err(|_| CoordinationError::TaskNotFound(task_id))?;

        let task_desc: String = row.get(0);
        let metadata_str: Option<String> = row.get(1);
        let claimed_by: Option<String> = row.get(2);

        // Verify this AI claimed the task
        if claimed_by.as_deref() != Some(&self.ai_id) {
            return Err(CoordinationError::TaskNotAvailable);
        }

        // Parse metadata
        let metadata: serde_json::Value = if let Some(meta_str) = metadata_str {
            serde_json::from_str(&meta_str).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        // Check if this is a verification task
        if metadata.get("task_type").and_then(|v| v.as_str()) != Some("verification") {
            return Err(CoordinationError::InvalidOperation(
                "Not a verification task".to_string(),
            ));
        }

        let original_task_id = metadata
            .get("original_task_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                CoordinationError::InvalidOperation("Missing original_task_id".to_string())
            })? as i32;

        let verify_type = metadata
            .get("verification_type")
            .and_then(|v| v.as_str())
            .unwrap_or("test");

        let test_command = metadata.get("test_command").and_then(|v| v.as_str());

        // Run verification based on type
        let result = if verify_type == "test" {
            if let Some(cmd) = test_command {
                self.run_test_command(cmd).await
            } else {
                VerificationResult {
                    passed: false,
                    details: "No test command specified - manual verification required"
                        .to_string(),
                    output: String::new(),
                }
            }
        } else {
            VerificationResult {
                passed: false,
                details: format!("Unknown verification type: {}", verify_type),
                output: String::new(),
            }
        };

        // Report results
        self.report_verification_result(task_id, original_task_id, result, &task_desc)
            .await
    }

    /// Run test command and capture result.
    ///
    /// # Arguments
    /// * `test_command` - Shell command to run
    ///
    /// # Returns
    /// VerificationResult with passed status, details, and output
    ///
    /// # Security
    /// - 5-minute timeout to prevent hanging
    /// - Captures both stdout and stderr
    async fn run_test_command(&self, test_command: &str) -> VerificationResult {
        use tokio::process::Command;
        use tokio::time::{timeout, Duration};

        let result = timeout(Duration::from_secs(300), async {
            #[cfg(target_os = "windows")]
            let mut cmd = Command::new("cmd");
            #[cfg(target_os = "windows")]
            cmd.args(&["/C", test_command]);

            #[cfg(not(target_os = "windows"))]
            let mut cmd = Command::new("sh");
            #[cfg(not(target_os = "windows"))]
            cmd.args(&["-c", test_command]);

            cmd.output().await
        })
        .await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let combined = format!("{}\n{}", stdout, stderr);

                if output.status.success() {
                    VerificationResult {
                        passed: true,
                        details: "All tests passed".to_string(),
                        output: stdout,
                    }
                } else {
                    VerificationResult {
                        passed: false,
                        details: format!(
                            "Tests failed with exit code {}",
                            output.status.code().unwrap_or(-1)
                        ),
                        output: combined,
                    }
                }
            }
            Ok(Err(e)) => VerificationResult {
                passed: false,
                details: format!("Test execution error: {}", e),
                output: String::new(),
            },
            Err(_) => VerificationResult {
                passed: false,
                details: "Tests timed out after 5 minutes".to_string(),
                output: String::new(),
            },
        }
    }

    /// Report verification outcome and take appropriate action.
    ///
    /// # Arguments
    /// * `verify_task_id` - Verification task ID
    /// * `original_task_id` - Original task that was verified
    /// * `result` - Verification result
    /// * `task_desc` - Task description
    ///
    /// # Returns
    /// Result string with outcome details
    ///
    /// # Behavior
    /// - If passed: broadcasts success, completes verification task
    /// - If failed: broadcasts failure, creates fix task (priority 9), completes with failure info
    async fn report_verification_result(
        &self,
        verify_task_id: i32,
        original_task_id: i32,
        result: VerificationResult,
        task_desc: &str,
    ) -> Result<String> {
        if result.passed {
            // SUCCESS PATH

            // Complete verification task
            self.complete_task(verify_task_id, Some("verification_passed"))
                .await?;

            Ok(format!(
                "verify:{}|passed|original:{}",
                verify_task_id, original_task_id
            ))
        } else {
            // FAILURE PATH

            // Create fix task
            let failure_msg = if result.details.len() > 100 {
                &result.details[..100]
            } else {
                &result.details
            };

            let fix_desc = format!(
                "Fix failed verification: {}",
                if task_desc.len() > 60 {
                    &task_desc[..60]
                } else {
                    task_desc
                }
            );

            let test_output = if result.output.len() > 500 {
                &result.output[..500]
            } else {
                &result.output
            };

            let fix_metadata = serde_json::json!({
                "task_type": "bugfix",
                "failed_verify": verify_task_id,
                "original_task": original_task_id,
                "failure_details": failure_msg,
                "test_output": test_output
            });

            let fix_task_id = self
                .queue_task(&fix_desc, 9, Some(fix_metadata))
                .await?;

            // Complete verification task with failure info
            self.complete_task(
                verify_task_id,
                Some(&format!("verification_failed|fix:{}", fix_task_id)),
            )
            .await?;

            Ok(format!(
                "verify:{}|failed|fix:{}|original:{}",
                verify_task_id, fix_task_id, original_task_id
            ))
        }
    }

    // ============= COORDINATION PROJECTS & TASKS =============

    /// Create a coordination project for structured multi-AI task management.
    ///
    /// # Arguments
    /// * `name` - Project name (required, unique)
    /// * `goal` - Project goal/description (required)
    /// * `description` - Extended description (optional)
    ///
    /// # Returns
    /// ProjectInfo with project_id and details
    ///
    /// # Example
    /// ```ignore
    /// let project = service.create_project(
    ///     "All Tools Upgrade",
    ///     "Enterprise-grade improvements",
    ///     Some("Complete system overhaul")
    /// ).await?;
    /// ```
    pub async fn create_project(
        &self,
        name: &str,
        goal: &str,
        description: Option<&str>,
    ) -> Result<ProjectInfo> {
        if name.is_empty() {
            return Err(CoordinationError::InvalidOperation(
                "project_name_required".to_string(),
            ));
        }
        if goal.is_empty() {
            return Err(CoordinationError::InvalidOperation(
                "project_goal_required".to_string(),
            ));
        }

        let conn = self.get_conn().await?;

        // Ensure tables exist
        self.init_coordination_tables(&conn).await?;

        let row = conn
            .query_one(
                "INSERT INTO projects (name, overview, details, root_directory, created_by)
                 VALUES ($1, $2, $3, $4, $5)
                 RETURNING id, name, overview, details, created_by, created_at",
                &[&name, &goal, &description.unwrap_or(""), &name, &self.ai_id],
            )
            .await?;

        Ok(ProjectInfo {
            project_id: row.get(0),
            name: row.get(1),
            goal: row.get(2),
            description: row.get(3),
            created_by: row.get(4),
            created_at: row.get(5),
        })
    }

    /// Add a task to a coordination project.
    ///
    /// # Arguments
    /// * `project_id` - Parent project ID
    /// * `title` - Task title
    /// * `description` - Task description (optional)
    /// * `status` - Task status (default: pending)
    /// * `priority` - Task priority 1-10 (default: 5)
    /// * `assigned_to` - AI ID assigned to task (optional)
    ///
    /// # Returns
    /// Task ID of created task
    pub async fn add_task_to_project(
        &self,
        project_id: i32,
        title: &str,
        description: Option<&str>,
        status: Option<&str>,
        priority: Option<i32>,
        assigned_to: Option<&str>,
    ) -> Result<i32> {
        if title.is_empty() {
            return Err(CoordinationError::InvalidOperation(
                "task_title_required".to_string(),
            ));
        }

        let status = status.unwrap_or("pending");
        let priority = priority.unwrap_or(5).clamp(0, 9);

        // Validate status
        let valid_statuses = [
            "pending",
            "claimed",
            "in_progress",
            "completed",
            "blocked",
            "cancelled",
        ];
        if !valid_statuses.contains(&status) {
            return Err(CoordinationError::InvalidOperation(format!(
                "invalid_status:{}",
                status
            )));
        }

        let conn = self.get_conn().await?;

        let row = conn
            .query_one(
                "INSERT INTO coordination_tasks
                 (project_id, title, description, status, priority, assigned_to, created_at, teambook_name)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 RETURNING id",
                &[
                    &project_id,
                    &title,
                    &description.unwrap_or(""),
                    &status,
                    &priority,
                    &assigned_to,
                    &Utc::now(),
                    &self.teambook_name,
                ],
            )
            .await?;

        Ok(row.get(0))
    }

    /// List tasks for a coordination project.
    ///
    /// # Arguments
    /// * `project_id` - Project ID to list tasks for
    /// * `status` - Filter by status (optional)
    /// * `assigned_to` - Filter by assigned_to AI ID (optional)
    ///
    /// # Returns
    /// Vector of CoordinationTask structs
    pub async fn list_project_tasks(
        &self,
        project_id: i32,
        status: Option<&str>,
        assigned_to: Option<&str>,
    ) -> Result<Vec<CoordinationTask>> {
        let conn = self.get_conn().await?;

        // Use different queries based on filters to avoid lifetime issues
        let rows = match (status, assigned_to) {
            (Some(s), Some(a)) => {
                conn.query(
                    "SELECT id, project_id, title, description, status, priority, assigned_to,
                            created_at, claimed_at, completed_at, result, notes
                     FROM coordination_tasks
                     WHERE project_id = $1 AND status = $2 AND assigned_to = $3
                     ORDER BY priority DESC, created_at ASC",
                    &[&project_id, &s, &a],
                )
                .await?
            }
            (Some(s), None) => {
                conn.query(
                    "SELECT id, project_id, title, description, status, priority, assigned_to,
                            created_at, claimed_at, completed_at, result, notes
                     FROM coordination_tasks
                     WHERE project_id = $1 AND status = $2
                     ORDER BY priority DESC, created_at ASC",
                    &[&project_id, &s],
                )
                .await?
            }
            (None, Some(a)) => {
                conn.query(
                    "SELECT id, project_id, title, description, status, priority, assigned_to,
                            created_at, claimed_at, completed_at, result, notes
                     FROM coordination_tasks
                     WHERE project_id = $1 AND assigned_to = $2
                     ORDER BY priority DESC, created_at ASC",
                    &[&project_id, &a],
                )
                .await?
            }
            (None, None) => {
                conn.query(
                    "SELECT id, project_id, title, description, status, priority, assigned_to,
                            created_at, claimed_at, completed_at, result, notes
                     FROM coordination_tasks
                     WHERE project_id = $1
                     ORDER BY priority DESC, created_at ASC",
                    &[&project_id],
                )
                .await?
            }
        };

        let tasks = rows
            .iter()
            .map(|row| CoordinationTask {
                task_id: row.get(0),
                project_id: row.get(1),
                title: row.get(2),
                description: row.get(3),
                status: row.get(4),
                priority: row.get(5),
                assigned_to: row.get(6),
                created_at: row.get(7),
                claimed_at: row.get(8),
                completed_at: row.get(9),
                result: row.get(10),
                notes: row.get(11),
            })
            .collect();

        Ok(tasks)
    }

    /// Claim a task by ID (atomic operation).
    ///
    /// # Arguments
    /// * `task_id` - Task ID to claim
    ///
    /// # Returns
    /// CoordinationTask if claim successful
    ///
    /// # Security
    /// - Atomic operation: only claims if status is 'pending'
    /// - No race conditions
    pub async fn claim_task_by_id(&self, task_id: i32) -> Result<Option<CoordinationTask>> {
        let conn = self.get_conn().await?;
        let now = Utc::now();

        // Atomic claim: only claim if status is 'pending'
        let rows = conn
            .query(
                "UPDATE coordination_tasks
                 SET status = 'claimed',
                     assigned_to = $1,
                     claimed_at = $2,
                     updated_at = $2,
                     updated_by = $1
                 WHERE id = $3 AND status = 'pending'
                 RETURNING id, project_id, title, description, status, priority, assigned_to,
                           created_at, claimed_at, completed_at, result, notes",
                &[&self.ai_id, &now, &task_id],
            )
            .await?;

        if rows.is_empty() {
            return Ok(None);
        }

        let row = &rows[0];
        Ok(Some(CoordinationTask {
            task_id: row.get(0),
            project_id: row.get(1),
            title: row.get(2),
            description: row.get(3),
            status: row.get(4),
            priority: row.get(5),
            assigned_to: row.get(6),
            created_at: row.get(7),
            claimed_at: row.get(8),
            completed_at: row.get(9),
            result: row.get(10),
            notes: row.get(11),
        }))
    }

    /// Update task status and metadata.
    ///
    /// # Arguments
    /// * `task_id` - Task ID to update
    /// * `status` - New status
    /// * `notes` - Status notes/comments (optional)
    /// * `result` - Completion result (optional, for completed tasks)
    ///
    /// # Returns
    /// Updated CoordinationTask
    pub async fn update_task_status(
        &self,
        task_id: i32,
        status: &str,
        notes: Option<&str>,
        result: Option<&str>,
    ) -> Result<CoordinationTask> {
        // Validate status
        let valid_statuses = [
            "pending",
            "claimed",
            "in_progress",
            "completed",
            "blocked",
            "cancelled",
        ];
        if !valid_statuses.contains(&status) {
            return Err(CoordinationError::InvalidOperation(format!(
                "invalid_status:{}",
                status
            )));
        }

        let conn = self.get_conn().await?;
        let now = Utc::now();

        // Use different queries based on optional fields to avoid lifetime issues
        let row = match (notes, result, status) {
            (Some(n), Some(r), "in_progress") => {
                conn.query_one(
                    "UPDATE coordination_tasks
                     SET status = $1, updated_at = $2, updated_by = $3, notes = $4, result = $5, started_at = NOW()
                     WHERE id = $6
                     RETURNING id, project_id, title, description, status, priority, assigned_to,
                               created_at, claimed_at, completed_at, result, notes",
                    &[&status, &now, &self.ai_id, &n, &r, &task_id],
                )
                .await?
            }
            (Some(n), Some(r), "completed") => {
                conn.query_one(
                    "UPDATE coordination_tasks
                     SET status = $1, updated_at = $2, updated_by = $3, notes = $4, result = $5, completed_at = NOW()
                     WHERE id = $6
                     RETURNING id, project_id, title, description, status, priority, assigned_to,
                               created_at, claimed_at, completed_at, result, notes",
                    &[&status, &now, &self.ai_id, &n, &r, &task_id],
                )
                .await?
            }
            (Some(n), Some(r), _) => {
                conn.query_one(
                    "UPDATE coordination_tasks
                     SET status = $1, updated_at = $2, updated_by = $3, notes = $4, result = $5
                     WHERE id = $6
                     RETURNING id, project_id, title, description, status, priority, assigned_to,
                               created_at, claimed_at, completed_at, result, notes",
                    &[&status, &now, &self.ai_id, &n, &r, &task_id],
                )
                .await?
            }
            (Some(n), None, "in_progress") => {
                conn.query_one(
                    "UPDATE coordination_tasks
                     SET status = $1, updated_at = $2, updated_by = $3, notes = $4, started_at = NOW()
                     WHERE id = $5
                     RETURNING id, project_id, title, description, status, priority, assigned_to,
                               created_at, claimed_at, completed_at, result, notes",
                    &[&status, &now, &self.ai_id, &n, &task_id],
                )
                .await?
            }
            (Some(n), None, "completed") => {
                conn.query_one(
                    "UPDATE coordination_tasks
                     SET status = $1, updated_at = $2, updated_by = $3, notes = $4, completed_at = NOW()
                     WHERE id = $5
                     RETURNING id, project_id, title, description, status, priority, assigned_to,
                               created_at, claimed_at, completed_at, result, notes",
                    &[&status, &now, &self.ai_id, &n, &task_id],
                )
                .await?
            }
            (Some(n), None, _) => {
                conn.query_one(
                    "UPDATE coordination_tasks
                     SET status = $1, updated_at = $2, updated_by = $3, notes = $4
                     WHERE id = $5
                     RETURNING id, project_id, title, description, status, priority, assigned_to,
                               created_at, claimed_at, completed_at, result, notes",
                    &[&status, &now, &self.ai_id, &n, &task_id],
                )
                .await?
            }
            (None, Some(r), "in_progress") => {
                conn.query_one(
                    "UPDATE coordination_tasks
                     SET status = $1, updated_at = $2, updated_by = $3, result = $4, started_at = NOW()
                     WHERE id = $5
                     RETURNING id, project_id, title, description, status, priority, assigned_to,
                               created_at, claimed_at, completed_at, result, notes",
                    &[&status, &now, &self.ai_id, &r, &task_id],
                )
                .await?
            }
            (None, Some(r), "completed") => {
                conn.query_one(
                    "UPDATE coordination_tasks
                     SET status = $1, updated_at = $2, updated_by = $3, result = $4, completed_at = NOW()
                     WHERE id = $5
                     RETURNING id, project_id, title, description, status, priority, assigned_to,
                               created_at, claimed_at, completed_at, result, notes",
                    &[&status, &now, &self.ai_id, &r, &task_id],
                )
                .await?
            }
            (None, Some(r), _) => {
                conn.query_one(
                    "UPDATE coordination_tasks
                     SET status = $1, updated_at = $2, updated_by = $3, result = $4
                     WHERE id = $5
                     RETURNING id, project_id, title, description, status, priority, assigned_to,
                               created_at, claimed_at, completed_at, result, notes",
                    &[&status, &now, &self.ai_id, &r, &task_id],
                )
                .await?
            }
            (None, None, "in_progress") => {
                conn.query_one(
                    "UPDATE coordination_tasks
                     SET status = $1, updated_at = $2, updated_by = $3, started_at = NOW()
                     WHERE id = $4
                     RETURNING id, project_id, title, description, status, priority, assigned_to,
                               created_at, claimed_at, completed_at, result, notes",
                    &[&status, &now, &self.ai_id, &task_id],
                )
                .await?
            }
            (None, None, "completed") => {
                conn.query_one(
                    "UPDATE coordination_tasks
                     SET status = $1, updated_at = $2, updated_by = $3, completed_at = NOW()
                     WHERE id = $4
                     RETURNING id, project_id, title, description, status, priority, assigned_to,
                               created_at, claimed_at, completed_at, result, notes",
                    &[&status, &now, &self.ai_id, &task_id],
                )
                .await?
            }
            (None, None, _) => {
                conn.query_one(
                    "UPDATE coordination_tasks
                     SET status = $1, updated_at = $2, updated_by = $3
                     WHERE id = $4
                     RETURNING id, project_id, title, description, status, priority, assigned_to,
                               created_at, claimed_at, completed_at, result, notes",
                    &[&status, &now, &self.ai_id, &task_id],
                )
                .await?
            }
        };

        Ok(CoordinationTask {
            task_id: row.get(0),
            project_id: row.get(1),
            title: row.get(2),
            description: row.get(3),
            status: row.get(4),
            priority: row.get(5),
            assigned_to: row.get(6),
            created_at: row.get(7),
            claimed_at: row.get(8),
            completed_at: row.get(9),
            result: row.get(10),
            notes: row.get(11),
        })
    }

    /// Get tasks ready to work on (pending status).
    ///
    /// # Arguments
    /// * `project_id` - Optional project ID filter
    ///
    /// # Returns
    /// Vector of pending tasks with id, title, and priority
    pub async fn get_ready_tasks(
        &self,
        project_id: Option<i32>,
    ) -> Result<Vec<(i32, String, i32)>> {
        let conn = self.get_conn().await?;

        let rows = if let Some(pid) = project_id {
            conn.query(
                "SELECT id, title, priority FROM coordination_tasks
                 WHERE status = 'pending' AND project_id = $1
                 ORDER BY priority DESC, created_at ASC",
                &[&pid],
            )
            .await?
        } else {
            conn.query(
                "SELECT id, title, priority FROM coordination_tasks
                 WHERE status = 'pending'
                 ORDER BY priority DESC, created_at ASC",
                &[],
            )
            .await?
        };

        Ok(rows
            .iter()
            .map(|row| (row.get(0), row.get(1), row.get(2)))
            .collect())
    }

}

/// Verification result structure
#[derive(Debug, Clone)]
struct VerificationResult {
    passed: bool,
    details: String,
    output: String,
}

/// Project information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub project_id: i32,
    pub name: String,
    pub goal: String,
    pub description: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

/// Coordination task information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinationTask {
    pub task_id: i32,
    pub project_id: i32,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub priority: i32,
    pub assigned_to: Option<String>,
    pub created_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub result: Option<String>,
    pub notes: Option<String>,
}

// Tests will be added in a separate test module
#[cfg(test)]
mod tests {
    use super::*;

    // Note: Full integration tests require PostgreSQL connection
    // Unit tests for validation functions below

    #[test]
    fn test_task_status_conversions() {
        assert_eq!(TaskStatus::Pending.as_str(), "pending");
        assert_eq!(TaskStatus::Claimed.as_str(), "claimed");
        assert_eq!(TaskStatus::InProgress.as_str(), "in_progress");
        assert_eq!(TaskStatus::Completed.as_str(), "completed");

        assert_eq!(
            TaskStatus::from_str("pending"),
            Some(TaskStatus::Pending)
        );
        assert_eq!(
            TaskStatus::from_str("claimed"),
            Some(TaskStatus::Claimed)
        );
        assert_eq!(TaskStatus::from_str("invalid"), None);
    }

    #[test]
    fn test_constants() {
        assert_eq!(MAX_LOCK_DURATION_SECONDS, 300);
        assert_eq!(DEFAULT_LOCK_TIMEOUT, 30);
        assert_eq!(MAX_LOCKS_PER_AI, 10);
        assert_eq!(MAX_QUEUE_SIZE, 1000);
        assert_eq!(MAX_TASK_LENGTH, 2000);
        assert_eq!(MAX_RESOURCE_ID_LENGTH, 100);
    }
}
