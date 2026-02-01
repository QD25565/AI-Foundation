//! Global connection pool and runtime for PyO3 bindings
//!
//! PERFORMANCE OPTIMIZATION: Create singleton pool/runtime that persists across Python calls
//! instead of creating new ones every time (which was causing 10-13ms overhead per call).
//!
//! Expected improvement: 10-100x speedup for repeated operations.

use crate::storage::PostgresStorage;
use anyhow::Result;
use once_cell::sync::OnceCell;
use std::sync::Arc;
use tokio::runtime::Runtime;
use parking_lot::RwLock;

/// Global Tokio runtime (singleton)
static RUNTIME: OnceCell<Runtime> = OnceCell::new();

/// Global storage pool (singleton with RwLock for thread-safety)
static STORAGE: OnceCell<Arc<RwLock<Option<Arc<PostgresStorage>>>>> = OnceCell::new();

/// Get or create the global Tokio runtime
pub fn get_runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)  // 4 worker threads for parallel async operations
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime")
    })
}

/// Get or create the global PostgreSQL storage pool
pub fn get_storage(database_url: &str) -> Result<Arc<PostgresStorage>> {
    let storage_cell = STORAGE.get_or_init(|| Arc::new(RwLock::new(None)));

    // Fast path: storage already initialized
    {
        let read_guard = storage_cell.read();
        if let Some(storage) = &*read_guard {
            return Ok(Arc::clone(storage));
        }
    }

    // Slow path: need to initialize storage
    let mut write_guard = storage_cell.write();

    // Double-check: another thread might have initialized while we waited
    if let Some(storage) = &*write_guard {
        return Ok(Arc::clone(storage));
    }

    // Create new storage with connection pool
    let runtime = get_runtime();
    let storage = runtime.block_on(async {
        PostgresStorage::new(database_url).await
    })?;

    let storage_arc = Arc::new(storage);
    *write_guard = Some(Arc::clone(&storage_arc));

    Ok(storage_arc)
}

/// Reset the global storage (useful for testing or reconnecting)
#[allow(dead_code)]
pub fn reset_storage() {
    if let Some(storage_cell) = STORAGE.get() {
        let mut write_guard = storage_cell.write();
        *write_guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_singleton() {
        let rt1 = get_runtime();
        let rt2 = get_runtime();
        assert!(std::ptr::eq(rt1, rt2), "Runtime should be singleton");
    }
}
