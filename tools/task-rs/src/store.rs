//! TaskStore - Engram-style storage for AI tasks
//!
//! High-performance task storage with:
//! - Memory-mapped I/O for fast reads
//! - In-memory indexes for O(1) status filtering
//! - Priority heap for O(log n) get-next-task
//! - Pre-computed stats (no SQL parsing)
//! - Index persistence for O(1) startup

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use memmap2::Mmap;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

// ============================================================================
// CONSTANTS
// ============================================================================

/// Magic bytes: "TELOS\0\0\0" - matching Engram's 'EGRM' pattern
const MAGIC: [u8; 8] = [0x54, 0x45, 0x4C, 0x4F, 0x53, 0x00, 0x00, 0x00];

/// File format version
const VERSION: u32 = 1;

/// Header size (fixed)
const HEADER_SIZE: usize = 128;

/// Index section magic: "TASKIDX\0"
const INDEX_MAGIC: u64 = 0x5441534B49445800;

// ============================================================================
// ENUMS
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TaskStatus {
    Pending = 0,
    InProgress = 1,
    Completed = 2,
    Verified = 3,
    Blocked = 4,
}

impl TaskStatus {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => TaskStatus::InProgress,
            2 => TaskStatus::Completed,
            3 => TaskStatus::Verified,
            4 => TaskStatus::Blocked,
            _ => TaskStatus::Pending,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Completed => "completed",
            TaskStatus::Verified => "verified",
            TaskStatus::Blocked => "blocked",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "in_progress" => TaskStatus::InProgress,
            "completed" => TaskStatus::Completed,
            "verified" => TaskStatus::Verified,
            "blocked" => TaskStatus::Blocked,
            _ => TaskStatus::Pending,
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "Pending",
            TaskStatus::InProgress => "In Progress",
            TaskStatus::Completed => "Completed",
            TaskStatus::Verified => "Verified",
            TaskStatus::Blocked => "Blocked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TaskPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl TaskPriority {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => TaskPriority::Low,
            2 => TaskPriority::High,
            3 => TaskPriority::Critical,
            _ => TaskPriority::Normal,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TaskPriority::Low => "low",
            TaskPriority::Normal => "normal",
            TaskPriority::High => "high",
            TaskPriority::Critical => "critical",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "low" => TaskPriority::Low,
            "high" => TaskPriority::High,
            "critical" => TaskPriority::Critical,
            _ => TaskPriority::Normal,
        }
    }

    pub fn marker(&self) -> &'static str {
        match self {
            TaskPriority::Low => "",
            TaskPriority::Normal => "",
            TaskPriority::High => " [high]",
            TaskPriority::Critical => " [CRITICAL]",
        }
    }
}

// ============================================================================
// TASK STRUCT
// ============================================================================

/// In-memory task representation
#[derive(Debug, Clone)]
pub struct Task {
    pub id: u64,
    pub content: String,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub blocked_reason: Option<String>,
    pub note_id: Option<i64>,
}

/// Task entry for binary serialization (fixed header + variable data)
struct TaskEntry {
    // Header (48 bytes)
    id: u64,              // 8 bytes
    created_at: i64,      // 8 bytes (unix nanos)
    updated_at: i64,      // 8 bytes (unix nanos)
    status: u8,           // 1 byte
    priority: u8,         // 1 byte
    flags: u16,           // 2 bytes (deleted=1, has_blocked=2)
    content_len: u32,     // 4 bytes
    blocked_len: u16,     // 2 bytes
    note_id: i64,         // 8 bytes (0 = none)
    _reserved: [u8; 6],   // 6 bytes padding to 48

    // Variable data
    content: String,
    blocked_reason: Option<String>,
}

impl TaskEntry {
    const HEADER_SIZE: usize = 48;
    const FLAG_DELETED: u16 = 1;
    const FLAG_HAS_BLOCKED: u16 = 2;

    fn from_task(task: &Task) -> Self {
        let mut flags = 0u16;
        if task.blocked_reason.is_some() {
            flags |= Self::FLAG_HAS_BLOCKED;
        }

        Self {
            id: task.id,
            created_at: task.created.timestamp_nanos_opt().unwrap_or(0),
            updated_at: task.updated.timestamp_nanos_opt().unwrap_or(0),
            status: task.status as u8,
            priority: task.priority as u8,
            flags,
            content_len: task.content.len() as u32,
            blocked_len: task.blocked_reason.as_ref().map(|s| s.len() as u16).unwrap_or(0),
            note_id: task.note_id.unwrap_or(0),
            _reserved: [0; 6],
            content: task.content.clone(),
            blocked_reason: task.blocked_reason.clone(),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::HEADER_SIZE + self.content.len() + self.blocked_len as usize);

        buf.extend_from_slice(&self.id.to_le_bytes());
        buf.extend_from_slice(&self.created_at.to_le_bytes());
        buf.extend_from_slice(&self.updated_at.to_le_bytes());
        buf.push(self.status);
        buf.push(self.priority);
        buf.extend_from_slice(&self.flags.to_le_bytes());
        buf.extend_from_slice(&self.content_len.to_le_bytes());
        buf.extend_from_slice(&self.blocked_len.to_le_bytes());
        buf.extend_from_slice(&self.note_id.to_le_bytes());
        buf.extend_from_slice(&self._reserved);

        buf.extend_from_slice(self.content.as_bytes());
        if let Some(ref reason) = self.blocked_reason {
            buf.extend_from_slice(reason.as_bytes());
        }

        buf
    }

    fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < Self::HEADER_SIZE {
            anyhow::bail!("Task entry too small");
        }

        let id = u64::from_le_bytes(data[0..8].try_into()?);
        let created_at = i64::from_le_bytes(data[8..16].try_into()?);
        let updated_at = i64::from_le_bytes(data[16..24].try_into()?);
        let status = data[24];
        let priority = data[25];
        let flags = u16::from_le_bytes(data[26..28].try_into()?);
        let content_len = u32::from_le_bytes(data[28..32].try_into()?) as usize;
        let blocked_len = u16::from_le_bytes(data[32..34].try_into()?) as usize;
        let note_id = i64::from_le_bytes(data[34..42].try_into()?);

        let content_start = Self::HEADER_SIZE;
        let content_end = content_start + content_len;
        if data.len() < content_end + blocked_len {
            anyhow::bail!("Task entry data truncated");
        }

        let content = String::from_utf8_lossy(&data[content_start..content_end]).to_string();
        let blocked_reason = if blocked_len > 0 {
            Some(String::from_utf8_lossy(&data[content_end..content_end + blocked_len]).to_string())
        } else {
            None
        };

        Ok(Self {
            id,
            created_at,
            updated_at,
            status,
            priority,
            flags,
            content_len: content_len as u32,
            blocked_len: blocked_len as u16,
            note_id,
            _reserved: [0; 6],
            content,
            blocked_reason,
        })
    }

    fn to_task(&self) -> Task {
        Task {
            id: self.id,
            content: self.content.clone(),
            status: TaskStatus::from_u8(self.status),
            priority: TaskPriority::from_u8(self.priority),
            created: DateTime::from_timestamp_nanos(self.created_at),
            updated: DateTime::from_timestamp_nanos(self.updated_at),
            blocked_reason: self.blocked_reason.clone(),
            note_id: if self.note_id != 0 { Some(self.note_id) } else { None },
        }
    }

    fn is_deleted(&self) -> bool {
        self.flags & Self::FLAG_DELETED != 0
    }

    #[allow(dead_code)] // Kept for future memory management/serialization
    fn total_size(&self) -> usize {
        Self::HEADER_SIZE + self.content_len as usize + self.blocked_len as usize
    }
}

// ============================================================================
// PRIORITY QUEUE ENTRY
// ============================================================================

/// Entry for the priority heap (higher priority + older tasks first)
/// Per Lyra's review: use created_at as tiebreaker (older tasks waited longer)
#[derive(Debug, Clone, Eq, PartialEq)]
struct PriorityEntry {
    priority: TaskPriority,
    created: i64,  // Older tasks (lower timestamp) come first
    id: u64,
}

impl Ord for PriorityEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Higher priority first, then OLDER tasks first (waited longer)
        self.priority.cmp(&other.priority)
            .then_with(|| other.created.cmp(&self.created)) // Note: reversed for older-first
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PartialOrd for PriorityEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// ============================================================================
// STATS
// ============================================================================

/// Pre-computed task statistics (updated on write, not computed on read)
#[derive(Debug, Clone, Default)]
pub struct TaskStats {
    pub pending: u64,
    pub in_progress: u64,
    pub completed: u64,
    pub verified: u64,
    pub blocked: u64,
}

impl TaskStats {
    pub fn total(&self) -> u64 {
        self.pending + self.in_progress + self.completed + self.verified + self.blocked
    }

    fn increment(&mut self, status: TaskStatus) {
        match status {
            TaskStatus::Pending => self.pending += 1,
            TaskStatus::InProgress => self.in_progress += 1,
            TaskStatus::Completed => self.completed += 1,
            TaskStatus::Verified => self.verified += 1,
            TaskStatus::Blocked => self.blocked += 1,
        }
    }

    fn decrement(&mut self, status: TaskStatus) {
        match status {
            TaskStatus::Pending => self.pending = self.pending.saturating_sub(1),
            TaskStatus::InProgress => self.in_progress = self.in_progress.saturating_sub(1),
            TaskStatus::Completed => self.completed = self.completed.saturating_sub(1),
            TaskStatus::Verified => self.verified = self.verified.saturating_sub(1),
            TaskStatus::Blocked => self.blocked = self.blocked.saturating_sub(1),
        }
    }
}

// ============================================================================
// FILE HEADER
// ============================================================================

/// File header (128 bytes fixed)
struct FileHeader {
    magic: [u8; 8],           // 8 bytes: "TASKENGR"
    version: u32,             // 4 bytes
    ai_id_hash: u32,          // 4 bytes (for quick validation)
    created_at: i64,          // 8 bytes
    modified_at: i64,         // 8 bytes
    task_count: u64,          // 8 bytes
    active_tasks: u64,        // 8 bytes
    log_offset: u64,          // 8 bytes
    log_size: u64,            // 8 bytes
    index_offset: u64,        // 8 bytes (0 = no persisted index)
    index_size: u64,          // 8 bytes
    flags: u32,               // 4 bytes
    _reserved: [u8; 36],      // 36 bytes padding to 128
}

impl FileHeader {
    const FLAG_HAS_INDEX: u32 = 1;

    fn new(ai_id: &str) -> Self {
        let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        Self {
            magic: MAGIC,
            version: VERSION,
            ai_id_hash: crc32fast::hash(ai_id.as_bytes()),
            created_at: now,
            modified_at: now,
            task_count: 0,
            active_tasks: 0,
            log_offset: HEADER_SIZE as u64,
            log_size: 0,
            index_offset: 0,
            index_size: 0,
            flags: 0,
            _reserved: [0; 36],
        }
    }

    fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..8].copy_from_slice(&self.magic);
        buf[8..12].copy_from_slice(&self.version.to_le_bytes());
        buf[12..16].copy_from_slice(&self.ai_id_hash.to_le_bytes());
        buf[16..24].copy_from_slice(&self.created_at.to_le_bytes());
        buf[24..32].copy_from_slice(&self.modified_at.to_le_bytes());
        buf[32..40].copy_from_slice(&self.task_count.to_le_bytes());
        buf[40..48].copy_from_slice(&self.active_tasks.to_le_bytes());
        buf[48..56].copy_from_slice(&self.log_offset.to_le_bytes());
        buf[56..64].copy_from_slice(&self.log_size.to_le_bytes());
        buf[64..72].copy_from_slice(&self.index_offset.to_le_bytes());
        buf[72..80].copy_from_slice(&self.index_size.to_le_bytes());
        buf[80..84].copy_from_slice(&self.flags.to_le_bytes());
        buf
    }

    fn from_bytes(data: &[u8; HEADER_SIZE]) -> Result<Self> {
        if &data[0..8] != MAGIC {
            anyhow::bail!("Invalid magic bytes");
        }

        Ok(Self {
            magic: MAGIC,
            version: u32::from_le_bytes(data[8..12].try_into()?),
            ai_id_hash: u32::from_le_bytes(data[12..16].try_into()?),
            created_at: i64::from_le_bytes(data[16..24].try_into()?),
            modified_at: i64::from_le_bytes(data[24..32].try_into()?),
            task_count: u64::from_le_bytes(data[32..40].try_into()?),
            active_tasks: u64::from_le_bytes(data[40..48].try_into()?),
            log_offset: u64::from_le_bytes(data[48..56].try_into()?),
            log_size: u64::from_le_bytes(data[56..64].try_into()?),
            index_offset: u64::from_le_bytes(data[64..72].try_into()?),
            index_size: u64::from_le_bytes(data[72..80].try_into()?),
            flags: u32::from_le_bytes(data[80..84].try_into()?),
            _reserved: [0; 36],
        })
    }

    fn write_to(&self, file: &mut File) -> Result<()> {
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&self.to_bytes())?;
        Ok(())
    }

    fn touch(&mut self) {
        self.modified_at = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    }
}

// ============================================================================
// TASK STORE
// ============================================================================

/// High-performance task storage with Engram-style optimizations
#[allow(dead_code)] // path and ai_id kept for diagnostics/future use
pub struct TaskStore {
    path: PathBuf,
    file: File,
    mmap: Option<Mmap>,
    mmap_valid_size: u64,

    header: FileHeader,
    ai_id: String,

    // In-memory cache (all tasks - they're small)
    tasks: HashMap<u64, Task>,

    // Status indexes for O(1) filtering
    by_status: HashMap<TaskStatus, HashSet<u64>>,

    // Priority queue for O(log n) get-next-task (pending + in_progress only)
    priority_queue: BinaryHeap<PriorityEntry>,

    // Pre-computed stats
    stats: TaskStats,

    // Next ID
    next_id: u64,
}

