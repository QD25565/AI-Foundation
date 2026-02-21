//! B+Tree Page Structures
//!
//! Pages are 4KB fixed-size blocks that form the B+Tree.
//! Two types: Branch (internal) and Leaf (data).

use std::mem;
use crc32fast::Hasher;

/// Page size in bytes (4KB)
pub const PAGE_SIZE: usize = 4096;

/// Page identifier (file offset / PAGE_SIZE)
pub type PageId = u64;

/// Invalid/null page ID
pub const NULL_PAGE: PageId = u64::MAX;

/// Page type discriminator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PageType {
    /// Free/unallocated page
    Free = 0,
    /// Branch node (internal, contains keys + child pointers)
    Branch = 1,
    /// Leaf node (contains keys + values)
    Leaf = 2,
    /// Overflow page for large values
    Overflow = 3,
    /// File header/meta page
    Meta = 4,
}

impl From<u8> for PageType {
    fn from(v: u8) -> Self {
        match v {
            0 => PageType::Free,
            1 => PageType::Branch,
            2 => PageType::Leaf,
            3 => PageType::Overflow,
            4 => PageType::Meta,
            _ => PageType::Free,
        }
    }
}

/// Page header (32 bytes)
/// Present at the start of every page
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct PageHeader {
    /// Page type
    pub page_type: u8,
    /// Flags (reserved)
    pub flags: u8,
    /// Number of items in this page
    pub count: u16,
    /// Free space offset (for leaf pages)
    pub free_offset: u16,
    /// Reserved
    pub _reserved: u16,
    /// Page ID (for verification)
    pub page_id: PageId,
    /// Transaction ID that created this page
    pub txn_id: u64,
    /// CRC32 checksum of page contents
    pub checksum: u32,
    /// Padding to 32 bytes
    pub _padding: u32,
}

impl PageHeader {
    pub const SIZE: usize = 32;

    pub fn new(page_type: PageType, page_id: PageId, txn_id: u64) -> Self {
        Self {
            page_type: page_type as u8,
            flags: 0,
            count: 0,
            free_offset: PAGE_SIZE as u16,
            _reserved: 0,
            page_id,
            txn_id,
            checksum: 0,
            _padding: 0,
        }
    }

    pub fn page_type(&self) -> PageType {
        PageType::from(self.page_type)
    }
}

/// Branch page entry (key + child pointer)
/// Keys are stored inline for small keys, or as offset for large keys
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct BranchEntry {
    /// Child page ID (left of this key)
    pub child: PageId,
    /// Key length
    pub key_len: u16,
    /// Key offset in page (or inline if small)
    pub key_offset: u16,
    /// Reserved
    pub _reserved: u32,
}

impl BranchEntry {
    pub const SIZE: usize = 16;
}

/// Leaf page entry (key + value)
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct LeafEntry {
    /// Key length
    pub key_len: u16,
    /// Value length
    pub val_len: u16,
    /// Key offset in page data area
    pub key_offset: u16,
    /// Value offset in page data area
    pub val_offset: u16,
}

impl LeafEntry {
    pub const SIZE: usize = 8;
}

/// Meta page structure (file header)
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct MetaPage {
    /// Page header
    pub header: PageHeader,
    /// Magic number
    pub magic: u64,
    /// File format version
    pub version: u32,
    /// Page size
    pub page_size: u32,
    /// Primary root page ID
    pub root_primary: PageId,
    /// Shadow root page ID (for atomic commits)
    pub root_shadow: PageId,
    /// Which root is active (0 = primary, 1 = shadow)
    pub active_root: u8,
    /// Padding
    pub _pad1: [u8; 7],
    /// Free list head page
    pub free_list_head: PageId,
    /// Total pages in file
    pub total_pages: u64,
    /// Free page count
    pub free_pages: u64,
    /// Current transaction ID
    pub txn_id: u64,
    /// Timestamp of last commit
    pub last_commit: u64,
}

impl MetaPage {
    pub const SIZE: usize = mem::size_of::<Self>();

    pub fn new(page_size: u32) -> Self {
        Self {
            header: PageHeader::new(PageType::Meta, 0, 0),
            magic: crate::MAGIC,
            version: crate::VERSION,
            page_size,
            root_primary: NULL_PAGE,
            root_shadow: NULL_PAGE,
            active_root: 0,
            _pad1: [0; 7],
            free_list_head: NULL_PAGE,
            total_pages: 1, // Just the meta page initially
            free_pages: 0,
            txn_id: 0,
            last_commit: 0,
        }
    }

    /// Get the currently active root page
    pub fn active_root_page(&self) -> PageId {
        if self.active_root == 0 {
            self.root_primary
        } else {
            self.root_shadow
        }
    }

    /// Swap active root (for atomic commit)
    pub fn swap_roots(&mut self) {
        self.active_root = if self.active_root == 0 { 1 } else { 0 };
    }
}

/// A page buffer for in-memory manipulation
#[derive(Clone)]
pub struct Page {
    pub data: [u8; PAGE_SIZE],
}

