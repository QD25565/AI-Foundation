//! PyO3 bindings for Python integration
//!
//! Provides zero-cost Python bindings to Rust stigmergy implementation

use crate::{DigitalPheromone, PheromoneType, server::StigmergyServer};
use chrono::{DateTime, Utc};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::sync::Arc;
use tokio::runtime::Runtime;

/// Convert Rust PheromoneType to Python string
fn pheromone_type_to_str(ptype: PheromoneType) -> &'static str {
    match ptype {
        PheromoneType::Interest => "interest",
        PheromoneType::Working => "working",
        PheromoneType::Blocked => "blocked",
        PheromoneType::Success => "success",
        PheromoneType::Question => "question",
    }
}

/// Convert Python string to Rust PheromoneType
fn str_to_pheromone_type(s: &str) -> PyResult<PheromoneType> {
    PheromoneType::from_str(s)
        .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>(
            format!("Invalid pheromone type: {}", s)
        ))
}

/// Python-exposed DigitalPheromone
#[pyclass(name = "DigitalPheromone")]
struct PyDigitalPheromone {
    inner: DigitalPheromone,
}

#[pymethods]
impl PyDigitalPheromone {
    #[new]
    fn new(
        location: String,
        pheromone_type: String,
        intensity: f64,
        decay_rate: f64,
        agent_id: String,
    ) -> PyResult<Self> {
        let ptype = str_to_pheromone_type(&pheromone_type)?;
        Ok(Self {
            inner: DigitalPheromone::new(location, ptype, intensity, decay_rate, agent_id),
        })
    }

    #[getter]
    fn location(&self) -> String {
        self.inner.location.clone()
    }

    #[getter]
    fn pheromone_type(&self) -> String {
        pheromone_type_to_str(self.inner.pheromone_type).to_string()
    }

    #[getter]
    fn intensity(&self) -> f64 {
        self.inner.intensity
    }

    #[getter]
    fn decay_rate(&self) -> f64 {
        self.inner.decay_rate
    }

    #[getter]
    fn agent_id(&self) -> String {
        self.inner.agent_id.clone()
    }

    #[getter]
    fn created_at(&self) -> String {
        self.inner.created_at.to_rfc3339()
    }

    #[pyo3(signature = (ref_time=None))]
    fn current_intensity(&self, ref_time: Option<String>) -> PyResult<f64> {
        let ref_time = if let Some(t) = ref_time {
            Some(DateTime::parse_from_rfc3339(&t)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?
                .with_timezone(&Utc))
        } else {
            None
        };

        Ok(self.inner.current_intensity(ref_time))
    }

    fn is_expired(&self, threshold: f64) -> bool {
        self.inner.is_expired(threshold)
    }

    fn half_life(&self) -> Option<f64> {
        self.inner.half_life()
    }

    fn to_dict(&self, py: Python) -> PyResult<PyObject> {
        let dict = PyDict::new_bound(py);
        dict.set_item("location", &self.inner.location)?;
        dict.set_item("pheromone_type", pheromone_type_to_str(self.inner.pheromone_type))?;
        dict.set_item("intensity", self.inner.intensity)?;
        dict.set_item("decay_rate", self.inner.decay_rate)?;
        dict.set_item("agent_id", &self.inner.agent_id)?;
        dict.set_item("created_at", self.inner.created_at.to_rfc3339())?;
        dict.set_item("current_intensity", self.inner.current_intensity(None))?;
        Ok(dict.into())
    }
}

/// Python-exposed StigmergyServer
#[pyclass(name = "StigmergyServer")]
struct PyStigmergyServer {
    server: Arc<StigmergyServer>,
    runtime: Arc<Runtime>,
}

#[pymethods]
impl PyStigmergyServer {
    #[new]
    fn new(database_url: String) -> PyResult<Self> {
        let runtime = Runtime::new()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let server = runtime.block_on(async {
            StigmergyServer::new(&database_url).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        Ok(Self {
            server: Arc::new(server),
            runtime: Arc::new(runtime),
        })
    }

    fn leave_trace(&self, pheromone: &PyDigitalPheromone) -> PyResult<bool> {
        let pheromone = pheromone.inner.clone();
        self.runtime.block_on(async {
            self.server.leave_trace(pheromone).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn sense_environment(
        &self,
        py: Python,
        location: String,
        pheromone_type: Option<String>,
    ) -> PyResult<PyObject> {
        let ptype = if let Some(t) = pheromone_type {
            Some(str_to_pheromone_type(&t)?)
        } else {
            None
        };

        let pheromones = self.runtime.block_on(async {
            self.server.sense_environment(&location, ptype).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let list = PyList::empty(py);
        for pheromone in pheromones {
            let py_pheromone = PyDigitalPheromone { inner: pheromone };
            list.append(py_pheromone)?;
        }

        Ok(list.into())
    }

    fn get_intensity(
        &self,
        location: String,
        pheromone_type: Option<String>,
    ) -> PyResult<f64> {
        let ptype = if let Some(t) = pheromone_type {
            Some(str_to_pheromone_type(&t)?)
        } else {
            None
        };

        self.runtime.block_on(async {
            self.server.get_intensity(&location, ptype).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn refresh_pheromone(
        &self,
        location: String,
        pheromone_type: String,
        agent_id: String,
    ) -> PyResult<bool> {
        let ptype = str_to_pheromone_type(&pheromone_type)?;

        self.runtime.block_on(async {
            self.server.refresh_pheromone(&location, ptype, &agent_id).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn clear_pheromone(
        &self,
        location: String,
        pheromone_type: String,
        agent_id: Option<String>,
    ) -> PyResult<()> {
        let ptype = str_to_pheromone_type(&pheromone_type)?;

        self.runtime.block_on(async {
            self.server.clear_pheromone(&location, ptype, agent_id.as_deref()).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn is_available(&self, location: String) -> PyResult<bool> {
        self.runtime.block_on(async {
            self.server.is_available(&location).await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn cleanup_expired(&self) -> PyResult<u64> {
        self.runtime.block_on(async {
            self.server.cleanup_expired().await
        }).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }
}

/// Initialize Python module
#[pymodule]
fn stigmergy_core(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<PyDigitalPheromone>()?;
    m.add_class::<PyStigmergyServer>()?;
    Ok(())
}
