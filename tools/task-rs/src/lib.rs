//! Telos - AI-specific task engine
//!
//! High-performance task storage following the Engram pattern:
//! - Memory-mapped I/O for fast reads
//! - In-memory indexes for O(1) status filtering
//! - Priority heap for O(log n) get-next-task
//! - Pre-computed stats (no SQL parsing)
//! - Index persistence for O(1) startup
//!
//! Named "Telos" (Greek for purpose/goal) because tasks exist to achieve PURPOSE.
//! Part of the AI-Foundation cognitive stack:
//! - Engram = Memory storage
//! - Telos = Goal-directed action

pub mod store;

pub use store::{Task, TaskPriority, TaskStats, TaskStatus, TaskStore};