impl Page {
    /// Create a new zeroed page
    pub fn new() -> Self {
        Self {
            data: [0u8; PAGE_SIZE],
        }
    }

    /// Get header reference
    pub fn header(&self) -> &PageHeader {
        unsafe { &*(self.data.as_ptr() as *const PageHeader) }
    }

    /// Get mutable header reference
    pub fn header_mut(&mut self) -> &mut PageHeader {
        unsafe { &mut *(self.data.as_mut_ptr() as *mut PageHeader) }
    }

    /// Initialize as a leaf page
    pub fn init_leaf(&mut self, page_id: PageId, txn_id: u64) {
        let header = self.header_mut();
        *header = PageHeader::new(PageType::Leaf, page_id, txn_id);
        header.free_offset = PAGE_SIZE as u16;
    }

    /// Initialize as a branch page
    pub fn init_branch(&mut self, page_id: PageId, txn_id: u64) {
        let header = self.header_mut();
        *header = PageHeader::new(PageType::Branch, page_id, txn_id);
    }

    /// Get leaf entries slice
    pub fn leaf_entries(&self) -> &[LeafEntry] {
        let count = self.header().count as usize;
        let ptr = unsafe {
            self.data.as_ptr().add(PageHeader::SIZE) as *const LeafEntry
        };
        unsafe { std::slice::from_raw_parts(ptr, count) }
    }

    /// Get mutable leaf entries slice
    pub fn leaf_entries_mut(&mut self) -> &mut [LeafEntry] {
        let count = self.header().count as usize;
        let ptr = unsafe {
            self.data.as_mut_ptr().add(PageHeader::SIZE) as *mut LeafEntry
        };
        unsafe { std::slice::from_raw_parts_mut(ptr, count) }
    }

    /// Get branch entries slice
    pub fn branch_entries(&self) -> &[BranchEntry] {
        let count = self.header().count as usize;
        let ptr = unsafe {
            self.data.as_ptr().add(PageHeader::SIZE) as *const BranchEntry
        };
        unsafe { std::slice::from_raw_parts(ptr, count) }
    }

    /// Calculate available space for new entries in a leaf page
    pub fn leaf_free_space(&self) -> usize {
        let header = self.header();
        let entries_end = PageHeader::SIZE + (header.count as usize * LeafEntry::SIZE);
        let data_start = header.free_offset as usize;
        if data_start > entries_end {
            data_start - entries_end
        } else {
            0
        }
    }

    /// Insert a key-value pair into a leaf page at the correct sorted position
    /// Returns true if successful, false if not enough space
    pub fn leaf_insert(&mut self, key: &[u8], value: &[u8]) -> bool {
        let needed = LeafEntry::SIZE + key.len() + value.len();
        if self.leaf_free_space() < needed {
            return false;
        }

        // Find the insertion point to maintain sorted order
        let insert_pos = match self.leaf_search(key) {
            Ok(_) => return false, // Key already exists - use leaf_upsert instead
            Err(pos) => pos,
        };

        // Read values we need from header first
        let count = self.header().count as usize;
        let free_offset = self.header().free_offset as usize;

        // Allocate space for value (grows down from end)
        let val_offset = free_offset - value.len();
        self.data[val_offset..val_offset + value.len()].copy_from_slice(value);

        // Allocate space for key
        let key_offset = val_offset - key.len();
        self.data[key_offset..key_offset + key.len()].copy_from_slice(key);

        // Create the new entry
        let entry = LeafEntry {
            key_len: key.len() as u16,
            val_len: value.len() as u16,
            key_offset: key_offset as u16,
            val_offset: val_offset as u16,
        };

        // Shift existing entries to make room at insert_pos
        if insert_pos < count {
            // Move entries from insert_pos to count-1 one position to the right
            let src_offset = PageHeader::SIZE + insert_pos * LeafEntry::SIZE;
            let dst_offset = PageHeader::SIZE + (insert_pos + 1) * LeafEntry::SIZE;
            let bytes_to_move = (count - insert_pos) * LeafEntry::SIZE;

            // Use copy_within for safe overlapping copy
            self.data.copy_within(src_offset..src_offset + bytes_to_move, dst_offset);
        }

        // Insert new entry at the correct position
        let entry_offset = PageHeader::SIZE + insert_pos * LeafEntry::SIZE;
        unsafe {
            let ptr = self.data.as_mut_ptr().add(entry_offset) as *mut LeafEntry;
            *ptr = entry;
        }

        // Update header at end
        let header = self.header_mut();
        header.free_offset = key_offset as u16;
        header.count += 1;
        true
    }

    /// Get key bytes for a leaf entry
    pub fn leaf_key(&self, index: usize) -> &[u8] {
        let entries = self.leaf_entries();
        if index >= entries.len() {
            return &[];
        }
        let entry = &entries[index];
        let start = entry.key_offset as usize;
        let end = start + entry.key_len as usize;
        &self.data[start..end]
    }

