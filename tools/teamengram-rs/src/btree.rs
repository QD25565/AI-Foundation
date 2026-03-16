//! B+Tree Implementation
//!
//! A simple B+Tree for key-value storage.
//! Uses shadow paging for atomic commits.

use crate::page::{Page, PageId, NULL_PAGE, PageType};
use crate::shadow::ShadowAllocator;
use anyhow::{Result, bail};

/// B+Tree configuration
pub struct BTreeConfig {
    /// Maximum entries per leaf before split
    pub leaf_max: usize,
    /// Maximum entries per branch before split
    pub branch_max: usize,
}

impl Default for BTreeConfig {
    fn default() -> Self {
        Self {
            leaf_max: 64,
            // branch_max must account for variable-length keys
            // With avg 30-byte keys: entries(32+64*16=1056) + keys(64*30=1920) = 2976 < 4096
            branch_max: 64,
        }
    }
}

/// B+Tree operations
pub struct BTree<'a> {
    allocator: &'a mut ShadowAllocator,
    config: BTreeConfig,
}

impl<'a> BTree<'a> {
    /// Create a new B+Tree handle
    pub fn new(allocator: &'a mut ShadowAllocator) -> Self {
        Self {
            allocator,
            config: BTreeConfig::default(),
        }
    }

    /// Get a value by key
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let root = self.allocator.root_page();
        if root == NULL_PAGE {
            return Ok(None);
        }

