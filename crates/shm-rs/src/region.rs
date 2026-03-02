//! Shared Memory Region Management
//!
//! Creates and manages a memory-mapped file that multiple AI processes
//! can attach to for ultra-low-latency communication.

use memmap2::{MmapMut, MmapOptions};
use std::fs::OpenOptions;
use std::path::PathBuf;
use anyhow::{Context, Result, bail};

use crate::ring_buffer::RingBufferHeader;
use crate::mailbox::{Mailbox, MailboxMeta};
use crate::{MAGIC, VERSION, MAX_MAILBOXES};

/// Header at the start of shared memory region
#[repr(C)]
pub struct RegionHeader {
    /// Magic number to identify valid regions
    pub magic: u64,
    /// Protocol version
    pub version: u32,
    /// Number of active mailboxes
    pub num_mailboxes: u32,
    /// Total region size
    pub region_size: u64,
    /// Mailbox buffer size (per mailbox)
    pub mailbox_buffer_size: u32,
    /// Creation timestamp
    pub created_at: u64,
    /// Padding to 64 bytes
    _padding: [u8; 28],
}

impl RegionHeader {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    fn is_valid(&self) -> bool {
        self.magic == MAGIC && self.version == VERSION
    }
}

/// Shared memory region for AI communication
pub struct SharedRegion {
    mmap: MmapMut,
    mailbox_buffer_size: usize,
}

impl SharedRegion {
    /// Calculate required region size
    fn calculate_size(mailbox_buffer_size: usize) -> usize {
        RegionHeader::SIZE
            + MAX_MAILBOXES * MailboxMeta::SIZE
            + MAX_MAILBOXES * RingBufferHeader::SIZE
            + MAX_MAILBOXES * mailbox_buffer_size
    }

    /// Get the default shared memory path
    pub fn default_path() -> PathBuf {
        let base = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join(".ai-foundation").join("shm").join("ai_ipc.shm")
    }

    /// Create or open a shared memory region
    pub fn open(path: Option<PathBuf>, mailbox_buffer_size: usize) -> Result<Self> {
        let path = path.unwrap_or_else(Self::default_path);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .context("Failed to create shared memory directory")?;
        }

        let region_size = Self::calculate_size(mailbox_buffer_size);
        let file_exists = path.exists();

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .context("Failed to open shared memory file")?;

        // Set file size if new
        if !file_exists || file.metadata()?.len() < region_size as u64 {
            file.set_len(region_size as u64)
                .context("Failed to set shared memory size")?;
        }

        let mut mmap = unsafe {
            MmapOptions::new()
                .len(region_size)
                .map_mut(&file)
                .context("Failed to memory-map shared region")?
        };

        // Initialize header if new
        let header = unsafe { &mut *(mmap.as_mut_ptr() as *mut RegionHeader) };
        if !header.is_valid() {
            header.magic = MAGIC;
            header.version = VERSION;
            header.num_mailboxes = 0;
            header.region_size = region_size as u64;
            header.mailbox_buffer_size = mailbox_buffer_size as u32;
            header.created_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            // Initialize all mailbox metadata as inactive
            for i in 0..MAX_MAILBOXES {
                let meta = unsafe { Self::mailbox_meta_ptr(mmap.as_mut_ptr(), i) };
                unsafe {
                    (*meta).status = 0;
                    (*meta).owner_pid = 0;
                }
            }

            mmap.flush().context("Failed to flush shared memory")?;
        }

