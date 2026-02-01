//! Bulletin Board - Zero-latency awareness data sharing
//!
//! The daemon writes awareness data to a fixed location in shared memory.
//! Hooks read directly from memory - no request/response needed.
//!
//! Latency: ~100ns (memory read) vs ~150ms (subprocess + HTTP)

use memmap2::{MmapMut, MmapOptions};
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use anyhow::{Context, Result};

/// Magic number for bulletin board
const BULLETIN_MAGIC: u64 = 0x4255_4C4C_4554_494E; // "BULLETIN"

/// Version of bulletin board format
/// v3: Added timestamps to DM/Broadcast
/// v4: MAX_STRING_LEN increased from 1024 to 8192 (struct layout change)
/// v5: Path fields increased from 128 to 512 bytes (NO TRUNCATION policy)
const BULLETIN_VERSION: u32 = 5;

/// Maximum DMs in bulletin
const MAX_DMS: usize = 10;
/// Maximum file actions in bulletin
const MAX_FILE_ACTIONS: usize = 10;
/// Maximum presence entries in bulletin
const MAX_PRESENCE: usize = 8;
/// Maximum broadcasts in bulletin
const MAX_BROADCASTS: usize = 10;
/// Maximum votes in bulletin
const MAX_VOTES: usize = 10;
/// Maximum dialogues in bulletin
const MAX_DIALOGUES: usize = 5;
/// Maximum locks in bulletin
const MAX_LOCKS: usize = 10;
/// Maximum string length - NO TRUNCATION policy
/// Context starvation is worse than context bloat. Truncation renders features dangerous.
/// 8KB per message allows full AI-to-AI communication without loss.
const MAX_STRING_LEN: usize = 8192;

/// Maximum path length - NO TRUNCATION policy
/// Windows long paths can be ~32K, but 512 handles all practical cases.
/// Typical project paths are 100-300 chars.
const MAX_PATH_LEN: usize = 512;

/// Header for the bulletin board
#[repr(C)]
pub struct BulletinHeader {
    /// Magic number
    pub magic: u64,
    /// Version
    pub version: u32,
    /// Sequence number (incremented on each update)
    pub sequence: AtomicU64,
    /// Last update timestamp (unix millis)
    pub last_update: u64,
    /// Number of DMs
    pub dm_count: u32,
    /// Number of broadcasts
    pub broadcast_count: u32,
    /// Number of votes
    pub vote_count: u32,
    /// Number of dialogues
    pub dialogue_count: u32,
    /// Number of locks
    pub lock_count: u32,
    /// Number of file actions (v2)
    pub file_action_count: u32,
    /// Number of presence entries (v2)
    pub presence_count: u32,
    /// Padding
    _padding: [u8; 8],
}

impl BulletinHeader {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    pub fn is_valid(&self) -> bool {
        self.magic == BULLETIN_MAGIC && self.version == BULLETIN_VERSION
    }
}

/// A DM entry in the bulletin
#[repr(C)]
pub struct DmEntry {
    /// Message ID
    pub id: i64,
    /// Created timestamp (unix seconds)
    pub created_at: i64,
    /// From AI (null-terminated)
    pub from_ai: [u8; 32],
    /// To AI (null-terminated) - recipient for filtering
    pub to_ai: [u8; 32],
    /// Content (null-terminated)
    pub content: [u8; MAX_STRING_LEN],
}

impl DmEntry {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    pub fn to_ai_str(&self) -> &str {
        let end = self.to_ai.iter().position(|&b| b == 0).unwrap_or(32);
        std::str::from_utf8(&self.to_ai[..end]).unwrap_or("")
    }

    pub fn from_ai_str(&self) -> &str {
        let end = self.from_ai.iter().position(|&b| b == 0).unwrap_or(32);
        std::str::from_utf8(&self.from_ai[..end]).unwrap_or("")
    }

    pub fn content_str(&self) -> &str {
        let end = self.content.iter().position(|&b| b == 0).unwrap_or(MAX_STRING_LEN);
        std::str::from_utf8(&self.content[..end]).unwrap_or("")
    }

