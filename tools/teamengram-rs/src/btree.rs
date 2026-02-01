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
                // FIX: Entries may not be sorted (add_branch_entry_static appends).
                // Collect all entries and sort them before traversing.
                let entries = page.branch_entries();
                let mut sorted_entries: Vec<(Vec<u8>, PageId)> = Vec::with_capacity(entries.len());
                for (i, entry) in entries.iter().enumerate() {
                    let entry_key = Self::get_branch_key_owned(page, i);
                    sorted_entries.push((entry_key, entry.child));
                }
                // Sort by key (empty keys go last - they represent rightmost child)
                sorted_entries.sort_by(|a, b| {
                    if a.0.is_empty() { std::cmp::Ordering::Greater }
                    else if b.0.is_empty() { std::cmp::Ordering::Less }
                    else { a.0.cmp(&b.0) }
                });

                // Find the child to descend into
                let mut child = NULL_PAGE;
                for (entry_key, entry_child) in &sorted_entries {
                    if key < entry_key.as_slice() {
                        child = *entry_child;
                        break;
                    }
                    child = *entry_child;
                }

                if child == NULL_PAGE {
                    // Use rightmost child
                    if let Some((_, last_child)) = sorted_entries.last() {
                        child = *last_child;
                    }
                }

                if child == NULL_PAGE {
                    return Ok(None);
                }

                self.search_leaf(child, key)
            }
            _ => Ok(None),
        }
    }

    /// Get branch key at index (returns owned Vec to avoid borrow issues)
    fn get_branch_key_owned(page: &Page, index: usize) -> Vec<u8> {
        let entries = page.branch_entries();
        if index >= entries.len() {
            return Vec::new();
        }
        let entry = &entries[index];
        let start = entry.key_offset as usize;
        let end = start + entry.key_len as usize;

        // Bounds check - if corrupted, return empty
        if end > crate::page::PAGE_SIZE || start > end {
            return Vec::new();
        }

        page.data[start..end].to_vec()
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

                // Need to split (upsert returned false due to space)
                self.split_leaf(shadow_id, key, value)
            }
            PageType::Branch => {
                // Find child to descend into
                let entries = page.branch_entries();
                let mut child_idx = entries.len();

                for (i, _entry) in entries.iter().enumerate() {
                    let entry_key = Self::get_branch_key_owned(page, i);
                    if key < entry_key.as_slice() {
                        child_idx = i;
                        break;
                    }
                }

                // If no entry matched (key >= all keys), use the last entry
                if child_idx >= entries.len() && !entries.is_empty() {
                    child_idx = entries.len() - 1;
                }

                let child_id = if child_idx < entries.len() {
                    entries[child_idx].child
                } else {
                    bail!("Empty branch node");
                };

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
                        // Insert new entry for split
                        let (shadow_id, shadow) = self.allocator.write_page(page_id)?;

                        // Update existing child to point to left
                        Self::update_branch_child_static(shadow, child_idx, left)?;

                        // Try to add entry for right
                        let branch_max = self.config.branch_max;
                        if Self::can_add_branch_entry_static(shadow, branch_max, &split_key) {
                            Self::add_branch_entry_static(shadow, right, &split_key)?;
                            shadow.update_checksum();
                            Ok(InsertResult::Done(shadow_id))
                        } else {
                            // Need to split branch
                            self.split_branch(shadow_id, right, &split_key)
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
        entries.sort_by(|a, b| a.0.cmp(&b.0));

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

    /// Add a branch entry (static version to avoid borrow issues)
    fn add_branch_entry_static(page: &mut Page, child: PageId, key: &[u8]) -> Result<()> {
        use crate::page::{BranchEntry, PageHeader, PAGE_SIZE};

        // Read count first
        let count = page.header().count as usize;

        // Calculate key offset (store at end of page, growing down)
        let key_offset = if count == 0 {
            PAGE_SIZE - key.len()
        } else {
            // Find lowest key offset
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

        // Write key
        page.data[key_offset..key_offset + key.len()].copy_from_slice(key);

        // Write entry
        let entry = BranchEntry {
            child,
            key_len: key.len() as u16,
            key_offset: key_offset as u16,
            _reserved: 0,
        };

        let entry_offset = PageHeader::SIZE + count * BranchEntry::SIZE;
        unsafe {
            let ptr = page.data.as_mut_ptr().add(entry_offset) as *mut BranchEntry;
            *ptr = entry;
        }

        // Update count at end
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
                // FIX: Entries may not be sorted. Collect, sort, and find correct child.
                // Track original index for update_branch_child_static
                let entries = page.branch_entries();
                let mut sorted_entries: Vec<(Vec<u8>, PageId, usize)> = Vec::with_capacity(entries.len());
                for (i, entry) in entries.iter().enumerate() {
                    let entry_key = Self::get_branch_key_owned(page, i);
                    sorted_entries.push((entry_key, entry.child, i)); // Track original index
                }
                // Sort by key (empty keys go last)
                sorted_entries.sort_by(|a, b| {
                    if a.0.is_empty() { std::cmp::Ordering::Greater }
                    else if b.0.is_empty() { std::cmp::Ordering::Less }
                    else { a.0.cmp(&b.0) }
                });

                // Find correct child
                let mut child_id = NULL_PAGE;
                let mut child_idx = 0usize; // Original index in page
                for (entry_key, entry_child, orig_idx) in &sorted_entries {
                    if key < entry_key.as_slice() {
                        child_id = *entry_child;
                        child_idx = *orig_idx;
                        break;
                    }
                    child_id = *entry_child;
                    child_idx = *orig_idx;
                }

                // If no match, use the last sorted entry
                if child_id == NULL_PAGE {
                    if let Some((_, last_child, last_idx)) = sorted_entries.last() {
                        child_id = *last_child;
                        child_idx = *last_idx;
                    } else {
                        return Ok(DeleteResult::NotFound);
                    }
                }

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
}
