//! Graph-RS - Task Graph Management
//!
//! High-performance graph algorithms for task dependencies,
//! entity relationships, and knowledge graphs
//!
//! Uses petgraph for efficient graph operations

use anyhow::Result;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::algo::{dijkstra, is_cyclic_directed};
use petgraph::Direction;
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============= TYPES =============

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub status: String,
    pub priority: i32,
    pub dependencies: Vec<String>,
}

pub struct TaskGraph {
    graph: DiGraph<Task, ()>,
    task_indices: HashMap<String, NodeIndex>,
}

impl TaskGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            task_indices: HashMap::new(),
        }
    }

    pub fn add_task(&mut self, task: Task) -> Result<()> {
        let task_id = task.id.clone();
        let node_idx = self.graph.add_node(task);
        self.task_indices.insert(task_id, node_idx);
        Ok(())
    }

    pub fn add_dependency(&mut self, from_task: &str, to_task: &str) -> Result<()> {
        let from_idx = *self.task_indices.get(from_task)
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", from_task))?;
        let to_idx = *self.task_indices.get(to_task)
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", to_task))?;

        self.graph.add_edge(from_idx, to_idx, ());
        Ok(())
    }

    pub fn has_cycle(&self) -> bool {
        is_cyclic_directed(&self.graph)
    }

    pub fn get_ready_tasks(&self) -> Vec<Task> {
        self.graph
            .node_indices()
            .filter(|&idx| {
                let task = &self.graph[idx];
                task.status == "pending" &&
                self.graph.neighbors_directed(idx, Direction::Incoming).count() == 0
            })
            .map(|idx| self.graph[idx].clone())
            .collect()
    }

    pub fn complete_task(&mut self, task_id: &str) -> Result<()> {
        let idx = *self.task_indices.get(task_id)
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", task_id))?;

        if let Some(task) = self.graph.node_weight_mut(idx) {
            task.status = "completed".to_string();
        }

        Ok(())
    }

    pub fn get_task_path(&self, from: &str, to: &str) -> Result<Vec<String>> {
        let from_idx = *self.task_indices.get(from)
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", from))?;
        let to_idx = *self.task_indices.get(to)
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", to))?;

        let paths = dijkstra(&self.graph, from_idx, Some(to_idx), |_| 1);

        if !paths.contains_key(&to_idx) {
            return Ok(Vec::new());
        }

        // Reconstruct path
        let mut path = vec![to];
        let mut current = to_idx;

        while current != from_idx {
            let predecessors: Vec<_> = self.graph
                .neighbors_directed(current, Direction::Incoming)
                .collect();

            if let Some(&pred) = predecessors.first() {
                path.push(&self.graph[pred].id);
                current = pred;
            } else {
                break;
            }
        }

        path.reverse();
        Ok(path.into_iter().map(|s| s.to_string()).collect())
    }
}

// ============= PYO3 BINDINGS =============

#[pyclass]
pub struct TaskGraphPy {
    inner: TaskGraph,
}

#[pymethods]
impl TaskGraphPy {
    #[new]
    fn new() -> Self {
        Self {
            inner: TaskGraph::new(),
        }
    }

    fn add_task(&mut self, id: &str, title: &str, status: &str, priority: i32, dependencies: Vec<String>) -> PyResult<()> {
        let task = Task {
            id: id.to_string(),
            title: title.to_string(),
            status: status.to_string(),
            priority,
            dependencies,
        };

        self.inner.add_task(task)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn add_dependency(&mut self, from_task: &str, to_task: &str) -> PyResult<()> {
        self.inner.add_dependency(from_task, to_task)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn has_cycle(&self) -> bool {
        self.inner.has_cycle()
    }

    fn get_ready_tasks(&self) -> PyResult<String> {
        let tasks = self.inner.get_ready_tasks();
        serde_json::to_string(&tasks)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn complete_task(&mut self, task_id: &str) -> PyResult<()> {
        self.inner.complete_task(task_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn get_task_path(&self, from: &str, to: &str) -> PyResult<Vec<String>> {
        self.inner.get_task_path(from, to)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }
}

#[pymodule]
fn graph_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<TaskGraphPy>()?;
    Ok(())
}
