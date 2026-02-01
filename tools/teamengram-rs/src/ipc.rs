//! IPC Integration Layer
//!
//! Bridges TeamEngram's NotifyCallback with shared memory IPC.
//! This module connects persistence events to the notification ring buffer.

use std::sync::atomic::{AtomicU64, AtomicU32, AtomicU8, Ordering};
use std::path::Path;
use std::fs::{File, OpenOptions};
use memmap2::{MmapMut, MmapOptions};
use anyhow::{Result, Context};

use crate::{NotifyCallback, NotifyType};

/// Maximum notification slots in ring buffer
pub const NOTIFICATION_SLOTS: usize = 1024;

/// Size of content preview
pub const CONTENT_SIZE: usize = 128;

/// IPC notification types (matches shared_memory.rs)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcNotificationType {
    Empty = 0,
    Broadcast = 1,
    DirectMessage = 2,
    Mention = 3,
    Urgent = 4,
    System = 5,  // Used for Dialogue, Vote, Task
}

impl From<NotifyType> for IpcNotificationType {
    fn from(nt: NotifyType) -> Self {
        match nt {
            NotifyType::Broadcast => IpcNotificationType::Broadcast,
            NotifyType::DirectMessage => IpcNotificationType::DirectMessage,
            NotifyType::Mention => IpcNotificationType::Mention,
            NotifyType::Urgent => IpcNotificationType::Urgent,
            // Map new TeamEngram types to System for IPC layer
            NotifyType::Dialogue => IpcNotificationType::System,
            NotifyType::Vote => IpcNotificationType::System,
            NotifyType::Task => IpcNotificationType::System,
            NotifyType::Project => IpcNotificationType::System,
            NotifyType::Feature => IpcNotificationType::System,
            NotifyType::Vault => IpcNotificationType::System,
        }
    }
}

/// Simple hash function for AI IDs (djb2)
pub fn hash_ai_id(ai_id: &str) -> u32 {
    let mut hash: u32 = 5381;
    for byte in ai_id.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u32);
    }
    hash
}

/// Notification slot in ring buffer
#[repr(C)]
pub struct NotificationSlot {
    /// Sequence number for ABA prevention
    pub sequence: AtomicU64,
    /// Notification type
    pub msg_type: AtomicU8,
    /// Sender AI ID hash
    pub from_hash: AtomicU32,
    /// Target AI ID hash (0 = broadcast)
    pub to_hash: AtomicU32,
    /// Timestamp (Unix ms)
    pub timestamp: AtomicU64,
    /// Content length
    pub content_len: AtomicU8,
    /// Content preview
    pub content: [u8; CONTENT_SIZE],
}

/// Lock-free ring buffer for notifications (Vyukov-style MPMC)
#[repr(C)]
pub struct NotificationRing {
    /// Write position
    pub head: AtomicU64,
    /// Read position (each AI tracks their own)
    pub tail: AtomicU64,
    /// Notification slots
    pub slots: [NotificationSlot; NOTIFICATION_SLOTS],
}

impl NotificationRing {
    /// Publish a notification
    pub fn publish(&self, msg_type: IpcNotificationType, from_hash: u32, to_hash: u32, content: &str) {
        let head = self.head.fetch_add(1, Ordering::AcqRel);
        let idx = (head as usize) % NOTIFICATION_SLOTS;
        let slot = &self.slots[idx];

        // Write slot data
        slot.msg_type.store(msg_type as u8, Ordering::Release);
        slot.from_hash.store(from_hash, Ordering::Release);
        slot.to_hash.store(to_hash, Ordering::Release);
        slot.timestamp.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            Ordering::Release
        );

