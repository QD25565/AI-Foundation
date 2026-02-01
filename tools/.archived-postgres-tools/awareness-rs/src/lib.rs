//! # Awareness Core - High-Performance Multi-AI Coordination
//!
//! Rust implementation of awareness and file tracking system.
//! Replaces Python `actioned_awareness.py` with 10-100x performance.
//!
//! ## Architecture
//!
//! ```text
//! Python Layer (inject_presence.py)
//!          ↓ PyO3 FFI
//! ┌─────────────────────────────────┐
//! │   Rust Awareness Core           │
//! │  - File Actions                 │
//! │  - Team Activity                │
//! │  - PostgreSQL Pool              │
//! │  - Redis Cache                  │
//! └─────────────────────────────────┘
//! ```

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[cfg(feature = "python")]
use pyo3::prelude::*;

pub mod database;
pub mod file_actions;
pub mod sensors;
pub mod stigmergy;

// Re-exports
pub use file_actions::FileActionManager;
pub use sensors::{analyze, FileContext, FreshnessStatus, GitStatus, DependencyStatus, SafetyStatus};

/// File action data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAction {
    pub id: Option<i32>,  // PostgreSQL SERIAL is 32-bit
    pub ai_id: String,
    pub timestamp: NaiveDateTime,  // PostgreSQL timestamp without time zone
    pub action_type: String,
    pub file_path: String,
    pub file_type: Option<String>,
    pub file_size: Option<i32>,  // PostgreSQL INTEGER is 32-bit
    pub working_directory: Option<String>,
}

impl Default for FileAction {
    fn default() -> Self {
        Self {
            id: None,
            ai_id: String::new(),
            timestamp: chrono::Utc::now().naive_utc(),
            action_type: String::new(),
            file_path: String::new(),
            file_type: None,
            file_size: None,
            working_directory: None,
        }
    }
}

// Python bindings - async→sync bridge using tokio Runtime
#[cfg(feature = "python")]
use once_cell::sync::Lazy;
#[cfg(feature = "python")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "python")]
static RUNTIME: Lazy<tokio::runtime::Runtime> = Lazy::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime")
});

#[cfg(feature = "python")]
static FILE_ACTION_MANAGER: Lazy<Mutex<Option<Arc<FileActionManager>>>> = Lazy::new(|| {
    Mutex::new(None)
});

#[cfg(feature = "python")]
fn get_or_create_manager() -> Result<Arc<FileActionManager>, String> {
    let mut manager_lock = FILE_ACTION_MANAGER.lock()
        .map_err(|e| format!("Failed to lock manager: {}", e))?;

    if manager_lock.is_none() {
        // Load .env
        dotenvy::dotenv().ok();

        // Create database pool and manager
        let manager = RUNTIME.block_on(async {
            let db_pool = database::DatabasePool::from_env()
                .await
                .map_err(|e| format!("Failed to create database pool: {}", e))?;
            Ok::<_, String>(Arc::new(FileActionManager::new(db_pool)))
        })?;

        *manager_lock = Some(manager);
    }

    Ok(manager_lock.as_ref().unwrap().clone())
}

#[cfg(feature = "python")]
#[pymodule]
fn awareness_core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(log_file_action_py, m)?)?;
    m.add_function(wrap_pyfunction!(get_recent_actions_py, m)?)?;
    m.add_function(wrap_pyfunction!(format_recent_actions_py, m)?)?;
    m.add_function(wrap_pyfunction!(analyze_file_py, m)?)?;
    Ok(())
}

#[cfg(feature = "python")]
#[pyfunction]
#[pyo3(signature = (ai_id, action_type, file_path, file_type=None, file_size=None, working_directory=None))]
fn log_file_action_py(
    ai_id: String,
    action_type: String,
    file_path: String,
    file_type: Option<String>,
    file_size: Option<i32>,
    working_directory: Option<String>,
) -> PyResult<i64> {
    let manager = get_or_create_manager()
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

    let action = FileAction {
        id: None,
        ai_id,
        timestamp: chrono::Local::now().naive_local(),
        action_type,
        file_path,
        file_type,
        file_size,
        working_directory,
    };

    RUNTIME
        .block_on(async { manager.log(action).await })
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Failed to log action: {}", e)))
}

#[cfg(feature = "python")]
#[pyfunction]
fn get_recent_actions_py(limit: i64) -> PyResult<Vec<String>> {
    let manager = get_or_create_manager()
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

    let actions = RUNTIME
        .block_on(async { manager.get_recent(limit).await })
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Failed to get actions: {}", e)))?;

    // Convert FileAction structs to JSON strings
    let json_strings: Vec<String> = actions
        .iter()
        .map(|action| serde_json::to_string(action).unwrap_or_default())
        .collect();

    Ok(json_strings)
}

#[cfg(feature = "python")]
#[pyfunction]
fn format_recent_actions_py(limit: i64) -> PyResult<String> {
    let manager = get_or_create_manager()
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

    RUNTIME
        .block_on(async { manager.format_for_display(limit).await })
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Failed to format actions: {}", e)))
}

/// Python binding for sensors::analyze()
/// Returns JSON string with FileContext data
#[cfg(feature = "python")]
#[pyfunction]
fn analyze_file_py(path: String) -> PyResult<String> {
    let context = sensors::analyze(&path);
    serde_json::to_string_pretty(&context)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Failed to serialize: {}", e)))
}