        self.search_leaf(root, key)
    }

    /// Search for a key starting from a page
    fn search_leaf(&self, page_id: PageId, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let page = self.allocator.read_page(page_id)?;

        match page.header().page_type() {
            PageType::Leaf => {
                match page.leaf_search(key) {
                    Ok(idx) => Ok(Some(page.leaf_value(idx).to_vec())),
                    Err(_) => Ok(None),
                }
            }
            PageType::Branch => {
                let entries = page.branch_entries();
                if entries.is_empty() {
                    return Ok(None);
                }
                // Binary search: O(log b) per level instead of O(b)
                let child_idx = page.branch_search(key).min(entries.len() - 1);
                self.search_leaf(entries[child_idx].child, key)
            }
            _ => Ok(None),
        }
    }

    /// Get branch key at index (returns owned Vec to avoid borrow issues)
    fn get_branch_key_owned(page: &Page, index: usize) -> Vec<u8> {
        page.branch_key(index).to_vec()
    }

    /// Insert a key-value pair
    pub fn insert(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        let root = self.allocator.root_page();

        if root == NULL_PAGE {
            // Create first leaf
            let (leaf_id, leaf) = self.allocator.allocate_leaf()?;
            leaf.leaf_insert(key, value);
            leaf.update_checksum();
            self.allocator.commit(leaf_id)?;
            return Ok(());
        }

        // Insert into existing tree
        self.allocator.begin_txn();

        match self.insert_recursive(root, key, value)? {
            InsertResult::Done(new_root) => {
                self.allocator.commit(new_root)?;
            }
            InsertResult::Split { left, right, key: split_key } => {
                // Root split - create new root
                let (new_root_id, new_root) = self.allocator.allocate_branch()?;

                // Add entries for left and right children
                // Entry with split_key: keys < split_key go to left
                // Entry with empty key: fallback to right for keys >= split_key
                Self::add_branch_entry_static(new_root, left, &split_key)?;
                Self::add_branch_entry_static(new_root, right, &[])?;
                new_root.update_checksum();

                self.allocator.commit(new_root_id)?;
            }
        }

        Ok(())
    }

    /// Batch insert multiple key-value pairs in a single transaction
    /// This is critical for operations that need multiple keys to be atomically inserted
    /// (e.g., start_dialogue needs dg:id, dg:ai:initiator, dg:ai:responder)
    pub fn batch_insert(&mut self, entries: &[(&[u8], &[u8])]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        let root = self.allocator.root_page();

        // If tree is empty, create first leaf with first entry
        if root == NULL_PAGE {
            let (leaf_id, leaf) = self.allocator.allocate_leaf()?;
            leaf.leaf_insert(entries[0].0, entries[0].1);
            leaf.update_checksum();
            self.allocator.commit(leaf_id)?;

            // If only one entry, we're done
            if entries.len() == 1 {
                return Ok(());
            }

            // For remaining entries, start a new transaction
            self.allocator.begin_txn();
            let mut current_root = leaf_id;

            for (key, value) in entries.iter().skip(1) {
                match self.insert_recursive(current_root, key, value)? {
                    InsertResult::Done(new_root) => {
                        current_root = new_root;
                    }
                    InsertResult::Split { left, right, key: split_key } => {
                        let (new_root_id, new_root_page) = self.allocator.allocate_branch()?;
                        Self::add_branch_entry_static(new_root_page, left, &split_key)?;
                        Self::add_branch_entry_static(new_root_page, right, &[])?;
                        new_root_page.update_checksum();
                        current_root = new_root_id;
                    }
                }
            }

            self.allocator.commit(current_root)?;
            return Ok(());
        }

        // Tree already has data - single transaction for all entries
        self.allocator.begin_txn();
        let mut current_root = root;

        for (key, value) in entries.iter() {
            match self.insert_recursive(current_root, key, value)? {
                InsertResult::Done(new_root) => {
                    current_root = new_root;
                }
                InsertResult::Split { left, right, key: split_key } => {
                    let (new_root_id, new_root_page) = self.allocator.allocate_branch()?;
                    Self::add_branch_entry_static(new_root_page, left, &split_key)?;
                    Self::add_branch_entry_static(new_root_page, right, &[])?;
                    new_root_page.update_checksum();
                    current_root = new_root_id;
                }
            }
        }

        self.allocator.commit(current_root)?;
        Ok(())
    }

    /// Recursive insert
    fn insert_recursive(&mut self, page_id: PageId, key: &[u8], value: &[u8]) -> Result<InsertResult> {
        let page = self.allocator.read_page(page_id)?;
        let page_type = page.header().page_type();

        match page_type {
            PageType::Leaf => {
                // Get writable copy (shadow page)
                let (shadow_id, shadow) = self.allocator.write_page(page_id)?;

                // Use upsert which handles both insert and update
                if shadow.leaf_upsert(key, value) {
                    shadow.update_checksum();
                    return Ok(InsertResult::Done(shadow_id));
                }

                // Compact to reclaim garbage from old value replacements, then retry
                shadow.leaf_compact();
                if shadow.leaf_upsert(key, value) {
                    shadow.update_checksum();
                    return Ok(InsertResult::Done(shadow_id));
                }

                // Still no space after compaction — split is genuinely needed
                self.split_leaf(shadow_id, key, value)
            }
            PageType::Branch => {
                let entries = page.branch_entries();
                if entries.is_empty() {
                    bail!("Empty branch node");
                }
                // Binary search: O(log b) per level instead of O(b)
                let child_idx = page.branch_search(key).min(entries.len() - 1);
                let child_id = entries[child_idx].child;

                // Recurse
                match self.insert_recursive(child_id, key, value)? {
                    InsertResult::Done(new_child) => {
                        // Update child pointer
                        let (shadow_id, shadow) = self.allocator.write_page(page_id)?;
                        Self::update_branch_child_static(shadow, child_idx, new_child)?;
                        shadow.update_checksum();
                        Ok(InsertResult::Done(shadow_id))
                    }
                    InsertResult::Split { left, right, key: split_key } => {
                        // Insert new entry for split.
                        // The existing entry keeps its key (= upper bound) and gets RIGHT
                        // (right inherits the old range's upper bound).
                        // A new entry (split_key, LEFT) is inserted in sorted position
                        // (left is bounded by split_key from above).
                        let (shadow_id, shadow) = self.allocator.write_page(page_id)?;

                        // Existing entry gets right child (inherits upper bound)
                        Self::update_branch_child_static(shadow, child_idx, right)?;

                        // New entry for left child (bounded by split_key)
                        let branch_max = self.config.branch_max;
                        if Self::can_add_branch_entry_static(shadow, branch_max, &split_key) {
                            Self::add_branch_entry_static(shadow, left, &split_key)?;
                            shadow.update_checksum();
                            Ok(InsertResult::Done(shadow_id))
                        } else {
                            // Need to split branch
                            self.split_branch(shadow_id, left, &split_key)
                        }
                    }
                }
            }
            _ => {
                bail!("Invalid page type for insert: {:?} at page_id={}", page_type, page_id)
            }
        }
    }

    /// Split a leaf page
    fn split_leaf(&mut self, page_id: PageId, new_key: &[u8], new_value: &[u8]) -> Result<InsertResult> {
        let page = self.allocator.read_page(page_id)?;
        let count = page.header().count as usize;

        // Collect all entries, handling upsert case (key might already exist)
        let mut entries: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(count + 1);
        let mut key_found = false;
        for i in 0..count {
            let existing_key = page.leaf_key(i).to_vec();
            if existing_key == new_key {
                // Key exists - use new value (upsert)
                entries.push((existing_key, new_value.to_vec()));
                key_found = true;
            } else {
                entries.push((existing_key, page.leaf_value(i).to_vec()));
            }
        }
        // Only add new entry if key wasn't already in the page
        if !key_found {
            entries.push((new_key.to_vec(), new_value.to_vec()));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        // Split point
        let mid = entries.len() / 2;

        // Get txn_id before mutable borrows
        let txn_id = self.allocator.txn_id() + 1;

        // Create left page (reuse shadow of original)
        let (left_id, left) = self.allocator.write_page(page_id)?;
        left.init_leaf(left_id, txn_id);
        for (k, v) in &entries[..mid] {
            left.leaf_insert(k, v);
        }
        left.update_checksum();

        // Create right page (new)
        let (right_id, right) = self.allocator.allocate_leaf()?;
        for (k, v) in &entries[mid..] {
            right.leaf_insert(k, v);
        }
        right.update_checksum();

        // Split key is first key of right page
        let split_key = entries[mid].0.clone();

        Ok(InsertResult::Split {
            left: left_id,
            right: right_id,
            key: split_key,
        })
    }

    /// Split a branch page
    fn split_branch(&mut self, page_id: PageId, new_child: PageId, new_key: &[u8]) -> Result<InsertResult> {
        let page = self.allocator.read_page(page_id)?;
        let count = page.header().count as usize;

        // Collect all entries + new entry: (key, child)
        let mut entries: Vec<(Vec<u8>, PageId)> = Vec::with_capacity(count + 1);
        let branch_entries = page.branch_entries();
        for i in 0..count {
            let key = Self::get_branch_key_owned(page, i);
            entries.push((key, branch_entries[i].child));
        }
        entries.push((new_key.to_vec(), new_child));
        // Sort with empty key "" last (catch-all = +infinity)
        entries.sort_by(|a, b| {
            if a.0.is_empty() && b.0.is_empty() { std::cmp::Ordering::Equal }
            else if a.0.is_empty() { std::cmp::Ordering::Greater }
            else if b.0.is_empty() { std::cmp::Ordering::Less }
            else { a.0.cmp(&b.0) }
        });

        // Split point
        let mid = entries.len() / 2;

        // Get txn_id before mutable borrows
        let txn_id = self.allocator.txn_id() + 1;

        // Create left page (reuse shadow of original)
        let (left_id, left) = self.allocator.write_page(page_id)?;
        left.init_branch(left_id, txn_id);
        for (k, child) in &entries[..mid] {
            Self::add_branch_entry_static(left, *child, k)?;
        }
        left.update_checksum();

        // Create right page (new)
        let (right_id, right) = self.allocator.allocate_branch()?;
        for (k, child) in &entries[mid..] {
            Self::add_branch_entry_static(right, *child, k)?;
        }
        right.update_checksum();

        // Split key is first key of right page
        let split_key = entries[mid].0.clone();

        Ok(InsertResult::Split {
            left: left_id,
            right: right_id,
            key: split_key,
        })
    }

    /// Check if we can add another branch entry with given key (static version)
    /// Must check both count AND available space to prevent entry/key collision
    fn can_add_branch_entry_static(page: &Page, branch_max: usize, key: &[u8]) -> bool {
        use crate::page::{BranchEntry, PageHeader, PAGE_SIZE};

        let count = page.header().count as usize;
        if count >= branch_max {
            return false;
        }

        // Calculate where the new entry would end
        let entries_end_after = PageHeader::SIZE + (count + 1) * BranchEntry::SIZE;

        // Calculate where the new key would start
        let current_min_offset = if count == 0 {
            PAGE_SIZE
        } else {
            page.branch_entries()
                .iter()
                .map(|e| e.key_offset as usize)
                .min()
                .unwrap_or(PAGE_SIZE)
        };
        let key_offset_after = current_min_offset - key.len();

        // Ensure entries don't collide with keys (leave some margin)
        key_offset_after > entries_end_after + 16  // 16 byte safety margin
    }

    /// Add a branch entry in sorted position (static version to avoid borrow issues).
    /// Maintains sorted invariant: entries ordered by key, with empty key "" last (+infinity).
    fn add_branch_entry_static(page: &mut Page, child: PageId, key: &[u8]) -> Result<()> {
        use crate::page::{BranchEntry, PageHeader, PAGE_SIZE};

        let count = page.header().count as usize;

        // Calculate key offset (store at end of page, growing down)
        let key_offset = if count == 0 {
            PAGE_SIZE - key.len()
        } else {
            let entries = page.branch_entries();
            let min_offset = entries.iter().map(|e| e.key_offset as usize).min().unwrap_or(PAGE_SIZE);
            min_offset - key.len()
        };

        // Safety check: ensure key doesn't collide with entry area
        let entries_end = PageHeader::SIZE + (count + 1) * BranchEntry::SIZE;
        if key_offset < entries_end {
            bail!("Branch page overflow: key_offset={} < entries_end={} (count={})",
                  key_offset, entries_end, count);
        }

        // Write key data
        page.data[key_offset..key_offset + key.len()].copy_from_slice(key);

        // Binary search for sorted insert position (empty key "" sorts last = +infinity)
        let insert_pos = if key.is_empty() {
            count // "" always goes at the end
        } else {
            page.branch_search(key)
        };

        // Shift entries at insert_pos..count forward by one slot
        if insert_pos < count {
            let src_start = PageHeader::SIZE + insert_pos * BranchEntry::SIZE;
            let src_end = PageHeader::SIZE + count * BranchEntry::SIZE;
            let dst_start = PageHeader::SIZE + (insert_pos + 1) * BranchEntry::SIZE;
            page.data.copy_within(src_start..src_end, dst_start);
        }

        // Write new entry at insert_pos
        let entry = BranchEntry {
            child,
            key_len: key.len() as u16,
            key_offset: key_offset as u16,
            _reserved: 0,
        };
        let entry_offset = PageHeader::SIZE + insert_pos * BranchEntry::SIZE;
        unsafe {
            let ptr = page.data.as_mut_ptr().add(entry_offset) as *mut BranchEntry;
            *ptr = entry;
        }

        page.header_mut().count += 1;
        Ok(())
    }

    /// Update a branch child pointer (static version)
    fn update_branch_child_static(page: &mut Page, index: usize, new_child: PageId) -> Result<()> {
        use crate::page::{BranchEntry, PageHeader};

        let entry_offset = PageHeader::SIZE + index * BranchEntry::SIZE;
        unsafe {
            let ptr = page.data.as_mut_ptr().add(entry_offset) as *mut BranchEntry;
            (*ptr).child = new_child;
        }
        Ok(())
    }

    /// Delete a key from the B+Tree
    /// Returns true if the key was found and deleted, false if not found
    /// Note: This is a simple delete that doesn't handle underflow/merge.
    /// Pages may become underfull but the tree remains valid.
    pub fn delete(&mut self, key: &[u8]) -> Result<bool> {
        let root = self.allocator.root_page();
        if root == NULL_PAGE {
            return Ok(false);
        }

        self.allocator.begin_txn();

        match self.delete_recursive(root, key)? {
            DeleteResult::NotFound => {
                self.allocator.abort();
                Ok(false)
            }
            DeleteResult::Done(new_root) => {
                self.allocator.commit(new_root)?;
                Ok(true)
            }
            DeleteResult::Empty => {
                // Root became empty - set root to NULL_PAGE
                // This is a special commit that sets the root to NULL
                let meta = self.allocator.meta_page_mut();
                meta.txn_id += 1;
                if meta.active_root == 0 {
                    meta.root_shadow = NULL_PAGE;
                    meta.active_root = 1;
                } else {
                    meta.root_primary = NULL_PAGE;
                    meta.active_root = 0;
                }
                Ok(true)
            }
        }
    }

    /// Recursive delete helper
    fn delete_recursive(&mut self, page_id: PageId, key: &[u8]) -> Result<DeleteResult> {
        let page = self.allocator.read_page(page_id)?;
        let page_type = page.header().page_type();

        match page_type {
            PageType::Leaf => {
                // Search for the key
                match page.leaf_search(key) {
                    Ok(index) => {
                        // Key found - get writable page and delete
                        let (shadow_id, shadow) = self.allocator.write_page(page_id)?;
                        shadow.leaf_delete(index);
                        shadow.update_checksum();

                        // Check if leaf is now empty
                        if shadow.header().count == 0 {
                            Ok(DeleteResult::Empty)
                        } else {
                            Ok(DeleteResult::Done(shadow_id))
                        }
                    }
                    Err(_) => {
                        // Key not found
                        Ok(DeleteResult::NotFound)
                    }
                }
            }
            PageType::Branch => {
                let entries = page.branch_entries();
                if entries.is_empty() {
                    return Ok(DeleteResult::NotFound);
                }
                // Binary search: O(log b) per level instead of O(b)
                let child_idx = page.branch_search(key).min(entries.len() - 1);
                let child_id = entries[child_idx].child;

                // Recurse
                match self.delete_recursive(child_id, key)? {
                    DeleteResult::NotFound => Ok(DeleteResult::NotFound),
                    DeleteResult::Done(new_child) => {
                        // Update child pointer
                        let (shadow_id, shadow) = self.allocator.write_page(page_id)?;
                        Self::update_branch_child_static(shadow, child_idx, new_child)?;
                        shadow.update_checksum();
                        Ok(DeleteResult::Done(shadow_id))
                    }
                    DeleteResult::Empty => {
                        // Child became empty - remove from branch
                        // For simplicity, we don't handle this case fully
                        // The child pointer remains but points to an empty leaf
                        // A full implementation would remove the branch entry
                        let (shadow_id, shadow) = self.allocator.write_page(page_id)?;
                        shadow.update_checksum();
                        Ok(DeleteResult::Done(shadow_id))
                    }
                }
            }
            _ => Ok(DeleteResult::NotFound),
        }
    }

    /// Iterate over all key-value pairs
    pub fn iter(&self) -> Result<BTreeIterator<'_>> {
        Ok(BTreeIterator {
            allocator: self.allocator,
            stack: vec![],
            started: false,
        })
    }

    /// Iterate over entries matching a key prefix.
    ///
    /// Uses physical-order branch traversal (same as BTreeIterator) to
    /// correctly handle the B+Tree's unsorted branch entries. Within each
    /// leaf, uses binary search to skip to the first potential match.
    ///
    /// Yields only entries whose key starts with the given prefix.
    pub fn prefix_iter(&self, prefix: &[u8]) -> Result<PrefixIterator<'_>> {
        Ok(PrefixIterator {
            allocator: self.allocator,
            prefix: prefix.to_vec(),
            stack: vec![],
            started: false,
            done: false,
        })
    }

    /// Create a range iterator over keys in [start_key, end_key).
    /// Half-open interval: includes start_key, excludes end_key.
    /// If end_key is empty, iterates from start_key to the end of the tree.
    pub fn range_iter(&self, start_key: &[u8], end_key: &[u8]) -> Result<RangeIterator<'_>> {
        Ok(RangeIterator {
            allocator: self.allocator,
            start_key: start_key.to_vec(),
            end_key: end_key.to_vec(),
            stack: vec![],
            started: false,
            done: false,
        })
    }
}