    pub fn set_to_ai(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(31);
        self.to_ai[..len].copy_from_slice(&bytes[..len]);
        self.to_ai[len] = 0;
    }

    pub fn set_from_ai(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(31);
        self.from_ai[..len].copy_from_slice(&bytes[..len]);
        self.from_ai[len] = 0;
    }

    pub fn set_content(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(MAX_STRING_LEN - 1);
        self.content[..len].copy_from_slice(&bytes[..len]);
        self.content[len] = 0;
    }
}

/// A broadcast entry in the bulletin
#[repr(C)]
pub struct BroadcastEntry {
    /// Message ID
    pub id: i64,
    /// Created timestamp (unix seconds)
    pub created_at: i64,
    /// From AI (null-terminated)
    pub from_ai: [u8; 32],
    /// To AI (null-terminated) - recipient for filtering
    pub to_ai: [u8; 32],
    /// Channel (null-terminated)
    pub channel: [u8; 32],
    /// Content (null-terminated)
    pub content: [u8; MAX_STRING_LEN],
}

impl BroadcastEntry {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    pub fn to_ai_str(&self) -> &str {
        let end = self.to_ai.iter().position(|&b| b == 0).unwrap_or(32);
        std::str::from_utf8(&self.to_ai[..end]).unwrap_or("")
    }

    pub fn from_ai_str(&self) -> &str {
        let end = self.from_ai.iter().position(|&b| b == 0).unwrap_or(32);
        std::str::from_utf8(&self.from_ai[..end]).unwrap_or("")
    }

    pub fn channel_str(&self) -> &str {
        let end = self.channel.iter().position(|&b| b == 0).unwrap_or(32);
        std::str::from_utf8(&self.channel[..end]).unwrap_or("")
    }

    pub fn content_str(&self) -> &str {
        let end = self.content.iter().position(|&b| b == 0).unwrap_or(MAX_STRING_LEN);
        std::str::from_utf8(&self.content[..end]).unwrap_or("")
    }

    pub fn set_to_ai(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(31);
        self.to_ai[..len].copy_from_slice(&bytes[..len]);
        self.to_ai[len] = 0;
    }

    pub fn set_from_ai(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(31);
        self.from_ai[..len].copy_from_slice(&bytes[..len]);
        self.from_ai[len] = 0;
    }

    pub fn set_channel(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(31);
        self.channel[..len].copy_from_slice(&bytes[..len]);
        self.channel[len] = 0;
    }

    pub fn set_content(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(MAX_STRING_LEN - 1);
        self.content[..len].copy_from_slice(&bytes[..len]);
        self.content[len] = 0;
    }
}

/// A vote entry in the bulletin
#[repr(C)]
pub struct VoteEntry {
    /// Vote ID
    pub id: i64,
    /// Topic (null-terminated)
    pub topic: [u8; 64],
    /// Votes cast
    pub cast: u32,
    /// Total required
    pub total: u32,
}

impl VoteEntry {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    pub fn topic_str(&self) -> &str {
        let end = self.topic.iter().position(|&b| b == 0).unwrap_or(64);
        std::str::from_utf8(&self.topic[..end]).unwrap_or("")
    }

    pub fn set_topic(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(63);
        self.topic[..len].copy_from_slice(&bytes[..len]);
        self.topic[len] = 0;
    }
}

/// A dialogue entry in the bulletin
#[repr(C)]
pub struct DialogueEntry {
    /// Session ID
    pub id: i64,
    /// Topic (null-terminated)
    pub topic: [u8; 64],
}

impl DialogueEntry {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    pub fn topic_str(&self) -> &str {
        let end = self.topic.iter().position(|&b| b == 0).unwrap_or(64);
        std::str::from_utf8(&self.topic[..end]).unwrap_or("")
    }

    pub fn set_topic(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(63);
        self.topic[..len].copy_from_slice(&bytes[..len]);
        self.topic[len] = 0;
    }
}

/// A lock entry in the bulletin
#[repr(C)]
pub struct LockEntry {
    /// Resource path (null-terminated) - NO TRUNCATION
    pub resource: [u8; MAX_PATH_LEN],
    /// Owner AI (null-terminated)
    pub owner: [u8; 32],
    /// Working on (null-terminated)
    pub working_on: [u8; 64],
}

