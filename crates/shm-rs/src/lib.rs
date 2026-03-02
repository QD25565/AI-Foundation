//! Ultra-low-latency Shared Memory IPC for AI-Foundation
//!
//! Provides ~0.01ms message passing between AI processes, compared to
//! ~1ms for named pipes. Uses memory-mapped files with lock-free
//! SPSC (Single Producer Single Consumer) ring buffers.
//!
//! Architecture:
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │              Shared Memory Region                    │
//! ├─────────────────────────────────────────────────────┤
//! │  Header: magic, version, num_mailboxes              │
//! ├─────────────────────────────────────────────────────┤
//! │  Mailbox 0: [ring buffer] AI_ID: lyra-584           │
//! │  Mailbox 1: [ring buffer] AI_ID: sage-724           │
//! │  ...                                                │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! ## Bulletin Board (Zero-latency hooks)
//!
//! For hooks that need awareness data, we use a bulletin board pattern:
//! - Daemon writes awareness data to shared memory
//! - Hooks read directly from memory (no request/response)
//! - Latency: ~100ns vs ~150ms for subprocess calls

pub mod ring_buffer;
pub mod mailbox;
pub mod region;
pub mod zerocopy;
pub mod bulletin;
pub mod context;
pub mod enrichment;

pub use ring_buffer::SpscRingBuffer;
pub use mailbox::{Mailbox, Message, MessageType};
pub use region::SharedRegion;
pub use bulletin::BulletinBoard;
pub use zerocopy::{ZcMessage, ZcMessageType, ArchivedZcMessage, access_message, ZcMessageReader};
pub use context::{ContextWriter, ContextReader, ContextFingerprint, ContextError};
pub use enrichment::{extract_keywords, ContextAccumulator, scan_fp_bytes, RecallHit, RecentlyRecalled, engram_fp_path, compute_urgency, is_urgent, OwnedClaim, URGENCY_THRESHOLD, OutcomeRing, ToolOutcome, classify_outcome, format_anomaly_pulse};

/// Default size for shared memory region (16MB)
pub const DEFAULT_REGION_SIZE: usize = 16 * 1024 * 1024;

/// Maximum number of AI mailboxes
pub const MAX_MAILBOXES: usize = 64;

/// Maximum message size (64KB)
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024;

/// Magic number to identify valid shared memory regions
pub const MAGIC: u64 = 0x4149_464F_554E_4421; // "AIFOUND!"

/// Protocol version
pub const VERSION: u32 = 1;
