///! PyO3 bindings for presence-rs

use pyo3::prelude::*;
use pyo3::exceptions::PyRuntimeError;
use crate::{PresenceStatus, update_ai_presence};

#[pyclass(name = "PresenceStatus")]
#[derive(Clone)]
pub struct PyPresenceStatus {
    inner: PresenceStatus,
}

#[pymethods]
impl PyPresenceStatus {
    #[new]
    fn new(status: &str) -> PyResult<Self> {
        let inner = match status.to_lowercase().as_str() {
            "active" => PresenceStatus::Active,
            "standby" => PresenceStatus::Standby,
            "idle" => PresenceStatus::Idle,
            "offline" => PresenceStatus::Offline,
            _ => return Err(PyRuntimeError::new_err(format!("Invalid status: {}", status))),
        };
        Ok(Self { inner })
    }

    fn __str__(&self) -> String {
        self.inner.as_str().to_string()
    }
}

/// Update AI presence in Redis
#[pyfunction]
#[pyo3(signature = (redis_url, ai_id, status, detail=None))]
fn update_presence(
    redis_url: String,
    ai_id: String,
    status: String,
    detail: Option<String>,
) -> PyResult<()> {
    let status_enum = match status.to_lowercase().as_str() {
        "active" => PresenceStatus::Active,
        "standby" => PresenceStatus::Standby,
        "idle" => PresenceStatus::Idle,
        "offline" => PresenceStatus::Offline,
        _ => return Err(PyRuntimeError::new_err(format!("Invalid status: {}", status))),
    };

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(update_ai_presence(
            &redis_url,
            &ai_id,
            status_enum,
            detail.as_deref(),
        ))
        .map_err(|e| PyRuntimeError::new_err(format!("Failed to update presence: {}", e)))?;

    Ok(())
}

/// Python module for presence_rs
#[pymodule]
fn presence_rs(m: &Bound<'_, pyo3::types::PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(update_presence, m)?)?;
    m.add_class::<PyPresenceStatus>()?;
    Ok(())
}