impl LockEntry {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    pub fn resource_str(&self) -> &str {
        let end = self.resource.iter().position(|&b| b == 0).unwrap_or(MAX_PATH_LEN);
        std::str::from_utf8(&self.resource[..end]).unwrap_or("")
    }

    pub fn owner_str(&self) -> &str {
        let end = self.owner.iter().position(|&b| b == 0).unwrap_or(32);
        std::str::from_utf8(&self.owner[..end]).unwrap_or("")
    }

    pub fn working_on_str(&self) -> &str {
        let end = self.working_on.iter().position(|&b| b == 0).unwrap_or(64);
        std::str::from_utf8(&self.working_on[..end]).unwrap_or("")
    }

    pub fn set_resource(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(MAX_PATH_LEN - 1);
        self.resource[..len].copy_from_slice(&bytes[..len]);
        self.resource[len] = 0;
    }

    pub fn set_owner(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(31);
        self.owner[..len].copy_from_slice(&bytes[..len]);
        self.owner[len] = 0;
    }

    pub fn set_working_on(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(63);
        self.working_on[..len].copy_from_slice(&bytes[..len]);
        self.working_on[len] = 0;
    }
}

/// A file action entry in the bulletin (v2)
#[repr(C)]
pub struct FileActionEntry {
    /// AI that performed the action (null-terminated)
    pub ai_id: [u8; 32],
    /// Action type: Read, Write, Edit, etc. (null-terminated)
    pub action: [u8; 16],
    /// File path (null-terminated) - NO TRUNCATION
    pub file_path: [u8; MAX_PATH_LEN],
    /// Timestamp (unix millis)
    pub timestamp: u64,
}

impl FileActionEntry {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    pub fn ai_id_str(&self) -> &str {
        let end = self.ai_id.iter().position(|&b| b == 0).unwrap_or(32);
        std::str::from_utf8(&self.ai_id[..end]).unwrap_or("")
    }

    pub fn action_str(&self) -> &str {
        let end = self.action.iter().position(|&b| b == 0).unwrap_or(16);
        std::str::from_utf8(&self.action[..end]).unwrap_or("")
    }

    pub fn file_path_str(&self) -> &str {
        let end = self.file_path.iter().position(|&b| b == 0).unwrap_or(MAX_PATH_LEN);
        std::str::from_utf8(&self.file_path[..end]).unwrap_or("")
    }

    pub fn set_ai_id(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(31);
        self.ai_id[..len].copy_from_slice(&bytes[..len]);
        self.ai_id[len] = 0;
    }

    pub fn set_action(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(15);
        self.action[..len].copy_from_slice(&bytes[..len]);
        self.action[len] = 0;
    }

    pub fn set_file_path(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(MAX_PATH_LEN - 1);
        self.file_path[..len].copy_from_slice(&bytes[..len]);
        self.file_path[len] = 0;
    }
}

/// A presence entry in the bulletin (v2)
#[repr(C)]
pub struct PresenceEntry {
    /// AI ID (null-terminated)
    pub ai_id: [u8; 32],
    /// Status: active, standby, idle (null-terminated)
    pub status: [u8; 16],
    /// Current task (null-terminated)
    pub current_task: [u8; 64],
    /// Last seen timestamp (unix millis)
    pub last_seen: u64,
}

impl PresenceEntry {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    pub fn ai_id_str(&self) -> &str {
        let end = self.ai_id.iter().position(|&b| b == 0).unwrap_or(32);
        std::str::from_utf8(&self.ai_id[..end]).unwrap_or("")
    }

    pub fn status_str(&self) -> &str {
        let end = self.status.iter().position(|&b| b == 0).unwrap_or(16);
        std::str::from_utf8(&self.status[..end]).unwrap_or("")
    }

    pub fn current_task_str(&self) -> &str {
        let end = self.current_task.iter().position(|&b| b == 0).unwrap_or(64);
        std::str::from_utf8(&self.current_task[..end]).unwrap_or("")
    }

