//! Teambook-RS - High-performance AI coordination system
//!
//! Full Rust replacement for Python teambook with:
//! - Redis pub/sub messaging (sub-millisecond latency)
//! - Redis Streams for guaranteed delivery (exactly-once semantics)
//! - PostgreSQL persistence (ACID guarantees)
//! - Zero-copy operations
//! - Type-safe message handling
//! - Async/await throughout

pub mod projects;
pub mod formatters;

pub mod types;
pub mod storage;
pub mod messaging;
pub mod cli;

// Real-time event delivery (enterprise-grade)
pub mod pubsub;
pub mod streams;

// Advanced coordination features (Pure Rust)
pub mod rooms;
pub mod dialogue;
pub mod evolution;
pub mod bccs;
pub mod intent;
pub mod sync;

pub use types::*;
pub use storage::*;
pub use messaging::*;
pub use intent::{Intent, IntentManager, IntentStatus};
pub use projects::{ProjectContext, find_project_for_file, format_context};

use anyhow::Result;

#[cfg(feature = "python")]
use pyo3::prelude::*;

/// Teambook client for AI coordination
pub struct TeambookClient {
    storage: PostgresStorage,
    messaging: RedisMessaging,
    ai_id: String,
}

impl TeambookClient {
    /// Create new teambook client
    pub async fn new(ai_id: String, pg_url: String, redis_url: String) -> Result<Self> {
        let storage = PostgresStorage::new(&pg_url).await?;
        let messaging = RedisMessaging::new(&redis_url).await?;

        Ok(Self {
            storage,
            messaging,
            ai_id,
        })
    }

    /// Get the current AI's ID
    pub fn ai_id(&self) -> &str {
        &self.ai_id
    }

    /// Write a note to teambook
    pub async fn write(&self, content: String, tags: Vec<String>) -> Result<Note> {
        let note = Note::new(self.ai_id.clone(), content, tags);
        
        // Save to PostgreSQL
        self.storage.save_note(&note).await?;
        
        // Publish to Redis
        self.messaging.publish_note_created(&note).await?;
        
        Ok(note)
    }

    /// Read recent notes
    pub async fn read(&self, limit: i32) -> Result<Vec<Note>> {
        self.storage.get_recent_notes(limit).await
    }

    /// Broadcast message to all AIs
    pub async fn broadcast(&self, content: String, channel: String) -> Result<Message> {
        let msg = Message::broadcast(self.ai_id.clone(), content, channel);

        // Save to PostgreSQL
        self.storage.save_message(&msg).await?;

        // Publish to Redis
        self.messaging.publish_message(&msg).await?;

        // Auto-update presence (we're active if we're broadcasting)
        let presence = Presence {
            ai_id: self.ai_id.clone(),
            last_seen: chrono::Utc::now(),
            status: "active".to_string(),
            current_task: None,
        };
        let _ = self.storage.update_presence(&presence).await; // Best-effort, don't fail on presence

        Ok(msg)
    }

    /// Send direct message to specific AI
    pub async fn direct_message(&self, to_ai: String, content: String) -> Result<Message> {
        let msg = Message::direct(self.ai_id.clone(), to_ai, content);

        // Save to PostgreSQL
        self.storage.save_message(&msg).await?;

        // Publish to Redis
        self.messaging.publish_message(&msg).await?;

        // Auto-update presence (we're active if we're sending DMs)
        let presence = Presence {
            ai_id: self.ai_id.clone(),
            last_seen: chrono::Utc::now(),
            status: "active".to_string(),
            current_task: None,
        };
        let _ = self.storage.update_presence(&presence).await; // Best-effort, don't fail on presence

        Ok(msg)
    }

    /// Get recent messages
    pub async fn get_messages(&self, limit: i32) -> Result<Vec<Message>> {
        self.storage.get_recent_messages(limit).await
    }

    /// Get direct messages for this AI
    pub async fn get_direct_messages(&self, limit: i32) -> Result<Vec<Message>> {
        self.storage.get_direct_messages(&self.ai_id, limit).await
    }
}

