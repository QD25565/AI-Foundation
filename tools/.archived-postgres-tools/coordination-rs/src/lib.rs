use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use chrono::Utc;
use uuid::Uuid;

// Coordination service module (database-backed distributed coordination)
pub mod coordination_service;

/// Task priority levels
#[pyclass]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TaskPriority {
    Low = 1,
    Medium = 3,
    High = 5,
    Critical = 10,
}

/// Task status
#[pyclass]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

/// Task representation
#[pyclass]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    #[pyo3(get)]
    pub id: String,
    #[pyo3(get)]
    pub title: String,
    #[pyo3(get)]
    pub description: String,
    #[pyo3(get)]
    pub priority: u8,
    #[pyo3(get)]
    pub status: String,
    #[pyo3(get)]
    pub assigned_to: Option<String>,
    #[pyo3(get)]
    pub created_at: String,
    #[pyo3(get)]
    pub updated_at: String,
    #[pyo3(get)]
    pub completed_at: Option<String>,
}

#[pymethods]
impl Task {
    #[new]
    fn new(title: String, description: String, priority: u8) -> Self {
        let now = Utc::now();
        Task {
            id: Uuid::new_v4().to_string(),
            title,
            description,
            priority,
            status: "pending".to_string(),
            assigned_to: None,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            completed_at: None,
        }
    }

    /// Assign task to an AI
    fn assign(&mut self, ai_id: &str) -> PyResult<()> {
        self.assigned_to = Some(ai_id.to_string());
        self.status = "in_progress".to_string();
        self.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    /// Mark task as completed
    fn complete(&mut self) -> PyResult<()> {
        self.status = "completed".to_string();
        let now = Utc::now();
        self.completed_at = Some(now.to_rfc3339());
        self.updated_at = now.to_rfc3339();
        Ok(())
    }

    /// Mark task as failed
    fn fail(&mut self) -> PyResult<()> {
        self.status = "failed".to_string();
        self.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    fn __repr__(&self) -> String {
        format!(
            "Task(id='{}', title='{}', priority={}, status='{}')",
            self.id, self.title, self.priority, self.status
        )
    }
}

/// Workflow step
#[pyclass]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    #[pyo3(get)]
    pub id: String,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub description: String,
    #[pyo3(get)]
    pub order: u32,
    #[pyo3(get)]
    pub required: bool,
}

#[pymethods]
impl WorkflowStep {
    #[new]
    fn new(name: String, description: String, order: u32, required: bool) -> Self {
        WorkflowStep {
            id: Uuid::new_v4().to_string(),
            name,
            description,
            order,
            required,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "WorkflowStep(id='{}', name='{}', order={})",
            self.id, self.name, self.order
        )
    }
}

/// Workflow template
#[pyclass]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    #[pyo3(get)]
    pub id: String,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub description: String,
    pub steps: Vec<WorkflowStep>,
    #[pyo3(get)]
    pub created_at: String,
}

#[pymethods]
impl Workflow {
    #[new]
    fn new(name: String, description: String) -> Self {
        Workflow {
            id: Uuid::new_v4().to_string(),
            name,
            description,
            steps: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
        }
    }

    /// Add a step to the workflow
    fn add_step(&mut self, step: WorkflowStep) -> PyResult<()> {
        self.steps.push(step);
        // Sort steps by order
        self.steps.sort_by_key(|s| s.order);
        Ok(())
    }

    /// Get number of steps
    fn step_count(&self) -> usize {
        self.steps.len()
    }

    fn __repr__(&self) -> String {
        format!(
            "Workflow(id='{}', name='{}', steps={})",
            self.id,
            self.name,
            self.steps.len()
        )
    }
}

/// Task manager for local operations
#[pyclass]
pub struct TaskManager {
    tasks: Vec<Task>,
}

#[pymethods]
impl TaskManager {
    #[new]
    fn new() -> Self {
        TaskManager { tasks: Vec::new() }
    }

    /// Create a new task
    fn create_task(&mut self, title: String, description: String, priority: u8) -> Task {
        let task = Task::new(title, description, priority);
        self.tasks.push(task.clone());
        task
    }

    /// Get task by ID
    fn get_task(&self, task_id: &str) -> Option<Task> {
        self.tasks.iter().find(|t| t.id == task_id).cloned()
    }

    /// List all tasks
    fn list_tasks(&self) -> Vec<Task> {
        self.tasks.clone()
    }

    /// List pending tasks
    fn list_pending(&self) -> Vec<Task> {
        self.tasks
            .iter()
            .filter(|t| t.status == "pending")
            .cloned()
            .collect()
    }

    /// List tasks by status
    fn list_by_status(&self, status: &str) -> Vec<Task> {
        self.tasks
            .iter()
            .filter(|t| t.status == status)
            .cloned()
            .collect()
    }

    /// Update task status
    fn update_task(&mut self, task_id: &str, status: &str) -> PyResult<bool> {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = status.to_string();
            task.updated_at = Utc::now().to_rfc3339();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Assign task to AI
    fn assign_task(&mut self, task_id: &str, ai_id: &str) -> PyResult<bool> {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.assign(ai_id)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Complete task
    fn complete_task(&mut self, task_id: &str) -> PyResult<bool> {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.complete()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get task count
    fn task_count(&self) -> usize {
        self.tasks.len()
    }

    fn __repr__(&self) -> String {
        format!("TaskManager(tasks={})", self.tasks.len())
    }
}

/// Workflow manager
#[pyclass]
pub struct WorkflowManager {
    workflows: Vec<Workflow>,
}

#[pymethods]
impl WorkflowManager {
    #[new]
    fn new() -> Self {
        WorkflowManager {
            workflows: Vec::new(),
        }
    }

    /// Create a new workflow
    fn create_workflow(&mut self, name: String, description: String) -> Workflow {
        let workflow = Workflow::new(name, description);
        self.workflows.push(workflow.clone());
        workflow
    }

    /// Get workflow by ID
    fn get_workflow(&self, workflow_id: &str) -> Option<Workflow> {
        self.workflows
            .iter()
            .find(|w| w.id == workflow_id)
            .cloned()
    }

    /// List all workflows
    fn list_workflows(&self) -> Vec<Workflow> {
        self.workflows.clone()
    }

    fn __repr__(&self) -> String {
        format!("WorkflowManager(workflows={})", self.workflows.len())
    }
}

/// Python module
#[pymodule]
fn ai_foundation_coordination(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Task>()?;
    m.add_class::<WorkflowStep>()?;
    m.add_class::<Workflow>()?;
    m.add_class::<TaskManager>()?;
    m.add_class::<WorkflowManager>()?;
    Ok(())
}