    pub fn set_ai_id(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(31);
        self.ai_id[..len].copy_from_slice(&bytes[..len]);
        self.ai_id[len] = 0;
    }

    pub fn set_status(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(15);
        self.status[..len].copy_from_slice(&bytes[..len]);
        self.status[len] = 0;
    }

    pub fn set_current_task(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(63);
        self.current_task[..len].copy_from_slice(&bytes[..len]);
        self.current_task[len] = 0;
    }
}

/// Calculate total bulletin size
const fn bulletin_size() -> usize {
    BulletinHeader::SIZE
        + MAX_DMS * DmEntry::SIZE
        + MAX_BROADCASTS * BroadcastEntry::SIZE
        + MAX_VOTES * VoteEntry::SIZE
        + MAX_DIALOGUES * DialogueEntry::SIZE
        + MAX_LOCKS * LockEntry::SIZE
        + MAX_FILE_ACTIONS * FileActionEntry::SIZE
        + MAX_PRESENCE * PresenceEntry::SIZE
}

/// The bulletin board - shared memory for awareness data
pub struct BulletinBoard {
    mmap: MmapMut,
    path: PathBuf,
}

impl BulletinBoard {
    /// Get the default bulletin path
    pub fn default_path() -> PathBuf {
        let base = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join(".ai-foundation").join("shm").join("bulletin.shm")
    }

    /// Open or create the bulletin board
    pub fn open(path: Option<PathBuf>) -> Result<Self> {
        let path = path.unwrap_or_else(Self::default_path);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .context("Failed to create bulletin directory")?;
        }

        let size = bulletin_size();
        let file_exists = path.exists();

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .context("Failed to open bulletin file")?;

        // Set file size if new
        if !file_exists || file.metadata()?.len() < size as u64 {
            file.set_len(size as u64)
                .context("Failed to set bulletin size")?;
        }

        let mut mmap = unsafe {
            MmapOptions::new()
                .len(size)
                .map_mut(&file)
                .context("Failed to memory-map bulletin")?
        };

        // Initialize header if new
        let header = unsafe { &mut *(mmap.as_mut_ptr() as *mut BulletinHeader) };
        if !header.is_valid() {
            header.magic = BULLETIN_MAGIC;
            header.version = BULLETIN_VERSION;
            header.sequence = AtomicU64::new(0);
            header.last_update = 0;
            header.dm_count = 0;
            header.broadcast_count = 0;
            header.vote_count = 0;
            header.dialogue_count = 0;
            header.lock_count = 0;
            header.file_action_count = 0;
            header.presence_count = 0;
            mmap.flush().context("Failed to flush bulletin")?;
        }

