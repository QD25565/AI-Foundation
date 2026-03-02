//! Bloom filter for fast negative lookups
//!
//! A Bloom filter is a probabilistic data structure that can tell you:
//! - "Definitely not in set" (100% accurate)
//! - "Possibly in set" (may have false positives)
//!
//! Use cases in Engram:
//! - Fast check if a tag has any notes (avoid index scan for non-existent tags)
//! - Fast check if a note ID might exist (avoid hash lookup)
//!
//! This implementation uses multiple hash functions derived from a single
//! 64-bit hash using the technique from Kirsch & Mitzenmacher (2006).

use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

/// Bloom filter with configurable size and number of hash functions
pub struct BloomFilter {
    /// Bit array stored as bytes
    bits: Vec<u8>,
    /// Number of bits in the filter
    num_bits: usize,
    /// Number of hash functions to use
    num_hashes: u8,
    /// Number of items inserted
    count: usize,
}

impl BloomFilter {
    /// Create a new Bloom filter
    ///
    /// # Arguments
    /// * `expected_items` - Expected number of items to insert
    /// * `false_positive_rate` - Desired false positive rate (e.g., 0.01 for 1%)
    ///
    /// # Example
    /// ```
    /// use engram::bloom::BloomFilter;
    /// let filter = BloomFilter::new(10000, 0.01); // 10K items, 1% FP rate
    /// ```
    pub fn new(expected_items: usize, false_positive_rate: f64) -> Self {
        // Calculate optimal number of bits: m = -n*ln(p) / (ln(2)^2)
        let ln2_squared = std::f64::consts::LN_2.powi(2);
        let num_bits = (-(expected_items as f64) * false_positive_rate.ln() / ln2_squared).ceil() as usize;

        // Ensure at least 64 bits
        let num_bits = num_bits.max(64);

        // Calculate optimal number of hash functions: k = (m/n) * ln(2)
        let num_hashes = ((num_bits as f64 / expected_items as f64) * std::f64::consts::LN_2).ceil() as u8;
        let num_hashes = num_hashes.clamp(1, 16); // Reasonable bounds

        // Round up to nearest byte
        let num_bytes = (num_bits + 7) / 8;

        Self {
            bits: vec![0u8; num_bytes],
            num_bits,
            num_hashes,
            count: 0,
        }
    }

    /// Create a Bloom filter with explicit parameters
    pub fn with_params(num_bits: usize, num_hashes: u8) -> Self {
        let num_bits = num_bits.max(64); // Minimum 64 bits to prevent divide-by-zero
        let num_bytes = (num_bits + 7) / 8;
        Self {
            bits: vec![0u8; num_bytes],
            num_bits,
            num_hashes: num_hashes.clamp(1, 16),
            count: 0,
        }
    }

    /// Create a small, fast filter (higher false positive rate but faster)
    /// Good for quick checks, ~5% false positive rate with 1000 items
    pub fn fast(expected_items: usize) -> Self {
        Self::new(expected_items, 0.05)
    }

    /// Create a precise filter (lower false positive rate, more memory)
    /// ~0.1% false positive rate
    pub fn precise(expected_items: usize) -> Self {
        Self::new(expected_items, 0.001)
    }

    /// Insert an item into the filter
    pub fn insert<T: Hash>(&mut self, item: &T) {
        let (h1, h2) = self.hash_pair(item);

        for i in 0..self.num_hashes {
            let bit_idx = self.get_bit_index(h1, h2, i);
            self.set_bit(bit_idx);
        }

        self.count += 1;
    }

    /// Check if an item might be in the filter
    ///
    /// Returns:
    /// - `false`: Item is DEFINITELY NOT in the set
    /// - `true`: Item MIGHT be in the set (possible false positive)
    pub fn might_contain<T: Hash>(&self, item: &T) -> bool {
        let (h1, h2) = self.hash_pair(item);

        for i in 0..self.num_hashes {
            let bit_idx = self.get_bit_index(h1, h2, i);
            if !self.get_bit(bit_idx) {
                return false; // Definitely not in set
            }
        }

        true // Possibly in set
    }

    /// Clear all bits
    pub fn clear(&mut self) {
        self.bits.fill(0);
        self.count = 0;
    }

    /// Get the number of items inserted
    pub fn count(&self) -> usize {
        self.count
    }

    /// Get the size in bytes
    pub fn size_bytes(&self) -> usize {
        self.bits.len()
    }

    /// Get the number of bits
    pub fn num_bits(&self) -> usize {
        self.num_bits
    }

    /// Estimate the current false positive rate
    pub fn estimated_fp_rate(&self) -> f64 {
        let bits_set = self.bits.iter().map(|b| b.count_ones() as usize).sum::<usize>();
        let fill_ratio = bits_set as f64 / self.num_bits as f64;
        fill_ratio.powi(self.num_hashes as i32)
    }

    // === Serialization ===

    /// Serialize the Bloom filter to bytes for persistence
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(21 + self.bits.len());

