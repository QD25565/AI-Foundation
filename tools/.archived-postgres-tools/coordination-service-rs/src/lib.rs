//! Coordination Service - Distributed locks, task queues, atomic operations
//!
//! High-performance Rust implementation replacing Python coordination_service.py (62.6 KB)
//!
//! Features:
//! - Distributed locks with automatic expiration
//! - Task queue with atomic claiming
//! - File claim system
//! - Project/task management
//! - PostgreSQL-backed with async Tokio

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use pyo3::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio_postgres::NoTls;
use tracing::{debug, error, info};

// Security limits
const MAX_LOCK_DURATION_SECONDS: i64 = 300; // 5 minutes
const DEFAULT_LOCK_TIMEOUT: i64 = 30;
const MAX_LOCKS_PER_AI: i32 = 10;
const MAX_QUEUE_SIZE: i32 = 1000;
const MAX_TASK_LENGTH: usize = 2000;
const MAX_RESOURCE_ID_LENGTH: usize = 100;

// ============= TYPES =============

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockInfo {
    pub resource_id: String,
    pub held_by: String,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: i32,
    pub task: String,
    pub priority: i32,
    pub status: String,
    pub claimed_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub result: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectTaskInfo {
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

// ============= COORDINATION SERVICE =============

pub struct CoordinationService {
    pool: Pool,
    teambook_name: String,
    ai_id: String,
}

impl CoordinationService {
    pub async fn new(postgres_url: &str, teambook_name: &str, ai_id: &str) -> Result<Self> {
        info!("Connecting to PostgreSQL: {}", postgres_url);

        let pg_config: tokio_postgres::Config = postgres_url.parse()
            .context("Invalid PostgreSQL URL")?;

        let mut config = Config::new();
        config.host = pg_config.get_hosts().get(0).map(|h| {
            if let tokio_postgres::config::Host::Tcp(s) = h {
                s.clone()
            } else {
                "localhost".to_string()
            }
        });
        config.port = pg_config.get_ports().get(0).copied();
        config.dbname = pg_config.get_dbname().map(|s| s.to_string());
        config.user = pg_config.get_user().map(|s| s.to_string());
        config.password = pg_config.get_password().map(|p| String::from_utf8_lossy(p).to_string());
        config.manager = Some(ManagerConfig { recycling_method: RecyclingMethod::Fast });

        let pool = config.create_pool(Some(Runtime::Tokio1), NoTls)?;

        let service = Self {
            pool,
            teambook_name: teambook_name.to_string(),
            ai_id: ai_id.to_string(),
        };

        service.init_schema().await?;

        Ok(service)
    }

    async fn init_schema(&self) -> Result<()> {
        let client = self.pool.get().await?;

        // Locks table
        client.execute(
            "CREATE TABLE IF NOT EXISTS locks (
                resource_id VARCHAR(100) PRIMARY KEY,
                held_by VARCHAR(100) NOT NULL,
                acquired_at TIMESTAMPTZ NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL,
                teambook_name VARCHAR(50)
            )",
            &[],
        ).await?;

        client.execute("CREATE INDEX IF NOT EXISTS idx_locks_expires ON locks(expires_at)", &[]).await.ok();
        client.execute("CREATE INDEX IF NOT EXISTS idx_locks_holder ON locks(held_by)", &[]).await.ok();

        // Task queue table
        client.execute(
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
        ).await?;

        client.execute("CREATE INDEX IF NOT EXISTS idx_queue_status_priority ON task_queue(status, priority DESC, created_at)", &[]).await.ok();
        client.execute("CREATE INDEX IF NOT EXISTS idx_queue_claimed ON task_queue(claimed_by, status)", &[]).await.ok();

        // Projects table
        client.execute(
            "CREATE TABLE IF NOT EXISTS projects (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                overview TEXT NOT NULL,
                details TEXT,
                root_directory TEXT NOT NULL UNIQUE,
                created_at TIMESTAMPTZ DEFAULT NOW(),
                created_by TEXT,
                updated_at TIMESTAMPTZ DEFAULT NOW(),
                updated_by TEXT,
                status TEXT DEFAULT 'active',
                config JSONB DEFAULT '{}'::jsonb
            )",
            &[],
        ).await?;

        client.execute("CREATE INDEX IF NOT EXISTS idx_projects_root_directory ON projects(root_directory)", &[]).await.ok();
        client.execute("CREATE INDEX IF NOT EXISTS idx_projects_status ON projects(status)", &[]).await.ok();
        client.execute("CREATE INDEX IF NOT EXISTS idx_projects_name ON projects(name)", &[]).await.ok();

        // Coordination tasks table
        client.execute(
            "CREATE TABLE IF NOT EXISTS coordination_tasks (
                id SERIAL PRIMARY KEY,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                title VARCHAR(500) NOT NULL,
                description TEXT,
                status VARCHAR(20) DEFAULT 'pending' NOT NULL,
                priority INTEGER DEFAULT 5 NOT NULL,
                assigned_to VARCHAR(100),
                created_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
                created_by VARCHAR(100),
                claimed_at TIMESTAMPTZ,
                started_at TIMESTAMPTZ,
                completed_at TIMESTAMPTZ,
                updated_at TIMESTAMPTZ DEFAULT NOW() NOT NULL,
                updated_by VARCHAR(100),
                result TEXT,
                notes TEXT,
                teambook_name VARCHAR(50),
                metadata JSONB DEFAULT '{}'::jsonb,
                CHECK (status IN ('pending', 'claimed', 'in_progress', 'completed', 'blocked', 'cancelled')),
                CHECK (priority >= 1 AND priority <= 10)
            )",
            &[],
        ).await?;

        client.execute("CREATE INDEX IF NOT EXISTS idx_coord_tasks_project_id ON coordination_tasks(project_id)", &[]).await.ok();
        client.execute("CREATE INDEX IF NOT EXISTS idx_coord_tasks_status ON coordination_tasks(status)", &[]).await.ok();
        client.execute("CREATE INDEX IF NOT EXISTS idx_coord_tasks_priority ON coordination_tasks(priority DESC)", &[]).await.ok();

        info!("Database schema initialized");
        Ok(())
    }

    // ============= INPUT VALIDATION =============

    fn sanitize_resource_id(&self, resource_id: &str) -> Option<String> {
        if resource_id.is_empty() || resource_id.len() > MAX_RESOURCE_ID_LENGTH {
            return None;
        }

        let re = Regex::new(r"^[A-Za-z0-9_:\-\./]+$").unwrap();
        if !re.is_match(resource_id) {
            return None;
        }

        Some(resource_id.to_string())
    }

    fn validate_timeout(&self, timeout: i64) -> i64 {
        timeout.max(1).min(MAX_LOCK_DURATION_SECONDS)
    }

    fn validate_priority(&self, priority: i32) -> i32 {
        priority.max(0).min(9)
    }

    // ============= DISTRIBUTED LOCKS =============

    pub async fn acquire_lock(&self, resource_id: &str, timeout: i64) -> Result<String> {
        let resource_id = self.sanitize_resource_id(resource_id)
            .ok_or_else(|| anyhow::anyhow!("invalid_resource_id"))?;

        let timeout = self.validate_timeout(timeout);

        let client = self.pool.get().await?;

        // Check AI lock limit
        let lock_count: i64 = client.query_one(
            "SELECT COUNT(*) FROM locks WHERE held_by = $1 AND expires_at > NOW()",
            &[&self.ai_id],
        ).await?.get(0);

        if lock_count >= MAX_LOCKS_PER_AI as i64 {
            return Ok(format!("!lock_limit:max_{}", MAX_LOCKS_PER_AI));
        }

        let now = Utc::now();
        let expires_at = now + Duration::seconds(timeout);

        // Atomic check-and-acquire
        client.execute(
            "INSERT INTO locks (resource_id, held_by, acquired_at, expires_at, teambook_name)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT(resource_id) DO UPDATE SET
                 held_by = EXCLUDED.held_by,
                 acquired_at = EXCLUDED.acquired_at,
                 expires_at = EXCLUDED.expires_at
             WHERE locks.expires_at < $3",
            &[&resource_id, &self.ai_id, &now, &expires_at, &self.teambook_name],
        ).await?;

        // Verify we got the lock
        let lock: Option<String> = client.query_opt(
            "SELECT held_by FROM locks WHERE resource_id = $1",
            &[&resource_id],
        ).await?.and_then(|row| row.get(0));

        if lock.as_ref() != Some(&self.ai_id) {
            let holder = lock.unwrap_or_else(|| "unknown".to_string());
            return Ok(format!("!locked_by:{}", holder));
        }

        let remaining = (expires_at - now).num_seconds();
        Ok(format!("{}|expires:{}s", resource_id, remaining))
    }

    pub async fn release_lock(&self, resource_id: &str) -> Result<String> {
        let resource_id = self.sanitize_resource_id(resource_id)
            .ok_or_else(|| anyhow::anyhow!("invalid_resource_id"))?;

        let client = self.pool.get().await?;

        // Verify ownership
        let lock: Option<String> = client.query_opt(
            "SELECT held_by FROM locks WHERE resource_id = $1",
            &[&resource_id],
        ).await?.and_then(|row| row.get(0));

        match lock {
            None => Ok("!not_locked".to_string()),
            Some(holder) if holder != self.ai_id => Ok(format!("!not_your_lock:held_by_{}", holder)),
            Some(_) => {
                client.execute("DELETE FROM locks WHERE resource_id = $1", &[&resource_id]).await?;
                Ok(resource_id)
            }
        }
    }

    pub async fn list_locks(&self, show_all: bool) -> Result<Vec<LockInfo>> {
        let client = self.pool.get().await?;

        // Cleanup expired locks
        client.execute("DELETE FROM locks WHERE expires_at < NOW()", &[]).await?;

        let rows = if show_all {
            client.query(
                "SELECT resource_id, held_by, acquired_at, expires_at
                 FROM locks
                 WHERE expires_at > NOW()
                 ORDER BY expires_at",
                &[],
            ).await?
        } else {
            client.query(
                "SELECT resource_id, held_by, acquired_at, expires_at
                 FROM locks
                 WHERE held_by = $1 AND expires_at > NOW()
                 ORDER BY expires_at",
                &[&self.ai_id],
            ).await?
        };

        Ok(rows.iter().map(|row| LockInfo {
            resource_id: row.get(0),
            held_by: row.get(1),
            acquired_at: row.get(2),
            expires_at: row.get(3),
        }).collect())
    }

    // ============= TASK QUEUE =============

    pub async fn queue_task(&self, task: &str, priority: i32, metadata: Option<&str>) -> Result<String> {
        if task.is_empty() {
            return Ok("!empty_task".to_string());
        }

        let task = if task.len() > MAX_TASK_LENGTH {
            &task[..MAX_TASK_LENGTH]
        } else {
            task
        };

        let priority = self.validate_priority(priority);

        let client = self.pool.get().await?;

        // Check queue size
        let count: i64 = client.query_one(
            "SELECT COUNT(*) FROM task_queue WHERE status = 'pending'",
            &[],
        ).await?.get(0);

        if count >= MAX_QUEUE_SIZE as i64 {
            return Ok(format!("!queue_full|max:{}|pending:{}", MAX_QUEUE_SIZE, count));
        }

        let row = client.query_one(
            "INSERT INTO task_queue (task, priority, status, created_at, teambook_name, metadata)
             VALUES ($1, $2, 'pending', NOW(), $3, $4)
             RETURNING id",
            &[&task, &priority, &self.teambook_name, &metadata],
        ).await?;

        let task_id: i32 = row.get(0);
        Ok(format!("task:{}|priority:{}", task_id, priority))
    }

    pub async fn claim_task(&self, prefer_priority: bool) -> Result<Option<TaskInfo>> {
        let client = self.pool.get().await?;

        let task = if prefer_priority {
            // Try verification tasks first
            client.query_opt(
                "SELECT id, task, priority, created_at, metadata
                 FROM task_queue
                 WHERE status = 'pending' AND metadata LIKE '%verification%'
                 ORDER BY priority DESC, created_at ASC
                 LIMIT 1",
                &[],
            ).await?
                .or_else(|| client.query_opt(
                    "SELECT id, task, priority, created_at, metadata
                     FROM task_queue
                     WHERE status = 'pending'
                     ORDER BY priority DESC, created_at ASC
                     LIMIT 1",
                    &[],
                ).await.ok().flatten())
        } else {
            client.query_opt(
                "SELECT id, task, priority, created_at, metadata
                 FROM task_queue
                 WHERE status = 'pending'
                 ORDER BY created_at ASC
                 LIMIT 1",
                &[],
            ).await?
        };

        let task = match task {
            None => return Ok(None),
            Some(t) => t,
        };

        let task_id: i32 = task.get(0);

        // Atomic claim
        client.execute(
            "UPDATE task_queue
             SET status = 'claimed', claimed_by = $1, claimed_at = NOW()
             WHERE id = $2 AND status = 'pending'",
            &[&self.ai_id, &task_id],
        ).await?;

        // Verify we got it
        let claimed: Option<String> = client.query_opt(
            "SELECT claimed_by FROM task_queue WHERE id = $1",
            &[&task_id],
        ).await?.and_then(|row| row.get(0));

        if claimed.as_ref() != Some(&self.ai_id) {
            return Ok(None);
        }

        Ok(Some(TaskInfo {
            id: task.get(0),
            task: task.get(1),
            priority: task.get(2),
            status: "claimed".to_string(),
            claimed_by: Some(self.ai_id.clone()),
            created_at: task.get(3),
            claimed_at: Some(Utc::now()),
            completed_at: None,
            result: None,
            metadata: task.get(4),
        }))
    }

    pub async fn complete_task(&self, task_id: i32, result: Option<&str>) -> Result<String> {
        let client = self.pool.get().await?;

        // Verify ownership
        let task: Option<(String, String)> = client.query_opt(
            "SELECT claimed_by, status FROM task_queue WHERE id = $1",
            &[&task_id],
        ).await?.map(|row| (row.get(0), row.get(1)));

        match task {
            None => Ok("!task_not_found".to_string()),
            Some((_, status)) if status == "completed" => Ok("!already_completed".to_string()),
            Some((claimed_by, _)) if claimed_by != self.ai_id => {
                Ok(format!("!not_your_task|claimed_by:{}", claimed_by))
            }
            Some(_) => {
                client.execute(
                    "UPDATE task_queue
                     SET status = 'completed', completed_at = NOW(), result = $1
                     WHERE id = $2",
                    &[&result, &task_id],
                ).await?;

                Ok(format!("task:{}", task_id))
            }
        }
    }

    pub async fn queue_stats(&self) -> Result<String> {
        let client = self.pool.get().await?;

        let row = client.query_one(
            "SELECT
                COUNT(*) as total,
                COUNT(CASE WHEN status = 'pending' THEN 1 END) as pending,
                COUNT(CASE WHEN status = 'claimed' THEN 1 END) as claimed,
                COUNT(CASE WHEN status = 'completed' THEN 1 END) as completed,
                COUNT(CASE WHEN claimed_by = $1 THEN 1 END) as my_tasks
             FROM task_queue",
            &[&self.ai_id],
        ).await?;

        let total: i64 = row.get(0);
        let pending: i64 = row.get(1);
        let claimed: i64 = row.get(2);
        let completed: i64 = row.get(3);
        let my_tasks: i64 = row.get(4);

        Ok(format!(
            "TASK QUEUE: {} total | {} pending | {} in progress | {} done | {} assigned to me",
            total, pending, claimed, completed, my_tasks
        ))
    }

    // ============= PROJECT/TASK MANAGEMENT =============

    pub async fn create_project(&self, name: &str, goal: &str, description: Option<&str>) -> Result<(i32, String)> {
        let client = self.pool.get().await?;

        let row = client.query_one(
            "INSERT INTO projects (name, overview, details, root_directory, created_by)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id, name",
            &[&name, &goal, &description, &format!("/projects/{}", name), &self.ai_id],
        ).await?;

        Ok((row.get(0), row.get(1)))
    }

    pub async fn add_task_to_project(
        &self,
        project_id: i32,
        title: &str,
        description: Option<&str>,
        status: &str,
        priority: i32,
        assigned_to: Option<&str>,
    ) -> Result<i32> {
        let client = self.pool.get().await?;

        let priority = self.validate_priority(priority);

        let row = client.query_one(
            "INSERT INTO coordination_tasks
             (project_id, title, description, status, priority, assigned_to, created_by, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
             RETURNING id",
            &[&project_id, &title, &description, &status, &priority, &assigned_to, &self.ai_id],
        ).await?;

        Ok(row.get(0))
    }

    pub async fn list_project_tasks(
        &self,
        project_id: i32,
        status: Option<&str>,
        assigned_to: Option<&str>,
    ) -> Result<Vec<ProjectTaskInfo>> {
        let client = self.pool.get().await?;

        let query = if status.is_some() && assigned_to.is_some() {
            "SELECT id, project_id, title, description, status, priority, assigned_to,
                    created_at, claimed_at, completed_at, result, notes
             FROM coordination_tasks
             WHERE project_id = $1 AND status = $2 AND assigned_to = $3
             ORDER BY priority DESC, created_at ASC"
        } else if status.is_some() {
            "SELECT id, project_id, title, description, status, priority, assigned_to,
                    created_at, claimed_at, completed_at, result, notes
             FROM coordination_tasks
             WHERE project_id = $1 AND status = $2
             ORDER BY priority DESC, created_at ASC"
        } else if assigned_to.is_some() {
            "SELECT id, project_id, title, description, status, priority, assigned_to,
                    created_at, claimed_at, completed_at, result, notes
             FROM coordination_tasks
             WHERE project_id = $1 AND assigned_to = $2
             ORDER BY priority DESC, created_at ASC"
        } else {
            "SELECT id, project_id, title, description, status, priority, assigned_to,
                    created_at, claimed_at, completed_at, result, notes
             FROM coordination_tasks
             WHERE project_id = $1
             ORDER BY priority DESC, created_at ASC"
        };

        let rows = match (status, assigned_to) {
            (Some(s), Some(a)) => client.query(query, &[&project_id, &s, &a]).await?,
            (Some(s), None) => client.query(query, &[&project_id, &s]).await?,
            (None, Some(a)) => client.query(query, &[&project_id, &a]).await?,
            (None, None) => client.query(query, &[&project_id]).await?,
        };

        Ok(rows.iter().map(|row| ProjectTaskInfo {
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
        }).collect())
    }

    pub async fn claim_task_by_id(&self, task_id: i32) -> Result<Option<ProjectTaskInfo>> {
        let client = self.pool.get().await?;

        let row = client.query_opt(
            "UPDATE coordination_tasks
             SET status = 'claimed',
                 assigned_to = $1,
                 claimed_at = NOW(),
                 updated_at = NOW(),
                 updated_by = $1
             WHERE id = $2 AND status = 'pending'
             RETURNING id, project_id, title, description, status, priority, assigned_to,
                       created_at, claimed_at, completed_at, result, notes",
            &[&self.ai_id, &task_id],
        ).await?;

        Ok(row.map(|r| ProjectTaskInfo {
            task_id: r.get(0),
            project_id: r.get(1),
            title: r.get(2),
            description: r.get(3),
            status: r.get(4),
            priority: r.get(5),
            assigned_to: r.get(6),
            created_at: r.get(7),
            claimed_at: r.get(8),
            completed_at: r.get(9),
            result: r.get(10),
            notes: r.get(11),
        }))
    }

    pub async fn update_task_status(
        &self,
        task_id: i32,
        status: &str,
        notes: Option<&str>,
        result: Option<&str>,
    ) -> Result<Option<ProjectTaskInfo>> {
        let client = self.pool.get().await?;

        let timestamp_update = match status {
            "in_progress" => ", started_at = NOW()",
            "completed" => ", completed_at = NOW()",
            _ => "",
        };

        let query = format!(
            "UPDATE coordination_tasks
             SET status = $1, updated_at = NOW(), updated_by = $2, notes = $3, result = $4 {}
             WHERE id = $5
             RETURNING id, project_id, title, description, status, priority, assigned_to,
                       created_at, claimed_at, completed_at, result, notes",
            timestamp_update
        );

        let row = client.query_opt(
            &query,
            &[&status, &self.ai_id, &notes, &result, &task_id],
        ).await?;

        Ok(row.map(|r| ProjectTaskInfo {
            task_id: r.get(0),
            project_id: r.get(1),
            title: r.get(2),
            description: r.get(3),
            status: r.get(4),
            priority: r.get(5),
            assigned_to: r.get(6),
            created_at: r.get(7),
            claimed_at: r.get(8),
            completed_at: r.get(9),
            result: r.get(10),
            notes: r.get(11),
        }))
    }
}

