//! Vector storage and operations
//!
//! Contiguous float32 array for embeddings with SIMD-accelerated operations.

use crate::{error::Result, EngramError, DEFAULT_DIMENSIONS};

/// Vector dimensions (512 for EmbeddingGemma)
pub const DIMS: usize = DEFAULT_DIMENSIONS as usize;

/// Size of one vector in bytes
pub const VECTOR_SIZE: usize = DIMS * std::mem::size_of::<f32>();

/// A 512-dimensional embedding vector
pub type Vector = [f32; DIMS];

/// Compute dot product of two vectors
///
/// This is the hot path for similarity search. SIMD-optimized.
#[inline]
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());

    // Try SIMD path first
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            return unsafe { dot_product_avx2(a, b) };
        }
    }

    // Fallback: unrolled scalar
    dot_product_scalar(a, b)
}

/// Scalar dot product (fallback)
#[inline]
fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    let mut sum = 0.0f32;

    // Process 8 elements at a time (helps compiler auto-vectorize)
    let chunks = a.len() / 8;
    for i in 0..chunks {
        let base = i * 8;
        sum += a[base] * b[base];
        sum += a[base + 1] * b[base + 1];
        sum += a[base + 2] * b[base + 2];
        sum += a[base + 3] * b[base + 3];
        sum += a[base + 4] * b[base + 4];
        sum += a[base + 5] * b[base + 5];
        sum += a[base + 6] * b[base + 6];
        sum += a[base + 7] * b[base + 7];
    }

    // Handle remainder
    for i in (chunks * 8)..a.len() {
        sum += a[i] * b[i];
    }

    sum
}

/// AVX2 SIMD dot product (8 floats at a time)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn dot_product_avx2(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let mut sum = _mm256_setzero_ps();
    let chunks = a.len() / 8;

    for i in 0..chunks {
        let base = i * 8;
        let va = _mm256_loadu_ps(a.as_ptr().add(base));
        let vb = _mm256_loadu_ps(b.as_ptr().add(base));
        sum = _mm256_fmadd_ps(va, vb, sum); // Fused multiply-add
    }

    // Horizontal sum of 8 floats
    let sum128 = _mm_add_ps(
        _mm256_castps256_ps128(sum),
        _mm256_extractf128_ps(sum, 1),
    );
    let sum64 = _mm_add_ps(sum128, _mm_movehl_ps(sum128, sum128));
    let sum32 = _mm_add_ss(sum64, _mm_shuffle_ps(sum64, sum64, 1));

    let mut result = 0.0f32;
    _mm_store_ss(&mut result, sum32);

    // Handle remainder
    for i in (chunks * 8)..a.len() {
        result += a[i] * b[i];
    }

    result
}

/// Compute cosine similarity between two vectors
#[inline]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot = dot_product(a, b);
    let norm_a = dot_product(a, a).sqrt();
    let norm_b = dot_product(b, b).sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Compute L2 (Euclidean) distance squared
#[inline]
pub fn l2_distance_squared(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());

    let mut sum = 0.0f32;
    for i in 0..a.len() {
        let diff = a[i] - b[i];
        sum += diff * diff;
    }
    sum
}

/// Normalize a vector to unit length (in-place)
#[inline]
pub fn normalize(v: &mut [f32]) {
    let norm = dot_product(v, v).sqrt();
    if norm > 0.0 {
        let inv_norm = 1.0 / norm;
        for x in v.iter_mut() {
            *x *= inv_norm;
        }
    }
}

/// Create a zero vector
#[inline]
pub fn zero_vector() -> Vector {
    [0.0f32; DIMS]
}

/// Vector store - manages contiguous array of embeddings
pub struct VectorStore {
    /// Contiguous storage: vectors[i] = vector for note ID i
    /// Note IDs are 1-indexed, so index 0 is unused
    vectors: Vec<f32>,

    /// Number of vectors stored
    count: usize,
}

impl VectorStore {
    /// Create a new vector store
    pub fn new() -> Self {
        Self {
            vectors: Vec::new(),
            count: 0,
        }
    }

    /// Create with pre-allocated capacity
    pub fn with_capacity(num_vectors: usize) -> Self {
        Self {
            vectors: Vec::with_capacity(num_vectors * DIMS),
            count: 0,
        }
    }

    /// Add a vector for a note ID
    /// Note: assumes sequential IDs starting from 1
    pub fn add(&mut self, id: u64, vector: &[f32]) -> Result<()> {
        if vector.len() != DIMS {
            return Err(EngramError::DimensionMismatch {
                expected: DIMS as u32,
                got: vector.len() as u32,
            });
        }

        // Extend storage if needed
        let needed_len = (id as usize) * DIMS;
        if self.vectors.len() < needed_len {
            self.vectors.resize(needed_len, 0.0);
        }

        // Copy vector
        let start = ((id - 1) as usize) * DIMS;
        self.vectors[start..start + DIMS].copy_from_slice(vector);
        self.count = self.count.max(id as usize);

        Ok(())
    }

