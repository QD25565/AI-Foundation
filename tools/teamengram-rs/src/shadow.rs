//! Shadow Paging Allocator
//!
//! Implements copy-on-write semantics for atomic commits.
//! When a page needs to be modified:
//! 1. Allocate a new page (shadow)
//! 2. Copy contents and modify
//! 3. Update parent pointers to shadow
//! 4. On commit: swap root pointer atomically
//! 5. Old pages go to free list

use crate::page::{Page, PageId, PAGE_SIZE, NULL_PAGE, MetaPage, PageType};
use memmap2::MmapMut;
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::path::Path;
use anyhow::{Context, Result, bail};
use fs2::FileExt; // Cross-process file locking for SWMR

/// Shadow page allocator
/// Manages page allocation with copy-on-write semantics
pub struct ShadowAllocator {
    /// Memory-mapped file
    mmap: MmapMut,
    /// Underlying file handle
    file: File,
    /// Current transaction's dirty pages (page_id -> shadow_page_id)
    dirty_pages: HashMap<PageId, PageId>,
    /// Pages allocated in current transaction
    new_pages: HashSet<PageId>,
    /// Pages to be freed after commit
    pending_free: Vec<PageId>,
    /// Current transaction ID
    current_txn: u64,
    /// File size in pages
    file_pages: u64,
}

impl ShadowAllocator {
    /// Open or create a TeamEngram file
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file_exists = path.exists();

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .context("Failed to open TeamEngram file")?;

        // SWMR: Acquire exclusive lock - blocks if another process has the file
        file.lock_exclusive().context("Failed to acquire exclusive file lock - another process is using this store")?;

        let initial_size = if file_exists {
            file.metadata()?.len() as usize
        } else {
            // Initial file: 1 meta page + some initial pages
            PAGE_SIZE * 16
        };

        // Ensure minimum size
        let size = initial_size.max(PAGE_SIZE * 16);
        if file.metadata()?.len() < size as u64 {
            file.set_len(size as u64)?;
        }

        let mmap = unsafe {
            MmapMut::map_mut(&file).context("Failed to mmap file")?
        };

        let mut allocator = Self {
            mmap,
            file,
            dirty_pages: HashMap::new(),
            new_pages: HashSet::new(),
            pending_free: Vec::new(),
            current_txn: 0,
            file_pages: (size / PAGE_SIZE) as u64,
        };

        // Initialize if new file
        if !file_exists {
            allocator.initialize()?;
        } else {
            // Load current txn from meta
            let meta = allocator.meta_page();
            allocator.current_txn = meta.txn_id;
        }

