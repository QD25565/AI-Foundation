//! Task state machine and in-flight task registry.
//!
//! Every A2A task has:
//! - A UUID identity
//! - A status snapshot (submitted → working → completed/failed/cancelled)
//! - A broadcast channel so multiple SSE subscribers can receive the same events
//! - A CancellationToken for cooperative subprocess cancellation
//!
//! TaskStore uses DashMap for fine-grained shard locking — no global Mutex.
//! State transitions are driven by events, never by polling.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::rpc::{Artifact, Message, Task, TaskStatus};
use crate::streaming::StreamEvent;

// Broadcast channel capacity per task.
// 128 slots: burst-friendly for typical CLI output rates.
const CHANNEL_CAPACITY: usize = 128;

// ─── TaskStore ────────────────────────────────────────────────────────────────

/// Shared registry of all in-flight and recently-completed tasks.
pub struct TaskStore {
    entries: DashMap<Uuid, TaskEntry>,
}

struct TaskEntry {
    /// Current task snapshot, kept up-to-date on every transition.
    task: Task,
    /// Live event feed — clone the sender to subscribe.
    event_tx: broadcast::Sender<StreamEvent>,
    /// Cancel signal — call `.cancel()` to request subprocess termination.
    cancel: CancellationToken,
}

impl TaskStore {
    pub fn new() -> Self {
        Self { entries: DashMap::new() }
    }

    // ─── Lifecycle ────────────────────────────────────────────────────────────

    /// Register a new task in `Submitted` state.
    ///
    /// Returns:
    /// - `event_tx`: the broadcast sender the CLI task uses to publish events.
    /// - `cancel`: the token the CLI task polls for cooperative cancellation.
    pub fn create(&self, id: Uuid) -> (broadcast::Sender<StreamEvent>, CancellationToken) {
        let (event_tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let cancel = CancellationToken::new();
        let task = Task::new(id, TaskStatus::submitted());
        self.entries.insert(
            id,
            TaskEntry { task, event_tx: event_tx.clone(), cancel: cancel.clone() },
        );
        (event_tx, cancel)
    }

    // ─── Subscriptions ────────────────────────────────────────────────────────

    /// Subscribe to live events for an existing task.
    ///
    /// Returns `None` if the task is unknown (already cleaned up or never existed).
    /// Multiple callers can subscribe concurrently — each gets an independent receiver.
    pub fn subscribe(&self, id: Uuid) -> Option<broadcast::Receiver<StreamEvent>> {
        self.entries.get(&id).map(|e| e.event_tx.subscribe())
    }

    // ─── Read ─────────────────────────────────────────────────────────────────

    /// Snapshot the current task state for a `tasks/get` response.
    pub fn get(&self, id: Uuid) -> Option<Task> {
        self.entries.get(&id).map(|e| e.task.clone())
    }

    // ─── Cancellation ─────────────────────────────────────────────────────────

    /// Request cancellation of a running task.
    ///
    /// Returns `true` if the cancel signal was sent.
    /// Returns `false` if the task is unknown or already in a terminal state.
    pub fn request_cancel(&self, id: Uuid) -> bool {
        match self.entries.get(&id) {
            None => false,
            Some(entry) => {
                if entry.task.status.state.is_terminal() {
                    false
                } else {
                    entry.cancel.cancel();
                    true
                }
            }
        }
    }

    // ─── State transitions (called by CLI task, not by callers directly) ──────

    /// Transition the task to `Working` and broadcast the status event.
    pub fn set_working(&self, id: Uuid) {
        self.transition(id, TaskStatus::working(), false);
    }

    /// Append an incremental artifact chunk and broadcast the artifact event.
    pub fn push_artifact(&self, id: Uuid, artifact: Artifact) {
        if let Some(mut entry) = self.entries.get_mut(&id) {
            let is_last = artifact.last_chunk.unwrap_or(false);
            let event = StreamEvent::artifact_update(id, artifact.clone(), false);
            entry.task.artifacts.push(artifact);
            // Ignore send errors — no active subscribers is fine.
            let _ = entry.event_tx.send(event);
            let _ = is_last;
        }
    }

    /// Mark the task completed with an optional agent reply, broadcast final event.
    pub fn complete(&self, id: Uuid, reply: Option<Message>) {
        self.transition(id, TaskStatus::completed(reply), true);
    }

    /// Mark the task failed with an error message, broadcast final event.
    pub fn fail(&self, id: Uuid, reason: impl Into<String>) {
        self.transition(id, TaskStatus::failed(reason), true);
    }

    /// Mark the task cancelled, broadcast final event.
    pub fn mark_cancelled(&self, id: Uuid) {
        self.transition(id, TaskStatus::cancelled(), true);
    }

    // ─── Cleanup ──────────────────────────────────────────────────────────────

    /// Remove a task entry from the registry.
    ///
    /// Call after all SSE subscribers have disconnected to free memory.
    #[allow(dead_code)]
    pub fn remove(&self, id: Uuid) {
        self.entries.remove(&id);
    }

    // ─── Private ──────────────────────────────────────────────────────────────

    fn transition(&self, id: Uuid, status: TaskStatus, is_final: bool) {
        if let Some(mut entry) = self.entries.get_mut(&id) {
            entry.task.status = status.clone();
            let event = StreamEvent::status_update(id, status, is_final);
            let _ = entry.event_tx.send(event);
        }
    }
}

impl Default for TaskStore {
    fn default() -> Self {
        Self::new()
    }
}

// Allow TaskStore to be shared across async tasks.
// DashMap handles its own synchronisation internally.
unsafe impl Send for TaskStore {}
unsafe impl Sync for TaskStore {}

/// Convenience wrapper so callers can write `Arc<TaskStore>` in spawned tasks.
pub type SharedTaskStore = Arc<TaskStore>;
