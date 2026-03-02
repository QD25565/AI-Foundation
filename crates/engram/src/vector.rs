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

/// A quantized 512-dimensional embedding vector.
/// Symmetric per-vector quantization: q[i] = round(v[i] / scale * 127).
/// Scales cancel in cosine similarity, so the hot path is pure integer arithmetic.
#[derive(Clone)]
pub struct QuantizedVector {
    /// Quantized values in [-127, 127]
    pub values: Vec<i8>,
    /// Scale factor: max(|original[i]|), for dequantization only
    pub scale: f32,
    /// Precomputed L2 norm of quantized values: sqrt(sum(q[i]²))
    pub norm: f32,
}

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

/// Quantize an f32 vector to i8 with symmetric per-vector scaling.
/// Scale = max(|v[i]|), quantized[i] = round(v[i] / scale * 127).
/// Precomputes the L2 norm of quantized values for fast cosine similarity.
pub fn quantize(v: &[f32]) -> QuantizedVector {
    let max_abs = v.iter().fold(0.0f32, |acc, &x| acc.max(x.abs()));
    let scale = if max_abs > 0.0 { max_abs } else { 1.0 };
    let inv_scale = 127.0 / scale;

    let values: Vec<i8> = v.iter()
        .map(|&x| (x * inv_scale).round().clamp(-127.0, 127.0) as i8)
        .collect();

    let norm_sq: i32 = values.iter().map(|&x| (x as i32) * (x as i32)).sum();
    let norm = (norm_sq as f32).sqrt();

    QuantizedVector { values, scale, norm }
}

/// Dequantize back to f32: v[i] = q[i] * scale / 127
pub fn dequantize(q: &QuantizedVector) -> Vec<f32> {
    let factor = q.scale / 127.0;
    q.values.iter().map(|&x| x as f32 * factor).collect()
}

/// Integer dot product of two quantized vectors.
/// Returns raw i32 sum — caller divides by norms for cosine similarity.
#[inline]
pub fn quantized_dot_product_i32(a: &[i8], b: &[i8]) -> i32 {
    debug_assert_eq!(a.len(), b.len());

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            return unsafe { quantized_dot_product_avx2(a, b) };
        }
    }

    quantized_dot_product_scalar(a, b)
}

/// Scalar i8 dot product (fallback)
#[inline]
fn quantized_dot_product_scalar(a: &[i8], b: &[i8]) -> i32 {
    let mut sum: i32 = 0;
    let chunks = a.len() / 8;
    for i in 0..chunks {
        let base = i * 8;
        sum += a[base] as i32 * b[base] as i32;
        sum += a[base + 1] as i32 * b[base + 1] as i32;
        sum += a[base + 2] as i32 * b[base + 2] as i32;
        sum += a[base + 3] as i32 * b[base + 3] as i32;
        sum += a[base + 4] as i32 * b[base + 4] as i32;
        sum += a[base + 5] as i32 * b[base + 5] as i32;
        sum += a[base + 6] as i32 * b[base + 6] as i32;
        sum += a[base + 7] as i32 * b[base + 7] as i32;
    }
    for i in (chunks * 8)..a.len() {
        sum += a[i] as i32 * b[i] as i32;
    }
    sum
}

/// AVX2 SIMD i8 dot product — processes 32 elements per iteration (4x more than f32 AVX2).
/// Uses the sign trick: dot(a,b) = dot(|a|, b*sign(a)) via maddubs_epi16.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn quantized_dot_product_avx2(a: &[i8], b: &[i8]) -> i32 {
    use std::arch::x86_64::*;

    let mut acc = _mm256_setzero_si256();
    let ones_16 = _mm256_set1_epi16(1);
    let chunks = a.len() / 32;

    for i in 0..chunks {
        let base = i * 32;
        let va = _mm256_loadu_si256(a.as_ptr().add(base) as *const __m256i);
        let vb = _mm256_loadu_si256(b.as_ptr().add(base) as *const __m256i);

        // Signed i8 × i8 via abs/sign trick:
        // |a| is unsigned (0..127 since we clamp to [-127,127])
        // sign(b, a) flips b's sign where a is negative
        let abs_a = _mm256_abs_epi8(va);
        let sign_b = _mm256_sign_epi8(vb, va);

        // u8 × i8 → i16 with pairwise addition (32 → 16 values)
        let prod_16 = _mm256_maddubs_epi16(abs_a, sign_b);

        // i16 pairs → i32 accumulation (16 → 8 values)
        let prod_32 = _mm256_madd_epi16(prod_16, ones_16);

        acc = _mm256_add_epi32(acc, prod_32);
    }

    // Horizontal sum of 8 i32 values
    let sum128 = _mm_add_epi32(
        _mm256_castsi256_si128(acc),
        _mm256_extracti128_si256(acc, 1),
    );
    let sum64 = _mm_add_epi32(sum128, _mm_srli_si128(sum128, 8));
    let sum32 = _mm_add_epi32(sum64, _mm_srli_si128(sum64, 4));
    let mut result = _mm_cvtsi128_si32(sum32);

    // Handle remainder
    for i in (chunks * 32)..a.len() {
        result += a[i] as i32 * b[i] as i32;
    }

    result
}

