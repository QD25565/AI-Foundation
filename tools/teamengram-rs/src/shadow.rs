//! Shadow Paging Allocator
//!
//! Implements copy-on-write semantics for atomic commits.
//! When a page needs to be modified:
//! 1. Allocate a new page (shadow)
//! 2. Copy contents and modify
//! 3. Update parent pointers to shadow
//! 4. On commit: swap root pointer atomically
//! 5. Old pages go to free list

use crate::page::{Page, PageId, PAGE_SIZE, NULL_PAGE, MetaPage, PageType, BranchEntry, PageHeader};
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
            // Load current txn and version from meta (copy to locals to release borrow)
            let meta = allocator.meta_page();
            let txn_id = meta.txn_id;
            let version = meta.version;

            allocator.current_txn = txn_id;

            // Migrate V1 → V2: sort unsorted branch entries
            if version < 2 {
                let sorted = allocator.migrate_v1_to_v2()?;
                if sorted > 0 {
                    eprintln!("[teamengram] Migrated V1→V2: sorted {} branch page(s)", sorted);
                }
            }
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
        let page = unsafe { &*page_ptr };

        // Verify integrity of committed pages read from disk.
        // Skip dirty pages (mid-transaction, checksum may be stale) and newly
        // allocated pages (may not have final checksum yet).
        if !self.dirty_pages.contains_key(&page_id)
            && !self.new_pages.contains(&actual_id)
            && !page.verify_checksum()
        {
            bail!(
                "Page {} (physical {}) checksum mismatch — data corruption detected",
                page_id, actual_id
            );
        }

        Ok(page)
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

        // Verify all modified pages before persisting to disk.
        // Every page allocated or copy-on-written in this transaction is tracked
        // in new_pages. All write paths call update_checksum() after mutation,
        // so a checksum mismatch here means either a missed update_checksum()
        // call (code bug) or in-memory corruption — refuse to persist bad data.
        for &page_id in &self.new_pages {
            let offset = page_id as usize * PAGE_SIZE;
            let page = unsafe { &*(self.mmap.as_ptr().add(offset) as *const Page) };
            if !page.verify_checksum() {
                bail!(
                    "Pre-commit integrity check failed: page {} has invalid checksum — \
                     refusing to flush potentially corrupt data to disk",
                    page_id
                );
            }
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
            if let Err(e) = self.free_page(page_id) {
                eprintln!("[SHADOW] Failed to free page {} during abort: {}", page_id, e);
            }
        }

        self.dirty_pages.clear();
        self.pending_free.clear();
    }

    /// Direct mutable page access (no copy-on-write).
    /// Only safe during exclusive-lock operations like migration.
    fn page_mut_raw(&mut self, page_id: PageId) -> Result<&mut Page> {
        if page_id >= self.file_pages || page_id == NULL_PAGE {
            bail!("Invalid page ID: {}", page_id);
        }
        let offset = page_id as usize * PAGE_SIZE;
        let page_ptr = unsafe { self.mmap.as_mut_ptr().add(offset) as *mut Page };
        Ok(unsafe { &mut *page_ptr })
    }

    /// Migrate V1 files to V2: sort branch entries in-place.
    /// V1 files may have unsorted branch entries from the old append-only add_branch_entry.
    /// V2 requires sorted entries for binary_search correctness in search_leaf.
    /// Returns the number of branch pages sorted.
    pub fn migrate_v1_to_v2(&mut self) -> Result<u32> {
        let version = self.meta_page().version;
        if version >= 2 {
            return Ok(0); // Already V2
        }

        let root = self.root_page();
        if root == NULL_PAGE {
            // Empty tree — just bump version
            self.meta_page_mut().version = 2;
            self.mmap.flush()?;
            return Ok(0);
        }

        // First pass: DFS to collect all branch pages and their entry data.
        // We collect everything upfront to avoid borrow conflicts with page_mut_raw.
        struct BranchData {
            page_id: PageId,
            entries: Vec<(BranchEntry, Vec<u8>)>,
        }

        let mut to_sort: Vec<BranchData> = Vec::new();
        let mut stack = vec![root];

        while let Some(page_id) = stack.pop() {
            let page = self.read_page(page_id)?;
            let pt = page.header().page_type();
            let count = page.header().count as usize;

            if pt == PageType::Branch {
                let page_entries = page.branch_entries();
                let mut entries = Vec::with_capacity(count);
                for i in 0..count {
                    let entry = page_entries[i];
                    stack.push(entry.child);
                    let key = if entry.key_len == 0 {
                        Vec::new()
                    } else {
                        let s = entry.key_offset as usize;
                        let e = s + entry.key_len as usize;
                        if e <= PAGE_SIZE {
                            page.data[s..e].to_vec()
                        } else {
                            Vec::new() // Corrupted entry — treat as empty key
                        }
                    };
                    entries.push((entry, key));
                }
                if count > 1 {
                    to_sort.push(BranchData { page_id, entries });
                }
            }
            // Leaf pages: nothing to sort or descend into
        }

        // Second pass: sort entries within each branch page and write back
        let mut sorted_count = 0u32;
        for mut bd in to_sort {
            // Sort: empty key last (= +infinity), otherwise lexicographic
            bd.entries.sort_by(|a, b| {
                match (a.1.is_empty(), b.1.is_empty()) {
                    (true, true) => std::cmp::Ordering::Equal,
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    (false, false) => a.1.cmp(&b.1),
                }
            });

            // Write sorted entries back in-place and update checksum
            let page = self.page_mut_raw(bd.page_id)?;
            for (i, (entry, _)) in bd.entries.iter().enumerate() {
                let offset = PageHeader::SIZE + i * BranchEntry::SIZE;
                unsafe {
                    let ptr = page.data.as_mut_ptr().add(offset) as *mut BranchEntry;
                    *ptr = *entry;
                }
            }
            page.update_checksum();
            sorted_count += 1;
        }

        // Bump version and flush
        self.meta_page_mut().version = 2;
        self.mmap.flush()?;

        Ok(sorted_count)
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

        let (id1, _page1) = alloc.allocate_leaf().unwrap();
        assert!(id1 > 0);

        let (id2, _page2) = alloc.allocate_leaf().unwrap();
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

    #[test]
    fn test_new_file_gets_version_2() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_v2.teamengram");
        let alloc = ShadowAllocator::open(&path).unwrap();
        { let v = alloc.meta_page().version; assert_eq!(v, 2); }
    }

    #[test]
    fn test_migrate_v1_to_v2_sorts_branches() {
        use crate::btree::BTree;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test_migrate.teamengram");

        // Step 1: Create a file with enough entries to have branch pages
        {
            let mut alloc = ShadowAllocator::open(&path).unwrap();
            let mut tree = BTree::new(&mut alloc);
            // Insert 200 keys across multiple prefixes to force branch splits
            for i in 0..200u32 {
                let key = format!("key:{:04}", i);
                let val = format!("val:{:04}", i);
                tree.insert(key.as_bytes(), val.as_bytes()).unwrap();
            }
            // Version is 2 for new files
            { let v = alloc.meta_page().version; assert_eq!(v, 2); }
        }

        // Step 2: Manually downgrade version to 1 and scramble branch entries
        {
            let mut alloc = ShadowAllocator::open(&path).unwrap();
            // Downgrade version to simulate V1 file
            alloc.meta_page_mut().version = 1;

            // Find a branch page and swap two entries to create unsorted state
            let root = alloc.root_page();
            let page = alloc.read_page(root).unwrap();
            if page.header().page_type() == PageType::Branch {
                let count = page.header().count as usize;
                if count >= 2 {
                    // Swap first and second entries to unsort
                    let page_mut = alloc.page_mut_raw(root).unwrap();
                    let e0_off = PageHeader::SIZE;
                    let e1_off = PageHeader::SIZE + BranchEntry::SIZE;
                    let mut e0 = [0u8; BranchEntry::SIZE];
                    let mut e1 = [0u8; BranchEntry::SIZE];
                    e0.copy_from_slice(&page_mut.data[e0_off..e0_off + BranchEntry::SIZE]);
                    e1.copy_from_slice(&page_mut.data[e1_off..e1_off + BranchEntry::SIZE]);
                    page_mut.data[e0_off..e0_off + BranchEntry::SIZE].copy_from_slice(&e1);
                    page_mut.data[e1_off..e1_off + BranchEntry::SIZE].copy_from_slice(&e0);
                    // Update checksum to simulate a proper V1 file (V1 code
                    // always called update_checksum after writes — entries are
                    // unsorted but checksum is valid for the unsorted state)
                    page_mut.update_checksum();
                }
            }
            alloc.mmap.flush().unwrap();
        }

        // Step 3: Reopen — migration should run automatically
        {
            let mut alloc = ShadowAllocator::open(&path).unwrap();
            // Version should now be 2
            { let v = alloc.meta_page().version; assert_eq!(v, 2); }

            // All 200 keys should be findable via BTree
            let tree = BTree::new(&mut alloc);
            for i in 0..200u32 {
                let key = format!("key:{:04}", i);
                let expected = format!("val:{:04}", i);
                let val = tree.get(key.as_bytes()).unwrap();
                assert!(val.is_some(), "key {} not found after migration", key);
                assert_eq!(val.unwrap(), expected.as_bytes(), "wrong value for key {}", key);
            }
        }
    }

    #[test]
    fn test_read_page_detects_corruption() {
        use crate::btree::BTree;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test_corruption.teamengram");

        // Create a tree with some data
        {
            let mut alloc = ShadowAllocator::open(&path).unwrap();
            let mut tree = BTree::new(&mut alloc);
            for i in 0..50u32 {
                let key = format!("k:{:04}", i);
                let val = format!("v:{:04}", i);
                tree.insert(key.as_bytes(), val.as_bytes()).unwrap();
            }
        }

        // Corrupt a page by flipping a byte, then verify read_page catches it
        {
            let mut alloc = ShadowAllocator::open(&path).unwrap();
            let root = alloc.root_page();

            // Verify the page reads fine before corruption
            assert!(alloc.read_page(root).is_ok());

            // Corrupt the page data directly via mmap (flip a byte in the data area)
            let offset = root as usize * PAGE_SIZE + 100; // somewhere in page data
            alloc.mmap[offset] ^= 0xFF;
            alloc.mmap.flush().unwrap();
        }

        // Reopen and verify corruption is detected
        {
            let alloc = ShadowAllocator::open(&path).unwrap();
            let root = alloc.root_page();
            let result = alloc.read_page(root);
            assert!(result.is_err(), "read_page should detect corrupted page");
            let err_msg = result.err().unwrap().to_string();
            assert!(
                err_msg.contains("checksum mismatch"),
                "error should mention checksum: {}",
                err_msg
            );
        }
    }

    #[test]
    fn test_commit_rejects_corrupt_dirty_page() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_commit_corrupt.teamengram");

        let mut alloc = ShadowAllocator::open(&path).unwrap();
        alloc.begin_txn();

        // Allocate a leaf, insert data, update checksum (normal path)
        let (leaf_id, leaf) = alloc.allocate_leaf().unwrap();
        leaf.leaf_insert(b"hello", b"world");
        leaf.update_checksum();

        // Corrupt the page data directly via mmap AFTER update_checksum
        // This simulates in-memory corruption between write and commit
        let offset = leaf_id as usize * PAGE_SIZE + 64; // data area, past header
        alloc.mmap[offset] ^= 0xFF;

        // Commit should detect the corruption and refuse to flush
        let result = alloc.commit(leaf_id);
        assert!(result.is_err(), "commit should reject page with corrupt checksum");
        let err_msg = result.err().unwrap().to_string();
        assert!(
            err_msg.contains("Pre-commit integrity check failed"),
            "error should mention pre-commit check: {}",
            err_msg
        );
    }

    #[test]
    fn test_commit_succeeds_with_valid_checksums() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_commit_valid.teamengram");

        let mut alloc = ShadowAllocator::open(&path).unwrap();
        alloc.begin_txn();

        // Normal write path — checksum should be valid
        let (leaf_id, leaf) = alloc.allocate_leaf().unwrap();
        leaf.leaf_insert(b"key1", b"val1");
        leaf.leaf_insert(b"key2", b"val2");
        leaf.update_checksum();

        // Commit should succeed — checksums are valid
        assert!(alloc.commit(leaf_id).is_ok());

        // Verify data survived
        let page = alloc.read_page(alloc.root_page()).unwrap();
        assert!(page.verify_checksum());
    }

    #[test]
    fn test_migrate_empty_tree_v1() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_empty_migrate.teamengram");

        // Create empty file, downgrade to V1
        {
            let mut alloc = ShadowAllocator::open(&path).unwrap();
            alloc.meta_page_mut().version = 1;
            alloc.mmap.flush().unwrap();
        }

        // Reopen — migration should handle empty tree gracefully
        {
            let alloc = ShadowAllocator::open(&path).unwrap();
            { let v = alloc.meta_page().version; assert_eq!(v, 2); }
        }
    }
}