        // Copy content (truncated)
        let bytes = content.as_bytes();
        let len = bytes.len().min(CONTENT_SIZE);
        unsafe {
            let content_ptr = slot.content.as_ptr() as *mut u8;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), content_ptr, len);
            if len < CONTENT_SIZE {
                std::ptr::write_bytes(content_ptr.add(len), 0, CONTENT_SIZE - len);
            }
        }
        slot.content_len.store(len as u8, Ordering::Release);

        // Update sequence to signal slot is ready
        slot.sequence.store(head + 1, Ordering::Release);
    }

    /// Read pending notifications (event-driven, not polling) since given position
    pub fn drain_notifications(&self, from_pos: u64) -> Vec<(u64, IpcNotificationType, u32, u32, String)> {
        let head = self.head.load(Ordering::Acquire);
        let mut results = Vec::new();

        for pos in from_pos..head {
            let idx = (pos as usize) % NOTIFICATION_SLOTS;
            let slot = &self.slots[idx];

            // Check if slot is ready
            let seq = slot.sequence.load(Ordering::Acquire);
            if seq != pos + 1 {
                continue;
            }

            let msg_type = match slot.msg_type.load(Ordering::Acquire) {
                1 => IpcNotificationType::Broadcast,
                2 => IpcNotificationType::DirectMessage,
                3 => IpcNotificationType::Mention,
                4 => IpcNotificationType::Urgent,
                5 => IpcNotificationType::System,
                _ => IpcNotificationType::Empty,
            };
            let from_hash = slot.from_hash.load(Ordering::Acquire);
            let to_hash = slot.to_hash.load(Ordering::Acquire);
            let content_len = slot.content_len.load(Ordering::Acquire) as usize;
            let content = String::from_utf8_lossy(&slot.content[..content_len]).to_string();

            results.push((pos, msg_type, from_hash, to_hash, content));
        }

        results
    }

    /// Get current head position
    pub fn current_position(&self) -> u64 {
        self.head.load(Ordering::Acquire)
    }
}

// ============================================================================
// SHARED MEMORY NOTIFY CALLBACK
// ============================================================================

/// Header for IPC shared memory file
#[repr(C)]
pub struct IpcHeader {
    /// Magic: "TEAMIPC\0"
    pub magic: [u8; 8],
    /// Version
    pub version: AtomicU32,
    /// Init state (0=uninit, 1=initializing, 2=ready)
    pub init_state: AtomicU32,
}

impl IpcHeader {
    pub const MAGIC: [u8; 8] = *b"TEAMIPC\0";
    pub const VERSION: u32 = 1;

    pub fn is_valid(&self) -> bool {
        self.magic == Self::MAGIC
    }
}

/// Total shared memory size for IPC
pub const IPC_SHM_SIZE: usize =
    std::mem::size_of::<IpcHeader>() +
    std::mem::size_of::<NotificationRing>() +
    std::mem::size_of::<PresenceRegion>(); // Added for instant presence updates

/// Shared memory notification callback
/// Implements NotifyCallback to bridge TeamEngram events to IPC ring buffer
pub struct ShmNotifyCallback {
    mmap: MmapMut,
    #[allow(dead_code)]
    file: File,
}

impl ShmNotifyCallback {
    /// Open or create IPC shared memory file
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .context("Failed to open IPC shared memory file")?;

        // Ensure correct size
        let metadata = file.metadata()?;
        if metadata.len() < IPC_SHM_SIZE as u64 {
            file.set_len(IPC_SHM_SIZE as u64)
                .context("Failed to resize IPC file")?;
        }

        let mmap = unsafe {
            MmapOptions::new()
                .len(IPC_SHM_SIZE)
                .map_mut(&file)
                .context("Failed to mmap IPC file")?
        };

        let mut callback = Self { mmap, file };
        callback.ensure_initialized()?;

        Ok(callback)
    }

    fn ensure_initialized(&mut self) -> Result<()> {
        let header = self.header_mut();

        if header.is_valid() && header.init_state.load(Ordering::Acquire) == 2 {
            return Ok(());
        }

        // Try to claim initialization
        let prev = header.init_state.compare_exchange(
            0, 1, Ordering::AcqRel, Ordering::Acquire
        );

        match prev {
            Ok(_) => {
                // Initialize
                header.magic = IpcHeader::MAGIC;
                header.version.store(IpcHeader::VERSION, Ordering::Release);

                // Zero the ring buffer
                let ring = self.ring_mut();
                ring.head.store(0, Ordering::Release);
                ring.tail.store(0, Ordering::Release);

                // Mark ready
                self.header_mut().init_state.store(2, Ordering::Release);
                self.mmap.flush()?;
                Ok(())
            }
            Err(1) => {
                // Wait for initialization (spin with hint - brief wait, no sleep)
                while header.init_state.load(Ordering::Acquire) == 1 {
                    std::hint::spin_loop(); // CPU hint, no syscall
                }
                Ok(())
            }
            Err(2) => Ok(()),
            Err(_) => anyhow::bail!("Unexpected init state"),
        }
    }

    #[allow(dead_code)]
    fn header(&self) -> &IpcHeader {
        unsafe { &*(self.mmap.as_ptr() as *const IpcHeader) }
    }

    fn header_mut(&mut self) -> &mut IpcHeader {
        unsafe { &mut *(self.mmap.as_mut_ptr() as *mut IpcHeader) }
    }

    /// Get the notification ring buffer
    pub fn ring(&self) -> &NotificationRing {
        let offset = std::mem::size_of::<IpcHeader>();
        unsafe { &*(self.mmap.as_ptr().add(offset) as *const NotificationRing) }
    }

    fn ring_mut(&mut self) -> &mut NotificationRing {
        let offset = std::mem::size_of::<IpcHeader>();
        unsafe { &mut *(self.mmap.as_mut_ptr().add(offset) as *mut NotificationRing) }
    }

    /// Get the presence region (instant presence updates, ~100ns)
    pub fn presence_region(&self) -> &PresenceRegion {
        let offset = std::mem::size_of::<IpcHeader>() + std::mem::size_of::<NotificationRing>();
        unsafe { &*(self.mmap.as_ptr().add(offset) as *const PresenceRegion) }
    }

    /// Get mutable presence region
    pub fn presence_region_mut(&mut self) -> &mut PresenceRegion {
        let offset = std::mem::size_of::<IpcHeader>() + std::mem::size_of::<NotificationRing>();
        unsafe { &mut *(self.mmap.as_mut_ptr().add(offset) as *mut PresenceRegion) }
    }

    /// Flush changes to disk
    pub fn flush(&self) -> std::io::Result<()> {
        self.mmap.flush()
    }
}