impl TaskStore {
    /// Open or create a TaskStore
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if path.exists() {
            Self::open_existing(&path)
        } else {
            Self::create_new(&path)
        }
    }

    /// Get the default store path for an AI
    pub fn default_path() -> PathBuf {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".to_string());
        let ai_id = Self::get_ai_id();
        let dir = PathBuf::from(home).join(".ai-foundation");
        std::fs::create_dir_all(&dir).ok();
        dir.join(format!("tasks-{}.engram", ai_id))
    }

    /// Get AI ID from environment
    pub fn get_ai_id() -> String {
        std::env::var("AI_ID")
            .or_else(|_| std::env::var("AGENT_ID"))
            .unwrap_or_else(|_| "default".to_string())
    }

    fn create_new(path: &Path) -> Result<Self> {
        let ai_id = Self::get_ai_id();

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .context("Failed to create task store")?;

        let header = FileHeader::new(&ai_id);
        header.write_to(&mut file)?;
        file.set_len(HEADER_SIZE as u64)?;
        file.sync_all()?;

        Ok(Self {
            path: path.to_path_buf(),
            file,
            mmap: None,
            mmap_valid_size: 0,
            header,
            ai_id,
            tasks: HashMap::new(),
            by_status: HashMap::new(),
            priority_queue: BinaryHeap::new(),
            stats: TaskStats::default(),
            next_id: 1,
        })
    }

    fn open_existing(path: &Path) -> Result<Self> {
        let ai_id = Self::get_ai_id();

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .context("Failed to open task store")?;

        // Read header
        let mut header_buf = [0u8; HEADER_SIZE];
        file.read_exact(&mut header_buf)?;
        let header = FileHeader::from_bytes(&header_buf)?;

        // Verify AI ID
        if header.ai_id_hash != crc32fast::hash(ai_id.as_bytes()) {
            anyhow::bail!("Task store belongs to different AI");
        }

        // Create mmap
        let file_len = file.metadata()?.len();
        let mmap = if file_len > HEADER_SIZE as u64 {
            Some(unsafe { Mmap::map(&file)? })
        } else {
            None
        };
        let mmap_valid_size = if mmap.is_some() { file_len } else { 0 };

        let mut store = Self {
            path: path.to_path_buf(),
            file,
            mmap,
            mmap_valid_size,
            header,
            ai_id,
            tasks: HashMap::new(),
            by_status: HashMap::new(),
            priority_queue: BinaryHeap::new(),
            stats: TaskStats::default(),
            next_id: 1,
        };

        // Try to load persisted indexes, fall back to rebuild
        if !store.load_persisted_indexes()? {
            store.rebuild_indexes()?;
        }

        Ok(store)
    }

    /// Rebuild indexes by scanning the task log
    fn rebuild_indexes(&mut self) -> Result<()> {
        self.tasks.clear();
        self.by_status.clear();
        self.priority_queue.clear();
        self.stats = TaskStats::default();
        self.next_id = 1;

        if self.header.log_size == 0 {
            return Ok(());
        }

        // Use mmap for fast scanning if available
        // Take mmap out temporarily to avoid borrow conflict (Engram pattern)
        let has_mmap = self.mmap.is_some();
        if has_mmap {
            let mmap = self.mmap.take().unwrap();
            self.rebuild_indexes_mmap(&mmap)?;
            self.mmap = Some(mmap);
        } else {
            self.rebuild_indexes_file()?;
        }

        // Rebuild priority queue from active tasks
        self.rebuild_priority_queue();

        Ok(())
    }

    fn rebuild_indexes_mmap(&mut self, mmap: &Mmap) -> Result<()> {
        let mut offset = self.header.log_offset as usize;
        let end_offset = offset + self.header.log_size as usize;

        while offset + TaskEntry::HEADER_SIZE <= end_offset && offset + TaskEntry::HEADER_SIZE <= mmap.len() {
            // Read entry header to get size
            let content_len = u32::from_le_bytes(mmap[offset + 28..offset + 32].try_into()?) as usize;
            let blocked_len = u16::from_le_bytes(mmap[offset + 32..offset + 34].try_into()?) as usize;
            let entry_size = TaskEntry::HEADER_SIZE + content_len + blocked_len;

            if offset + entry_size > mmap.len() {
                break;
            }

            let entry = TaskEntry::from_bytes(&mmap[offset..offset + entry_size])?;

            // Update next_id
            if entry.id >= self.next_id {
                self.next_id = entry.id + 1;
            }

            // Skip deleted entries
            if !entry.is_deleted() {
                let task = entry.to_task();
                self.index_task(&task);
            }

            offset += entry_size;
        }

        Ok(())
    }

    fn rebuild_indexes_file(&mut self) -> Result<()> {
        let mut offset = self.header.log_offset;
        let end_offset = offset + self.header.log_size;

        while offset < end_offset {
            self.file.seek(SeekFrom::Start(offset))?;

            let mut header_buf = [0u8; TaskEntry::HEADER_SIZE];
            if self.file.read_exact(&mut header_buf).is_err() {
                break;
            }

            let content_len = u32::from_le_bytes(header_buf[28..32].try_into()?) as usize;
            let blocked_len = u16::from_le_bytes(header_buf[32..34].try_into()?) as usize;
            let entry_size = TaskEntry::HEADER_SIZE + content_len + blocked_len;

            // Read full entry
            let mut full_buf = vec![0u8; entry_size];
            full_buf[..TaskEntry::HEADER_SIZE].copy_from_slice(&header_buf);
            self.file.seek(SeekFrom::Start(offset + TaskEntry::HEADER_SIZE as u64))?;
            self.file.read_exact(&mut full_buf[TaskEntry::HEADER_SIZE..])?;

            let entry = TaskEntry::from_bytes(&full_buf)?;

            if entry.id >= self.next_id {
                self.next_id = entry.id + 1;
            }

            if !entry.is_deleted() {
                let task = entry.to_task();
                self.index_task(&task);
            }

            offset += entry_size as u64;
        }

        Ok(())
    }

    /// Index a task in all data structures
    fn index_task(&mut self, task: &Task) {
        let id = task.id;

        // Add to main cache
        self.tasks.insert(id, task.clone());

        // Add to status index
        self.by_status
            .entry(task.status)
            .or_insert_with(HashSet::new)
            .insert(id);

        // Update stats
        self.stats.increment(task.status);
    }

    /// Remove a task from indexes
    fn unindex_task(&mut self, id: u64) -> Option<Task> {
        if let Some(task) = self.tasks.remove(&id) {
            // Remove from status index
            if let Some(set) = self.by_status.get_mut(&task.status) {
                set.remove(&id);
            }
            // Update stats
            self.stats.decrement(task.status);
            Some(task)
        } else {
            None
        }
    }

    /// Rebuild priority queue from active tasks
    fn rebuild_priority_queue(&mut self) {
        self.priority_queue.clear();

        for (id, task) in &self.tasks {
            // Only pending and in_progress tasks go in priority queue
            if task.status == TaskStatus::Pending || task.status == TaskStatus::InProgress {
                self.priority_queue.push(PriorityEntry {
                    priority: task.priority,
                    created: task.created.timestamp_nanos_opt().unwrap_or(0),
                    id: *id,
                });
            }
        }
    }

    // ========================================================================
    // WRITE OPERATIONS
    // ========================================================================

    /// Add a new task
    pub fn add(&mut self, content: &str, priority: TaskPriority, note_id: Option<i64>) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;

        let now = Utc::now();
        let task = Task {
            id,
            content: content.to_string(),
            status: TaskStatus::Pending,
            priority,
            created: now,
            updated: now,
            blocked_reason: None,
            note_id,
        };

        // Write to file
        let entry = TaskEntry::from_task(&task);
        self.write_entry(&entry)?;

        // Update indexes
        self.index_task(&task);

        // Add to priority queue
        self.priority_queue.push(PriorityEntry {
            priority: task.priority,
            created: now.timestamp_nanos_opt().unwrap_or(0),
            id,
        });

        // Update header
        self.header.task_count += 1;
        self.header.active_tasks += 1;
        self.header.touch();
        self.header.write_to(&mut self.file)?;

        Ok(id)
    }

    /// Update task status
    pub fn update_status(&mut self, id: u64, new_status: TaskStatus, blocked_reason: Option<&str>) -> Result<bool> {
        let task = match self.tasks.get(&id) {
            Some(t) => t.clone(),
            None => return Ok(false),
        };

        let old_status = task.status;

        // Create updated task
        let mut updated = task.clone();
        updated.status = new_status;
        updated.updated = Utc::now();
        updated.blocked_reason = blocked_reason.map(|s| s.to_string());

        // Write updated entry
        let entry = TaskEntry::from_task(&updated);
        self.write_entry(&entry)?;

        // Update indexes
        if let Some(set) = self.by_status.get_mut(&old_status) {
            set.remove(&id);
        }
        self.by_status
            .entry(new_status)
            .or_insert_with(HashSet::new)
            .insert(id);

        // Update stats
        self.stats.decrement(old_status);
        self.stats.increment(new_status);

        // Update cache
        self.tasks.insert(id, updated.clone());

        // Rebuild priority queue if status affects it
        let was_active = old_status == TaskStatus::Pending || old_status == TaskStatus::InProgress;
        let is_active = new_status == TaskStatus::Pending || new_status == TaskStatus::InProgress;
        if was_active != is_active {
            self.rebuild_priority_queue();
        }

        // Update header
        self.header.touch();
        self.header.write_to(&mut self.file)?;

        Ok(true)
    }

    /// Delete a task
    pub fn delete(&mut self, id: u64) -> Result<bool> {
        let task = match self.unindex_task(id) {
            Some(t) => t,
            None => return Ok(false),
        };

        // Write tombstone entry
        let mut entry = TaskEntry::from_task(&task);
        entry.flags |= TaskEntry::FLAG_DELETED;
        self.write_entry(&entry)?;

        // Rebuild priority queue
        self.rebuild_priority_queue();

        // Update header
        self.header.active_tasks = self.header.active_tasks.saturating_sub(1);
        self.header.touch();
        self.header.write_to(&mut self.file)?;

        Ok(true)
    }

    fn write_entry(&mut self, entry: &TaskEntry) -> Result<()> {
        // Invalidate persisted indexes
        self.invalidate_persisted_indexes()?;

        let bytes = entry.to_bytes();

        // Seek to end of log
        let write_offset = self.header.log_offset + self.header.log_size;
        self.file.seek(SeekFrom::Start(write_offset))?;
        self.file.write_all(&bytes)?;

        // Update header
        self.header.log_size += bytes.len() as u64;

        Ok(())
    }

    // ========================================================================
    // READ OPERATIONS
    // ========================================================================

    /// Get a task by ID (O(1))
    pub fn get(&self, id: u64) -> Option<&Task> {
        self.tasks.get(&id)
    }

    /// Get tasks by status (O(k) where k = number of matching tasks)
    pub fn by_status(&self, status: TaskStatus) -> Vec<&Task> {
        self.by_status
            .get(&status)
            .map(|ids| ids.iter().filter_map(|id| self.tasks.get(id)).collect())
            .unwrap_or_default()
    }

    /// Get the next highest priority active task (O(1) peek, may need rebuilds)
    pub fn next_priority(&self) -> Option<&Task> {
        // Find highest priority task that's still active
        for entry in self.priority_queue.iter() {
            if let Some(task) = self.tasks.get(&entry.id) {
                if task.status == TaskStatus::Pending || task.status == TaskStatus::InProgress {
                    return Some(task);
                }
            }
        }
        None
    }

    /// Get active tasks (pending + in_progress + blocked)
    pub fn active_tasks(&self) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self.tasks
            .values()
            .filter(|t| {
                t.status == TaskStatus::Pending ||
                t.status == TaskStatus::InProgress ||
                t.status == TaskStatus::Blocked
            })
            .collect();

        // Sort: in_progress first, then blocked, then pending, then by updated desc
        tasks.sort_by(|a, b| {
            let status_ord = |s: TaskStatus| match s {
                TaskStatus::InProgress => 0,
                TaskStatus::Blocked => 1,
                TaskStatus::Pending => 2,
                _ => 3,
            };
            status_ord(a.status).cmp(&status_ord(b.status))
                .then_with(|| b.updated.cmp(&a.updated))
        });

        tasks
    }

    /// Get completed tasks (completed + verified)
    pub fn completed_tasks(&self) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self.tasks
            .values()
            .filter(|t| t.status == TaskStatus::Completed || t.status == TaskStatus::Verified)
            .collect();
        tasks.sort_by(|a, b| b.updated.cmp(&a.updated));
        tasks
    }

    /// Get all tasks sorted
    pub fn all_tasks(&self) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self.tasks.values().collect();
        tasks.sort_by(|a, b| {
            let status_ord = |s: TaskStatus| match s {
                TaskStatus::InProgress => 0,
                TaskStatus::Blocked => 1,
                TaskStatus::Pending => 2,
                TaskStatus::Completed => 3,
                TaskStatus::Verified => 4,
            };
            status_ord(a.status).cmp(&status_ord(b.status))
                .then_with(|| b.updated.cmp(&a.updated))
        });
        tasks
    }

    /// Get pre-computed stats (O(1))
    pub fn stats(&self) -> &TaskStats {
        &self.stats
    }

    // ========================================================================
    // INDEX PERSISTENCE
    // ========================================================================

    /// Persist indexes for O(1) startup
    pub fn persist_indexes(&mut self) -> Result<()> {
        let index_data = self.serialize_indexes();
        let checksum = crc32fast::hash(&index_data);

        let mut section = Vec::with_capacity(12 + index_data.len());
        section.extend_from_slice(&INDEX_MAGIC.to_le_bytes());
        section.extend_from_slice(&checksum.to_le_bytes());
        section.extend_from_slice(&index_data);

        let write_offset = self.header.log_offset + self.header.log_size;
        self.file.seek(SeekFrom::Start(write_offset))?;
        self.file.write_all(&section)?;

        self.header.index_offset = write_offset;
        self.header.index_size = section.len() as u64;
        self.header.flags |= FileHeader::FLAG_HAS_INDEX;
        self.header.touch();
        self.header.write_to(&mut self.file)?;

        self.file.sync_all()?;
        self.remap()?;

        Ok(())
    }

    fn serialize_indexes(&self) -> Vec<u8> {
        let mut data = Vec::new();

        // next_id
        data.extend_from_slice(&self.next_id.to_le_bytes());

        // stats
        data.extend_from_slice(&self.stats.pending.to_le_bytes());
        data.extend_from_slice(&self.stats.in_progress.to_le_bytes());
        data.extend_from_slice(&self.stats.completed.to_le_bytes());
        data.extend_from_slice(&self.stats.verified.to_le_bytes());
        data.extend_from_slice(&self.stats.blocked.to_le_bytes());

        // task count
        data.extend_from_slice(&(self.tasks.len() as u64).to_le_bytes());

        // Serialize each task
        for task in self.tasks.values() {
            let entry = TaskEntry::from_task(task);
            let bytes = entry.to_bytes();
            data.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            data.extend_from_slice(&bytes);
        }

        data
    }

    fn load_persisted_indexes(&mut self) -> Result<bool> {
        if self.header.flags & FileHeader::FLAG_HAS_INDEX == 0 {
            return Ok(false);
        }

        if self.header.index_size == 0 {
            return Ok(false);
        }

        self.file.seek(SeekFrom::Start(self.header.index_offset))?;

        let section_size = self.header.index_size as usize;
        if section_size < 12 {
            return Ok(false);
        }

        let mut section = vec![0u8; section_size];
        self.file.read_exact(&mut section)?;

        // Verify magic
        let magic = u64::from_le_bytes(section[0..8].try_into()?);
        if magic != INDEX_MAGIC {
            return Ok(false);
        }

        // Verify checksum
        let stored_checksum = u32::from_le_bytes(section[8..12].try_into()?);
        let computed_checksum = crc32fast::hash(&section[12..]);
        if stored_checksum != computed_checksum {
            return Ok(false);
        }

        self.deserialize_indexes(&section[12..])?;
        self.rebuild_priority_queue();

        Ok(true)
    }

    fn deserialize_indexes(&mut self, data: &[u8]) -> Result<()> {
        let mut cursor = 0;

        // next_id
        self.next_id = u64::from_le_bytes(data[cursor..cursor + 8].try_into()?);
        cursor += 8;

        // stats
        self.stats.pending = u64::from_le_bytes(data[cursor..cursor + 8].try_into()?);
        cursor += 8;
        self.stats.in_progress = u64::from_le_bytes(data[cursor..cursor + 8].try_into()?);
        cursor += 8;
        self.stats.completed = u64::from_le_bytes(data[cursor..cursor + 8].try_into()?);
        cursor += 8;
        self.stats.verified = u64::from_le_bytes(data[cursor..cursor + 8].try_into()?);
        cursor += 8;
        self.stats.blocked = u64::from_le_bytes(data[cursor..cursor + 8].try_into()?);
        cursor += 8;

        // task count
        let task_count = u64::from_le_bytes(data[cursor..cursor + 8].try_into()?) as usize;
        cursor += 8;

        // Deserialize tasks
        self.tasks.clear();
        self.by_status.clear();

        for _ in 0..task_count {
            let entry_len = u32::from_le_bytes(data[cursor..cursor + 4].try_into()?) as usize;
            cursor += 4;

            let entry = TaskEntry::from_bytes(&data[cursor..cursor + entry_len])?;
            let task = entry.to_task();

            self.tasks.insert(task.id, task.clone());
            self.by_status
                .entry(task.status)
                .or_insert_with(HashSet::new)
                .insert(task.id);

            cursor += entry_len;
        }

        Ok(())
    }

    fn invalidate_persisted_indexes(&mut self) -> Result<()> {
        if self.header.flags & FileHeader::FLAG_HAS_INDEX != 0 {
            self.header.flags &= !FileHeader::FLAG_HAS_INDEX;
            self.header.index_size = 0;
            self.header.touch();
            self.header.write_to(&mut self.file)?;
        }
        Ok(())
    }

    fn remap(&mut self) -> Result<()> {
        let file_len = self.file.metadata()?.len();
        if file_len > HEADER_SIZE as u64 {
            self.mmap = Some(unsafe { Mmap::map(&self.file)? });
            self.mmap_valid_size = file_len;
        }
        Ok(())
    }

    /// Force sync to disk
    pub fn sync(&mut self) -> Result<()> {
        self.file.sync_all()?;
        self.remap()
    }

    // ========================================================================
    // MIGRATION FROM SQLITE
    // ========================================================================

    /// Get the legacy SQLite database path
    pub fn legacy_sqlite_path() -> PathBuf {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".ai-foundation").join("tasks.db")
    }

    /// Check if legacy SQLite database exists
    pub fn has_legacy_sqlite() -> bool {
        Self::legacy_sqlite_path().exists()
    }

    /// Migrate tasks from legacy SQLite to Telos format
    /// Returns the number of tasks migrated
    pub fn migrate_from_sqlite(&mut self) -> Result<u64> {
        use rusqlite::{params, Connection};

        let sqlite_path = Self::legacy_sqlite_path();
        if !sqlite_path.exists() {
            return Ok(0);
        }

        let conn = Connection::open(&sqlite_path)
            .context("Failed to open legacy SQLite database")?;

        let ai_id = Self::get_ai_id();

        // Query all tasks for this AI
        let mut stmt = conn.prepare(
            "SELECT id, content, status, priority, created_at, updated_at, blocked_reason, note_id
             FROM tasks WHERE ai_id = ?1 ORDER BY id ASC"
        )?;

        let rows = stmt.query_map(params![ai_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,           // id
                row.get::<_, String>(1)?,        // content
                row.get::<_, String>(2)?,        // status
                row.get::<_, String>(3)?,        // priority
                row.get::<_, String>(4)?,        // created_at
                row.get::<_, String>(5)?,        // updated_at
                row.get::<_, Option<String>>(6)?, // blocked_reason
                row.get::<_, Option<i64>>(7)?,   // note_id
            ))
        })?;

        let mut count = 0u64;
        let mut max_id = 0u64;

        for row_result in rows {
            let (id, content, status_str, priority_str, created_str, updated_str, blocked_reason, note_id) =
                row_result.context("Failed to read task row")?;

            // Parse timestamps (RFC3339 format)
            let created = chrono::DateTime::parse_from_rfc3339(&created_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let updated = chrono::DateTime::parse_from_rfc3339(&updated_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            // Parse enums
            let status = TaskStatus::from_str(&status_str);
            let priority = TaskPriority::from_str(&priority_str);

            // Create task with original ID
            let task = Task {
                id: id as u64,
                content,
                status,
                priority,
                created,
                updated,
                blocked_reason,
                note_id,
            };

            // Import the task
            self.import_task(task)?;
            count += 1;

            if id as u64 > max_id {
                max_id = id as u64;
            }
        }

        // Update next_id to be after the highest imported ID
        if max_id >= self.next_id {
            self.next_id = max_id + 1;
        }

        // Rebuild priority queue with imported tasks
        self.rebuild_priority_queue();

        // Update header
        self.header.task_count = count;
        self.header.active_tasks = self.tasks.len() as u64;
        self.header.touch();
        self.header.write_to(&mut self.file)?;

        // Persist indexes for fast startup
        self.persist_indexes()?;

        Ok(count)
    }

    /// Import a task with a specific ID (used during migration)
    fn import_task(&mut self, task: Task) -> Result<()> {
        let id = task.id;

        // Write to file
        let entry = TaskEntry::from_task(&task);
        self.write_entry(&entry)?;

        // Update indexes
        self.index_task(&task);

        // Track max ID
        if id >= self.next_id {
            self.next_id = id + 1;
        }

        Ok(())
    }

    /// Auto-migrate from SQLite if needed (call on first open)
    /// Returns (store, tasks_migrated)
    pub fn open_with_migration(path: impl AsRef<Path>) -> Result<(Self, u64)> {
        let path = path.as_ref().to_path_buf();
        let is_new = !path.exists();

        let mut store = Self::open(&path)?;

        // If new store and legacy SQLite exists, migrate
        let migrated = if is_new && Self::has_legacy_sqlite() {
            store.migrate_from_sqlite()?
        } else {
            0
        };

        Ok((store, migrated))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_create_and_open() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        {
            let store = TaskStore::open(&path).unwrap();
            assert_eq!(store.stats().total(), 0);
        }

        {
            let store = TaskStore::open(&path).unwrap();
            assert_eq!(store.stats().total(), 0);
        }
    }

    #[test]
    fn test_add_and_get() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut store = TaskStore::open(&path).unwrap();

        let id = store.add("Test task", TaskPriority::Normal, None).unwrap();
        assert_eq!(id, 1);

        let task = store.get(id).unwrap();
        assert_eq!(task.content, "Test task");
        assert_eq!(task.status, TaskStatus::Pending);
        assert_eq!(task.priority, TaskPriority::Normal);

        assert_eq!(store.stats().pending, 1);
        assert_eq!(store.stats().total(), 1);
    }

    #[test]
    fn test_status_update() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut store = TaskStore::open(&path).unwrap();

        let id = store.add("Test task", TaskPriority::High, None).unwrap();
        assert_eq!(store.stats().pending, 1);

        store.update_status(id, TaskStatus::InProgress, None).unwrap();
        assert_eq!(store.stats().pending, 0);
        assert_eq!(store.stats().in_progress, 1);

        store.update_status(id, TaskStatus::Completed, None).unwrap();
        assert_eq!(store.stats().in_progress, 0);
        assert_eq!(store.stats().completed, 1);
    }

    #[test]
    fn test_priority_queue() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let mut store = TaskStore::open(&path).unwrap();

        store.add("Low priority", TaskPriority::Low, None).unwrap();
        store.add("Critical task", TaskPriority::Critical, None).unwrap();
        store.add("Normal task", TaskPriority::Normal, None).unwrap();

        let next = store.next_priority().unwrap();
        assert_eq!(next.content, "Critical task");
    }

    #[test]
    fn test_persistence() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        let id = {
            let mut store = TaskStore::open(&path).unwrap();
            let id = store.add("Persistent task", TaskPriority::High, None).unwrap();
            store.update_status(id, TaskStatus::InProgress, None).unwrap();
            store.sync().unwrap();
            id
        };

        {
            let store = TaskStore::open(&path).unwrap();
            let task = store.get(id).unwrap();
            assert_eq!(task.content, "Persistent task");
            assert_eq!(task.status, TaskStatus::InProgress);
        }
    }

    #[test]
    fn test_index_persistence() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.engram");

        {
            let mut store = TaskStore::open(&path).unwrap();
            store.add("Task 1", TaskPriority::Normal, None).unwrap();
            store.add("Task 2", TaskPriority::High, None).unwrap();
            store.add("Task 3", TaskPriority::Critical, None).unwrap();
            store.persist_indexes().unwrap();
        }

        {
            let store = TaskStore::open(&path).unwrap();
            assert_eq!(store.stats().pending, 3);
            let next = store.next_priority().unwrap();
            assert_eq!(next.content, "Task 3"); // Critical priority
        }
    }

    #[test]
    fn test_sqlite_migration() {
        use rusqlite::{params, Connection};

        let dir = tempdir().unwrap();
        let sqlite_path = dir.path().join("tasks.db");
        let telos_path = dir.path().join("tasks.engram");

        // Create and populate SQLite database
        {
            let conn = Connection::open(&sqlite_path).unwrap();
            conn.execute(
                "CREATE TABLE tasks (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    ai_id TEXT NOT NULL,
                    content TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'pending',
                    priority TEXT NOT NULL DEFAULT 'normal',
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    blocked_reason TEXT,
                    note_id INTEGER
                )",
                [],
            ).unwrap();

            let now = Utc::now().to_rfc3339();
            let ai_id = "default"; // Match TaskStore::get_ai_id() default

            // Insert test tasks
            conn.execute(
                "INSERT INTO tasks (ai_id, content, status, priority, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![ai_id, "Migrate me 1", "pending", "normal", &now, &now],
            ).unwrap();
            conn.execute(
                "INSERT INTO tasks (ai_id, content, status, priority, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![ai_id, "Migrate me 2", "in_progress", "high", &now, &now],
            ).unwrap();
            conn.execute(
                "INSERT INTO tasks (ai_id, content, status, priority, created_at, updated_at, blocked_reason)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![ai_id, "Migrate me 3", "blocked", "critical", &now, &now, "Waiting for review"],
            ).unwrap();
        }

        // Create Telos store and manually migrate (since we can't override the sqlite path in test)
        {
            let mut store = TaskStore::open(&telos_path).unwrap();

            // Manual migration using rusqlite directly
            let conn = Connection::open(&sqlite_path).unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, content, status, priority, created_at, updated_at, blocked_reason, note_id
                 FROM tasks WHERE ai_id = ?1 ORDER BY id ASC"
            ).unwrap();

            let rows = stmt.query_map(params!["default"], |row| {
                Ok((
                    row.get::<_, i64>(0).unwrap(),
                    row.get::<_, String>(1).unwrap(),
                    row.get::<_, String>(2).unwrap(),
                    row.get::<_, String>(3).unwrap(),
                    row.get::<_, String>(4).unwrap(),
                    row.get::<_, String>(5).unwrap(),
                    row.get::<_, Option<String>>(6).unwrap(),
                    row.get::<_, Option<i64>>(7).unwrap(),
                ))
            }).unwrap();

            for row in rows {
                let (id, content, status_str, priority_str, created_str, updated_str, blocked_reason, note_id) = row.unwrap();

                let created = chrono::DateTime::parse_from_rfc3339(&created_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let updated = chrono::DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                let task = Task {
                    id: id as u64,
                    content,
                    status: TaskStatus::from_str(&status_str),
                    priority: TaskPriority::from_str(&priority_str),
                    created,
                    updated,
                    blocked_reason,
                    note_id,
                };

                // Write to file
                let entry = TaskEntry::from_task(&task);
                store.write_entry(&entry).unwrap();
                store.index_task(&task);
                if task.id as u64 >= store.next_id {
                    store.next_id = task.id as u64 + 1;
                }
            }

            store.rebuild_priority_queue();
            store.persist_indexes().unwrap();
        }

        // Verify migration
        {
            let store = TaskStore::open(&telos_path).unwrap();

            // Check stats
            assert_eq!(store.stats().pending, 1);
            assert_eq!(store.stats().in_progress, 1);
            assert_eq!(store.stats().blocked, 1);
            assert_eq!(store.stats().total(), 3);

            // Check task content
            let task1 = store.get(1).unwrap();
            assert_eq!(task1.content, "Migrate me 1");
            assert_eq!(task1.status, TaskStatus::Pending);

            let task2 = store.get(2).unwrap();
            assert_eq!(task2.content, "Migrate me 2");
            assert_eq!(task2.status, TaskStatus::InProgress);
            assert_eq!(task2.priority, TaskPriority::High);

            let task3 = store.get(3).unwrap();
            assert_eq!(task3.content, "Migrate me 3");
            assert_eq!(task3.status, TaskStatus::Blocked);
            assert_eq!(task3.blocked_reason, Some("Waiting for review".to_string()));

            // Check priority queue (Critical blocked task shouldn't be in queue, High in_progress should be next)
            let next = store.next_priority().unwrap();
            assert_eq!(next.content, "Migrate me 2"); // High priority in_progress
        }
    }
}