/// Python bindings for benchmarking
#[cfg(feature = "python")]
#[pymodule]
fn teambook_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    use pyo3::exceptions::PyRuntimeError;

    /// Create a new PostgreSQL storage backend and initialize schema
    #[pyfn(m)]
    fn create_postgres_storage(database_url: String) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&database_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            // Initialize schema tables
            storage.init_schema().await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to init schema: {}", e)))?;

            Ok("PostgresStorage created and schema initialized".to_string())
        })
    }

    /// Benchmark: Save a message to PostgreSQL
    #[pyfn(m)]
    fn benchmark_save_message(database_url: String, ai_id: String, content: String) -> PyResult<f64> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&database_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Storage error: {}", e)))?;

            // Ensure schema exists (idempotent)
            storage.init_schema().await
                .map_err(|e| PyRuntimeError::new_err(format!("Schema error: {}", e)))?;

            let msg = Message::broadcast(ai_id, content, "benchmark".to_string());

            let start = std::time::Instant::now();
            storage.save_message(&msg).await
                .map_err(|e| PyRuntimeError::new_err(format!("Save error: {}", e)))?;
            let elapsed = start.elapsed();

            Ok(elapsed.as_secs_f64() * 1000.0) // Return milliseconds
        })
    }

    /// Benchmark: Get recent messages from PostgreSQL
    #[pyfn(m)]
    fn benchmark_get_messages(database_url: String, limit: i32) -> PyResult<f64> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&database_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Storage error: {}", e)))?;

            // Ensure schema exists
            storage.init_schema().await
                .map_err(|e| PyRuntimeError::new_err(format!("Schema error: {}", e)))?;

            let start = std::time::Instant::now();
            storage.get_recent_messages(limit).await
                .map_err(|e| PyRuntimeError::new_err(format!("Query error: {}", e)))?;
            let elapsed = start.elapsed();

            Ok(elapsed.as_secs_f64() * 1000.0) // Return milliseconds
        })
    }

    /// Benchmark: Update presence in PostgreSQL
    #[pyfn(m)]
    fn benchmark_update_presence(database_url: String, ai_id: String) -> PyResult<f64> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&database_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Storage error: {}", e)))?;

            // Ensure schema exists
            storage.init_schema().await
                .map_err(|e| PyRuntimeError::new_err(format!("Schema error: {}", e)))?;

            let presence = Presence {
                ai_id,
                last_seen: chrono::Utc::now(),
                status: "active".to_string(),
                current_task: Some("benchmarking".to_string()),
            };

            let start = std::time::Instant::now();
            storage.update_presence(&presence).await
                .map_err(|e| PyRuntimeError::new_err(format!("Update error: {}", e)))?;
            let elapsed = start.elapsed();

            Ok(elapsed.as_secs_f64() * 1000.0) // Return milliseconds
        })
    }

    // ===== PRODUCTION BINDINGS =====

    /// Create a new TeambookClient
    #[pyfn(m)]
    fn create_client(ai_id: String, pg_url: String, redis_url: String) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let _client = TeambookClient::new(ai_id.clone(), pg_url, redis_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create client: {}", e)))?;

            Ok(format!("TeambookClient created for {}", ai_id))
        })
    }

    /// Write a note to teambook
    #[pyfn(m)]
    fn write_note(ai_id: String, pg_url: String, redis_url: String, content: String, tags: Vec<String>) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let client = TeambookClient::new(ai_id, pg_url, redis_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create client: {}", e)))?;

            let note = client.write(content, tags).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to write note: {}", e)))?;

            Ok(note.id.to_string())
        })
    }

    /// Read recent notes from teambook
    #[pyfn(m)]
    fn read_notes(ai_id: String, pg_url: String, redis_url: String, limit: i32) -> PyResult<Vec<String>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let client = TeambookClient::new(ai_id, pg_url, redis_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create client: {}", e)))?;

            let notes = client.read(limit).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to read notes: {}", e)))?;

            Ok(notes.iter().map(|n| n.content.clone()).collect())
        })
    }

    /// Broadcast message to all AIs
    #[pyfn(m)]
    fn broadcast_message(ai_id: String, pg_url: String, redis_url: String, content: String, channel: String) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let client = TeambookClient::new(ai_id, pg_url, redis_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create client: {}", e)))?;

            let msg = client.broadcast(content, channel).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to broadcast: {}", e)))?;

            Ok(msg.id.to_string())
        })
    }

    /// Send direct message to specific AI
    #[pyfn(m)]
    fn send_direct_message(ai_id: String, pg_url: String, redis_url: String, to_ai: String, content: String) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let client = TeambookClient::new(ai_id, pg_url, redis_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create client: {}", e)))?;

            let msg = client.direct_message(to_ai, content).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to send DM: {}", e)))?;

            Ok(msg.id.to_string())
        })
    }


    // ===== VAULT OPERATIONS =====

    /// Store value in vault
    #[pyfn(m)]
    fn vault_store(pg_url: String, key: String, value: String) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.vault_store(&key, &value).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to store: {}", e)))?;

            Ok(format!("stored:{}", key))
        })
    }

    /// Retrieve value from vault
    #[pyfn(m)]
    fn vault_retrieve(pg_url: String, key: String) -> PyResult<Option<String>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.vault_retrieve(&key).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to retrieve: {}", e)))
        })
    }

    /// List all vault keys
    #[pyfn(m)]
    fn vault_list(pg_url: String) -> PyResult<Vec<String>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.vault_list().await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to list: {}", e)))
        })
    }

    // ===== NOTE PIN OPERATIONS =====

    /// Pin a note
    #[pyfn(m)]
    fn pin_note(pg_url: String, note_id: String) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.pin_note(&note_id).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to pin: {}", e)))?;

            Ok(format!("pinned:{}", note_id))
        })
    }

    /// Unpin a note
    #[pyfn(m)]
    fn unpin_note(pg_url: String, note_id: String) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.unpin_note(&note_id).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to unpin: {}", e)))?;

            Ok(format!("unpinned:{}", note_id))
        })
    }

    // ===== TASK QUEUE OPERATIONS =====

    /// Queue a new task
    #[pyfn(m)]
    fn queue_task(pg_url: String, task: String, priority: i32) -> PyResult<i32> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.queue_task(&task, priority).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to queue task: {}", e)))
        })
    }

    /// Claim a task (atomic)
    #[pyfn(m)]
    fn claim_task(pg_url: String, ai_id: String) -> PyResult<Option<i32>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.claim_task(&ai_id).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to claim task: {}", e)))
        })
    }

    /// Claim specific task by ID
    #[pyfn(m)]
    fn claim_task_by_id(pg_url: String, task_id: i32, ai_id: String) -> PyResult<bool> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.claim_task_by_id(task_id, &ai_id).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to claim task: {}", e)))
        })
    }

    /// Complete a task
    #[pyfn(m)]
    fn complete_task(pg_url: String, task_id: i32, result: String) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.complete_task(task_id, &result).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to complete task: {}", e)))?;

            Ok(format!("completed:{}", task_id))
        })
    }

    /// Update task status
    #[pyfn(m)]
    fn update_task_status(pg_url: String, task_id: i32, status: String) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.update_task_status(task_id, &status).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to update status: {}", e)))?;

            Ok(format!("updated:{}:{}", task_id, status))
        })
    }

    /// Get queue statistics
    #[pyfn(m)]
    fn queue_stats(pg_url: String) -> PyResult<(i32, i32, i32)> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.queue_stats().await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to get stats: {}", e)))
        })
    }

    // ===== PROJECT MANAGEMENT =====

    /// Create a new project
    #[pyfn(m)]
    fn create_project(pg_url: String, name: String, goal: String) -> PyResult<i32> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.create_project(&name, &goal).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create project: {}", e)))
        })
    }

    /// List all projects
    #[pyfn(m)]
    fn list_projects(pg_url: String) -> PyResult<Vec<(i32, String, String)>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.list_projects().await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to list projects: {}", e)))
        })
    }

    /// Add task to project
    #[pyfn(m)]
    fn add_task_to_project(pg_url: String, project_id: i32, title: String, priority: i32) -> PyResult<i32> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.add_task_to_project(project_id, &title, priority).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to add task: {}", e)))
        })
    }

    /// List project tasks
    #[pyfn(m)]
    fn list_project_tasks(pg_url: String, project_id: i32) -> PyResult<Vec<(i32, String, String, i32)>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.list_project_tasks(project_id).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to list tasks: {}", e)))
        })
    }

    // ===== FILE CLAIMS =====

    /// Claim a file
    #[pyfn(m)]
    fn claim_file(pg_url: String, file_path: String, ai_id: String, duration_minutes: i32) -> PyResult<bool> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.claim_file(&file_path, &ai_id, duration_minutes).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to claim file: {}", e)))
        })
    }

    /// Release a file claim
    #[pyfn(m)]
    fn release_file(pg_url: String, file_path: String, ai_id: String) -> PyResult<bool> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.release_file(&file_path, &ai_id).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to release file: {}", e)))
        })
    }

    /// Check if file is claimed
    #[pyfn(m)]
    fn is_file_claimed(pg_url: String, file_path: String) -> PyResult<Option<String>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.is_file_claimed(&file_path).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to check claim: {}", e)))
        })
    }

    /// Get all active file claims
    #[pyfn(m)]
    fn get_active_claims(pg_url: String) -> PyResult<Vec<(String, String)>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.get_active_claims().await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to get claims: {}", e)))
        })
    }

    /// Force release all claims by AI
    #[pyfn(m)]
    fn force_release_all_claims(pg_url: String, ai_id: String) -> PyResult<i32> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.force_release_all_claims(&ai_id).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to force release: {}", e)))
        })
    }

    // ===== PRESENCE OPERATIONS =====

    /// Get active AIs (who is here)
    #[pyfn(m)]
    fn who_is_here(pg_url: String, minutes: i64) -> PyResult<Vec<(String, String, String)>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            let presences = storage.get_active_ais(minutes).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to get active AIs: {}", e)))?;

            Ok(presences.iter().map(|p| (
                p.ai_id.clone(),
                p.status.clone(),
                p.current_task.clone().unwrap_or_default()
            )).collect())
        })
    }

    // ===== MESSAGING FUNCTIONS =====

    #[pyfn(m)]
    fn get_messages(pg_url: String, channel: String, limit: i32) -> PyResult<Vec<String>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            let messages = storage.get_messages(&channel, limit).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to get messages: {}", e)))?;

            Ok(messages.iter().map(|(id, ai_id, content, _channel)| {
                format!("{}|{}|{}", id, ai_id, content)
            }).collect())
        })
    }

    #[pyfn(m)]
    fn read_dms(pg_url: String, ai_id: String, limit: i32) -> PyResult<Vec<String>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            let messages = storage.read_dms(&ai_id, limit).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to read DMs: {}", e)))?;

            Ok(messages.iter().map(|(id, from_ai, content)| {
                format!("dm:{}|from:{}|{}", id, from_ai, content)
            }).collect())
        })
    }

    // ===== PRESENCE & STATUS FUNCTIONS =====

    #[pyfn(m)]
    fn what_are_they_doing(pg_url: String, limit: i32) -> PyResult<Vec<(String, String, String)>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.what_are_they_doing(limit).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to get activity: {}", e)))
        })
    }

    #[pyfn(m)]
    fn get_presence(pg_url: String, ai_id: String) -> PyResult<Option<(String, String, String)>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            let presence = storage.get_presence(&ai_id).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to get presence: {}", e)))?;

            Ok(presence.map(|p| (
                p.ai_id,
                p.status,
                p.current_task.unwrap_or_else(|| "idle".to_string())
            )))
        })
    }

    #[pyfn(m)]
    fn connection_health(pg_url: String) -> PyResult<(bool, String)> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.connection_health().await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to check health: {}", e)))
        })
    }

    #[pyfn(m)]
    fn get_status(pg_url: String) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.get_status().await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to get status: {}", e)))
        })
    }

    // ===== PROJECT CONTEXT LOOKUP (FAST) =====

    /// Find project and feature for a file path (Rust-optimized)
    #[pyfn(m)]
    fn find_project_for_file(pg_url: String, file_path: String) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            use tokio_postgres::NoTls;

            let (client, connection) = tokio_postgres::connect(&pg_url, NoTls).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to connect: {}", e)))?;

            // Spawn connection to run in background
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    eprintln!("Connection error: {}", e);
                }
            });

            let ctx = crate::projects::find_project_for_file(&client, &file_path).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to find project: {}", e)))?;

            // Serialize to JSON for Python
            serde_json::to_string(&ctx)
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to serialize: {}", e)))
        })
    }


    // ===== POSTGRESQL-ONLY MESSAGING (NO REDIS REQUIRED) =====

    /// Broadcast message using PostgreSQL only (no Redis pub/sub)
    #[pyfn(m)]
    fn broadcast_message_pg(ai_id: String, pg_url: String, content: String, channel: String) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            let msg = Message::broadcast(ai_id, content, channel);
            storage.save_message(&msg).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to save broadcast: {}", e)))?;

            Ok(msg.id.to_string())
        })
    }

    /// Send direct message using PostgreSQL only (no Redis pub/sub)
    #[pyfn(m)]
    fn send_direct_message_pg(ai_id: String, pg_url: String, to_ai: String, content: String) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            let msg = Message::direct(ai_id, to_ai, content);
            storage.save_message(&msg).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to save DM: {}", e)))?;

            Ok(msg.id.to_string())
        })
    }

    /// Write note using PostgreSQL only (no Redis pub/sub)
    #[pyfn(m)]
    fn write_note_pg(ai_id: String, pg_url: String, content: String, tags: Vec<String>) -> PyResult<String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            storage.init_schema().await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to init schema: {}", e)))?;

            let note = Note::new(ai_id, content, tags);
            storage.save_note(&note).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to save note: {}", e)))?;

            Ok(note.id.to_string())
        })
    }

    /// Read notes using PostgreSQL only
    #[pyfn(m)]
    fn read_notes_pg(pg_url: String, limit: i32) -> PyResult<Vec<String>> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            let storage = PostgresStorage::new(&pg_url).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create storage: {}", e)))?;

            let notes = storage.get_recent_notes(limit).await
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to read notes: {}", e)))?;

            Ok(notes.iter().map(|n| n.content.clone()).collect())
        })
    }

    Ok(())
}