impl NotifyCallback for ShmNotifyCallback {
    fn notify(&self, notify_type: NotifyType, from_ai: &str, to_ai: &str, content_preview: &str) {
        let ipc_type = IpcNotificationType::from(notify_type);
        let from_hash = hash_ai_id(from_ai);
        let to_hash = if to_ai.is_empty() { 0 } else { hash_ai_id(to_ai) };

        self.ring().publish(ipc_type, from_hash, to_hash, content_preview);
    }
}

// Thread-safe wrapper for use with TeamEngram
use std::sync::Arc;

/// Thread-safe shared memory notify callback
pub struct SharedShmNotify(pub Arc<ShmNotifyCallback>);

impl NotifyCallback for SharedShmNotify {
    fn notify(&self, notify_type: NotifyType, from_ai: &str, to_ai: &str, content_preview: &str) {
        self.0.notify(notify_type, from_ai, to_ai, content_preview);
    }
}

// ============================================================================
// PRESENCE TRACKING
// ============================================================================

/// Maximum number of AI slots supported
pub const MAX_AI_SLOTS: usize = 64;

/// Size of AI ID string
pub const AI_ID_SIZE: usize = 32;

/// Presence status values
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceStatus {
    Offline = 0,
    Active = 1,
    Busy = 2,
    Standby = 3,
}

impl From<u8> for PresenceStatus {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::Active,
            2 => Self::Busy,
            3 => Self::Standby,
            _ => Self::Offline,
        }
    }
}

/// Single presence slot for one AI
#[repr(C)]
pub struct PresenceSlot {
    /// 0 = slot free, 1+ = slot in use
    pub in_use: AtomicU8,
    /// Current status
    pub status: AtomicU8,
    /// Last heartbeat timestamp (Unix ms)
    pub last_heartbeat: AtomicU64,
    /// Hash of current task
    pub task_hash: AtomicU32,
    /// AI identifier (null-padded)
    pub ai_id: [u8; AI_ID_SIZE],
}

impl PresenceSlot {
    pub fn get_ai_id(&self) -> Option<String> {
        if self.in_use.load(Ordering::Acquire) == 0 {
            return None;
        }
        let end = self.ai_id.iter().position(|&b| b == 0).unwrap_or(AI_ID_SIZE);
        Some(String::from_utf8_lossy(&self.ai_id[..end]).to_string())
    }
}

/// Region containing all presence slots
#[repr(C)]
pub struct PresenceRegion {
    /// Number of active AIs
    pub active_count: AtomicU32,
    /// Presence slots
    pub slots: [PresenceSlot; MAX_AI_SLOTS],
}