    /// Get a vector by note ID
    pub fn get(&self, id: u64) -> Option<&[f32]> {
        if id == 0 || id as usize > self.count {
            return None;
        }

        let start = ((id - 1) as usize) * DIMS;
        let end = start + DIMS;

        if end <= self.vectors.len() {
            Some(&self.vectors[start..end])
        } else {
            None
        }
    }

    /// Find k nearest neighbors to a query vector
    pub fn nearest(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        let mut results: Vec<(u64, f32)> = Vec::with_capacity(self.count);

        for id in 1..=self.count {
            if let Some(vec) = self.get(id as u64) {
                let sim = cosine_similarity(query, vec);
                results.push((id as u64, sim));
            }
        }

        // Sort by similarity descending
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(k);

        results
    }

    /// Number of vectors stored
    pub fn len(&self) -> usize {
        self.count
    }

    /// Is the store empty?
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Total memory used in bytes
    pub fn memory_usage(&self) -> usize {
        self.vectors.len() * std::mem::size_of::<f32>()
    }

    /// Serialize vector store for persistence
    pub fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::new();

        // Write count
        data.extend_from_slice(&(self.count as u64).to_le_bytes());

        // Write raw vector data (already contiguous f32 array)
        // Convert f32 slice to bytes
        for &v in &self.vectors {
            data.extend_from_slice(&v.to_le_bytes());
        }

        data
    }

    /// Deserialize vector store from persisted data
    pub fn deserialize(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < 8 {
            return Ok(()); // Empty is valid
        }

        let mut offset = 0;

        // Read count
        self.count = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap()) as usize;
        offset += 8;

        // Read vector data
        let num_floats = (data.len() - offset) / 4;
        self.vectors = Vec::with_capacity(num_floats);

        while offset + 4 <= data.len() {
            let v = f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
            self.vectors.push(v);
            offset += 4;
        }

        Ok(())
    }

    /// Check if a non-zero embedding exists for the given ID
    /// Zero-vectors are not valid embeddings (sparse storage uses zeros for missing IDs)
    pub fn has(&self, id: u64) -> bool {
        if id == 0 || id as usize > self.count {
            return false;
        }
        let start = ((id - 1) as usize) * DIMS;
        let end = start + DIMS;
        if end > self.vectors.len() {
            return false;
        }
        // Check if any value is non-zero (a zero-vector is not a valid embedding)
        self.vectors[start..end].iter().any(|&v| v != 0.0)
    }
}

impl Default for VectorStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dot_product() {
        let a = [1.0, 2.0, 3.0, 4.0];
        let b = [5.0, 6.0, 7.0, 8.0];

        let result = dot_product(&a, &b);
        assert!((result - 70.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let a = [1.0, 2.0, 3.0, 4.0];
        let sim = cosine_similarity(&a, &a);
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = [1.0, 0.0, 0.0, 0.0];
        let b = [0.0, 1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 0.001);
    }

    #[test]
    fn test_normalize() {
        let mut v = [3.0, 4.0];
        normalize(&mut v);

        let norm = dot_product(&v, &v).sqrt();
        assert!((norm - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_vector_store() {
        let mut store = VectorStore::new();

        let v1 = [1.0f32; DIMS];
        let v2 = [2.0f32; DIMS];

        store.add(1, &v1).unwrap();
        store.add(2, &v2).unwrap();

        assert_eq!(store.len(), 2);

        let retrieved = store.get(1).unwrap();
        assert_eq!(retrieved[0], 1.0);

        let retrieved = store.get(2).unwrap();
        assert_eq!(retrieved[0], 2.0);
    }

    #[test]
    fn test_nearest_neighbors() {
        let mut store = VectorStore::new();

        // Create 10 vectors with different directions (not just magnitude)
        for i in 1..=10 {
            let mut v = [0.0f32; DIMS];
            v[0] = (i as f32).cos();
            v[1] = (i as f32).sin();
            store.add(i, &v).unwrap();
        }

        // Query for a specific direction
        let mut query = [0.0f32; DIMS];
        query[0] = (5.0_f32).cos();
        query[1] = (5.0_f32).sin();

        let nearest = store.nearest(&query, 3);
        assert_eq!(nearest.len(), 3);
        // ID 5 should be the closest since it matches the query direction
        assert_eq!(nearest[0].0, 5, "Closest should be ID 5");
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_simd_matches_scalar() {
        let a: Vec<f32> = (0..512).map(|i| i as f32 * 0.1).collect();
        let b: Vec<f32> = (0..512).map(|i| (512 - i) as f32 * 0.1).collect();

        let scalar = dot_product_scalar(&a, &b);
        let simd = dot_product(&a, &b);

        // Allow relative error up to 0.001% for SIMD vs scalar differences
        let relative_error = (scalar - simd).abs() / scalar.abs().max(1.0);
        assert!(relative_error < 0.0001, "scalar={} simd={} rel_error={}", scalar, simd, relative_error);
    }
}