// ============= PYO3 BINDINGS =============

#[pyclass]
pub struct CoordinationServicePy {
    runtime: tokio::runtime::Runtime,
    service: Arc<CoordinationService>,
}

#[pymethods]
impl CoordinationServicePy {
    #[new]
    fn new(postgres_url: &str, teambook_name: &str, ai_id: &str) -> PyResult<Self> {
        let runtime = tokio::runtime::Runtime::new()?;
        let service = runtime.block_on(async {
            CoordinationService::new(postgres_url, teambook_name, ai_id).await
        })?;

        Ok(Self {
            runtime,
            service: Arc::new(service),
        })
    }

    fn acquire_lock(&self, resource_id: &str, timeout: i64) -> PyResult<String> {
        self.runtime.block_on(async {
            self.service.acquire_lock(resource_id, timeout).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn release_lock(&self, resource_id: &str) -> PyResult<String> {
        self.runtime.block_on(async {
            self.service.release_lock(resource_id).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn queue_task(&self, task: &str, priority: i32, metadata: Option<&str>) -> PyResult<String> {
        self.runtime.block_on(async {
            self.service.queue_task(task, priority, metadata).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn claim_task(&self, prefer_priority: bool) -> PyResult<Option<String>> {
        self.runtime.block_on(async {
            self.service.claim_task(prefer_priority).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
            .map(|opt| opt.map(|info| serde_json::to_string(&info).unwrap()))
    }

    fn complete_task(&self, task_id: i32, result: Option<&str>) -> PyResult<String> {
        self.runtime.block_on(async {
            self.service.complete_task(task_id, result).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn queue_stats(&self) -> PyResult<String> {
        self.runtime.block_on(async {
            self.service.queue_stats().await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn create_project(&self, name: &str, goal: &str, description: Option<&str>) -> PyResult<(i32, String)> {
        self.runtime.block_on(async {
            self.service.create_project(name, goal, description).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn add_task_to_project(
        &self,
        project_id: i32,
        title: &str,
        description: Option<&str>,
        status: &str,
        priority: i32,
        assigned_to: Option<&str>,
    ) -> PyResult<i32> {
        self.runtime.block_on(async {
            self.service.add_task_to_project(project_id, title, description, status, priority, assigned_to).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }
}

#[pymodule]
fn coordination_service(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<CoordinationServicePy>()?;
    Ok(())
}