impl PresenceRegion {
    /// Update presence for an AI (instant, ~100ns, NO B+tree write!)
    pub fn update(&self, ai_id: &str, status: PresenceStatus) {
        let hash = hash_ai_id(ai_id) as usize % MAX_AI_SLOTS;
        let slot = &self.slots[hash];

        // Set AI ID if not already set
        if slot.in_use.load(Ordering::Acquire) == 0 {
            // First time - copy AI ID
            let bytes = ai_id.as_bytes();
            let len = bytes.len().min(AI_ID_SIZE);
            // Safety: slot.ai_id is [u8; 64], we copy up to 64 bytes
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    slot.ai_id.as_ptr() as *mut u8,
                    len
                );
            }
            self.active_count.fetch_add(1, Ordering::AcqRel);
        }

        slot.in_use.store(1, Ordering::Release);
        slot.status.store(status as u8, Ordering::Release);
        slot.last_heartbeat.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            Ordering::Release
        );
    }

    /// Get presence for an AI (instant read)
    pub fn get(&self, ai_id: &str) -> Option<(PresenceStatus, u64)> {
        let hash = hash_ai_id(ai_id) as usize % MAX_AI_SLOTS;
        let slot = &self.slots[hash];

        if slot.in_use.load(Ordering::Acquire) == 0 {
            return None;
        }

        let status = PresenceStatus::from(slot.status.load(Ordering::Acquire));
        let last_heartbeat = slot.last_heartbeat.load(Ordering::Acquire);
        Some((status, last_heartbeat))
    }

    /// List all active AIs
    pub fn list_active(&self) -> Vec<(String, PresenceStatus)> {
        let mut result = Vec::new();
        for slot in &self.slots {
            if let Some(ai_id) = slot.get_ai_id() {
                let status = PresenceStatus::from(slot.status.load(Ordering::Acquire));
                result.push((ai_id, status));
            }
        }
        result
    }

    /// Mark an AI as offline
    pub fn set_offline(&self, ai_id: &str) {
        let hash = hash_ai_id(ai_id) as usize % MAX_AI_SLOTS;
        let slot = &self.slots[hash];

        if slot.in_use.load(Ordering::Acquire) != 0 {
            slot.status.store(PresenceStatus::Offline as u8, Ordering::Release);
            slot.in_use.store(0, Ordering::Release);
            self.active_count.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

// ============================================================================
// WAKE TRIGGERS
// ============================================================================

/// Wake reasons
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeReason {
    None = 0,
    DirectMessage = 1,
    Mention = 2,
    Urgent = 3,
    TaskAssigned = 4,
    Manual = 5,
}

impl From<u8> for WakeReason {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::DirectMessage,
            2 => Self::Mention,
            3 => Self::Urgent,
            4 => Self::TaskAssigned,
            5 => Self::Manual,
            _ => Self::None,
        }
    }
}

/// Single wake trigger for one AI
#[repr(C)]
pub struct WakeTrigger {
    /// Wake flag (non-zero = wake requested)
    pub wake_flag: AtomicU8,
    /// Reason for wake
    pub wake_reason: AtomicU8,
    /// Hash of AI that triggered wake
    pub from_hash: AtomicU32,
    /// Timestamp of wake request
    pub wake_time: AtomicU64,
}

impl WakeTrigger {
    /// Set wake trigger
    pub fn trigger(&self, reason: WakeReason, from_hash: u32) {
        self.wake_reason.store(reason as u8, Ordering::Release);
        self.from_hash.store(from_hash, Ordering::Release);
        self.wake_time.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            Ordering::Release
        );
        self.wake_flag.store(1, Ordering::Release);
    }

    /// Check and clear wake trigger
    pub fn check_and_clear(&self) -> Option<WakeReason> {
        if self.wake_flag.swap(0, Ordering::AcqRel) != 0 {
            Some(WakeReason::from(self.wake_reason.load(Ordering::Acquire)))
        } else {
            None
        }
    }
}

/// Region containing all wake triggers
#[repr(C)]
pub struct WakeRegion {
    pub triggers: [WakeTrigger; MAX_AI_SLOTS],
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_ipc_callback() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.ipc");

        let callback = ShmNotifyCallback::open(&path).unwrap();

        // Publish via NotifyCallback trait
        callback.notify(
            NotifyType::Broadcast,
            "sage-724",
            "",
            "Hello team!"
        );

        callback.notify(
            NotifyType::DirectMessage,
            "sage-724",
            "lyra-584",
            "Hey Lyra!"
        );

        // Read notifications (event-driven)
        let notifs = callback.ring().drain_notifications(0);
        assert_eq!(notifs.len(), 2);
        assert_eq!(notifs[0].1, IpcNotificationType::Broadcast);
        assert_eq!(notifs[0].4, "Hello team!");
        assert_eq!(notifs[1].1, IpcNotificationType::DirectMessage);
        assert_eq!(notifs[1].4, "Hey Lyra!");
    }

    #[test]
    fn test_hash_consistency() {
        // Verify hash matches Lyra's implementation
        assert_eq!(hash_ai_id("sage-724"), hash_ai_id("sage-724"));
        assert_ne!(hash_ai_id("sage-724"), hash_ai_id("lyra-584"));
    }
}