        // Write metadata
        bytes.extend_from_slice(&(self.num_bits as u64).to_le_bytes());  // 8 bytes
        bytes.push(self.num_hashes);                                      // 1 byte
        bytes.extend_from_slice(&(self.count as u64).to_le_bytes());     // 8 bytes
        bytes.extend_from_slice(&(self.bits.len() as u32).to_le_bytes()); // 4 bytes

        // Write bit array
        bytes.extend_from_slice(&self.bits);

        bytes
    }

    /// Deserialize a Bloom filter from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 21 {
            return None;
        }

        let num_bits = u64::from_le_bytes(bytes[0..8].try_into().ok()?) as usize;
        let num_hashes = bytes[8];
        let count = u64::from_le_bytes(bytes[9..17].try_into().ok()?) as usize;
        let bits_len = u32::from_le_bytes(bytes[17..21].try_into().ok()?) as usize;

        if bytes.len() < 21 + bits_len {
            return None;
        }

        let bits = bytes[21..21 + bits_len].to_vec();

        Some(Self {
            bits,
            num_bits,
            num_hashes,
            count,
        })
    }

    /// Get the serialized size in bytes
    pub fn serialized_size(&self) -> usize {
        21 + self.bits.len()
    }

    // === Internal methods ===

    /// Compute two hash values for double hashing
    fn hash_pair<T: Hash>(&self, item: &T) -> (u64, u64) {
        let mut hasher = DefaultHasher::new();
        item.hash(&mut hasher);
        let hash = hasher.finish();

        // Split the 64-bit hash into two 32-bit values
        // Then use additional mixing for the second hash
        let h1 = hash;
        let h2 = hash.rotate_left(32) ^ 0x517cc1b727220a95; // FNV offset basis

        (h1, h2)
    }

    /// Get bit index using double hashing: h(i) = h1 + i*h2
    fn get_bit_index(&self, h1: u64, h2: u64, i: u8) -> usize {
        (h1.wrapping_add(h2.wrapping_mul(i as u64)) % self.num_bits as u64) as usize
    }

    /// Set a bit at the given index
    fn set_bit(&mut self, idx: usize) {
        let byte_idx = idx / 8;
        let bit_idx = idx % 8;
        self.bits[byte_idx] |= 1 << bit_idx;
    }

    /// Get a bit at the given index
    fn get_bit(&self, idx: usize) -> bool {
        let byte_idx = idx / 8;
        let bit_idx = idx % 8;
        (self.bits[byte_idx] >> bit_idx) & 1 == 1
    }
}

impl Default for BloomFilter {
    fn default() -> Self {
        Self::new(1000, 0.01) // Default: 1000 items, 1% FP rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let mut filter = BloomFilter::new(100, 0.01);

        // Insert some items
        filter.insert(&"hello");
        filter.insert(&"world");
        filter.insert(&42u64);

        // Should find inserted items
        assert!(filter.might_contain(&"hello"));
        assert!(filter.might_contain(&"world"));
        assert!(filter.might_contain(&42u64));

        // Should NOT find items that were never inserted
        // (with very high probability given low FP rate)
        assert!(!filter.might_contain(&"goodbye"));
        assert!(!filter.might_contain(&"universe"));
        assert!(!filter.might_contain(&12345u64));
    }

    #[test]
    fn test_false_positive_rate() {
        let expected_items = 1000;
        let target_fp_rate = 0.01; // 1%

        let mut filter = BloomFilter::new(expected_items, target_fp_rate);

        // Insert items 0-999
        for i in 0..expected_items {
            filter.insert(&i);
        }

        // Check items 1000-1999 (none should be in filter)
        let mut false_positives = 0;
        for i in expected_items..(expected_items * 2) {
            if filter.might_contain(&i) {
                false_positives += 1;
            }
        }

        // Actual FP rate should be close to target
        let actual_fp_rate = false_positives as f64 / expected_items as f64;

        // Allow 3x tolerance (statistical variation)
        assert!(
            actual_fp_rate < target_fp_rate * 3.0,
            "FP rate {} exceeds tolerance (target: {})",
            actual_fp_rate,
            target_fp_rate
        );
    }

    #[test]
    fn test_size_calculation() {
        // For 10K items at 1% FP rate, should be ~12KB
        let filter = BloomFilter::new(10_000, 0.01);
        let size_kb = filter.size_bytes() as f64 / 1024.0;

        // Should be roughly 10-15KB
        assert!(size_kb > 5.0 && size_kb < 20.0, "Size: {} KB", size_kb);
    }

    #[test]
    fn test_clear() {
        let mut filter = BloomFilter::new(100, 0.01);

        filter.insert(&"test");
        assert!(filter.might_contain(&"test"));

        filter.clear();
        assert!(!filter.might_contain(&"test"));
        assert_eq!(filter.count(), 0);
    }

    #[test]
    fn test_fast_filter() {
        let filter = BloomFilter::fast(1000);
        // Fast filter should use less memory
        assert!(filter.size_bytes() < BloomFilter::new(1000, 0.01).size_bytes());
    }

    #[test]
    fn test_precise_filter() {
        let filter = BloomFilter::precise(1000);
        // Precise filter should use more memory
        assert!(filter.size_bytes() > BloomFilter::new(1000, 0.01).size_bytes());
    }
}