/// Result of an insert operation
enum InsertResult {
    /// Insert completed, new root page ID
    Done(PageId),
    /// Page was split
    Split {
        left: PageId,
        right: PageId,
        key: Vec<u8>,
    },
}

/// Result of a delete operation
enum DeleteResult {
    /// Key was not found
    NotFound,
    /// Delete completed, new root page ID
    Done(PageId),
    /// Page became empty
    Empty,
}

/// Iterator over B+Tree entries
pub struct BTreeIterator<'a> {
    allocator: &'a ShadowAllocator,
    stack: Vec<(PageId, usize)>,
    started: bool,
}

impl<'a> BTreeIterator<'a> {
    /// Get next key-value pair
    pub fn next(&mut self) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        if !self.started {
            self.started = true;
            let root = self.allocator.root_page();
            if root == NULL_PAGE {
                return Ok(None);
            }
            // Descend to leftmost leaf
            self.descend_left(root)?;
        }

        loop {
            if self.stack.is_empty() {
                return Ok(None);
            }

            let (page_id, idx) = self.stack.last_mut().unwrap();
            let page_id_copy = *page_id;
            let idx_copy = *idx;
            let page = self.allocator.read_page(page_id_copy)?;

            if page.header().page_type() == PageType::Leaf {
                let count = page.header().count as usize;
                if idx_copy < count {
                    let key = page.leaf_key(idx_copy).to_vec();
                    let value = page.leaf_value(idx_copy).to_vec();
                    // Update idx in stack
                    if let Some((_, ref mut stack_idx)) = self.stack.last_mut() {
                        *stack_idx += 1;
                    }
                    return Ok(Some((key, value)));
                } else {
                    // Leaf exhausted, go back to parent
                    self.stack.pop();
                    continue;
                }
            } else {
                // Branch node - advance to next child
                let entries = page.branch_entries();
                if idx_copy < entries.len() {
                    let child = entries[idx_copy].child;
                    // Update idx in stack before descending
                    if let Some((_, ref mut stack_idx)) = self.stack.last_mut() {
                        *stack_idx += 1;
                    }
                    // Descend to leftmost leaf of this child
                    self.descend_left(child)?;
                } else {
                    // No more children at this branch, go up
                    self.stack.pop();
                }
            }
        }
    }

    fn descend_left(&mut self, page_id: PageId) -> Result<()> {
        let mut current = page_id;
        loop {
            let page = self.allocator.read_page(current)?;
            self.stack.push((current, 0));

            if page.header().page_type() == PageType::Leaf {
                break;
            }

            // Get first child
            let entries = page.branch_entries();
            if entries.is_empty() {
                break;
            }

            // Mark entries[0] as visited by setting idx to 1
            // This prevents revisiting the same subtree when we pop back up
            if let Some((_, ref mut idx)) = self.stack.last_mut() {
                *idx = 1;
            }

            current = entries[0].child;
        }
        Ok(())
    }
}

