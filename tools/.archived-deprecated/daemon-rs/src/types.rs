//! Common types for daemon-rs

use serde::{Deserialize, Serialize};

/// Daemon status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub status: String,
    pub uptime: u64,
    pub requests: u64,
    pub instance_id: String,
}

/// Module routing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    Teambook,
    Notebook,
    TaskManager,
    Daemon,
}

impl Module {
    pub fn from_method(method: &str) -> Option<Self> {
        if method.starts_with("daemon.") {
            Some(Self::Daemon)
        } else if method.starts_with("teambook.") {
            Some(Self::Teambook)
        } else if method.starts_with("notebook.") {
            Some(Self::Notebook)
        } else if method.starts_with("task_manager.") {
            Some(Self::TaskManager)
        } else {
            None
        }
    }
}