        Ok(Self { mmap, path })
    }

    /// Get the header (read-only)
    pub fn header(&self) -> &BulletinHeader {
        unsafe { &*(self.mmap.as_ptr() as *const BulletinHeader) }
    }

    /// Get the header (mutable)
    fn header_mut(&mut self) -> &mut BulletinHeader {
        unsafe { &mut *(self.mmap.as_mut_ptr() as *mut BulletinHeader) }
    }

    /// Get DM entries slice
    pub fn dms(&self) -> &[DmEntry] {
        let count = self.header().dm_count as usize;
        let ptr = unsafe {
            self.mmap.as_ptr().add(BulletinHeader::SIZE) as *const DmEntry
        };
        unsafe { std::slice::from_raw_parts(ptr, count.min(MAX_DMS)) }
    }

    /// Get broadcast entries slice
    pub fn broadcasts(&self) -> &[BroadcastEntry] {
        let count = self.header().broadcast_count as usize;
        let offset = BulletinHeader::SIZE + MAX_DMS * DmEntry::SIZE;
        let ptr = unsafe {
            self.mmap.as_ptr().add(offset) as *const BroadcastEntry
        };
        unsafe { std::slice::from_raw_parts(ptr, count.min(MAX_BROADCASTS)) }
    }

    /// Get vote entries slice
    pub fn votes(&self) -> &[VoteEntry] {
        let count = self.header().vote_count as usize;
        let offset = BulletinHeader::SIZE
            + MAX_DMS * DmEntry::SIZE
            + MAX_BROADCASTS * BroadcastEntry::SIZE;
        let ptr = unsafe {
            self.mmap.as_ptr().add(offset) as *const VoteEntry
        };
        unsafe { std::slice::from_raw_parts(ptr, count.min(MAX_VOTES)) }
    }

    /// Get dialogue entries slice
    pub fn dialogues(&self) -> &[DialogueEntry] {
        let count = self.header().dialogue_count as usize;
        let offset = BulletinHeader::SIZE
            + MAX_DMS * DmEntry::SIZE
            + MAX_BROADCASTS * BroadcastEntry::SIZE
            + MAX_VOTES * VoteEntry::SIZE;
        let ptr = unsafe {
            self.mmap.as_ptr().add(offset) as *const DialogueEntry
        };
        unsafe { std::slice::from_raw_parts(ptr, count.min(MAX_DIALOGUES)) }
    }

    /// Get lock entries slice
    pub fn locks(&self) -> &[LockEntry] {
        let count = self.header().lock_count as usize;
        let offset = BulletinHeader::SIZE
            + MAX_DMS * DmEntry::SIZE
            + MAX_BROADCASTS * BroadcastEntry::SIZE
            + MAX_VOTES * VoteEntry::SIZE
            + MAX_DIALOGUES * DialogueEntry::SIZE;
        let ptr = unsafe {
            self.mmap.as_ptr().add(offset) as *const LockEntry
        };
        unsafe { std::slice::from_raw_parts(ptr, count.min(MAX_LOCKS)) }
    }

    /// Get file action entries slice (v2)
    pub fn file_actions(&self) -> &[FileActionEntry] {
        let count = self.header().file_action_count as usize;
        let offset = BulletinHeader::SIZE
            + MAX_DMS * DmEntry::SIZE
            + MAX_BROADCASTS * BroadcastEntry::SIZE
            + MAX_VOTES * VoteEntry::SIZE
            + MAX_DIALOGUES * DialogueEntry::SIZE
            + MAX_LOCKS * LockEntry::SIZE;
        let ptr = unsafe {
            self.mmap.as_ptr().add(offset) as *const FileActionEntry
        };
        unsafe { std::slice::from_raw_parts(ptr, count.min(MAX_FILE_ACTIONS)) }
    }

    /// Get presence entries slice (v2)
    pub fn presences(&self) -> &[PresenceEntry] {
        let count = self.header().presence_count as usize;
        let offset = BulletinHeader::SIZE
            + MAX_DMS * DmEntry::SIZE
            + MAX_BROADCASTS * BroadcastEntry::SIZE
            + MAX_VOTES * VoteEntry::SIZE
            + MAX_DIALOGUES * DialogueEntry::SIZE
            + MAX_LOCKS * LockEntry::SIZE
            + MAX_FILE_ACTIONS * FileActionEntry::SIZE;
        let ptr = unsafe {
            self.mmap.as_ptr().add(offset) as *const PresenceEntry
        };
        unsafe { std::slice::from_raw_parts(ptr, count.min(MAX_PRESENCE)) }
    }

    /// Update DMs (daemon only)
    /// Format: (id, created_at_secs, from_ai, to_ai, content)
    /// Automatically deduplicates by ID (keeps first occurrence)
    pub fn set_dms(&mut self, dms: &[(i64, i64, &str, &str, &str)]) {
        use std::collections::HashSet;

        let ptr = unsafe {
            self.mmap.as_mut_ptr().add(BulletinHeader::SIZE) as *mut DmEntry
        };

        // Deduplicate by ID, keeping first occurrence
        let mut seen_ids: HashSet<i64> = HashSet::new();
        let mut write_idx = 0;

        for (id, created_at, from, to, content) in dms.iter() {
            if write_idx >= MAX_DMS {
                break;
            }
            if seen_ids.insert(*id) {
                // New ID, write it
                let entry = unsafe { &mut *ptr.add(write_idx) };
                entry.id = *id;
                entry.created_at = *created_at;
                entry.set_from_ai(from);
                entry.set_to_ai(to);
                entry.set_content(content);
                write_idx += 1;
            }
        }

        self.header_mut().dm_count = write_idx as u32;
    }

    /// Update broadcasts (daemon only)
    /// Format: (id, created_at_secs, from_ai, channel, content)
    /// Automatically deduplicates by ID (keeps first occurrence)
    pub fn set_broadcasts(&mut self, broadcasts: &[(i64, i64, &str, &str, &str)]) {
        use std::collections::HashSet;

        let offset = BulletinHeader::SIZE + MAX_DMS * DmEntry::SIZE;
        let ptr = unsafe {
            self.mmap.as_mut_ptr().add(offset) as *mut BroadcastEntry
        };

        // Deduplicate by ID, keeping first occurrence
        let mut seen_ids: HashSet<i64> = HashSet::new();
        let mut write_idx = 0;

        for (id, created_at, from, channel, content) in broadcasts.iter() {
            if write_idx >= MAX_BROADCASTS {
                break;
            }
            if seen_ids.insert(*id) {
                // New ID, write it
                let entry = unsafe { &mut *ptr.add(write_idx) };
                entry.id = *id;
                entry.created_at = *created_at;
                entry.set_from_ai(from);
                entry.set_channel(channel);
                entry.set_content(content);
                write_idx += 1;
            }
        }

        self.header_mut().broadcast_count = write_idx as u32;
    }

    /// Update votes (daemon only)
    pub fn set_votes(&mut self, votes: &[(i64, &str, u32, u32)]) {
        let count = votes.len().min(MAX_VOTES);
        let offset = BulletinHeader::SIZE
            + MAX_DMS * DmEntry::SIZE
            + MAX_BROADCASTS * BroadcastEntry::SIZE;
        let ptr = unsafe {
            self.mmap.as_mut_ptr().add(offset) as *mut VoteEntry
        };

        for (i, (id, topic, cast, total)) in votes.iter().take(count).enumerate() {
            let entry = unsafe { &mut *ptr.add(i) };
            entry.id = *id;
            entry.set_topic(topic);
            entry.cast = *cast;
            entry.total = *total;
        }

        self.header_mut().vote_count = count as u32;
    }

    /// Update dialogues (daemon only)
    pub fn set_dialogues(&mut self, dialogues: &[(i64, &str)]) {
        let count = dialogues.len().min(MAX_DIALOGUES);
        let offset = BulletinHeader::SIZE
            + MAX_DMS * DmEntry::SIZE
            + MAX_BROADCASTS * BroadcastEntry::SIZE
            + MAX_VOTES * VoteEntry::SIZE;
        let ptr = unsafe {
            self.mmap.as_mut_ptr().add(offset) as *mut DialogueEntry
        };

        for (i, (id, topic)) in dialogues.iter().take(count).enumerate() {
            let entry = unsafe { &mut *ptr.add(i) };
            entry.id = *id;
            entry.set_topic(topic);
        }

        self.header_mut().dialogue_count = count as u32;
    }

    /// Update locks (daemon only)
    pub fn set_locks(&mut self, locks: &[(&str, &str, &str)]) {
        let count = locks.len().min(MAX_LOCKS);
        let offset = BulletinHeader::SIZE
            + MAX_DMS * DmEntry::SIZE
            + MAX_BROADCASTS * BroadcastEntry::SIZE
            + MAX_VOTES * VoteEntry::SIZE
            + MAX_DIALOGUES * DialogueEntry::SIZE;
        let ptr = unsafe {
            self.mmap.as_mut_ptr().add(offset) as *mut LockEntry
        };

        for (i, (resource, owner, working_on)) in locks.iter().take(count).enumerate() {
            let entry = unsafe { &mut *ptr.add(i) };
            entry.set_resource(resource);
            entry.set_owner(owner);
            entry.set_working_on(working_on);
        }

        self.header_mut().lock_count = count as u32;
    }

    /// Update file actions (daemon only) - v2
    /// Format: (ai_id, action, file_path, timestamp_millis)
    pub fn set_file_actions(&mut self, actions: &[(&str, &str, &str, u64)]) {
        let count = actions.len().min(MAX_FILE_ACTIONS);
        let offset = BulletinHeader::SIZE
            + MAX_DMS * DmEntry::SIZE
            + MAX_BROADCASTS * BroadcastEntry::SIZE
            + MAX_VOTES * VoteEntry::SIZE
            + MAX_DIALOGUES * DialogueEntry::SIZE
            + MAX_LOCKS * LockEntry::SIZE;
        let ptr = unsafe {
            self.mmap.as_mut_ptr().add(offset) as *mut FileActionEntry
        };

        for (i, (ai_id, action, path, ts)) in actions.iter().take(count).enumerate() {
            let entry = unsafe { &mut *ptr.add(i) };
            entry.set_ai_id(ai_id);
            entry.set_action(action);
            entry.set_file_path(path);
            entry.timestamp = *ts;
        }

        self.header_mut().file_action_count = count as u32;
    }

    /// Update presence entries (daemon only) - v2
    /// Format: (ai_id, status, current_task, last_seen_millis)
    pub fn set_presences(&mut self, presences: &[(&str, &str, &str, u64)]) {
        let count = presences.len().min(MAX_PRESENCE);
        let offset = BulletinHeader::SIZE
            + MAX_DMS * DmEntry::SIZE
            + MAX_BROADCASTS * BroadcastEntry::SIZE
            + MAX_VOTES * VoteEntry::SIZE
            + MAX_DIALOGUES * DialogueEntry::SIZE
            + MAX_LOCKS * LockEntry::SIZE
            + MAX_FILE_ACTIONS * FileActionEntry::SIZE;
        let ptr = unsafe {
            self.mmap.as_mut_ptr().add(offset) as *mut PresenceEntry
        };

        for (i, (ai_id, status, task, last_seen)) in presences.iter().take(count).enumerate() {
            let entry = unsafe { &mut *ptr.add(i) };
            entry.set_ai_id(ai_id);
            entry.set_status(status);
            entry.set_current_task(task);
            entry.last_seen = *last_seen;
        }

        self.header_mut().presence_count = count as u32;
    }

    /// Commit updates (increment sequence, update timestamp, flush)
    pub fn commit(&mut self) -> Result<()> {
        let header = self.header_mut();
        header.sequence.fetch_add(1, Ordering::SeqCst);
        header.last_update = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.mmap.flush().context("Failed to flush bulletin")
    }

    /// Format relative time (e.g., "3d", "2h", "5m", "now")
    fn format_relative_time(created_at_secs: i64, now_secs: u64) -> String {
        if created_at_secs <= 0 {
            return String::new(); // No timestamp available
        }
        let age_secs = now_secs.saturating_sub(created_at_secs as u64);
        if age_secs < 60 {
            "now".to_string()
        } else if age_secs < 3600 {
            format!("{}m", age_secs / 60)
        } else if age_secs < 86400 {
            format!("{}h", age_secs / 3600)
        } else {
            format!("{}d", age_secs / 86400)
        }
    }

    /// Format as pipe-delimited output for hooks
    /// IMPORTANT: NO TRUNCATION - full content preserves context and AI collaboration effectiveness
    pub fn to_hook_output(&self) -> String {
        let mut parts = Vec::new();

        // UTC timestamp - inject current time for temporal awareness
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let hours = (now % 86400) / 3600;
        let minutes = (now % 3600) / 60;
        let time_str = format!("UTC {:02}:{:02}", hours, minutes);

        // DMs - FULL content, no truncation
        for dm in self.dms() {
            parts.push(format!("{}:\"{}\"", dm.from_ai_str(), dm.content_str()));
        }

        let dm_output = if !parts.is_empty() {
            Some(format!("|DMs|{}", parts.join(" | ")))
        } else {
            None
        };
        parts.clear();

        // Broadcasts - FULL content with relative timestamp
        for bc in self.broadcasts() {
            let age = Self::format_relative_time(bc.created_at, now);
            if age.is_empty() {
                parts.push(format!("[{}] {} {}: {}", bc.id, bc.from_ai_str(), bc.channel_str(), bc.content_str()));
            } else {
                parts.push(format!("[{}] {} ({}) {}: {}", bc.id, bc.from_ai_str(), age, bc.channel_str(), bc.content_str()));
            }
        }

        let bc_output = if !parts.is_empty() {
            Some(format!("|BROADCASTS|{}", parts.join(" | ")))
        } else {
            None
        };
        parts.clear();

        // Votes - full topic
        for vote in self.votes() {
            let pct = if vote.total > 0 { vote.cast * 100 / vote.total } else { 0 };
            parts.push(format!("[{}] {} ({}%)", vote.id, vote.topic_str(), pct));
        }

        let vote_output = if !parts.is_empty() {
            Some(format!("|VOTES|{}", parts.join(" | ")))
        } else {
            None
        };
        parts.clear();

        // Detangles - full topic
        for det in self.dialogues() {
            parts.push(format!("[{}] {}", det.id, det.topic_str()));
        }

        let det_output = if !parts.is_empty() {
            Some(format!("|YOUR TURN|{}", parts.join(", ")))
        } else {
            None
        };
        parts.clear();

        // Locks - FULL resource path, no truncation
        for lock in self.locks() {
            parts.push(format!("{}->{}:{}", lock.owner_str(), lock.resource_str(), lock.working_on_str()));
        }

        let lock_output = if !parts.is_empty() {
            Some(format!("|LOCKS|{}", parts.join(", ")))
        } else {
            None
        };
        parts.clear();

        // File Actions (v2) - FULL path, no truncation
        for fa in self.file_actions() {
            parts.push(format!("{}:{} {}", fa.ai_id_str(), fa.action_str(), fa.file_path_str()));
        }

        let file_action_output = if !parts.is_empty() {
            Some(format!("|FILES|{}", parts.join(" | ")))
        } else {
            None
        };
        parts.clear();

        // Presence (v2) - FULL task, no truncation
        for p in self.presences() {
            let task = p.current_task_str();
            let task_display = if task.is_empty() { "-" } else { task };
            parts.push(format!("{}[{}]:{}", p.ai_id_str(), p.status_str(), task_display));
        }

        let presence_output = if !parts.is_empty() {
            Some(format!("|TEAM|{}", parts.join(" | ")))
        } else {
            None
        };

        // Combine all outputs with UTC time first
        let mut all_parts = vec![time_str];
        all_parts.extend([dm_output, bc_output, vote_output, det_output, lock_output, file_action_output, presence_output]
            .into_iter()
            .flatten());
        all_parts.join(" | ")
    }

    /// Get sequence number for change detection
    pub fn sequence(&self) -> u64 {
        self.header().sequence.load(Ordering::SeqCst)
    }

    /// Get last update timestamp
    pub fn last_update(&self) -> u64 {
        self.header().last_update
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_bulletin_create_and_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_bulletin.shm");

        // Create and write
        {
            let mut board = BulletinBoard::open(Some(path.clone())).unwrap();
            board.set_dms(&[
                (1, 0, "lyra-584", "sage-724", "Hello Sage!"),
                (2, 0, "cascade-230", "lyra-584", "Test message"),
            ]);
            board.set_votes(&[
                (1, "Should we use Rust?", 3, 4),
            ]);
            board.commit().unwrap();
        }

        // Read back
        {
            let board = BulletinBoard::open(Some(path)).unwrap();
            let dms = board.dms();
            assert_eq!(dms.len(), 2);
            assert_eq!(dms[0].from_ai_str(), "lyra-584");
            assert_eq!(dms[0].content_str(), "Hello Sage!");

            let votes = board.votes();
            assert_eq!(votes.len(), 1);
            assert_eq!(votes[0].topic_str(), "Should we use Rust?");
        }
    }

    #[test]
    fn test_hook_output_format() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_bulletin.shm");

        let mut board = BulletinBoard::open(Some(path)).unwrap();
        board.set_dms(&[(1, 0, "lyra-584", "sage-724", "URGENT: Need help!")]);
        board.set_votes(&[(1, "Approve PR?", 2, 4)]);
        board.commit().unwrap();

        let output = board.to_hook_output();
        assert!(output.contains("|DMs|"));
        assert!(output.contains("lyra-584"));
        assert!(output.contains("|VOTES|"));
    }
}