/// Iterator that yields B+Tree entries matching a key prefix.
///
/// Seeks directly to the first leaf that could contain the prefix via sorted
/// branch descent, then iterates forward in key order. Stops globally when
/// a key exceeds the prefix range (early termination across leaves).
///
/// Complexity: O(log n + k) where k = matching entries, vs O(n) for full scan.
pub struct PrefixIterator<'a> {
    allocator: &'a ShadowAllocator,
    prefix: Vec<u8>,
    /// Stack of (page_id, next_entry_index) for branch backtracking
    stack: Vec<(PageId, usize)>,
    started: bool,
    done: bool,
}

impl<'a> PrefixIterator<'a> {
    /// Get next matching key-value pair, or None when exhausted.
    pub fn next(&mut self) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        if self.done {
            return Ok(None);
        }

        // First call: seek to the first leaf that could contain the prefix
        if !self.started {
            self.started = true;
            let root = self.allocator.root_page();
            if root == NULL_PAGE {
                self.done = true;
                return Ok(None);
            }
            self.seek_to_prefix(root)?;
        }

        loop {
            if self.stack.is_empty() {
                self.done = true;
                return Ok(None);
            }

            let (page_id, idx) = *self.stack.last().unwrap();
            let page = self.allocator.read_page(page_id)?;

            if page.header().page_type() == PageType::Leaf {
                let count = page.header().count as usize;
                if idx < count {
                    let key = page.leaf_key(idx);

                    // Advance index
                    if let Some((_, ref mut stack_idx)) = self.stack.last_mut() {
                        *stack_idx += 1;
                    }

                    if key.starts_with(&self.prefix) {
                        return Ok(Some((key.to_vec(), page.leaf_value(idx).to_vec())));
                    }

                    // Branches are sorted, so leaves are visited in global key order.
                    // If key > prefix, no more matches exist anywhere — stop.
                    if !self.prefix.is_empty() && key.as_ref() > self.prefix.as_slice() {
                        self.done = true;
                        return Ok(None);
                    }
                    // key < prefix: skip (we may have landed slightly before)
                    continue;
                } else {
                    // Leaf exhausted, go back to parent
                    self.stack.pop();
                    continue;
                }
            } else {
                // Branch node — advance to next child in sorted order
                let entries = page.branch_entries();
                if idx < entries.len() {
                    let child = entries[idx].child;
                    if let Some((_, ref mut stack_idx)) = self.stack.last_mut() {
                        *stack_idx += 1;
                    }
                    self.descend_left(child)?;
                } else {
                    self.stack.pop();
                }
            }
        }
    }

    /// Seek to the first leaf that could contain keys matching the prefix.
    /// Uses sorted branch entries to descend directly to the correct subtree.
    fn seek_to_prefix(&mut self, root: PageId) -> Result<()> {
        let mut current = root;
        loop {
            let page = self.allocator.read_page(current)?;
            match page.header().page_type() {
                PageType::Leaf => {
                    // Binary search for first key >= prefix
                    let idx = match page.leaf_search(&self.prefix) {
                        Ok(idx) => idx,
                        Err(idx) => idx,
                    };
                    self.stack.push((current, idx));
                    break;
                }
                PageType::Branch => {
                    let entries = page.branch_entries();
                    if entries.is_empty() {
                        break;
                    }
                    // Binary search: O(log b) per level
                    let child_idx = page.branch_search(&self.prefix).min(entries.len() - 1);
                    let child = entries[child_idx].child;
                    self.stack.push((current, child_idx + 1));
                    current = child;
                }
                _ => break,
            }
        }
        Ok(())
    }

    /// Descend to the leftmost leaf from a page, used when advancing
    /// to the next subtree after exhausting a leaf.
    fn descend_left(&mut self, page_id: PageId) -> Result<()> {
        let mut current = page_id;
        loop {
            let page = self.allocator.read_page(current)?;
            self.stack.push((current, 0));

            if page.header().page_type() == PageType::Leaf {
                break;
            }

            let entries = page.branch_entries();
            if entries.is_empty() {
                break;
            }

            if let Some((_, ref mut stack_idx)) = self.stack.last_mut() {
                *stack_idx = 1;
            }
            current = entries[0].child;
        }
        Ok(())
    }
}