        Ok(allocator)
    }

    /// Initialize a new file with meta page
    fn initialize(&mut self) -> Result<()> {
        let mut meta = MetaPage::new(PAGE_SIZE as u32);
        meta.total_pages = self.file_pages;
        meta.free_pages = self.file_pages - 1; // All except meta page

        // Write meta page
        let meta_bytes = unsafe {
            std::slice::from_raw_parts(
                &meta as *const MetaPage as *const u8,
                std::mem::size_of::<MetaPage>()
            )
        };
        self.mmap[..meta_bytes.len()].copy_from_slice(meta_bytes);
        self.mmap.flush()?;

        Ok(())
    }

    /// Get the meta page (read-only)
    pub fn meta_page(&self) -> &MetaPage {
        unsafe { &*(self.mmap.as_ptr() as *const MetaPage) }
    }

    /// Get mutable meta page
    pub fn meta_page_mut(&mut self) -> &mut MetaPage {
        unsafe { &mut *(self.mmap.as_mut_ptr() as *mut MetaPage) }
    }

    /// Read a page by ID
    pub fn read_page(&self, page_id: PageId) -> Result<&Page> {
        if page_id >= self.file_pages || page_id == NULL_PAGE {
            bail!("Invalid page ID: {}", page_id);
        }

        // Check if we have a shadow page for this
        let actual_id = self.dirty_pages.get(&page_id).copied().unwrap_or(page_id);

        let offset = actual_id as usize * PAGE_SIZE;
        let page_ptr = unsafe { self.mmap.as_ptr().add(offset) as *const Page };
        Ok(unsafe { &*page_ptr })
    }

    /// Get a page for writing (copy-on-write)
    /// Returns the shadow page ID and a mutable reference
    pub fn write_page(&mut self, page_id: PageId) -> Result<(PageId, &mut Page)> {
        // If already dirty in this txn, return existing shadow
        if let Some(&shadow_id) = self.dirty_pages.get(&page_id) {
            let offset = shadow_id as usize * PAGE_SIZE;
            let page_ptr = unsafe { self.mmap.as_mut_ptr().add(offset) as *mut Page };
            return Ok((shadow_id, unsafe { &mut *page_ptr }));
        }

        // If page_id is already a new page in this transaction, it's writable directly
        // This handles the case where split functions receive shadow_id and call write_page
        if self.new_pages.contains(&page_id) {
            let offset = page_id as usize * PAGE_SIZE;
            let page_ptr = unsafe { self.mmap.as_mut_ptr().add(offset) as *mut Page };
            return Ok((page_id, unsafe { &mut *page_ptr }));
        }

        // Allocate a new shadow page
        let shadow_id = self.allocate_page()?;

        // Copy original contents to shadow
        if page_id != NULL_PAGE && page_id < self.file_pages {
            let src_offset = page_id as usize * PAGE_SIZE;
            let dst_offset = shadow_id as usize * PAGE_SIZE;
            self.mmap.copy_within(src_offset..src_offset + PAGE_SIZE, dst_offset);
        }

        // Track the dirty mapping
        self.dirty_pages.insert(page_id, shadow_id);

        // Mark original for freeing after commit
        if page_id != NULL_PAGE && page_id != 0 {
            self.pending_free.push(page_id);
        }

        let offset = shadow_id as usize * PAGE_SIZE;
        let page_ptr = unsafe { self.mmap.as_mut_ptr().add(offset) as *mut Page };
        Ok((shadow_id, unsafe { &mut *page_ptr }))
    }

    /// Allocate a new page
    pub fn allocate_page(&mut self) -> Result<PageId> {
        // Try free list first to reclaim deleted pages
        let free_head = self.meta_page().free_list_head;
        if free_head != NULL_PAGE && free_head < self.file_pages {
            let free_id = free_head;

            // Read next free pointer from the free page
            let offset = free_id as usize * PAGE_SIZE;
            let next_free = unsafe {
                *(self.mmap.as_ptr().add(offset + 8) as *const PageId)
            };

            // Update free list head
            let meta = self.meta_page_mut();
            meta.free_list_head = next_free;
            if meta.free_pages > 0 {
                meta.free_pages -= 1;
            }

            // Zero out the page before reuse
            self.mmap[offset..offset + PAGE_SIZE].fill(0);

            self.new_pages.insert(free_id);
            return Ok(free_id);
        }

        // Extend file
        let new_id = self.file_pages;
        self.file_pages += 1;

        let new_size = self.file_pages as usize * PAGE_SIZE;
        self.file.set_len(new_size as u64)?;

        // Remap
        self.mmap = unsafe {
            MmapMut::map_mut(&self.file).context("Failed to remap after extend")?
        };

        // Update meta - copy file_pages first to avoid borrow conflict
        let file_pages = self.file_pages;
        let meta = self.meta_page_mut();
        meta.total_pages = file_pages;

        self.new_pages.insert(new_id);
        Ok(new_id)
    }

    /// Allocate and initialize a new leaf page
    pub fn allocate_leaf(&mut self) -> Result<(PageId, &mut Page)> {
        let page_id = self.allocate_page()?;
        let offset = page_id as usize * PAGE_SIZE;
        let page_ptr = unsafe { self.mmap.as_mut_ptr().add(offset) as *mut Page };
        let page = unsafe { &mut *page_ptr };
        page.init_leaf(page_id, self.current_txn + 1);
        Ok((page_id, page))
    }

    /// Allocate and initialize a new branch page
    pub fn allocate_branch(&mut self) -> Result<(PageId, &mut Page)> {
        let page_id = self.allocate_page()?;
        let offset = page_id as usize * PAGE_SIZE;
        let page_ptr = unsafe { self.mmap.as_mut_ptr().add(offset) as *mut Page };
        let page = unsafe { &mut *page_ptr };
        page.init_branch(page_id, self.current_txn + 1);
        Ok((page_id, page))
    }

    /// Begin a new transaction
    pub fn begin_txn(&mut self) {
        self.dirty_pages.clear();
        self.new_pages.clear();
        self.pending_free.clear();
    }

    /// Commit the current transaction
    /// This is the atomic operation - swaps the root pointer
    pub fn commit(&mut self, new_root: PageId) -> Result<()> {
        // Update meta page
        {
            let meta = self.meta_page_mut();

            // Increment transaction ID
            meta.txn_id += 1;

            // Update timestamp
            meta.last_commit = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            // Swap root pointer (atomic commit point)
            if meta.active_root == 0 {
                meta.root_shadow = new_root;
                meta.active_root = 1;
            } else {
                meta.root_primary = new_root;
                meta.active_root = 0;
            }
        }

        // Update our local txn counter
        self.current_txn = self.meta_page().txn_id;

        // Collect pages to free, then free them
        let pages_to_free: Vec<PageId> = self.pending_free.drain(..).collect();
        for page_id in pages_to_free {
            self.free_page(page_id)?;
        }

        // Flush to disk
        self.mmap.flush()?;

        // Clear transaction state
        self.dirty_pages.clear();
        self.new_pages.clear();

        Ok(())
    }

    /// Abort the current transaction
    pub fn abort(&mut self) {
        // Collect pages to free, then free them
        let pages_to_free: Vec<PageId> = self.new_pages.drain().collect();
        for page_id in pages_to_free {
            let _ = self.free_page(page_id);
        }

        self.dirty_pages.clear();
        self.pending_free.clear();
    }

    /// Add a page to the free list
    fn free_page(&mut self, page_id: PageId) -> Result<()> {
        if page_id == 0 || page_id == NULL_PAGE {
            return Ok(());
        }

        let offset = page_id as usize * PAGE_SIZE;

        // Write free page marker and next pointer
        let page_ptr = unsafe { self.mmap.as_mut_ptr().add(offset) };
        unsafe {
            // Mark as free
            *page_ptr = PageType::Free as u8;
            // Store next free pointer
            let next_ptr = page_ptr.add(8) as *mut PageId;
            *next_ptr = self.meta_page().free_list_head;
        }

        // Update free list head
        let meta = self.meta_page_mut();
        meta.free_list_head = page_id;
        meta.free_pages += 1;

        Ok(())
    }

    /// Get the current active root page ID
    pub fn root_page(&self) -> PageId {
        self.meta_page().active_root_page()
    }

    /// Get current transaction ID
    pub fn txn_id(&self) -> u64 {
        self.current_txn
    }

    /// Get file statistics
    pub fn stats(&self) -> AllocatorStats {
        let meta = self.meta_page();
        AllocatorStats {
            total_pages: meta.total_pages,
            free_pages: meta.free_pages,
            used_pages: meta.total_pages - meta.free_pages,
            file_size: meta.total_pages * PAGE_SIZE as u64,
            txn_id: meta.txn_id,
        }
    }
}