/// Cosine similarity using quantized vectors.
/// Scales cancel: cos(a,b) = dot_q(a,b) / (norm_a * norm_b).
/// Hot path is pure integer dot product + one float division.
#[inline]
pub fn quantized_cosine_similarity(a: &QuantizedVector, b: &QuantizedVector) -> f32 {
    if a.norm == 0.0 || b.norm == 0.0 {
        return 0.0;
    }
    let dot = quantized_dot_product_i32(&a.values, &b.values);
    dot as f32 / (a.norm * b.norm)
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

    /// Number of vectors stored
    pub fn count(&self) -> usize {
        self.count
    }

    /// Get all vectors as a HashMap<note_id, Vec<f32>> for HNSW repopulation
    pub fn all_vectors(&self) -> std::collections::HashMap<u64, Vec<f32>> {
        let mut map = std::collections::HashMap::with_capacity(self.count);
        for id in 1..=self.count as u64 {
            if let Some(vec) = self.get(id) {
                map.insert(id, vec.to_vec());
            }
        }
        map
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

    /// Serialize vector store in quantized format (3.94x smaller on disk).
    /// Format: [u64::MAX sentinel][u8 version=1][u64 count][i8 values...][f32 scales...]
    /// Reads back via deserialize() which auto-detects old f32 format for backward compat.
    pub fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(17 + self.count * (DIMS + 4));

        // Quantized format header
        data.extend_from_slice(&u64::MAX.to_le_bytes()); // sentinel
        data.push(1u8); // version
        data.extend_from_slice(&(self.count as u64).to_le_bytes());

        // Quantize each vector and write i8 values + collect scales
        let mut scales: Vec<f32> = Vec::with_capacity(self.count);
        for i in 0..self.count {
            let start = i * DIMS;
            let end = start + DIMS;
            if end <= self.vectors.len() {
                let q = quantize(&self.vectors[start..end]);
                for &v in &q.values {
                    data.push(v as u8);
                }
                scales.push(q.scale);
            } else {
                data.extend(std::iter::repeat(0u8).take(DIMS));
                scales.push(0.0);
            }
        }

        // Write scales (for dequantization on load)
        for &s in &scales {
            data.extend_from_slice(&s.to_le_bytes());
        }

        data
    }

    /// Deserialize vector store — auto-detects quantized (new) or f32 (legacy) format.
    /// Quantized data is dequantized back to f32 (~2% precision loss, negligible for embeddings).
    pub fn deserialize(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < 8 {
            return Ok(()); // Empty is valid
        }

        let sentinel = u64::from_le_bytes(data[0..8].try_into().unwrap());

        if sentinel == u64::MAX && data.len() >= 17 {
            // Quantized format: [sentinel][version][count][i8 values...][f32 scales...]
            let mut offset = 8;
            let _version = data[offset];
            offset += 1;

            self.count = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap()) as usize;
            offset += 8;

            // Checked arithmetic to prevent integer overflow from corrupted/malicious data
            let values_size = self.count.checked_mul(DIMS)
                .ok_or_else(|| EngramError::IntegrityError("Vector count overflow in values_size".into()))?;
            let scales_offset = offset.checked_add(values_size)
                .ok_or_else(|| EngramError::IntegrityError("Vector scales_offset overflow".into()))?;
            let scales_size = self.count.checked_mul(4)
                .ok_or_else(|| EngramError::IntegrityError("Vector count overflow in scales_size".into()))?;

            if scales_offset.checked_add(scales_size)
                .map_or(true, |end| end > data.len())
            {
                return Err(EngramError::IntegrityError(
                    "Quantized vector data truncated or corrupted".into()
                ));
            }

            // Dequantize i8 + scale → f32
            self.vectors = Vec::with_capacity(self.count * DIMS);
            for i in 0..self.count {
                let scale = f32::from_le_bytes(
                    data[scales_offset + i * 4..scales_offset + i * 4 + 4].try_into().unwrap()
                );
                let factor = if scale > 0.0 { scale / 127.0 } else { 0.0 };
                let q_start = offset + i * DIMS;
                for j in 0..DIMS {
                    let q_val = data[q_start + j] as i8;
                    self.vectors.push(q_val as f32 * factor);
                }
            }
        } else {
            // Legacy f32 format: [count][f32 values...]
            self.count = sentinel as usize;
            let mut offset = 8;

            let num_floats = (data.len() - offset) / 4;
            self.vectors = Vec::with_capacity(num_floats);
            while offset + 4 <= data.len() {
                let v = f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
                self.vectors.push(v);
                offset += 4;
            }
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

    #[test]
    fn test_quantize_dequantize_roundtrip() {
        let mut v = [0.0f32; DIMS];
        for i in 0..DIMS {
            v[i] = ((i as f32) * 0.01).sin();
        }

        let q = quantize(&v);
        let restored = dequantize(&q);

        // Check error: relative for large values, absolute for small values
        for i in 0..DIMS {
            let abs_err = (v[i] - restored[i]).abs();
            if v[i].abs() > 0.1 {
                let rel_err = abs_err / v[i].abs();
                // i8 quantization has ~1/127 step size; roundtrip doubles the error
                assert!(rel_err < 0.04, "dim {} rel_err={} orig={} restored={}", i, rel_err, v[i], restored[i]);
            } else {
                // For small values, absolute error is more meaningful
                assert!(abs_err < 0.01, "dim {} abs_err={} orig={} restored={}", i, abs_err, v[i], restored[i]);
            }
        }
    }

    #[test]
    fn test_quantized_cosine_matches_f32() {
        // Two vectors with known direction
        let mut a = [0.0f32; DIMS];
        let mut b = [0.0f32; DIMS];
        for i in 0..DIMS {
            a[i] = ((i as f32) * 0.1).cos();
            b[i] = ((i as f32) * 0.1 + 0.5).cos();
        }

        let f32_sim = cosine_similarity(&a, &b);
        let qa = quantize(&a);
        let qb = quantize(&b);
        let q_sim = quantized_cosine_similarity(&qa, &qb);

        // Quantized cosine should be within 2% of f32 cosine
        let error = (f32_sim - q_sim).abs();
        assert!(error < 0.02, "f32={} quantized={} error={}", f32_sim, q_sim, error);
    }

    #[test]
    fn test_quantize_zero_vector() {
        let v = [0.0f32; DIMS];
        let q = quantize(&v);
        assert_eq!(q.norm, 0.0);
        assert!(q.values.iter().all(|&x| x == 0));

        let sim = quantized_cosine_similarity(&q, &q);
        assert_eq!(sim, 0.0); // Zero vector has no direction
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_quantized_simd_matches_scalar() {
        let a_f32: Vec<f32> = (0..512).map(|i| ((i as f32) * 0.1).sin()).collect();
        let b_f32: Vec<f32> = (0..512).map(|i| ((i as f32) * 0.2).cos()).collect();

        let qa = quantize(&a_f32);
        let qb = quantize(&b_f32);

        let scalar = quantized_dot_product_scalar(&qa.values, &qb.values);
        let simd = quantized_dot_product_i32(&qa.values, &qb.values);

        assert_eq!(scalar, simd, "i8 SIMD must exactly match scalar (integer arithmetic)");
    }

    #[test]
    fn test_vector_store_quantized_serialization() {
        let mut store = VectorStore::new();

        // Add vectors with distinct directions
        for i in 1..=5u64 {
            let mut v = [0.0f32; DIMS];
            v[0] = (i as f32).cos();
            v[1] = (i as f32).sin();
            v[2] = (i as f32) * 0.1;
            store.add(i, &v).unwrap();
        }

        // Serialize (quantized format)
        let data = store.serialize();
        // Should be much smaller than f32: 17 + 5*(512+4) = 2597 vs 8 + 5*2048 = 10248
        assert!(data.len() < 5000, "quantized should be ~4x smaller, got {}", data.len());

        // Deserialize
        let mut restored = VectorStore::new();
        restored.deserialize(&data).unwrap();
        assert_eq!(restored.len(), 5);

        // Check values are close (dequantized, so ~2% error)
        for i in 1..=5u64 {
            let orig = store.get(i).unwrap();
            let rest = restored.get(i).unwrap();
            for d in 0..3 {
                let err = (orig[d] - rest[d]).abs();
                assert!(err < 0.02, "id={} dim={} orig={} restored={}", i, d, orig[d], rest[d]);
            }
        }
    }

    #[test]
    fn test_vector_store_legacy_deserialize() {
        // Manually create legacy f32 format: [u64 count][f32 values...]
        let count: u64 = 2;
        let mut data = Vec::new();
        data.extend_from_slice(&count.to_le_bytes());

        // Two vectors, each DIMS f32 values
        for i in 0..2 {
            for d in 0..DIMS {
                let v = if d == 0 { (i + 1) as f32 } else { 0.0f32 };
                data.extend_from_slice(&v.to_le_bytes());
            }
        }

        let mut store = VectorStore::new();
        store.deserialize(&data).unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.get(1).unwrap()[0], 1.0);
        assert_eq!(store.get(2).unwrap()[0], 2.0);
    }
}