// ─── RANGE ITERATOR ─────────────────────────────────────────────────────────

/// Iterator over keys in [start_key, end_key). Half-open interval.
/// Empty end_key means "no upper bound" (iterate to end of tree).
pub struct RangeIterator<'a> {
    allocator: &'a ShadowAllocator,
    start_key: Vec<u8>,
    end_key: Vec<u8>,
    stack: Vec<(PageId, usize)>,
    started: bool,
    done: bool,
}

impl<'a> RangeIterator<'a> {
    /// Get next key-value pair in range, or None when exhausted.
    pub fn next(&mut self) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        if self.done {
            return Ok(None);
        }

        if !self.started {
            self.started = true;
            let root = self.allocator.root_page();
            if root == NULL_PAGE {
                self.done = true;
                return Ok(None);
            }
            self.seek_to_start(root)?;
        }

        loop {
            if self.stack.is_empty() {
                self.done = true;
                return Ok(None);
            }

            let (page_id, idx) = *self.stack.last().unwrap();
            let page = self.allocator.read_page(page_id)?;

            if page.header().page_type() == PageType::Leaf {
                let count = page.header().count as usize;
                if idx < count {
                    let key = page.leaf_key(idx);

                    // Advance index
                    if let Some((_, ref mut stack_idx)) = self.stack.last_mut() {
                        *stack_idx += 1;
                    }

                    // Check upper bound (empty end_key = no upper bound)
                    if !self.end_key.is_empty() && key >= self.end_key.as_slice() {
                        self.done = true;
                        return Ok(None);
                    }

                    // Check lower bound (may have landed slightly before start)
                    if key >= self.start_key.as_slice() {
                        return Ok(Some((key.to_vec(), page.leaf_value(idx).to_vec())));
                    }
                    // key < start_key: skip
                    continue;
                } else {
                    self.stack.pop();
                    continue;
                }
            } else {
                // Branch node — advance to next child
                let entries = page.branch_entries();
                if idx < entries.len() {
                    let child = entries[idx].child;
                    if let Some((_, ref mut stack_idx)) = self.stack.last_mut() {
                        *stack_idx += 1;
                    }
                    self.descend_left(child)?;
                } else {
                    self.stack.pop();
                }
            }
        }
    }

    /// Seek to the first leaf that could contain start_key.
    fn seek_to_start(&mut self, root: PageId) -> Result<()> {
        let mut current = root;
        loop {
            let page = self.allocator.read_page(current)?;
            match page.header().page_type() {
                PageType::Leaf => {
                    let idx = match page.leaf_search(&self.start_key) {
                        Ok(idx) => idx,
                        Err(idx) => idx,
                    };
                    self.stack.push((current, idx));
                    break;
                }
                PageType::Branch => {
                    let entries = page.branch_entries();
                    if entries.is_empty() {
                        break;
                    }
                    // Binary search: O(log b) per level
                    let child_idx = page.branch_search(&self.start_key).min(entries.len() - 1);
                    let child = entries[child_idx].child;
                    self.stack.push((current, child_idx + 1));
                    current = child;
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn descend_left(&mut self, page_id: PageId) -> Result<()> {
        let mut current = page_id;
        loop {
            let page = self.allocator.read_page(current)?;
            self.stack.push((current, 0));
            if page.header().page_type() == PageType::Leaf {
                break;
            }
            let entries = page.branch_entries();
            if entries.is_empty() {
                break;
            }
            if let Some((_, ref mut stack_idx)) = self.stack.last_mut() {
                *stack_idx = 1;
            }
            current = entries[0].child;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_insert_and_get() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.teamengram");

        let mut alloc = ShadowAllocator::open(&path).unwrap();

        {
            let mut tree = BTree::new(&mut alloc);
            tree.insert(b"hello", b"world").unwrap();
        }

        {
            let tree = BTree::new(&mut alloc);
            let val = tree.get(b"hello").unwrap();
            assert_eq!(val, Some(b"world".to_vec()));

            let val = tree.get(b"notfound").unwrap();
            assert_eq!(val, None);
        }
    }

    #[test]
    fn test_multiple_inserts() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.teamengram");

        let mut alloc = ShadowAllocator::open(&path).unwrap();

        {
            let mut tree = BTree::new(&mut alloc);
            for i in 0..100 {
                let key = format!("key{:03}", i);
                let val = format!("value{}", i);
                tree.insert(key.as_bytes(), val.as_bytes()).unwrap();
            }
        }

        {
            let tree = BTree::new(&mut alloc);
            for i in 0..100 {
                let key = format!("key{:03}", i);
                let expected = format!("value{}", i);
                let val = tree.get(key.as_bytes()).unwrap();
                assert_eq!(val, Some(expected.into_bytes()));
            }
        }
    }

    #[test]
    fn test_prefix_iteration() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.teamengram");
        let mut alloc = ShadowAllocator::open(&path).unwrap();

        {
            let mut tree = BTree::new(&mut alloc);
            tree.insert(b"bc:1", b"broadcast1").unwrap();
            tree.insert(b"bc:2", b"broadcast2").unwrap();
            tree.insert(b"dm:1", b"dm1").unwrap();
            tree.insert(b"dm:2", b"dm2").unwrap();
            tree.insert(b"dm:3", b"dm3").unwrap();
            tree.insert(b"pr:1", b"presence1").unwrap();
            tree.insert(b"tk:1", b"task1").unwrap();
        }

        // Prefix "dm:" → exactly 3 results in key order
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.prefix_iter(b"dm:").unwrap();
            let mut results = Vec::new();
            while let Some((key, _value)) = iter.next().unwrap() {
                results.push(key);
            }
            assert_eq!(results.len(), 3);
            assert_eq!(results[0], b"dm:1");
            assert_eq!(results[1], b"dm:2");
            assert_eq!(results[2], b"dm:3");
        }

        // Prefix "bc:" → exactly 2
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.prefix_iter(b"bc:").unwrap();
            let mut count = 0;
            while iter.next().unwrap().is_some() { count += 1; }
            assert_eq!(count, 2);
        }

        // Nonexistent prefix → 0
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.prefix_iter(b"zz:").unwrap();
            assert!(iter.next().unwrap().is_none());
        }

        // Empty prefix → all 7 entries
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.prefix_iter(b"").unwrap();
            let mut count = 0;
            while iter.next().unwrap().is_some() { count += 1; }
            assert_eq!(count, 7);
        }
    }

    #[test]
    fn test_prefix_iteration_across_leaves() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.teamengram");
        let mut alloc = ShadowAllocator::open(&path).unwrap();

        // Insert enough entries to force splits (leaf_max=64)
        {
            let mut tree = BTree::new(&mut alloc);
            for i in 0..200 {
                let key = format!("dm:{:04}", i);
                let val = format!("value{}", i);
                tree.insert(key.as_bytes(), val.as_bytes()).unwrap();
            }
            for i in 0..50 {
                let key = format!("bc:{:04}", i);
                let val = format!("bc{}", i);
                tree.insert(key.as_bytes(), val.as_bytes()).unwrap();
            }
        }

        // "dm:" → exactly 200 results, in sorted key order
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.prefix_iter(b"dm:").unwrap();
            let mut results = Vec::new();
            while let Some((key, _)) = iter.next().unwrap() {
                results.push(key);
            }
            assert_eq!(results.len(), 200);
            for i in 1..results.len() {
                assert!(results[i - 1] <= results[i], "keys must be sorted");
            }
        }

        // "bc:" → exactly 50
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.prefix_iter(b"bc:").unwrap();
            let mut count = 0;
            while iter.next().unwrap().is_some() { count += 1; }
            assert_eq!(count, 50);
        }

        // "dm:01" → exactly 11 (dm:0100 through dm:0110... wait, dm:010x = 10 entries)
        // Actually "dm:01" matches dm:0100..dm:0199 = 100 entries
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.prefix_iter(b"dm:01").unwrap();
            let mut count = 0;
            while iter.next().unwrap().is_some() { count += 1; }
            assert_eq!(count, 100); // dm:0100 through dm:0199
        }
    }

    /// Regression test: mixed-prefix round-robin insertion forces branch splits
    /// with interleaved prefixes. Previously caused "" catch-all entries to end
    /// up in wrong positions, making prefix_iter miss entries at scale.
    #[test]
    fn test_mixed_prefix_correctness() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.teamengram");
        let mut alloc = ShadowAllocator::open(&path).unwrap();

        let prefixes = ["bc:", "dg:", "dm:", "pr:", "rm:", "tk:"];

        // Insert 1200 entries across 6 prefixes in round-robin order
        // (200 per prefix, interleaved — worst case for branch ordering)
        {
            let mut tree = BTree::new(&mut alloc);
            for i in 0u64..1200 {
                let prefix = prefixes[(i % 6) as usize];
                let key = format!("{}id:{:08}", prefix, i);
                let val = format!("v{}", i);
                tree.insert(key.as_bytes(), val.as_bytes()).unwrap();
            }
        }

        // Verify every key can be retrieved individually (get correctness)
        {
            let tree = BTree::new(&mut alloc);
            for i in 0u64..1200 {
                let prefix = prefixes[(i % 6) as usize];
                let key = format!("{}id:{:08}", prefix, i);
                let val = tree.get(key.as_bytes()).unwrap();
                assert!(val.is_some(), "get() missing key: {}", key);
                assert_eq!(val.unwrap(), format!("v{}", i).as_bytes());
            }
        }

        // Verify each prefix finds exactly 200 entries via prefix_iter
        for prefix in &prefixes {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.prefix_iter(prefix.as_bytes()).unwrap();
            let mut count = 0;
            let mut last_key: Option<Vec<u8>> = None;
            while let Some((key, _)) = iter.next().unwrap() {
                assert!(key.starts_with(prefix.as_bytes()),
                    "prefix_iter({}) yielded non-matching key: {:?}",
                    prefix, String::from_utf8_lossy(&key));
                if let Some(ref prev) = last_key {
                    assert!(prev <= &key, "prefix_iter({}) not in sorted order", prefix);
                }
                last_key = Some(key);
                count += 1;
            }
            assert_eq!(count, 200,
                "prefix_iter({}) found {} entries, expected 200", prefix, count);
        }

        // Verify full iterator finds all 1200
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.iter().unwrap();
            let mut total = 0;
            while iter.next().unwrap().is_some() { total += 1; }
            assert_eq!(total, 1200);
        }
    }

    #[test]
    fn test_range_iter_basic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_range.teamengram");
        let mut alloc = ShadowAllocator::open(&path).unwrap();

        {
            let mut tree = BTree::new(&mut alloc);
            for i in 0..10u32 {
                let key = format!("key:{:02}", i);
                let val = format!("val:{}", i);
                tree.insert(key.as_bytes(), val.as_bytes()).unwrap();
            }
        }

        // Range [key:03, key:07) → keys 03, 04, 05, 06
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.range_iter(b"key:03", b"key:07").unwrap();
            let mut results = Vec::new();
            while let Some((key, _)) = iter.next().unwrap() {
                results.push(String::from_utf8(key).unwrap());
            }
            assert_eq!(results, vec!["key:03", "key:04", "key:05", "key:06"]);
        }

        // Empty end_key → from key:07 to end of tree
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.range_iter(b"key:07", b"").unwrap();
            let mut results = Vec::new();
            while let Some((key, _)) = iter.next().unwrap() {
                results.push(String::from_utf8(key).unwrap());
            }
            assert_eq!(results, vec!["key:07", "key:08", "key:09"]);
        }

        // Range covering all keys
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.range_iter(b"", b"zzz").unwrap();
            let mut count = 0;
            while iter.next().unwrap().is_some() { count += 1; }
            assert_eq!(count, 10);
        }

        // Range with no matches (gap between keys)
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.range_iter(b"aaa", b"aab").unwrap();
            assert!(iter.next().unwrap().is_none());
        }
    }

    #[test]
    fn test_range_iter_across_leaves() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_range_large.teamengram");
        let mut alloc = ShadowAllocator::open(&path).unwrap();

        // Insert 500 entries to force multiple leaf pages
        {
            let mut tree = BTree::new(&mut alloc);
            for i in 0..500u32 {
                let key = format!("k:{:04}", i);
                let val = format!("v:{}", i);
                tree.insert(key.as_bytes(), val.as_bytes()).unwrap();
            }
        }

        // Range [k:0100, k:0200) → exactly 100 keys, sorted
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.range_iter(b"k:0100", b"k:0200").unwrap();
            let mut results = Vec::new();
            while let Some((key, _)) = iter.next().unwrap() {
                results.push(String::from_utf8(key).unwrap());
            }
            assert_eq!(results.len(), 100);
            assert_eq!(results[0], "k:0100");
            assert_eq!(results[99], "k:0199");
            for i in 1..results.len() {
                assert!(results[i - 1] < results[i], "range results must be sorted");
            }
        }

        // Range near end [k:0490, end) → exactly 10 keys
        {
            let tree = BTree::new(&mut alloc);
            let mut iter = tree.range_iter(b"k:0490", b"").unwrap();
            let mut count = 0;
            while iter.next().unwrap().is_some() { count += 1; }
            assert_eq!(count, 10);
        }
    }

    /// Test that leaf compaction prevents unnecessary splits.
    /// Repeatedly updating the same key creates garbage; compaction reclaims it.
    #[test]
    fn test_compaction_prevents_unnecessary_splits() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_compact.teamengram");
        let mut alloc = ShadowAllocator::open(&path).unwrap();

        // Insert a few keys with large values
        {
            let mut tree = BTree::new(&mut alloc);
            for i in 0..5u32 {
                let key = format!("key:{}", i);
                let val = vec![b'A'; 300]; // 300 bytes each
                tree.insert(key.as_bytes(), &val).unwrap();
            }
        }

        let pages_after_insert = alloc.stats().total_pages;

        // Update the same keys 50 times each with different values.
        // Without compaction: 50 × 300 = 15,000 bytes of garbage per key → splits.
        // With compaction: garbage reclaimed each time → no splits needed.
        {
            let mut tree = BTree::new(&mut alloc);
            for round in 0..50u32 {
                for i in 0..5u32 {
                    let key = format!("key:{}", i);
                    let val = vec![(round % 256) as u8; 300];
                    tree.insert(key.as_bytes(), &val).unwrap();
                }
            }
        }

        let pages_after_updates = alloc.stats().total_pages;

        // With compaction, page count should be modest (no explosion from garbage).
        // Without compaction, we'd need many more pages to hold the garbage.
        // The tree still has only 5 live keys — shouldn't need many leaf pages.
        assert!(pages_after_updates <= pages_after_insert + 4,
            "Too many pages after updates: before={}, after={} — compaction may not be working",
            pages_after_insert, pages_after_updates);

        // Verify all keys return latest values
        {
            let tree = BTree::new(&mut alloc);
            for i in 0..5u32 {
                let key = format!("key:{}", i);
                let val = tree.get(key.as_bytes()).unwrap();
                assert!(val.is_some(), "key {} not found", key);
                assert_eq!(val.unwrap(), vec![49u8; 300]); // last round = 49
            }
        }
    }
}
