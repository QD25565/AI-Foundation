//! TeamEngram - LMDB-style persistent storage for AI-Foundation
//!
//! Replaces PostgreSQL with a custom memory-mapped B+Tree implementation.
//! Key features:
//! - Shadow paging for atomic commits (no WAL needed)
//! - Copy-on-write for crash consistency
//! - Memory-mapped for fast reads
//! - Single-writer, multi-reader concurrency
//!
//! Architecture:
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    TeamEngram File                          │
//! ├─────────────────────────────────────────────────────────────┤
//! │  Header (4KB)                                               │
//! │  - Magic, version, page_size                                │
//! │  - Root page pointers (primary + shadow)                    │
//! │  - Free list head                                           │
//! │  - Transaction counter                                      │
//! ├─────────────────────────────────────────────────────────────┤
//! │  Page 0: Meta/Root                                          │
//! │  Page 1-N: B+Tree nodes (branch + leaf)                     │
//! │  ...                                                        │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! Shadow Paging:
//! - Writes go to shadow pages (copy-on-write)
//! - Atomic root pointer swap commits transaction
//! - Old pages added to free list after readers finish
//! - No write-ahead log needed!

// Re-export shm-rs for bulletin board access
pub extern crate shm_rs;

pub mod page;
pub mod shadow;
pub mod btree;
pub mod store;
pub mod ipc;
pub mod client;
pub mod mcp_adapter;
pub mod wake;
pub mod compat_types;
pub mod pipe;

// V2 Event Sourcing modules
pub mod event;
pub mod outbox;
pub mod event_log;
pub mod sequencer;
pub mod view;
pub mod v2_client;
pub mod migration;
pub mod crypto;

pub use page::{Page, PageId, PAGE_SIZE};
pub use shadow::ShadowAllocator;
pub use btree::BTree;
pub use store::{TeamEngram, Record, RecordType, RecordData, DirectMessage, Broadcast, Presence,
                Dialogue, DialogueStatus, Vote, VoteStatus, FileClaim, Room, JoinRoomResult,
                Task, TaskStatus, TaskPriority, Lock, TaskStats, now_millis};
pub use ipc::{ShmNotifyCallback, SharedShmNotify, NotificationRing, hash_ai_id,
              PresenceStatus, PresenceSlot, PresenceRegion, WakeReason, WakeTrigger, WakeRegion,
              MAX_AI_SLOTS, AI_ID_SIZE};
pub use client::TeamEngramClient;
pub use mcp_adapter::{TeamEngramStorage, V2Storage, V2Stats};
pub use wake::{WakeEvent, WakeCoordinator, WakeResult, WakeReason as CrossPlatformWakeReason, PlatformWakeEvent,
               PresenceMutex, is_ai_online, get_online_ais};
pub use migration::{Migrator, MigrationStats, run_migration};

// ============================================================================
// IPC NOTIFICATION INTERFACE
// ============================================================================

/// Notification types for IPC layer integration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NotifyType {
    /// Broadcast message sent
    Broadcast = 1,
    /// Direct message sent
    DirectMessage = 2,
    /// AI was mentioned (@ai_id)
    Mention = 3,
    /// Urgent message (contains urgent keywords)
    Urgent = 4,
    /// Dialogue started or updated
    Dialogue = 5,
    /// Vote created or updated
    Vote = 6,
    /// Task assigned
    Task = 7,
    /// Project created or updated
    Project = 8,
    /// Feature created or updated
    Feature = 9,
    /// Vault entry created or updated
    Vault = 10,
}

/// Trait for IPC notification callbacks
/// Implement this to receive notifications when data changes
pub trait NotifyCallback: Send + Sync {
    /// Called after a write operation commits
    /// - `notify_type`: Type of notification
    /// - `from_ai`: Source AI ID (or empty for system)
    /// - `to_ai`: Target AI ID (or empty for broadcast)
    /// - `content_preview`: First ~128 chars of content
    fn notify(&self, notify_type: NotifyType, from_ai: &str, to_ai: &str, content_preview: &str);
}

/// No-op notification callback (default)
pub struct NoOpNotify;
impl NotifyCallback for NoOpNotify {
    fn notify(&self, _: NotifyType, _: &str, _: &str, _: &str) {}
}

/// Magic number for TeamEngram files
pub const MAGIC: u64 = 0x5445_414D_454E_4752; // "TEAMENGR"

/// File format version (2 = sorted branch entries)
pub const VERSION: u32 = 2;

/// Default page size (4KB - matches OS page size)
pub const DEFAULT_PAGE_SIZE: usize = 4096;

/// Maximum key size (256 bytes)
pub const MAX_KEY_SIZE: usize = 256;

/// Maximum value size (64KB)
pub const MAX_VALUE_SIZE: usize = 64 * 1024;