    /// Get value bytes for a leaf entry
    pub fn leaf_value(&self, index: usize) -> &[u8] {
        let entries = self.leaf_entries();
        if index >= entries.len() {
            return &[];
        }
        let entry = &entries[index];
        let start = entry.val_offset as usize;
        let end = start + entry.val_len as usize;
        &self.data[start..end]
    }

    /// Upsert a key-value pair (insert or update)
    /// Returns true if successful, false if not enough space
    pub fn leaf_upsert(&mut self, key: &[u8], value: &[u8]) -> bool {
        match self.leaf_search(key) {
            Ok(index) => {
                // Key exists - update the value
                // Allocate new space for value (old space becomes garbage - acceptable)
                let free_offset = self.header().free_offset as usize;
                if free_offset < value.len() + PageHeader::SIZE + (self.header().count as usize) * LeafEntry::SIZE {
                    return false; // Not enough space
                }

                let val_offset = free_offset - value.len();
                self.data[val_offset..val_offset + value.len()].copy_from_slice(value);

                // Update entry to point to new value
                let entry_offset = PageHeader::SIZE + index * LeafEntry::SIZE;
                unsafe {
                    let ptr = self.data.as_mut_ptr().add(entry_offset) as *mut LeafEntry;
                    (*ptr).val_offset = val_offset as u16;
                    (*ptr).val_len = value.len() as u16;
                }

                // Update free offset
                self.header_mut().free_offset = val_offset as u16;
                true
            }
            Err(_) => {
                // Key doesn't exist - use regular insert
                self.leaf_insert(key, value)
            }
        }
    }

    /// Search for a key in a leaf page (binary search)
    /// Returns Ok(index) if found, Err(index) for insertion point
    pub fn leaf_search(&self, key: &[u8]) -> Result<usize, usize> {
        let count = self.header().count as usize;
        if count == 0 {
            return Err(0);
        }

        let mut lo = 0;
        let mut hi = count;

        while lo < hi {
            let mid = (lo + hi) / 2;
            let mid_key = self.leaf_key(mid);
            match mid_key.cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Ok(mid),
            }
        }

        Err(lo)
    }

    /// Calculate CRC32 checksum
    pub fn calculate_checksum(&self) -> u32 {
        let mut hasher = Hasher::new();
        // Checksum everything except the checksum field itself (offset 24, 4 bytes)
        hasher.update(&self.data[..24]); // Before checksum (bytes 0-23)
        hasher.update(&self.data[28..]); // After checksum (bytes 28-end)
        hasher.finalize()
    }

    /// Update the checksum in the header
    pub fn update_checksum(&mut self) {
        let checksum = self.calculate_checksum();
        self.header_mut().checksum = checksum;
    }

    /// Verify the checksum
    pub fn verify_checksum(&self) -> bool {
        self.header().checksum == self.calculate_checksum()
    }

    /// Delete an entry at the given index from a leaf page
    /// Shifts remaining entries left to fill the gap
    /// Returns true if successful, false if index out of bounds
    pub fn leaf_delete(&mut self, index: usize) -> bool {
        let count = self.header().count as usize;
        if index >= count {
            return false;
        }

        // Shift entries after index one position left to fill the gap
        if index < count - 1 {
            let src_offset = PageHeader::SIZE + (index + 1) * LeafEntry::SIZE;
            let dst_offset = PageHeader::SIZE + index * LeafEntry::SIZE;
            let bytes_to_move = (count - 1 - index) * LeafEntry::SIZE;

            // Use copy_within for safe overlapping copy
            self.data.copy_within(src_offset..src_offset + bytes_to_move, dst_offset);
        }

        // Decrement count
        // Note: We don't reclaim the key/value space - it becomes garbage
        // This is acceptable as the page will eventually be rewritten on split
        self.header_mut().count -= 1;
        true
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_sizes() {
        assert_eq!(PageHeader::SIZE, 32);
        assert_eq!(LeafEntry::SIZE, 8);
        assert_eq!(BranchEntry::SIZE, 16);
    }

    #[test]
    fn test_leaf_insert_and_search() {
        let mut page = Page::new();
        page.init_leaf(1, 1);

        assert!(page.leaf_insert(b"apple", b"red"));
        assert!(page.leaf_insert(b"banana", b"yellow"));
        assert!(page.leaf_insert(b"cherry", b"red"));

        // Copy count to avoid packed struct alignment issue
        let count = page.header().count;
        assert_eq!(count, 3);
        assert_eq!(page.leaf_search(b"banana"), Ok(1));
        assert_eq!(page.leaf_search(b"date"), Err(3));

        assert_eq!(page.leaf_key(0), b"apple");
        assert_eq!(page.leaf_value(0), b"red");
    }

    #[test]
    fn test_checksum() {
        let mut page = Page::new();
        page.init_leaf(1, 1);
        page.leaf_insert(b"test", b"value");
        page.update_checksum();

        assert!(page.verify_checksum());

        // Corrupt the page
        page.data[100] ^= 0xFF;
        assert!(!page.verify_checksum());
    }
}
