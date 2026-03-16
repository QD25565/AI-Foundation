//! Lock-free SPSC (Single Producer Single Consumer) Ring Buffer
//!
//! Uses atomic operations with Release/Acquire memory ordering for
//! safe cross-process communication without locks.
//!
//! Memory Layout:
//! ```text
//! ┌──────────┬──────────┬─────────────────────────────────┐
//! │ head (8B)│ tail (8B)│ data buffer (capacity bytes)    │
//! └──────────┴──────────┴─────────────────────────────────┘
//! ```

use std::sync::atomic::{AtomicU64, Ordering};

/// Header for the ring buffer stored in shared memory
#[repr(C)]
pub struct RingBufferHeader {
    /// Write position (owned by producer)
    head: AtomicU64,
    /// Read position (owned by consumer)
    tail: AtomicU64,
    /// Capacity of the data buffer
    capacity: u64,
    /// Padding to cache line boundary (64 bytes)
    _padding: [u8; 40],
}

impl RingBufferHeader {
    /// Size of the header in bytes
    pub const SIZE: usize = std::mem::size_of::<Self>();

    /// Initialize a new ring buffer header
    pub fn init(&mut self, capacity: u64) {
        self.head.store(0, Ordering::Relaxed);
        self.tail.store(0, Ordering::Relaxed);
        self.capacity = capacity;
    }

    /// Get current head position
    #[inline]
    pub fn head(&self) -> u64 {
        self.head.load(Ordering::Acquire)
    }

    /// Get current tail position
    #[inline]
    pub fn tail(&self) -> u64 {
        self.tail.load(Ordering::Acquire)
    }

    /// Available space for writing
    #[inline]
    pub fn available_write(&self) -> u64 {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        self.capacity - (head - tail)
    }

    /// Available data for reading
    #[inline]
    pub fn available_read(&self) -> u64 {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);
        head - tail
    }

    /// Advance head (after write)
    #[inline]
    pub fn advance_head(&self, amount: u64) {
        self.head.fetch_add(amount, Ordering::Release);
    }

    /// Advance tail (after read)
    #[inline]
    pub fn advance_tail(&self, amount: u64) {
        self.tail.fetch_add(amount, Ordering::Release);
    }
}

/// SPSC Ring Buffer operating on shared memory
pub struct SpscRingBuffer<'a> {
    header: &'a RingBufferHeader,
    data: &'a mut [u8],
}

impl<'a> SpscRingBuffer<'a> {
    /// Create a ring buffer from raw memory pointers
    ///
    /// # Safety
    /// The caller must ensure:
    /// - `header_ptr` points to valid RingBufferHeader
    /// - `data_ptr` points to `capacity` bytes of valid memory
    /// - Memory remains valid for lifetime 'a
    pub unsafe fn from_raw(header_ptr: *mut RingBufferHeader, data_ptr: *mut u8, capacity: usize) -> Self {
        Self {
            header: &*header_ptr,
            data: std::slice::from_raw_parts_mut(data_ptr, capacity),
        }
    }

    /// Try to write data to the buffer
    /// Returns number of bytes written, or 0 if no space available
    pub fn try_write(&mut self, data: &[u8]) -> usize {
        let available = self.header.available_write() as usize;
        if available < data.len() + 4 {
            return 0; // Need space for length prefix + data
        }

        let head = self.header.head() as usize;
        let capacity = self.data.len();

        // Write length prefix (4 bytes, little-endian)
        let len_bytes = (data.len() as u32).to_le_bytes();
        for (i, &b) in len_bytes.iter().enumerate() {
            self.data[(head + i) % capacity] = b;
        }

        // Write data
        for (i, &b) in data.iter().enumerate() {
            self.data[(head + 4 + i) % capacity] = b;
        }

        self.header.advance_head((data.len() + 4) as u64);
        data.len()
    }

    /// Try to read data from the buffer
    /// Returns the data if available, or None if empty
    pub fn try_read(&mut self) -> Option<Vec<u8>> {
        let available = self.header.available_read() as usize;
        if available < 4 {
            return None; // Not even length prefix available
        }

        let tail = self.header.tail() as usize;
        let capacity = self.data.len();

        // Read length prefix
        let len_bytes = [
            self.data[tail % capacity],
            self.data[(tail + 1) % capacity],
            self.data[(tail + 2) % capacity],
            self.data[(tail + 3) % capacity],
        ];
        let len = u32::from_le_bytes(len_bytes) as usize;

        if available < len + 4 {
            return None; // Message not fully available
        }

        // Read data
        let mut data = vec![0u8; len];
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = self.data[(tail + 4 + i) % capacity];
        }

        self.header.advance_tail((len + 4) as u64);
        Some(data)
    }

    /// Check if the buffer is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.header.available_read() == 0
    }

    /// Get available space for writing
    #[inline]
    pub fn available_space(&self) -> usize {
        self.header.available_write() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{alloc, dealloc, Layout};

    #[test]
    fn test_ring_buffer_basic() {
        // Allocate memory for test
        let header_layout = Layout::new::<RingBufferHeader>();
        let data_size = 1024;
        let data_layout = Layout::from_size_align(data_size, 8).unwrap();

        unsafe {
            let header_ptr = alloc(header_layout) as *mut RingBufferHeader;
            let data_ptr = alloc(data_layout);

            // Initialize
            (*header_ptr).init(data_size as u64);

            let mut rb = SpscRingBuffer::from_raw(header_ptr, data_ptr, data_size);

            // Write and read
            let msg = b"Hello, World!";
            assert_eq!(rb.try_write(msg), msg.len());

            let read = rb.try_read().expect("Should have data");
            assert_eq!(&read, msg);

            // Clean up
            dealloc(header_ptr as *mut u8, header_layout);
            dealloc(data_ptr, data_layout);
        }
    }

    #[test]
    fn test_ring_buffer_multiple_messages() {
        let header_layout = Layout::new::<RingBufferHeader>();
        let data_size = 256;
        let data_layout = Layout::from_size_align(data_size, 8).unwrap();

        unsafe {
            let header_ptr = alloc(header_layout) as *mut RingBufferHeader;
            let data_ptr = alloc(data_layout);
            (*header_ptr).init(data_size as u64);

            let mut rb = SpscRingBuffer::from_raw(header_ptr, data_ptr, data_size);

            // Write multiple messages
            for i in 0..5 {
                let msg = format!("Message {}", i);
                rb.try_write(msg.as_bytes());
            }

            // Read all back
            for i in 0..5 {
                let read = rb.try_read().expect("Should have data");
                let expected = format!("Message {}", i);
                assert_eq!(String::from_utf8(read).unwrap(), expected);
            }

            dealloc(header_ptr as *mut u8, header_layout);
            dealloc(data_ptr, data_layout);
        }
    }
}