/// Allocator statistics
#[derive(Debug, Clone)]
pub struct AllocatorStats {
    pub total_pages: u64,
    pub free_pages: u64,
    pub used_pages: u64,
    pub file_size: u64,
    pub txn_id: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_create_and_open() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.teamengram");

        // Create
        {
            let alloc = ShadowAllocator::open(&path).unwrap();
            let stats = alloc.stats();
            assert!(stats.total_pages >= 16);
        }

        // Reopen
        {
            let alloc = ShadowAllocator::open(&path).unwrap();
            let stats = alloc.stats();
            assert!(stats.total_pages >= 16);
        }
    }

    #[test]
    fn test_allocate_pages() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.teamengram");

        let mut alloc = ShadowAllocator::open(&path).unwrap();

        let (id1, page1) = alloc.allocate_leaf().unwrap();
        assert!(id1 > 0);

        let (id2, page2) = alloc.allocate_leaf().unwrap();
        assert!(id2 > id1);
    }

    #[test]
    fn test_commit() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.teamengram");

        let mut alloc = ShadowAllocator::open(&path).unwrap();
        alloc.begin_txn();

        let (root_id, page) = alloc.allocate_leaf().unwrap();
        page.leaf_insert(b"key1", b"value1");
        page.update_checksum();

        alloc.commit(root_id).unwrap();

        assert_eq!(alloc.root_page(), root_id);
        assert_eq!(alloc.txn_id(), 1);
    }
}