        Ok(Self {
            mmap,
            mailbox_buffer_size,
        })
    }

    /// Get pointer to mailbox metadata
    unsafe fn mailbox_meta_ptr(base: *mut u8, index: usize) -> *mut MailboxMeta {
        let offset = RegionHeader::SIZE + index * MailboxMeta::SIZE;
        base.add(offset) as *mut MailboxMeta
    }

    /// Get pointer to mailbox ring buffer header
    unsafe fn mailbox_header_ptr(base: *mut u8, index: usize) -> *mut RingBufferHeader {
        let offset = RegionHeader::SIZE
            + MAX_MAILBOXES * MailboxMeta::SIZE
            + index * std::mem::size_of::<RingBufferHeader>();
        base.add(offset) as *mut RingBufferHeader
    }

    /// Get pointer to mailbox data buffer
    unsafe fn mailbox_data_ptr(base: *mut u8, index: usize, buffer_size: usize) -> *mut u8 {
        let offset = RegionHeader::SIZE
            + MAX_MAILBOXES * MailboxMeta::SIZE
            + MAX_MAILBOXES * std::mem::size_of::<RingBufferHeader>()
            + index * buffer_size;
        base.add(offset)
    }

    /// Register a new mailbox for an AI
    pub fn register_mailbox(&mut self, ai_id: &str) -> Result<usize> {
        let base = self.mmap.as_mut_ptr();

        // Find existing or empty slot
        for i in 0..MAX_MAILBOXES {
            let meta = unsafe { &mut *Self::mailbox_meta_ptr(base, i) };

            if meta.is_active() && meta.ai_id_str() == ai_id {
                // Already registered, update PID
                meta.owner_pid = std::process::id();
                meta.last_activity = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                return Ok(i);
            }

            if !meta.is_active() {
                // Found empty slot
                meta.set_ai_id(ai_id);
                meta.owner_pid = std::process::id();
                meta.status = 1;
                meta.buffer_capacity = self.mailbox_buffer_size as u32;
                meta.last_activity = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                // Initialize ring buffer
                let header = unsafe { &mut *Self::mailbox_header_ptr(base, i) };
                header.init(self.mailbox_buffer_size as u64);

                // Update region header
                let region_header = unsafe { &mut *(base as *mut RegionHeader) };
                region_header.num_mailboxes = region_header.num_mailboxes.max((i + 1) as u32);

                self.mmap.flush().ok();
                return Ok(i);
            }
        }

        bail!("No available mailbox slots")
    }

    /// Get a mailbox by AI ID
    pub fn get_mailbox(&mut self, ai_id: &str) -> Option<Mailbox> {
        let base = self.mmap.as_mut_ptr();

        for i in 0..MAX_MAILBOXES {
            let meta = unsafe { Self::mailbox_meta_ptr(base, i) };
            let meta_ref = unsafe { &*meta };

            if meta_ref.is_active() && meta_ref.ai_id_str() == ai_id {
                let header = unsafe { Self::mailbox_header_ptr(base, i) };
                let data = unsafe { Self::mailbox_data_ptr(base, i, self.mailbox_buffer_size) };

                return Some(unsafe {
                    Mailbox::from_raw(meta, header, data, self.mailbox_buffer_size)
                });
            }
        }

        None
    }

    /// Get a mailbox by index
    pub fn get_mailbox_by_index(&mut self, index: usize) -> Option<Mailbox> {
        if index >= MAX_MAILBOXES {
            return None;
        }

        let base = self.mmap.as_mut_ptr();
        let meta = unsafe { Self::mailbox_meta_ptr(base, index) };
        let meta_ref = unsafe { &*meta };

        if !meta_ref.is_active() {
            return None;
        }

        let header = unsafe { Self::mailbox_header_ptr(base, index) };
        let data = unsafe { Self::mailbox_data_ptr(base, index, self.mailbox_buffer_size) };

        Some(unsafe { Mailbox::from_raw(meta, header, data, self.mailbox_buffer_size) })
    }

    /// List all active mailboxes
    pub fn list_mailboxes(&self) -> Vec<(usize, String)> {
        let base = self.mmap.as_ptr();
        let mut result = Vec::new();

        for i in 0..MAX_MAILBOXES {
            let meta = unsafe { &*(Self::mailbox_meta_ptr(base as *mut u8, i)) };
            if meta.is_active() {
                result.push((i, meta.ai_id_str().to_string()));
            }
        }

        result
    }

    /// Deregister a mailbox
    pub fn deregister_mailbox(&mut self, ai_id: &str) -> bool {
        let base = self.mmap.as_mut_ptr();

        for i in 0..MAX_MAILBOXES {
            let meta = unsafe { &mut *Self::mailbox_meta_ptr(base, i) };
            if meta.is_active() && meta.ai_id_str() == ai_id {
                meta.status = 0;
                self.mmap.flush().ok();
                return true;
            }
        }

        false
    }

    /// Get region statistics
    pub fn stats(&self) -> RegionStats {
        let header = unsafe { &*(self.mmap.as_ptr() as *const RegionHeader) };
        let active = self.list_mailboxes().len();

        RegionStats {
            total_size: header.region_size as usize,
            mailbox_buffer_size: self.mailbox_buffer_size,
            active_mailboxes: active,
            max_mailboxes: MAX_MAILBOXES,
            created_at: header.created_at,
        }
    }
}

/// Statistics about the shared region
#[derive(Debug, Clone)]
pub struct RegionStats {
    pub total_size: usize,
    pub mailbox_buffer_size: usize,
    pub active_mailboxes: usize,
    pub max_mailboxes: usize,
    pub created_at: u64,
}

impl std::fmt::Display for RegionStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SharedRegion: {}KB total, {}KB/mailbox, {}/{} active",
            self.total_size / 1024,
            self.mailbox_buffer_size / 1024,
            self.active_mailboxes,
            self.max_mailboxes
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_region_create_and_register() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.shm");

        let mut region = SharedRegion::open(Some(path), 4096).unwrap();

        // Register mailbox
        let idx = region.register_mailbox("test-ai").unwrap();
        assert_eq!(idx, 0);

        // List should show it
        let mailboxes = region.list_mailboxes();
        assert_eq!(mailboxes.len(), 1);
        assert_eq!(mailboxes[0].1, "test-ai");

        // Get mailbox
        let mailbox = region.get_mailbox("test-ai");
        assert!(mailbox.is_some());
    }
}
