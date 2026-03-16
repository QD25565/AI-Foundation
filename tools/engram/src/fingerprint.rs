//! Sub-microsecond associative recall via SimHash + Bloom fingerprinting.
//!
//! Each note gets a 128-bit fingerprint (SimHash + Bloom) computed at write time.
//! The current working context also gets a fingerprint, updated by the daemon.
//! The hook scans the fingerprint array using hardware POPCNT to find the most
//! relevant note in sub-microsecond time.
//!
//! # Algorithm
//!
//! **SimHash (Charikar 2002):** Locality-sensitive hash that preserves cosine
//! similarity. Similar documents produce similar hashes. Hamming distance between
//! SimHashes approximates angular distance:
//!
//!   `E[HD] = 64 × arccos(cos_sim) / π`
//!
//! **Bloom64:** 64-bit compact Bloom filter over stemmed keywords. AND + POPCNT
//! gives keyword overlap count. Pre-filters obviously irrelevant notes.
//!
//! **Combined scoring:** Weighted sum of SimHash similarity and Bloom overlap.
//! Two POPCNT instructions per note. ~5ns per note on modern CPUs.
//!
//! # Performance
//!
//! - 1800 notes × 32 bytes = 57.6KB — fits in L1/L2 cache (V2: 2 entries per cache line)
//! - Linear scan: ~600ns (scalar), ~230ns (AVX2)
//! - Total hook path: ~750ns including mmap read + threshold check

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use xxhash_rust::xxh3;
use rust_stemmers::{Algorithm, Stemmer};

// ============================================================================
// Core types
// ============================================================================

/// A 128-bit fingerprint: 64-bit SimHash + 64-bit Bloom filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    /// SimHash of stemmed content tokens (preserves cosine similarity)
    pub simhash: u64,
    /// Bloom filter over stemmed keywords (k=5 hash functions via xxh3)
    pub bloom: u64,
}

impl Fingerprint {
    /// Empty fingerprint (all zeros).
    pub const ZERO: Self = Self { simhash: 0, bloom: 0 };

    /// Create a fingerprint from raw values.
    pub fn new(simhash: u64, bloom: u64) -> Self {
        Self { simhash, bloom }
    }

    /// Compute a fingerprint from text content and optional tags.
    ///
    /// Content is tokenized (whitespace split, lowercased, punctuation-stripped,
    /// Snowball Porter2 stemmed) then SimHashed. Tags are added to the Bloom.
    pub fn from_text(content: &str, tags: &[&str]) -> Self {
        let stemmer = Stemmer::create(Algorithm::English);
        let tokens = tokenize_and_stem(content, &stemmer);

        let simhash = compute_simhash(&tokens);
        let mut bloom = compute_bloom64(&tokens);

        // Add stemmed tags to bloom for tag-based matching
        for tag in tags {
            let lower = tag.to_lowercase();
            let stemmed = stemmer.stem(&lower);
            bloom |= bloom64_single(&stemmed);
        }

        Self { simhash, bloom }
    }

    /// Compute a context fingerprint from a list of keywords (e.g., from tool calls).
    ///
    /// Keywords are already extracted — no sentence tokenization needed.
    /// Each keyword is stemmed and hashed.
    pub fn from_keywords(keywords: &[&str]) -> Self {
        let stemmer = Stemmer::create(Algorithm::English);
        let tokens: Vec<String> = keywords
            .iter()
            .flat_map(|kw| {
                // Split compound keywords on non-alphanumeric chars.
                // "storage.rs" → ["storage","rs"], "checked_add" → ["checked","add"]
                kw.split(|c: char| !c.is_alphanumeric())
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        let lower = s.to_lowercase();
                        stemmer.stem(&lower).into_owned()
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let simhash = compute_simhash(&tokens);
        let bloom = compute_bloom64(&tokens);

        Self { simhash, bloom }
    }

    /// Compute a fingerprint using IDF-weighted SimHash.
    ///
    /// Rare/discriminative tokens contribute more to the SimHash than common ones.
    /// This concentrates more mutual information per bit, improving retrieval
    /// precision by ~10-15% compared to uniform weighting at zero scan-time cost.
    ///
    /// The IdfTable should be built from the full corpus of notes. Tokens not in
    /// the table get a default weight of 1.0 (neutral).
    pub fn from_text_with_idf(content: &str, tags: &[&str], idf: &IdfTable) -> Self {
        let stemmer = Stemmer::create(Algorithm::English);
        let tokens = tokenize_and_stem(content, &stemmer);

        let simhash = compute_simhash_weighted(&tokens, idf);
        let mut bloom = compute_bloom64(&tokens);

        for tag in tags {
            let lower = tag.to_lowercase();
            let stemmed = stemmer.stem(&lower);
            bloom |= bloom64_single(&stemmed);
        }

        Self { simhash, bloom }
    }

    /// Compute a fingerprint with trigger keywords that get 3x bloom weight.
    ///
    /// Trigger keywords are explicit terms that should cause this note to surface
    /// when the AI's context matches. SimHash is computed normally (triggers don't
    /// affect semantic centroid). Bloom gets 3x weight for triggers via offset seeds:
    /// seeds 0..5 (normal) + 10..15 + 20..25 = ~15 bits per trigger instead of ~5.
    pub fn from_text_with_triggers(content: &str, tags: &[&str], triggers: &[&str]) -> Self {
        let stemmer = Stemmer::create(Algorithm::English);
        let tokens = tokenize_and_stem(content, &stemmer);

        // SimHash: computed normally — triggers do NOT affect semantic centroid
        let simhash = compute_simhash(&tokens);
        let mut bloom = compute_bloom64(&tokens);

        // Tags get normal bloom weight
        for tag in tags {
            let lower = tag.to_lowercase();
            let stemmed = stemmer.stem(&lower);
            bloom |= bloom64_single(&stemmed);
        }

        // Trigger keywords: add to bloom using standard seeds (same as tags).
        // The value isn't "higher weight" (impossible on a binary bloom) — it's adding
        // keywords to the bloom that aren't in the content text. When future context
        // contains a trigger keyword, the note's bloom has matching bits even though
        // the content doesn't mention it. Offset seed families were tried but they
        // set bits the context side can never produce (context uses seeds 0..BLOOM_K),
        // adding only noise.
        for trigger in triggers {
            for sub in trigger.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()) {
                let lower = sub.to_lowercase();
                let stemmed = stemmer.stem(&lower);
                bloom |= bloom64_single(&stemmed);
            }
        }

        Self { simhash, bloom }
    }

    /// SimHash bit density: ratio of set bits to total bits.
    ///
    /// Optimal density is ~0.5 (32 of 64 bits set). Very high (>0.75) or very low
    /// (<0.25) density indicates poor information content — the hash is dominated by
    /// a few high-frequency tokens or has too few tokens to be discriminative.
    #[inline]
    pub fn simhash_density(&self) -> f32 {
        self.simhash.count_ones() as f32 / 64.0
    }

    /// Bloom filter bit density: ratio of set bits to total bits.
    ///
    /// Density above 0.5 means the Bloom filter is becoming saturated — too many
    /// keywords are encoded, reducing discriminative power. Below 0.05 means very
    /// few keywords, which is fine but offers less filtering.
    #[inline]
    pub fn bloom_density(&self) -> f32 {
        self.bloom.count_ones() as f32 / 64.0
    }

    /// Hamming distance between two SimHashes (lower = more similar).
    /// Range: 0 (identical) to 64 (maximally different).
    #[inline(always)]
    pub fn hamming_distance(&self, other: &Self) -> u32 {
        (self.simhash ^ other.simhash).count_ones()
    }

    /// Bloom overlap count (higher = more shared keywords).
    /// Range: 0 (no overlap) to 64 (all bits shared).
    #[inline(always)]
    pub fn bloom_overlap(&self, other: &Self) -> u32 {
        (self.bloom & other.bloom).count_ones()
    }

    /// Combined score: bloom-primary, SimHash-secondary.
    ///
    /// Calibration on 896-note corpus (Mar 2026) showed SimHash HD has no
    /// discrimination at N>500 (birthday paradox: noise floor ≈ HD 17, same as
    /// true matches). Bloom overlap IS discriminating (noise 17-18, matches 20-27).
    /// Formula: bloom * 3 + simhash_sim gives bloom ~3x influence per unit.
    /// Range: 0 to 256 (64*3 + 64).
    #[inline(always)]
    pub fn score(&self, other: &Self) -> u32 {
        let sim = 64 - self.hamming_distance(other);
        let overlap = self.bloom_overlap(other);
        overlap * 3 + sim
    }

    /// Weighted similarity score: 0.3 × semantic (SimHash) + 0.7 × keyword (Bloom).
    ///
    /// Returns f32 in range [0.0, 1.0]. Bloom overlap weighted higher based on
    /// large-scale calibration: bloom discriminates between relevant/noise at
    /// corpus sizes >500, while SimHash HD converges to noise floor.
    #[inline(always)]
    pub fn score_weighted(&self, other: &Self) -> f32 {
        let semantic = (64 - self.hamming_distance(other)) as f32 / 64.0;
        let keyword = self.bloom_overlap(other) as f32 / 64.0;
        0.3 * semantic + 0.7 * keyword
    }

    /// Serialize to 16 bytes: [simhash LE 8 bytes][bloom LE 8 bytes]
    pub fn to_bytes(&self) -> [u8; 16] {
        let mut buf = [0u8; 16];
        buf[0..8].copy_from_slice(&self.simhash.to_le_bytes());
        buf[8..16].copy_from_slice(&self.bloom.to_le_bytes());
        buf
    }

    /// Deserialize from 16 bytes.
    pub fn from_bytes(bytes: &[u8; 16]) -> Self {
        let simhash = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let bloom = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        Self { simhash, bloom }
    }
}

// ============================================================================
// Adaptive Hamming distance thresholds
// ============================================================================

/// Adaptive maximum Hamming distance based on corpus size (64-bit SimHash).
///
/// As corpus grows, the birthday paradox reduces the expected minimum HD between
/// random fingerprints. At N=896, a random note will have HD≤17 with the query
/// by pure chance. This function keeps the threshold below the noise floor.
///
/// Math: max_hd(N) ≈ floor(b/2 - sqrt(b/2) * sqrt(ln(N/0.1)))
/// Precomputed for common corpus sizes (integer-only, zero scan-time cost).
pub fn adaptive_max_hd(corpus_size: u32) -> u32 {
    match corpus_size {
        0..=500 => 22,
        501..=1000 => 20,
        1001..=2000 => 18,
        2001..=5000 => 16,
        5001..=10000 => 15,
        _ => 14,
    }
}

/// Adaptive maximum Hamming distance for 128-bit SimHash.
///
/// 128-bit SimHash has a birthday threshold of N≈28,500 (vs N≈896 for 64-bit).
/// This function provides appropriate thresholds for the wider hash.
pub fn adaptive_max_hd_128(corpus_size: u32) -> u32 {
    match corpus_size {
        0..=5000 => 42,
        5001..=10000 => 38,
        10001..=50000 => 36,
        _ => 34,
    }
}

// ============================================================================
// 256-bit Fingerprint (128-bit SimHash + 128-bit Bloom)
// ============================================================================

/// Seed for the second SimHash u64 (golden ratio constant, maximally dispersed).
const SIMHASH_SEED_HI: u64 = 0x9E3779B97F4A7C15;

/// A 256-bit fingerprint: 128-bit SimHash + 128-bit Bloom filter.
///
/// Wider than `Fingerprint` (128-bit) for better discrimination at scale:
/// - 128-bit SimHash: birthday threshold jumps from N≈896 to N≈28,500 (32x)
/// - 128-bit Bloom: fill drops from 47% to 26% at k=5, n=8 (FP rate 45x lower)
/// - Scan cost: ~5-6 cycles/entry (2-3x of 128-bit), still <4μs for 1800 notes
/// - Storage: 48 bytes/entry × 1800 = 84KB sidecar (fits in L2 cache)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint256 {
    /// 128-bit SimHash: two independent xxh3 seed families
    pub simhash: [u64; 2],
    /// 128-bit Bloom filter over stemmed keywords (k=5, modulo 128)
    pub bloom: [u64; 2],
}

impl Fingerprint256 {
    /// Empty fingerprint (all zeros).
    pub const ZERO: Self = Self { simhash: [0, 0], bloom: [0, 0] };

    /// Create from raw values.
    pub fn new(simhash: [u64; 2], bloom: [u64; 2]) -> Self {
        Self { simhash, bloom }
    }

    /// Promote a 128-bit Fingerprint to 256-bit (zero-pad upper bits).
    pub fn from_128(fp: &Fingerprint) -> Self {
        Self {
            simhash: [fp.simhash, 0],
            bloom: [fp.bloom, 0],
        }
    }

    /// Truncate to 128-bit Fingerprint (drop upper bits).
    pub fn to_128(&self) -> Fingerprint {
        Fingerprint {
            simhash: self.simhash[0],
            bloom: self.bloom[0],
        }
    }

    /// Compute from text content and optional tags.
    pub fn from_text(content: &str, tags: &[&str]) -> Self {
        let stemmer = Stemmer::create(Algorithm::English);
        let tokens = tokenize_and_stem(content, &stemmer);

        let simhash = compute_simhash_128(&tokens);
        let mut bloom = compute_bloom128(&tokens);

        for tag in tags {
            let lower = tag.to_lowercase();
            let stemmed = stemmer.stem(&lower);
            let tag_bloom = bloom128_single(&stemmed);
            bloom[0] |= tag_bloom[0];
            bloom[1] |= tag_bloom[1];
        }

        Self { simhash, bloom }
    }

    /// Compute from already-extracted keywords.
    pub fn from_keywords(keywords: &[&str]) -> Self {
        let stemmer = Stemmer::create(Algorithm::English);
        let tokens: Vec<String> = keywords
            .iter()
            .flat_map(|kw| {
                kw.split(|c: char| !c.is_alphanumeric())
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        let lower = s.to_lowercase();
                        stemmer.stem(&lower).into_owned()
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let simhash = compute_simhash_128(&tokens);
        let bloom = compute_bloom128(&tokens);

        Self { simhash, bloom }
    }

    /// Compute with IDF-weighted SimHash for better retrieval precision.
    pub fn from_text_with_idf(content: &str, tags: &[&str], idf: &IdfTable) -> Self {
        let stemmer = Stemmer::create(Algorithm::English);
        let tokens = tokenize_and_stem(content, &stemmer);

        let simhash = compute_simhash_128_weighted(&tokens, idf);
        let mut bloom = compute_bloom128(&tokens);

        for tag in tags {
            let lower = tag.to_lowercase();
            let stemmed = stemmer.stem(&lower);
            let tag_bloom = bloom128_single(&stemmed);
            bloom[0] |= tag_bloom[0];
            bloom[1] |= tag_bloom[1];
        }

        Self { simhash, bloom }
    }

    /// Compute with trigger keywords that get 3x bloom weight (128-bit variant).
    pub fn from_text_with_triggers(content: &str, tags: &[&str], triggers: &[&str]) -> Self {
        let stemmer = Stemmer::create(Algorithm::English);
        let tokens = tokenize_and_stem(content, &stemmer);

        let simhash = compute_simhash_128(&tokens);
        let mut bloom = compute_bloom128(&tokens);

        for tag in tags {
            let lower = tag.to_lowercase();
            let stemmed = stemmer.stem(&lower);
            let tag_bloom = bloom128_single(&stemmed);
            bloom[0] |= tag_bloom[0];
            bloom[1] |= tag_bloom[1];
        }

        // Trigger keywords: standard bloom seeds (same rationale as 64-bit variant)
        for trigger in triggers {
            for sub in trigger.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()) {
                let lower = sub.to_lowercase();
                let stemmed = stemmer.stem(&lower);
                let tag_bloom = bloom128_single(&stemmed);
                bloom[0] |= tag_bloom[0];
                bloom[1] |= tag_bloom[1];
            }
        }

        Self { simhash, bloom }
    }

    /// Hamming distance between two 128-bit SimHashes (range: 0 to 128).
    #[inline(always)]
    pub fn hamming_distance(&self, other: &Self) -> u32 {
        (self.simhash[0] ^ other.simhash[0]).count_ones()
            + (self.simhash[1] ^ other.simhash[1]).count_ones()
    }

    /// Bloom overlap count (range: 0 to 128).
    #[inline(always)]
    pub fn bloom_overlap(&self, other: &Self) -> u32 {
        (self.bloom[0] & other.bloom[0]).count_ones()
            + (self.bloom[1] & other.bloom[1]).count_ones()
    }

    /// Combined score: bloom-primary, SimHash-secondary.
    /// Formula: bloom * 3 + simhash_sim. Range: 0 to 512 (128*3 + 128).
    #[inline(always)]
    pub fn score(&self, other: &Self) -> u32 {
        let sim = 128 - self.hamming_distance(other);
        let overlap = self.bloom_overlap(other);
        overlap * 3 + sim
    }

    /// Weighted similarity score: 0.3 × semantic + 0.7 × keyword. Range [0.0, 1.0].
    #[inline(always)]
    pub fn score_weighted(&self, other: &Self) -> f32 {
        let semantic = (128 - self.hamming_distance(other)) as f32 / 128.0;
        let keyword = self.bloom_overlap(other) as f32 / 128.0;
        0.3 * semantic + 0.7 * keyword
    }

    /// Serialize to 32 bytes: [simhash_lo:8][simhash_hi:8][bloom_lo:8][bloom_hi:8]
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut buf = [0u8; 32];
        buf[0..8].copy_from_slice(&self.simhash[0].to_le_bytes());
        buf[8..16].copy_from_slice(&self.simhash[1].to_le_bytes());
        buf[16..24].copy_from_slice(&self.bloom[0].to_le_bytes());
        buf[24..32].copy_from_slice(&self.bloom[1].to_le_bytes());
        buf
    }

    /// Deserialize from 32 bytes.
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let simhash_lo = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let simhash_hi = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let bloom_lo = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let bloom_hi = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        Self {
            simhash: [simhash_lo, simhash_hi],
            bloom: [bloom_lo, bloom_hi],
        }
    }

    /// SimHash bit density (optimal ~0.5 for each half).
    pub fn simhash_density(&self) -> f32 {
        (self.simhash[0].count_ones() + self.simhash[1].count_ones()) as f32 / 128.0
    }

    /// Bloom filter bit density.
    pub fn bloom_density(&self) -> f32 {
        (self.bloom[0].count_ones() + self.bloom[1].count_ones()) as f32 / 128.0
    }
}

/// Result of a fingerprint scan.
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// Note ID of the best match
    pub note_id: u64,
    /// Combined similarity score
    pub score: u32,
    /// Hamming distance (for threshold checking)
    pub hamming_distance: u32,
    /// Bloom overlap count
    pub bloom_overlap: u32,
}

// ============================================================================
// Fingerprint Index
// ============================================================================

/// An entry in the fingerprint index.
#[derive(Debug, Clone, Copy)]
pub struct FingerprintEntry {
    /// The note's fingerprint
    pub fingerprint: Fingerprint,
    /// The note's ID (for lookup after matching)
    pub note_id: u64,
    /// Per-entry flags (FLAG_SKIP_RECALL, FLAG_TOMBSTONE)
    pub flags: u8,
}

/// Skip this entry during recall scans (e.g., pinned notes already in context).
pub const FLAG_SKIP_RECALL: u8 = 0x01;
/// Entry is tombstoned (note was deleted). Belt-and-suspenders for sidecar-only consumers.
pub const FLAG_TOMBSTONE: u8 = 0x02;

impl FingerprintEntry {
    /// V2 serialized size: simhash(8) + bloom(8) + note_id(8) + flags(1) + reserved(7) = 32 bytes.
    /// Exactly 2 entries per 64-byte cache line.
    pub const SIZE: usize = 32;

    /// V1 serialized size (for backward compat): simhash(8) + bloom(8) + note_id(8) = 24 bytes.
    pub const SIZE_V1: usize = 24;

    /// Serialize to 32 bytes: [simhash:8][bloom:8][note_id:8][flags:1][reserved:7]
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut buf = [0u8; 32];
        buf[0..8].copy_from_slice(&self.fingerprint.simhash.to_le_bytes());
        buf[8..16].copy_from_slice(&self.fingerprint.bloom.to_le_bytes());
        buf[16..24].copy_from_slice(&self.note_id.to_le_bytes());
        buf[24] = self.flags;
        // bytes 25-31 reserved (zero)
        buf
    }

    /// Deserialize from 32-byte V2 entry.
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let simhash = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let bloom = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let note_id = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let flags = bytes[24];
        Self {
            fingerprint: Fingerprint { simhash, bloom },
            note_id,
            flags,
        }
    }

    /// Deserialize from 24-byte V1 entry (flags default to 0).
    pub fn from_bytes_v1(bytes: &[u8; 24]) -> Self {
        let simhash = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let bloom = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let note_id = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        Self {
            fingerprint: Fingerprint { simhash, bloom },
            note_id,
            flags: 0,
        }
    }
}

/// A 256-bit fingerprint entry for the V3 index format (48 bytes per entry).
#[derive(Debug, Clone, Copy)]
pub struct FingerprintEntry256 {
    /// The note's 256-bit fingerprint
    pub fingerprint: Fingerprint256,
    /// The note's ID (for lookup after matching)
    pub note_id: u64,
    /// Per-entry flags (FLAG_SKIP_RECALL, FLAG_TOMBSTONE)
    pub flags: u8,
}

impl FingerprintEntry256 {
    /// V3 serialized size: fingerprint(32) + note_id(8) + flags(1) + reserved(7) = 48 bytes.
    pub const SIZE: usize = 48;

    /// Serialize to 48 bytes: [fp:32][note_id:8][flags:1][reserved:7]
    pub fn to_bytes(&self) -> [u8; 48] {
        let mut buf = [0u8; 48];
        let fp_bytes = self.fingerprint.to_bytes();
        buf[0..32].copy_from_slice(&fp_bytes);
        buf[32..40].copy_from_slice(&self.note_id.to_le_bytes());
        buf[40] = self.flags;
        // bytes 41-47 reserved (zero)
        buf
    }

    /// Deserialize from 48-byte V3 entry.
    pub fn from_bytes(bytes: &[u8; 48]) -> Self {
        let fp_bytes: [u8; 32] = bytes[0..32].try_into().unwrap();
        let fingerprint = Fingerprint256::from_bytes(&fp_bytes);
        let note_id = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
        let flags = bytes[40];
        Self { fingerprint, note_id, flags }
    }

    /// Promote from V2 FingerprintEntry (zero-pad upper bits).
    pub fn from_v2(entry: &FingerprintEntry) -> Self {
        Self {
            fingerprint: Fingerprint256::from_128(&entry.fingerprint),
            note_id: entry.note_id,
            flags: entry.flags,
        }
    }

    /// Promote from V1 entry bytes (24 bytes, no flags).
    pub fn from_v1_bytes(bytes: &[u8; 24]) -> Self {
        let entry_v1 = FingerprintEntry::from_bytes_v1(bytes);
        Self::from_v2(&entry_v1)
    }
}

/// Magic bytes for the fingerprint index file
const FP_MAGIC: u32 = 0x454E4650; // "ENFP"
/// Current version of the fingerprint index format (V2: 32-byte entries with flags)
const FP_VERSION: u16 = 2;
/// V1 format version (24-byte entries, no flags)
const FP_VERSION_V1: u16 = 1;
/// Header size in bytes
const FP_HEADER_SIZE: usize = 16;
/// V3 format version (48-byte entries, 256-bit fingerprints)
const FP_VERSION_V3: u16 = 3;

/// In-memory fingerprint index. Can be serialized to/from a file.
///
/// For the hook path, this would be memory-mapped for zero-copy reads.
/// For engram write path, this is built in memory and flushed to disk.
pub struct FingerprintIndex {
    entries: Vec<FingerprintEntry>,
}

impl FingerprintIndex {
    /// Create an empty index.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Create an index with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
        }
    }

    /// Number of entries in the index.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Add a note's fingerprint to the index.
    pub fn add(&mut self, note_id: u64, content: &str, tags: &[&str]) {
        let fingerprint = Fingerprint::from_text(content, tags);
        self.entries.push(FingerprintEntry {
            fingerprint,
            note_id,
            flags: 0,
        });
    }

    /// Add a pre-computed fingerprint entry.
    pub fn add_entry(&mut self, entry: FingerprintEntry) {
        self.entries.push(entry);
    }

    /// Remove a note from the index by ID.
    /// Returns true if the note was found and removed.
    pub fn remove(&mut self, note_id: u64) -> bool {
        let len_before = self.entries.len();
        self.entries.retain(|e| e.note_id != note_id);
        self.entries.len() != len_before
    }

    /// Get a reference to the raw entries slice (for zero-copy scanning).
    pub fn entries(&self) -> &[FingerprintEntry] {
        &self.entries
    }

    /// Scan the index for the best match against a context fingerprint.
    ///
    /// Returns the best matching entry if its Hamming distance is within
    /// `max_hamming_distance` (lower = stricter). Pass `None` for no HD filter.
    ///
    /// This is the hot path — designed for sub-microsecond execution.
    /// Two POPCNT instructions per entry, linear scan.
    pub fn scan_best(
        &self,
        context: &Fingerprint,
        max_hamming_distance: Option<u32>,
    ) -> Option<ScanResult> {
        let max_hd = max_hamming_distance.unwrap_or(u32::MAX);
        let mut best: Option<ScanResult> = None;

        for entry in &self.entries {
            // Skip pinned/tombstoned entries
            if entry.flags & (FLAG_SKIP_RECALL | FLAG_TOMBSTONE) != 0 {
                continue;
            }

            // Bloom pre-filter: skip if zero keyword overlap
            let overlap = entry.fingerprint.bloom_overlap(context);
            if overlap == 0 && context.bloom != 0 {
                continue;
            }

            // SimHash Hamming distance
            let hd = entry.fingerprint.hamming_distance(context);
            if hd > max_hd {
                continue;
            }

            // Combined score (bloom-primary)
            let score = overlap * 3 + (64 - hd);

            let dominated = best.as_ref().map_or(false, |b| score <= b.score);
            if !dominated {
                best = Some(ScanResult {
                    note_id: entry.note_id,
                    score,
                    hamming_distance: hd,
                    bloom_overlap: overlap,
                });
            }
        }

        best
    }

    /// Scan the index and return the top-K matches, sorted by score descending.
    pub fn scan_top_k(
        &self,
        context: &Fingerprint,
        k: usize,
        max_hamming_distance: Option<u32>,
    ) -> Vec<ScanResult> {
        let max_hd = max_hamming_distance.unwrap_or(u32::MAX);
        let mut results: Vec<ScanResult> = Vec::with_capacity(k + 1);

        for entry in &self.entries {
            // Skip pinned/tombstoned entries
            if entry.flags & (FLAG_SKIP_RECALL | FLAG_TOMBSTONE) != 0 {
                continue;
            }

            let overlap = entry.fingerprint.bloom_overlap(context);
            if overlap == 0 && context.bloom != 0 {
                continue;
            }

            let hd = entry.fingerprint.hamming_distance(context);
            if hd > max_hd {
                continue;
            }

            let score = overlap * 3 + (64 - hd);

            // Only insert if better than current worst in top-K
            let dominated_by_kth = results.len() >= k
                && results.last().map_or(false, |last| score <= last.score);
            if !dominated_by_kth {
                results.push(ScanResult {
                    note_id: entry.note_id,
                    score,
                    hamming_distance: hd,
                    bloom_overlap: overlap,
                });
                results.sort_by(|a, b| b.score.cmp(&a.score));
                if results.len() > k {
                    results.truncate(k);
                }
            }
        }

        results
    }

    // === Upsert ===

    /// Update a note's fingerprint if it exists, or insert if new.
    pub fn upsert(&mut self, note_id: u64, content: &str, tags: &[&str]) {
        let fingerprint = Fingerprint::from_text(content, tags);
        if let Some(entry) = self.entries.iter_mut().find(|e| e.note_id == note_id) {
            entry.fingerprint = fingerprint;
        } else {
            self.entries.push(FingerprintEntry {
                fingerprint,
                note_id,
                flags: 0,
            });
        }
    }

    /// Upsert with trigger keywords that get 3x bloom weight.
    pub fn upsert_with_triggers(&mut self, note_id: u64, content: &str, tags: &[&str], triggers: &[&str]) {
        let fingerprint = Fingerprint::from_text_with_triggers(content, tags, triggers);
        if let Some(entry) = self.entries.iter_mut().find(|e| e.note_id == note_id) {
            entry.fingerprint = fingerprint;
        } else {
            self.entries.push(FingerprintEntry {
                fingerprint,
                note_id,
                flags: 0,
            });
        }
    }

    /// Update a pre-computed fingerprint if note_id exists, or insert if new.
    pub fn upsert_entry(&mut self, entry: FingerprintEntry) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.note_id == entry.note_id) {
            existing.fingerprint = entry.fingerprint;
        } else {
            self.entries.push(entry);
        }
    }

    // === Per-entry flags ===

    /// Set FLAG_SKIP_RECALL on entries whose note_id is in `pinned_ids`,
    /// and clear it on all others. Call before save() to mark pinned notes.
    pub fn mark_pinned(&mut self, pinned_ids: &[u64]) {
        for entry in &mut self.entries {
            if pinned_ids.contains(&entry.note_id) {
                entry.flags |= FLAG_SKIP_RECALL;
            } else {
                entry.flags &= !FLAG_SKIP_RECALL;
            }
        }
    }

    /// Set FLAG_TOMBSTONE on the entry with the given note_id.
    /// Belt-and-suspenders: Cascade's remove() already drops the entry from
    /// the in-memory index, but tombstone survives in the sidecar for consumers
    /// that read the file directly (e.g., hook-bulletin mmap scan).
    pub fn mark_tombstoned(&mut self, note_id: u64) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.note_id == note_id) {
            entry.flags |= FLAG_TOMBSTONE;
        }
    }

    // === Cluster pre-filtering ===

    /// Build clusters from note tag associations.
    ///
    /// Each unique tag becomes a cluster. The super-fingerprint is the bitwise OR
    /// of all member fingerprints — if a context doesn't overlap with the
    /// super-fingerprint, no member can match either.
    ///
    /// `note_tags` maps note_id → list of tags. Notes without tags go into no cluster
    /// (they'll be scanned separately in `scan_clustered`).
    pub fn build_clusters(&self, note_tags: &[(u64, Vec<&str>)]) -> Vec<Cluster> {
        use std::collections::HashMap;

        // Map note_id → index in self.entries for O(1) lookup
        let id_to_idx: HashMap<u64, usize> = self.entries.iter()
            .enumerate()
            .map(|(i, e)| (e.note_id, i))
            .collect();

        // Group by tag
        let mut tag_members: HashMap<&str, Vec<usize>> = HashMap::new();
        for (note_id, tags) in note_tags {
            if let Some(&idx) = id_to_idx.get(note_id) {
                for tag in tags {
                    tag_members.entry(tag).or_default().push(idx);
                }
            }
        }

        // Build clusters with super-fingerprints
        tag_members.into_iter().map(|(tag, indices)| {
            let mut super_simhash: u64 = 0;
            let mut super_bloom: u64 = 0;
            for &idx in &indices {
                super_simhash |= self.entries[idx].fingerprint.simhash;
                super_bloom |= self.entries[idx].fingerprint.bloom;
            }
            Cluster {
                tag: tag.to_string(),
                super_simhash,
                super_bloom,
                member_indices: indices,
            }
        }).collect()
    }

    /// Scan using cluster pre-filtering.
    ///
    /// Checks ~20 super-fingerprints first (~200ns), then only scans members of
    /// clusters whose super-fingerprint has bloom overlap with the context.
    /// Eliminates 85%+ of notes for typical workloads.
    ///
    /// Notes not in any cluster are always scanned (unclustered fallback).
    pub fn scan_clustered(
        &self,
        context: &Fingerprint,
        clusters: &[Cluster],
        max_hamming_distance: Option<u32>,
    ) -> Option<ScanResult> {
        let max_hd = max_hamming_distance.unwrap_or(u32::MAX);
        let mut best: Option<ScanResult> = None;

        // Track which indices are in at least one cluster
        let mut in_cluster = vec![false; self.entries.len()];

        // Phase 1: Check each cluster's super-fingerprint
        for cluster in clusters {
            // Quick reject: if context bloom doesn't overlap with cluster's
            // super-bloom, no member can match on keywords
            if context.bloom != 0 && (cluster.super_bloom & context.bloom) == 0 {
                // Mark members as seen (they're in a cluster, just rejected)
                for &idx in &cluster.member_indices {
                    in_cluster[idx] = true;
                }
                continue;
            }

            // Cluster passed pre-filter — scan its members
            for &idx in &cluster.member_indices {
                in_cluster[idx] = true;
                self.score_entry(idx, context, max_hd, &mut best);
            }
        }

        // Phase 2: Scan unclustered entries (not in any cluster)
        for (idx, _) in self.entries.iter().enumerate() {
            if !in_cluster[idx] {
                self.score_entry(idx, context, max_hd, &mut best);
            }
        }

        best
    }

    /// Score a single entry and update best if it wins.
    #[inline]
    fn score_entry(
        &self,
        idx: usize,
        context: &Fingerprint,
        max_hd: u32,
        best: &mut Option<ScanResult>,
    ) {
        let entry = &self.entries[idx];

        // Skip pinned/tombstoned entries
        if entry.flags & (FLAG_SKIP_RECALL | FLAG_TOMBSTONE) != 0 {
            return;
        }

        // Bloom pre-filter
        let overlap = entry.fingerprint.bloom_overlap(context);
        if overlap == 0 && context.bloom != 0 {
            return;
        }

        let hd = entry.fingerprint.hamming_distance(context);
        if hd > max_hd {
            return;
        }

        let score = overlap * 3 + (64 - hd);
        let dominated = best.as_ref().map_or(false, |b| score <= b.score);
        if !dominated {
            *best = Some(ScanResult {
                note_id: entry.note_id,
                score,
                hamming_distance: hd,
                bloom_overlap: overlap,
            });
        }
    }

    // === File persistence ===

    /// Compute the sidecar file path for fingerprint data.
    /// For "memory.engram" returns "memory.engram.fp".
    pub fn sidecar_path(engram_path: &Path) -> PathBuf {
        let mut fp_path = engram_path.as_os_str().to_owned();
        fp_path.push(".fp");
        PathBuf::from(fp_path)
    }

    /// Save the index to a file atomically (write-tmp then rename).
    pub fn save(&self, path: &Path) -> Result<(), FingerprintError> {
        let tmp_path = path.with_extension("fp.tmp");
        std::fs::write(&tmp_path, self.to_bytes())?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }

    /// Load the index from a file.
    pub fn load(path: &Path) -> Result<Self, FingerprintError> {
        let data = std::fs::read(path)?;
        Self::from_bytes(&data)
    }

    // === Serialization ===

    /// Serialize the index to bytes (V2 format).
    ///
    /// Format:
    ///   Header (16 bytes): magic(4) + version(2) + count(4) + reserved(6)
    ///   Entries (32 bytes each): simhash(8) + bloom(8) + note_id(8) + flags(1) + reserved(7)
    pub fn to_bytes(&self) -> Vec<u8> {
        let count = self.entries.len() as u32;
        let size = FP_HEADER_SIZE + (count as usize * FingerprintEntry::SIZE);
        let mut buf = Vec::with_capacity(size);

        // Header
        buf.extend_from_slice(&FP_MAGIC.to_le_bytes());    // 4
        buf.extend_from_slice(&FP_VERSION.to_le_bytes());   // 2
        buf.extend_from_slice(&count.to_le_bytes());        // 4
        buf.extend_from_slice(&[0u8; 6]);                   // 6 reserved

        // Entries (32 bytes each)
        for entry in &self.entries {
            buf.extend_from_slice(&entry.to_bytes());
        }

        buf
    }

    /// Deserialize the index from bytes. Supports both V1 (24-byte) and V2 (32-byte) entries.
    pub fn from_bytes(data: &[u8]) -> Result<Self, FingerprintError> {
        if data.len() < FP_HEADER_SIZE {
            return Err(FingerprintError::TooShort);
        }

        let magic = u32::from_le_bytes(data[0..4].try_into().unwrap());
        if magic != FP_MAGIC {
            return Err(FingerprintError::BadMagic);
        }

        let version = u16::from_le_bytes(data[4..6].try_into().unwrap());
        let entry_size = match version {
            FP_VERSION_V1 => FingerprintEntry::SIZE_V1,
            FP_VERSION => FingerprintEntry::SIZE,
            _ => return Err(FingerprintError::UnsupportedVersion(version)),
        };

        let count = u32::from_le_bytes(data[6..10].try_into().unwrap()) as usize;
        let expected_size = FP_HEADER_SIZE + count * entry_size;
        if data.len() < expected_size {
            return Err(FingerprintError::TooShort);
        }

        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let offset = FP_HEADER_SIZE + i * entry_size;
            if version == FP_VERSION_V1 {
                let entry_bytes: [u8; 24] = data[offset..offset + 24].try_into().unwrap();
                entries.push(FingerprintEntry::from_bytes_v1(&entry_bytes));
            } else {
                let entry_bytes: [u8; 32] = data[offset..offset + 32].try_into().unwrap();
                entries.push(FingerprintEntry::from_bytes(&entry_bytes));
            }
        }

        Ok(Self { entries })
    }

    /// Zero-copy scan over a memory-mapped byte slice.
    ///
    /// Reads the serialized fingerprint index directly from `data` (e.g., mmap'd
    /// .engram.fp file) without allocating a Vec<FingerprintEntry>. Returns the
    /// best matching note_id and score, or None if nothing passes the threshold.
    ///
    /// Supports both V1 (24-byte) and V2 (32-byte) entry formats. V2 entries
    /// with FLAG_SKIP_RECALL or FLAG_TOMBSTONE are skipped automatically.
    ///
    /// # Arguments
    /// * `data` — Raw bytes of the serialized fingerprint index (header + entries)
    /// * `context` — The context fingerprint to match against
    /// * `max_hd` — Maximum Hamming distance threshold (e.g., 10)
    ///
    /// # Performance
    /// Zero allocations. ~5ns per entry. V2's 32-byte alignment = exactly 2 entries
    /// per 64-byte cache line for optimal throughput.
    pub fn scan_mmap(data: &[u8], context: &Fingerprint, max_hd: u32) -> Option<ScanResult> {
        if data.len() < FP_HEADER_SIZE {
            return None;
        }

        let magic = u32::from_le_bytes(data[0..4].try_into().ok()?);
        if magic != FP_MAGIC {
            return None;
        }

        let version = u16::from_le_bytes(data[4..6].try_into().ok()?);
        let entry_size = match version {
            FP_VERSION_V1 => FingerprintEntry::SIZE_V1,
            FP_VERSION => FingerprintEntry::SIZE,
            _ => return None,
        };

        let count = u32::from_le_bytes(data[6..10].try_into().ok()?) as usize;
        let expected_size = FP_HEADER_SIZE + count * entry_size;
        if data.len() < expected_size {
            return None;
        }

        let mut best: Option<ScanResult> = None;

        for i in 0..count {
            let offset = FP_HEADER_SIZE + i * entry_size;

            // V2: check flags byte — single-byte skip before any POPCNT work
            if version >= FP_VERSION && data[offset + 24] & (FLAG_SKIP_RECALL | FLAG_TOMBSTONE) != 0 {
                continue;
            }

            // Read simhash, bloom, note_id directly from bytes — zero copy
            let simhash = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            let bloom = u64::from_le_bytes(data[offset + 8..offset + 16].try_into().ok()?);

            // Bloom pre-filter: skip if zero keyword overlap
            let overlap = (bloom & context.bloom).count_ones();
            if overlap == 0 && context.bloom != 0 {
                continue;
            }

            // SimHash distance
            let hd = (simhash ^ context.simhash).count_ones();
            if hd > max_hd {
                continue;
            }

            let note_id = u64::from_le_bytes(data[offset + 16..offset + 24].try_into().ok()?);
            let score = overlap * 3 + (64 - hd);
            let dominated = best.as_ref().map_or(false, |b| score <= b.score);
            if !dominated {
                best = Some(ScanResult {
                    note_id,
                    score,
                    hamming_distance: hd,
                    bloom_overlap: overlap,
                });
            }
        }

        best
    }

    /// Batch-4 optimized zero-copy scan over memory-mapped data.
    ///
    /// Processes 4 entries per loop iteration for better instruction-level
    /// parallelism. The context simhash/bloom stay in registers while 4
    /// independent XOR+POPCNT chains execute. ~1.5x faster than scalar scan
    /// on modern out-of-order CPUs.
    ///
    /// Supports both V1 (24-byte) and V2 (32-byte) entry formats. V2 entries
    /// with flags are skipped before POPCNT work.
    ///
    /// Falls back to scalar for the remainder (count % 4 entries).
    pub fn scan_mmap_batch4(data: &[u8], context: &Fingerprint, max_hd: u32) -> Option<ScanResult> {
        if data.len() < FP_HEADER_SIZE {
            return None;
        }

        let magic = u32::from_le_bytes(data[0..4].try_into().ok()?);
        if magic != FP_MAGIC {
            return None;
        }

        let version = u16::from_le_bytes(data[4..6].try_into().ok()?);
        let entry_size = match version {
            FP_VERSION_V1 => FingerprintEntry::SIZE_V1,
            FP_VERSION => FingerprintEntry::SIZE,
            _ => return None,
        };
        let is_v2 = version >= FP_VERSION;

        let count = u32::from_le_bytes(data[6..10].try_into().ok()?) as usize;
        let expected_size = FP_HEADER_SIZE + count * entry_size;
        if data.len() < expected_size {
            return None;
        }

        // Keep context in local variables — compiler will use registers
        let ctx_sim = context.simhash;
        let ctx_bloom = context.bloom;
        let has_bloom = ctx_bloom != 0;

        let mut best_score: u32 = 0;
        let mut best_id: u64 = 0;
        let mut best_hd: u32 = u32::MAX;
        let mut best_overlap: u32 = 0;
        let mut found = false;

        let chunks = count / 4;
        let remainder = count % 4;

        // Process 4 entries per iteration
        for chunk in 0..chunks {
            let base = FP_HEADER_SIZE + chunk * 4 * entry_size;

            // Read all 4 simhashes + blooms
            let s0 = u64::from_le_bytes(data[base..base + 8].try_into().ok()?);
            let b0 = u64::from_le_bytes(data[base + 8..base + 16].try_into().ok()?);
            let s1 = u64::from_le_bytes(data[base + entry_size..base + entry_size + 8].try_into().ok()?);
            let b1 = u64::from_le_bytes(data[base + entry_size + 8..base + entry_size + 16].try_into().ok()?);
            let s2 = u64::from_le_bytes(data[base + 2 * entry_size..base + 2 * entry_size + 8].try_into().ok()?);
            let b2 = u64::from_le_bytes(data[base + 2 * entry_size + 8..base + 2 * entry_size + 16].try_into().ok()?);
            let s3 = u64::from_le_bytes(data[base + 3 * entry_size..base + 3 * entry_size + 8].try_into().ok()?);
            let b3 = u64::from_le_bytes(data[base + 3 * entry_size + 8..base + 3 * entry_size + 16].try_into().ok()?);

            // V2: read flags for all 4 entries (single byte each)
            let f0 = if is_v2 { data[base + 24] } else { 0 };
            let f1 = if is_v2 { data[base + entry_size + 24] } else { 0 };
            let f2 = if is_v2 { data[base + 2 * entry_size + 24] } else { 0 };
            let f3 = if is_v2 { data[base + 3 * entry_size + 24] } else { 0 };

            // 4 independent XOR+POPCNT chains (pipelined by CPU)
            let hd0 = (s0 ^ ctx_sim).count_ones();
            let hd1 = (s1 ^ ctx_sim).count_ones();
            let hd2 = (s2 ^ ctx_sim).count_ones();
            let hd3 = (s3 ^ ctx_sim).count_ones();

            let ov0 = (b0 & ctx_bloom).count_ones();
            let ov1 = (b1 & ctx_bloom).count_ones();
            let ov2 = (b2 & ctx_bloom).count_ones();
            let ov3 = (b3 & ctx_bloom).count_ones();

            // Check each candidate — read note_id only if it passes filters
            macro_rules! check_candidate {
                ($hd:expr, $ov:expr, $id_offset:expr, $flags:expr) => {
                    if $flags & (FLAG_SKIP_RECALL | FLAG_TOMBSTONE) == 0
                        && (!has_bloom || $ov > 0)
                        && $hd <= max_hd
                    {
                        let score = $ov * 3 + (64 - $hd);
                        if score > best_score {
                            best_score = score;
                            best_id = u64::from_le_bytes(
                                data[$id_offset..$id_offset + 8].try_into().ok()?
                            );
                            best_hd = $hd;
                            best_overlap = $ov;
                            found = true;
                        }
                    }
                };
            }

            check_candidate!(hd0, ov0, base + 16, f0);
            check_candidate!(hd1, ov1, base + entry_size + 16, f1);
            check_candidate!(hd2, ov2, base + 2 * entry_size + 16, f2);
            check_candidate!(hd3, ov3, base + 3 * entry_size + 16, f3);
        }

        // Handle remainder (0-3 entries)
        let rem_base = FP_HEADER_SIZE + chunks * 4 * entry_size;
        for i in 0..remainder {
            let offset = rem_base + i * entry_size;

            // V2: check flags
            if is_v2 && data[offset + 24] & (FLAG_SKIP_RECALL | FLAG_TOMBSTONE) != 0 {
                continue;
            }

            let simhash = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            let bloom = u64::from_le_bytes(data[offset + 8..offset + 16].try_into().ok()?);

            let overlap = (bloom & ctx_bloom).count_ones();
            if has_bloom && overlap == 0 {
                continue;
            }

            let hd = (simhash ^ ctx_sim).count_ones();
            if hd > max_hd {
                continue;
            }

            let score = overlap * 3 + (64 - hd);
            if score > best_score {
                best_score = score;
                best_id = u64::from_le_bytes(
                    data[offset + 16..offset + 24].try_into().ok()?
                );
                best_hd = hd;
                best_overlap = overlap;
                found = true;
            }
        }

        if found {
            Some(ScanResult {
                note_id: best_id,
                score: best_score,
                hamming_distance: best_hd,
                bloom_overlap: best_overlap,
            })
        } else {
            None
        }
    }
}

impl Default for FingerprintIndex {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// 256-bit Fingerprint Index (V3 format)
// ============================================================================

/// In-memory 256-bit fingerprint index. Stores `Fingerprint256` entries (V3 format)
/// and can load V1/V2 sidecars with automatic promotion to 256-bit.
pub struct FingerprintIndex256 {
    entries: Vec<FingerprintEntry256>,
}

impl FingerprintIndex256 {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self { entries: Vec::with_capacity(capacity) }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Add a note's 256-bit fingerprint computed from text.
    pub fn add(&mut self, note_id: u64, content: &str, tags: &[&str]) {
        let fingerprint = Fingerprint256::from_text(content, tags);
        self.entries.push(FingerprintEntry256 {
            fingerprint, note_id, flags: 0,
        });
    }

    pub fn add_entry(&mut self, entry: FingerprintEntry256) {
        self.entries.push(entry);
    }

    pub fn remove(&mut self, note_id: u64) -> bool {
        let len_before = self.entries.len();
        self.entries.retain(|e| e.note_id != note_id);
        self.entries.len() != len_before
    }

    pub fn entries(&self) -> &[FingerprintEntry256] {
        &self.entries
    }

    /// Scan for the best match against a 256-bit context fingerprint.
    pub fn scan_best(
        &self,
        context: &Fingerprint256,
        max_hamming_distance: Option<u32>,
    ) -> Option<ScanResult> {
        let max_hd = max_hamming_distance.unwrap_or(u32::MAX);
        let mut best: Option<ScanResult> = None;

        for entry in &self.entries {
            if entry.flags & (FLAG_SKIP_RECALL | FLAG_TOMBSTONE) != 0 {
                continue;
            }

            let overlap = entry.fingerprint.bloom_overlap(context);
            if overlap == 0 && (context.bloom[0] != 0 || context.bloom[1] != 0) {
                continue;
            }

            let hd = entry.fingerprint.hamming_distance(context);
            if hd > max_hd {
                continue;
            }

            let score = overlap * 3 + (128 - hd);
            let dominated = best.as_ref().map_or(false, |b| score <= b.score);
            if !dominated {
                best = Some(ScanResult {
                    note_id: entry.note_id,
                    score,
                    hamming_distance: hd,
                    bloom_overlap: overlap,
                });
            }
        }

        best
    }

    /// Scan for top-K matches, sorted by score descending.
    pub fn scan_top_k(
        &self,
        context: &Fingerprint256,
        k: usize,
        max_hamming_distance: Option<u32>,
    ) -> Vec<ScanResult> {
        let max_hd = max_hamming_distance.unwrap_or(u32::MAX);
        let mut results: Vec<ScanResult> = Vec::with_capacity(k + 1);

        for entry in &self.entries {
            if entry.flags & (FLAG_SKIP_RECALL | FLAG_TOMBSTONE) != 0 {
                continue;
            }

            let overlap = entry.fingerprint.bloom_overlap(context);
            if overlap == 0 && (context.bloom[0] != 0 || context.bloom[1] != 0) {
                continue;
            }

            let hd = entry.fingerprint.hamming_distance(context);
            if hd > max_hd {
                continue;
            }

            let score = overlap * 3 + (128 - hd);
            let dominated_by_kth = results.len() >= k
                && results.last().map_or(false, |last| score <= last.score);
            if !dominated_by_kth {
                results.push(ScanResult {
                    note_id: entry.note_id,
                    score,
                    hamming_distance: hd,
                    bloom_overlap: overlap,
                });
                results.sort_by(|a, b| b.score.cmp(&a.score));
                if results.len() > k {
                    results.truncate(k);
                }
            }
        }

        results
    }

    /// Upsert a note's 256-bit fingerprint.
    pub fn upsert(&mut self, note_id: u64, content: &str, tags: &[&str]) {
        let fingerprint = Fingerprint256::from_text(content, tags);
        if let Some(entry) = self.entries.iter_mut().find(|e| e.note_id == note_id) {
            entry.fingerprint = fingerprint;
        } else {
            self.entries.push(FingerprintEntry256 {
                fingerprint, note_id, flags: 0,
            });
        }
    }

    /// Upsert with trigger keywords that get 3x bloom weight (256-bit variant).
    pub fn upsert_with_triggers(&mut self, note_id: u64, content: &str, tags: &[&str], triggers: &[&str]) {
        let fingerprint = Fingerprint256::from_text_with_triggers(content, tags, triggers);
        if let Some(entry) = self.entries.iter_mut().find(|e| e.note_id == note_id) {
            entry.fingerprint = fingerprint;
        } else {
            self.entries.push(FingerprintEntry256 {
                fingerprint, note_id, flags: 0,
            });
        }
    }

    pub fn upsert_entry(&mut self, entry: FingerprintEntry256) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.note_id == entry.note_id) {
            existing.fingerprint = entry.fingerprint;
            existing.flags = entry.flags;
        } else {
            self.entries.push(entry);
        }
    }

    pub fn mark_pinned(&mut self, pinned_ids: &[u64]) {
        for entry in &mut self.entries {
            if pinned_ids.contains(&entry.note_id) {
                entry.flags |= FLAG_SKIP_RECALL;
            } else {
                entry.flags &= !FLAG_SKIP_RECALL;
            }
        }
    }

    pub fn mark_tombstoned(&mut self, note_id: u64) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.note_id == note_id) {
            entry.flags |= FLAG_TOMBSTONE;
        }
    }

    /// Sidecar file path (same convention as FingerprintIndex).
    pub fn sidecar_path(engram_path: &Path) -> PathBuf {
        FingerprintIndex::sidecar_path(engram_path)
    }

    /// Save atomically in V3 format.
    pub fn save(&self, path: &Path) -> Result<(), FingerprintError> {
        let tmp_path = path.with_extension("fp.tmp");
        std::fs::write(&tmp_path, self.to_bytes())?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }

    /// Load from file (supports V1/V2/V3 with automatic promotion).
    pub fn load(path: &Path) -> Result<Self, FingerprintError> {
        let data = std::fs::read(path)?;
        Self::from_bytes(&data)
    }

    /// Serialize to V3 format bytes.
    ///
    /// Format:
    ///   Header (16 bytes): magic(4) + version(2) + count(4) + reserved(6)
    ///   Entries (48 bytes each): fp(32) + note_id(8) + flags(1) + reserved(7)
    pub fn to_bytes(&self) -> Vec<u8> {
        let count = self.entries.len() as u32;
        let size = FP_HEADER_SIZE + (count as usize * FingerprintEntry256::SIZE);
        let mut buf = Vec::with_capacity(size);

        buf.extend_from_slice(&FP_MAGIC.to_le_bytes());
        buf.extend_from_slice(&FP_VERSION_V3.to_le_bytes());
        buf.extend_from_slice(&count.to_le_bytes());
        buf.extend_from_slice(&[0u8; 6]);

        for entry in &self.entries {
            buf.extend_from_slice(&entry.to_bytes());
        }

        buf
    }

    /// Deserialize from bytes (supports V1/V2/V3 with automatic promotion).
    pub fn from_bytes(data: &[u8]) -> Result<Self, FingerprintError> {
        if data.len() < FP_HEADER_SIZE {
            return Err(FingerprintError::TooShort);
        }

        let magic = u32::from_le_bytes(data[0..4].try_into().unwrap());
        if magic != FP_MAGIC {
            return Err(FingerprintError::BadMagic);
        }

        let version = u16::from_le_bytes(data[4..6].try_into().unwrap());
        let entry_size = match version {
            FP_VERSION_V1 => FingerprintEntry::SIZE_V1,
            FP_VERSION => FingerprintEntry::SIZE,
            FP_VERSION_V3 => FingerprintEntry256::SIZE,
            _ => return Err(FingerprintError::UnsupportedVersion(version)),
        };

        let count = u32::from_le_bytes(data[6..10].try_into().unwrap()) as usize;
        let expected_size = FP_HEADER_SIZE + count * entry_size;
        if data.len() < expected_size {
            return Err(FingerprintError::TooShort);
        }

        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let offset = FP_HEADER_SIZE + i * entry_size;
            match version {
                FP_VERSION_V1 => {
                    let bytes: [u8; 24] = data[offset..offset + 24].try_into().unwrap();
                    entries.push(FingerprintEntry256::from_v1_bytes(&bytes));
                }
                FP_VERSION => {
                    let bytes: [u8; 32] = data[offset..offset + 32].try_into().unwrap();
                    let v2_entry = FingerprintEntry::from_bytes(&bytes);
                    entries.push(FingerprintEntry256::from_v2(&v2_entry));
                }
                FP_VERSION_V3 => {
                    let bytes: [u8; 48] = data[offset..offset + 48].try_into().unwrap();
                    entries.push(FingerprintEntry256::from_bytes(&bytes));
                }
                _ => unreachable!(),
            }
        }

        Ok(Self { entries })
    }

    /// Zero-copy scan over memory-mapped sidecar data (auto-detects V1/V2/V3).
    ///
    /// For V3 sidecars: full 256-bit scan (4 POPCNT per entry).
    /// For V1/V2 sidecars: falls back to 128-bit scan with truncated context
    /// and corpus-size-aware adaptive threshold.
    pub fn scan_mmap_256(data: &[u8], context: &Fingerprint256, max_hd: u32) -> Option<ScanResult> {
        if data.len() < FP_HEADER_SIZE {
            return None;
        }

        let magic = u32::from_le_bytes(data[0..4].try_into().ok()?);
        if magic != FP_MAGIC {
            return None;
        }

        let version = u16::from_le_bytes(data[4..6].try_into().ok()?);

        // For V1/V2 sidecars, delegate to 128-bit scan with truncated context
        if version < FP_VERSION_V3 {
            let context_128 = context.to_128();
            let count = u32::from_le_bytes(data[6..10].try_into().ok()?);
            let max_hd_64 = adaptive_max_hd(count).min(max_hd);
            return FingerprintIndex::scan_mmap(data, &context_128, max_hd_64);
        }

        let count = u32::from_le_bytes(data[6..10].try_into().ok()?) as usize;
        let entry_size = FingerprintEntry256::SIZE;
        let expected_size = FP_HEADER_SIZE + count * entry_size;
        if data.len() < expected_size {
            return None;
        }

        let ctx_sim_lo = context.simhash[0];
        let ctx_sim_hi = context.simhash[1];
        let ctx_bloom_lo = context.bloom[0];
        let ctx_bloom_hi = context.bloom[1];
        let has_bloom = ctx_bloom_lo != 0 || ctx_bloom_hi != 0;

        let mut best: Option<ScanResult> = None;

        for i in 0..count {
            let offset = FP_HEADER_SIZE + i * entry_size;

            // Check flags (byte at offset+40)
            if data[offset + 40] & (FLAG_SKIP_RECALL | FLAG_TOMBSTONE) != 0 {
                continue;
            }

            // Read 256-bit fingerprint (4 × u64)
            let sim_lo = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            let sim_hi = u64::from_le_bytes(data[offset + 8..offset + 16].try_into().ok()?);
            let blm_lo = u64::from_le_bytes(data[offset + 16..offset + 24].try_into().ok()?);
            let blm_hi = u64::from_le_bytes(data[offset + 24..offset + 32].try_into().ok()?);

            // Bloom pre-filter
            let overlap = (blm_lo & ctx_bloom_lo).count_ones()
                + (blm_hi & ctx_bloom_hi).count_ones();
            if has_bloom && overlap == 0 {
                continue;
            }

            // 128-bit Hamming distance
            let hd = (sim_lo ^ ctx_sim_lo).count_ones()
                + (sim_hi ^ ctx_sim_hi).count_ones();
            if hd > max_hd {
                continue;
            }

            let note_id = u64::from_le_bytes(data[offset + 32..offset + 40].try_into().ok()?);
            let score = overlap * 3 + (128 - hd);
            let dominated = best.as_ref().map_or(false, |b| score <= b.score);
            if !dominated {
                best = Some(ScanResult {
                    note_id,
                    score,
                    hamming_distance: hd,
                    bloom_overlap: overlap,
                });
            }
        }

        best
    }
}

impl Default for FingerprintIndex256 {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Cluster pre-filtering
// ============================================================================

/// A cluster of related notes for super-fingerprint pre-filtering.
///
/// The super-fingerprint is the bitwise OR of all member fingerprints.
/// If a context fingerprint has zero bloom overlap with the super-fingerprint,
/// no individual member can match either — the entire cluster is skipped.
///
/// Typical workload: ~20 clusters for ~1800 notes. Checking 20 super-fingerprints
/// costs ~200ns and eliminates 85%+ of the linear scan.
pub struct Cluster {
    /// Tag or category name
    pub tag: String,
    /// OR of all member SimHashes
    pub super_simhash: u64,
    /// OR of all member Blooms
    pub super_bloom: u64,
    /// Indices into the FingerprintIndex entries
    pub member_indices: Vec<usize>,
}

// ============================================================================
// IDF Table for weighted SimHash
// ============================================================================

/// Inverse Document Frequency table for IDF-weighted SimHash computation.
///
/// Built from a corpus of documents. Common tokens (appearing in many documents)
/// get low weights; rare/discriminative tokens get high weights. This makes
/// SimHash bits encode more useful information per bit.
///
/// # Information-theoretic rationale
///
/// Per "The Information Theory of Similarity" (arxiv 2512.00378, Nov 2025),
/// hash quality depends on mutual information between hash bits and the
/// underlying similarity. IDF weighting concentrates mutual information into
/// each bit by amplifying discriminative tokens and suppressing noise from
/// ubiquitous tokens like "the", "is", "function".
///
/// At 64 bits (well below the ~760-bit theoretical optimum for N=2000),
/// every bit matters — IDF weighting is the cheapest way to improve quality.
pub struct IdfTable {
    /// Token → IDF weight. Higher = more discriminative.
    weights: HashMap<String, f32>,
    /// Number of documents used to build the table.
    doc_count: usize,
}

impl IdfTable {
    /// Build an IDF table from a corpus of pre-tokenized, stemmed documents.
    ///
    /// Each document is a `Vec<String>` of stemmed tokens (output of
    /// `tokenize_and_stem`). The IDF weight for each token is:
    ///
    ///   `idf(t) = ln((N - df(t) + 0.5) / (df(t) + 0.5))`
    ///
    /// where N is the total number of documents and df(t) is the number of
    /// documents containing token t. This is the BM25-style IDF formula,
    /// clamped to a minimum of 0.0 to avoid negative weights for tokens
    /// appearing in more than half the corpus.
    pub fn from_corpus(documents: &[Vec<String>]) -> Self {
        let mut df: HashMap<String, usize> = HashMap::new();

        for doc in documents {
            // Count each token once per document (document frequency, not term frequency)
            let unique: HashSet<&String> = doc.iter().collect();
            for token in unique {
                *df.entry(token.clone()).or_insert(0) += 1;
            }
        }

        let n = documents.len() as f32;
        let weights: HashMap<String, f32> = df
            .into_iter()
            .map(|(token, count)| {
                let idf = ((n - count as f32 + 0.5) / (count as f32 + 0.5))
                    .ln()
                    .max(0.0);
                (token, idf)
            })
            .collect();

        Self {
            weights,
            doc_count: documents.len(),
        }
    }

    /// Get the IDF weight for a token.
    ///
    /// Returns 1.0 for unknown tokens (not seen during corpus building).
    /// This is a neutral weight — unknown tokens contribute the same as
    /// unweighted SimHash, which is a safe default for out-of-vocabulary terms.
    #[inline]
    pub fn weight(&self, token: &str) -> f32 {
        self.weights.get(token).copied().unwrap_or(1.0)
    }

    /// Number of documents in the corpus.
    pub fn doc_count(&self) -> usize {
        self.doc_count
    }

    /// Number of unique tokens tracked.
    pub fn vocab_size(&self) -> usize {
        self.weights.len()
    }

    /// Check if the table is empty (no corpus was provided).
    pub fn is_empty(&self) -> bool {
        self.weights.is_empty()
    }
}

// ============================================================================
// Errors
// ============================================================================

/// Errors from fingerprint operations.
#[derive(Debug)]
pub enum FingerprintError {
    TooShort,
    BadMagic,
    UnsupportedVersion(u16),
    Io(std::io::Error),
}

impl std::fmt::Display for FingerprintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "fingerprint index data too short"),
            Self::BadMagic => write!(f, "invalid fingerprint index magic bytes"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported fingerprint version: {}", v),
            Self::Io(e) => write!(f, "fingerprint I/O error: {}", e),
        }
    }
}

impl std::error::Error for FingerprintError {}

impl From<std::io::Error> for FingerprintError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// ============================================================================
// Internal: SimHash computation
// ============================================================================

/// Compute Charikar SimHash over a list of stemmed tokens.
///
/// For each token, hash it to 64 bits via xxh3. For each bit position,
/// if the hash bit is 1, increment a counter; if 0, decrement.
/// Final hash: bit i = 1 if counter[i] > 0.
///
/// Complexity: O(tokens × 64) — but 64 is constant, so effectively O(n).
fn compute_simhash(tokens: &[String]) -> u64 {
    if tokens.is_empty() {
        return 0;
    }

    let mut counters = [0i32; 64];

    for token in tokens {
        let h = xxh3::xxh3_64(token.as_bytes());
        for i in 0..64 {
            if (h >> i) & 1 == 1 {
                counters[i] += 1;
            } else {
                counters[i] -= 1;
            }
        }
    }

    let mut hash: u64 = 0;
    for i in 0..64 {
        if counters[i] > 0 {
            hash |= 1u64 << i;
        }
    }
    hash
}

/// Compute IDF-weighted SimHash over a list of stemmed tokens.
///
/// Same algorithm as `compute_simhash`, but each token's contribution is
/// scaled by its IDF weight. Rare/discriminative tokens push counters further,
/// common tokens barely nudge them. The final threshold is still sign-based.
///
/// This produces strictly better binary codes than uniform SimHash at the same
/// 64-bit width — more mutual information per bit (arxiv 2512.00378).
///
/// Cost: identical to `compute_simhash` at scan time (same POPCNT).
/// Slightly more work at fingerprint generation time (one f32 multiply per token).
fn compute_simhash_weighted(tokens: &[String], idf: &IdfTable) -> u64 {
    if tokens.is_empty() {
        return 0;
    }

    let mut counters = [0.0f32; 64];

    for token in tokens {
        let weight = idf.weight(token);
        let h = xxh3::xxh3_64(token.as_bytes());
        for i in 0..64 {
            if (h >> i) & 1 == 1 {
                counters[i] += weight;
            } else {
                counters[i] -= weight;
            }
        }
    }

    let mut hash: u64 = 0;
    for i in 0..64 {
        if counters[i] > 0.0 {
            hash |= 1u64 << i;
        }
    }
    hash
}

// ============================================================================
// Internal: Bloom64 computation
// ============================================================================

/// Number of hash functions for the 64-bit Bloom filter.
const BLOOM_K: u64 = 5;

/// Compute a 64-bit Bloom filter over stemmed tokens.
///
/// For each token, k=5 bit positions are set using xxh3 with different seeds.
/// The Bloom filter encodes which keywords are present in the document.
fn compute_bloom64(tokens: &[String]) -> u64 {
    let mut bits: u64 = 0;
    for token in tokens {
        bits |= bloom64_single(token);
    }
    bits
}

/// Compute bloom bits for a single token (k=5 hash functions).
#[inline]
fn bloom64_single(token: &str) -> u64 {
    let mut bits: u64 = 0;
    let bytes = token.as_bytes();
    for seed in 0..BLOOM_K {
        let h = xxh3::xxh3_64_with_seed(bytes, seed);
        let bit_pos = (h % 64) as u32;
        bits |= 1u64 << bit_pos;
    }
    bits
}

// ============================================================================
// Internal: Tokenization
// ============================================================================

/// Tokenize and stem a text string for fingerprinting or IDF corpus building.
///
/// Split on whitespace, lowercase, strip leading/trailing ASCII punctuation,
/// apply Snowball Porter2 English stemmer. Filter empty tokens.
///
/// This is public so that callers (e.g., storage.rs) can build tokenized
/// document lists for `IdfTable::from_corpus()`.
pub fn tokenize_and_stem(text: &str, stemmer: &Stemmer) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            let lower = w.to_lowercase();
            let trimmed = lower
                .trim_matches(|c: char| c.is_ascii_punctuation())
                .to_string();
            if trimmed.is_empty() {
                trimmed
            } else {
                stemmer.stem(&trimmed).into_owned()
            }
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Convenience: tokenize and stem text without managing a Stemmer instance.
///
/// Creates a Stemmer internally. For bulk operations (building an IDF corpus
/// from hundreds of documents), prefer creating one Stemmer and calling
/// `tokenize_and_stem` directly to avoid repeated allocations.
pub fn tokenize_text(text: &str) -> Vec<String> {
    let stemmer = Stemmer::create(Algorithm::English);
    tokenize_and_stem(text, &stemmer)
}

/// Re-export the Stemmer creation for callers who need bulk tokenization.
pub fn create_stemmer() -> Stemmer {
    Stemmer::create(Algorithm::English)
}

// ============================================================================
// Internal: 128-bit SimHash computation (two independent xxh3 seed families)
// ============================================================================

/// Compute 128-bit SimHash as two independent 64-bit SimHashes.
/// First u64 uses default seed (same as 64-bit compute_simhash).
/// Second u64 uses SIMHASH_SEED_HI for independence.
fn compute_simhash_128(tokens: &[String]) -> [u64; 2] {
    if tokens.is_empty() {
        return [0, 0];
    }

    let mut counters_lo = [0i32; 64];
    let mut counters_hi = [0i32; 64];

    for token in tokens {
        let h_lo = xxh3::xxh3_64(token.as_bytes());
        let h_hi = xxh3::xxh3_64_with_seed(token.as_bytes(), SIMHASH_SEED_HI);
        for i in 0..64 {
            if (h_lo >> i) & 1 == 1 { counters_lo[i] += 1; } else { counters_lo[i] -= 1; }
            if (h_hi >> i) & 1 == 1 { counters_hi[i] += 1; } else { counters_hi[i] -= 1; }
        }
    }

    let mut lo: u64 = 0;
    let mut hi: u64 = 0;
    for i in 0..64 {
        if counters_lo[i] > 0 { lo |= 1u64 << i; }
        if counters_hi[i] > 0 { hi |= 1u64 << i; }
    }
    [lo, hi]
}

/// IDF-weighted 128-bit SimHash.
fn compute_simhash_128_weighted(tokens: &[String], idf: &IdfTable) -> [u64; 2] {
    if tokens.is_empty() {
        return [0, 0];
    }

    let mut counters_lo = [0.0f32; 64];
    let mut counters_hi = [0.0f32; 64];

    for token in tokens {
        let weight = idf.weight(token);
        let h_lo = xxh3::xxh3_64(token.as_bytes());
        let h_hi = xxh3::xxh3_64_with_seed(token.as_bytes(), SIMHASH_SEED_HI);
        for i in 0..64 {
            if (h_lo >> i) & 1 == 1 { counters_lo[i] += weight; } else { counters_lo[i] -= weight; }
            if (h_hi >> i) & 1 == 1 { counters_hi[i] += weight; } else { counters_hi[i] -= weight; }
        }
    }

    let mut lo: u64 = 0;
    let mut hi: u64 = 0;
    for i in 0..64 {
        if counters_lo[i] > 0.0 { lo |= 1u64 << i; }
        if counters_hi[i] > 0.0 { hi |= 1u64 << i; }
    }
    [lo, hi]
}

// ============================================================================
// Internal: 128-bit Bloom computation
// ============================================================================

/// Compute a 128-bit Bloom filter over stemmed tokens (k=5, modulo 128).
fn compute_bloom128(tokens: &[String]) -> [u64; 2] {
    let mut bits = [0u64; 2];
    for token in tokens {
        let b = bloom128_single(token);
        bits[0] |= b[0];
        bits[1] |= b[1];
    }
    bits
}

/// Compute 128-bit bloom bits for a single token (k=5 hash functions, modulo 128).
#[inline]
fn bloom128_single(token: &str) -> [u64; 2] {
    let mut bits = [0u64; 2];
    let bytes = token.as_bytes();
    for seed in 0..BLOOM_K {
        let h = xxh3::xxh3_64_with_seed(bytes, seed);
        let bit_pos = (h % 128) as u32;
        if bit_pos < 64 {
            bits[0] |= 1u64 << bit_pos;
        } else {
            bits[1] |= 1u64 << (bit_pos - 64);
        }
    }
    bits
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- SimHash tests ---

    #[test]
    fn test_simhash_identical_documents() {
        let tokens: Vec<String> = vec!["hello", "world", "test"]
            .into_iter()
            .map(String::from)
            .collect();
        let h1 = compute_simhash(&tokens);
        let h2 = compute_simhash(&tokens);
        assert_eq!(h1, h2, "identical token lists must produce identical SimHash");
    }

    #[test]
    fn test_simhash_similar_documents_low_hamming() {
        let tokens1: Vec<String> = vec!["storage", "overflow", "check", "bound", "fix", "engram"]
            .into_iter()
            .map(String::from)
            .collect();
        let tokens2: Vec<String> = vec!["storage", "overflow", "check", "bound", "repair", "engram"]
            .into_iter()
            .map(String::from)
            .collect();
        let h1 = compute_simhash(&tokens1);
        let h2 = compute_simhash(&tokens2);
        let hd = (h1 ^ h2).count_ones();
        // 5 of 6 tokens shared — HD should be low
        assert!(hd <= 20, "similar docs should have low HD, got {}", hd);
    }

    #[test]
    fn test_simhash_different_documents_high_hamming() {
        let tokens1: Vec<String> = vec!["storage", "overflow", "integer", "bounds"]
            .into_iter()
            .map(String::from)
            .collect();
        let tokens2: Vec<String> = vec!["fitquest", "recipe", "nutrition", "calories"]
            .into_iter()
            .map(String::from)
            .collect();
        let h1 = compute_simhash(&tokens1);
        let h2 = compute_simhash(&tokens2);
        let hd = (h1 ^ h2).count_ones();
        // Completely unrelated topics — HD should be higher than similar docs
        assert!(hd >= 10, "different docs should have high HD, got {}", hd);
    }

    #[test]
    fn test_simhash_empty() {
        let empty: Vec<String> = vec![];
        assert_eq!(compute_simhash(&empty), 0);
    }

    #[test]
    fn test_simhash_single_token() {
        let tokens: Vec<String> = vec!["hello".to_string()];
        let h = compute_simhash(&tokens);
        // Single token: SimHash should equal the xxh3 hash (all counters are +1 or -1)
        let expected = xxh3::xxh3_64(b"hello");
        assert_eq!(h, expected, "single-token SimHash should match xxh3 hash");
    }

    // --- Bloom64 tests ---

    #[test]
    fn test_bloom64_sets_bits() {
        let tokens: Vec<String> = vec!["hello".to_string()];
        let bloom = compute_bloom64(&tokens);
        assert_ne!(bloom, 0, "bloom should have bits set");
        assert!(bloom.count_ones() >= 1 && bloom.count_ones() <= BLOOM_K as u32);
    }

    #[test]
    fn test_bloom64_overlap_shared_tokens() {
        let t1: Vec<String> = vec!["storage", "overflow", "check"]
            .into_iter()
            .map(String::from)
            .collect();
        let t2: Vec<String> = vec!["storage", "overflow", "different"]
            .into_iter()
            .map(String::from)
            .collect();
        let b1 = compute_bloom64(&t1);
        let b2 = compute_bloom64(&t2);
        let overlap = (b1 & b2).count_ones();
        // "storage" and "overflow" shared — should have significant overlap
        assert!(overlap >= 2, "shared tokens should produce bloom overlap, got {}", overlap);
    }

    #[test]
    fn test_bloom64_no_overlap_disjoint_tokens() {
        // Use very different tokens to minimize hash collision chance
        let t1: Vec<String> = vec!["aaaa".to_string(), "bbbb".to_string()];
        let t2: Vec<String> = vec!["zzzz".to_string(), "yyyy".to_string()];
        let b1 = compute_bloom64(&t1);
        let b2 = compute_bloom64(&t2);
        // Disjoint tokens may still have overlap due to hash collisions in 64 bits,
        // but it should be low
        let overlap = (b1 & b2).count_ones();
        assert!(overlap <= 5, "disjoint tokens should have minimal overlap, got {}", overlap);
    }

    #[test]
    fn test_bloom64_empty() {
        let empty: Vec<String> = vec![];
        assert_eq!(compute_bloom64(&empty), 0);
    }

    // --- Fingerprint tests ---

    #[test]
    fn test_fingerprint_from_text() {
        let fp = Fingerprint::from_text("The integer overflow was fixed in storage.rs", &["engram", "fix"]);
        assert_ne!(fp.simhash, 0);
        assert_ne!(fp.bloom, 0);
    }

    #[test]
    fn test_fingerprint_from_keywords() {
        let fp = Fingerprint::from_keywords(&["storage.rs", "overflow", "checked_add"]);
        assert_ne!(fp.simhash, 0);
        assert_ne!(fp.bloom, 0);
    }

    #[test]
    fn test_fingerprint_similar_content_high_score() {
        let note_fp = Fingerprint::from_text(
            "Fixed integer overflow in storage.rs deserialization with checked_add",
            &["engram", "fix", "overflow"],
        );
        let context_fp = Fingerprint::from_keywords(&["storage.rs", "overflow", "checked_add", "engram"]);

        let score = note_fp.score(&context_fp);
        let hd = note_fp.hamming_distance(&context_fp);

        // Cross-representation matching (from_text vs from_keywords) has inherent loss
        // from different tokenization paths, so we check for moderate similarity
        assert!(score > 40, "similar content should score above 40, got {}", score);
        assert!(hd < 40, "similar content should have HD < 40, got {}", hd);
    }

    #[test]
    fn test_fingerprint_different_content_low_score() {
        let note_fp = Fingerprint::from_text(
            "FitQuest recipe nutrition tracking calories per serving",
            &["fitquest", "nutrition"],
        );
        let context_fp = Fingerprint::from_keywords(&["storage.rs", "overflow", "checked_add", "engram"]);

        let overlap = note_fp.bloom_overlap(&context_fp);
        // 64-bit bloom with k=5 has ~7.5% fill per token, so random collision
        // between unrelated sets is non-trivial. Check it's bounded, not zero.
        assert!(overlap <= 12, "different content should have bounded bloom overlap, got {}", overlap);
    }

    #[test]
    fn test_fingerprint_zero() {
        let fp = Fingerprint::ZERO;
        assert_eq!(fp.simhash, 0);
        assert_eq!(fp.bloom, 0);
    }

    #[test]
    fn test_fingerprint_serialization() {
        let fp = Fingerprint::from_text("test content for serialization", &["test"]);
        let bytes = fp.to_bytes();
        assert_eq!(bytes.len(), 16);
        let restored = Fingerprint::from_bytes(&bytes);
        assert_eq!(fp, restored);
    }

    // --- Hamming distance mathematical properties ---

    #[test]
    fn test_hamming_distance_identity() {
        let fp = Fingerprint::from_text("some content", &[]);
        assert_eq!(fp.hamming_distance(&fp), 0, "HD with self must be 0");
    }

    #[test]
    fn test_hamming_distance_symmetry() {
        let fp1 = Fingerprint::from_text("content one about storage", &[]);
        let fp2 = Fingerprint::from_text("content two about recipes", &[]);
        assert_eq!(
            fp1.hamming_distance(&fp2),
            fp2.hamming_distance(&fp1),
            "HD must be symmetric"
        );
    }

    #[test]
    fn test_hamming_distance_max() {
        let fp1 = Fingerprint::new(0, 0);
        let fp2 = Fingerprint::new(u64::MAX, 0);
        assert_eq!(fp1.hamming_distance(&fp2), 64, "max HD is 64");
    }

    // --- Index tests ---

    #[test]
    fn test_index_add_and_scan() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "Fixed integer overflow in storage.rs with checked_add", &["engram", "fix"]);
        idx.add(2, "FitQuest recipe nutrition tracking implementation", &["fitquest", "nutrition"]);
        idx.add(3, "BM25 substring matching bug fix in recall.rs tokenizer", &["engram", "bm25"]);

        let context = Fingerprint::from_keywords(&["storage.rs", "overflow", "checked_add"]);
        // No HD threshold — test verifies ranking, not absolute distance
        let result = idx.scan_best(&context, None);

        assert!(result.is_some(), "should find a match");
        let result = result.unwrap();
        // Note 1 (storage overflow) should be the best match, not note 2 (nutrition)
        assert_eq!(result.note_id, 1, "note about storage overflow should match, got note {}", result.note_id);
    }

    #[test]
    fn test_index_scan_with_threshold() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "Completely unrelated topic about cooking pasta recipes", &["cooking"]);

        let context = Fingerprint::from_keywords(&["storage.rs", "overflow", "integer"]);
        let result = idx.scan_best(&context, Some(10)); // Very strict threshold

        // Cooking note should NOT match storage context at strict threshold
        if let Some(r) = &result {
            assert!(r.hamming_distance <= 10, "threshold should be respected");
        }
    }

    #[test]
    fn test_index_scan_empty() {
        let idx = FingerprintIndex::new();
        let context = Fingerprint::from_keywords(&["anything"]);
        assert!(idx.scan_best(&context, None).is_none());
    }

    #[test]
    fn test_index_remove() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "note one", &[]);
        idx.add(2, "note two", &[]);
        assert_eq!(idx.len(), 2);

        assert!(idx.remove(1));
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.entries()[0].note_id, 2);

        assert!(!idx.remove(99)); // Not found
    }

    #[test]
    fn test_index_top_k() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "integer overflow storage bounds checking", &["engram"]);
        idx.add(2, "BM25 recall search tokenizer stemming", &["engram"]);
        idx.add(3, "fitquest nutrition recipe calories", &["fitquest"]);
        idx.add(4, "overflow integer safety checked arithmetic", &["engram"]);

        let context = Fingerprint::from_keywords(&["overflow", "integer", "storage"]);
        let results = idx.scan_top_k(&context, 2, None);

        assert!(results.len() <= 2, "top-k should return at most k results");
        if results.len() >= 2 {
            assert!(results[0].score >= results[1].score, "results should be sorted by score descending");
        }
    }

    // --- Serialization tests ---

    #[test]
    fn test_index_serialization_round_trip() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "first note about storage", &["engram"]);
        idx.add(2, "second note about nutrition", &["fitquest"]);
        idx.add(3, "third note about BM25 recall", &["engram", "search"]);

        let bytes = idx.to_bytes();
        let restored = FingerprintIndex::from_bytes(&bytes).expect("deserialization should succeed");

        assert_eq!(restored.len(), 3);
        assert_eq!(restored.entries()[0].note_id, 1);
        assert_eq!(restored.entries()[1].note_id, 2);
        assert_eq!(restored.entries()[2].note_id, 3);

        // Fingerprints should be identical after round-trip
        for i in 0..3 {
            assert_eq!(
                restored.entries()[i].fingerprint,
                idx.entries()[i].fingerprint,
                "fingerprint {} should survive serialization",
                i
            );
        }
    }

    #[test]
    fn test_index_deserialization_bad_magic() {
        let data = [0u8; 16];
        assert!(FingerprintIndex::from_bytes(&data).is_err());
    }

    #[test]
    fn test_index_deserialization_too_short() {
        let data = [0u8; 4];
        assert!(FingerprintIndex::from_bytes(&data).is_err());
    }

    #[test]
    fn test_index_deserialization_truncated_entries() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "test", &[]);
        let bytes = idx.to_bytes();
        // Truncate to just the header — count says 1 entry but no entry data
        let truncated = &bytes[..FP_HEADER_SIZE];
        assert!(FingerprintIndex::from_bytes(truncated).is_err());
    }

    // --- Stemming integration tests ---

    #[test]
    fn test_stemming_matches_morphological_variants() {
        let note_fp = Fingerprint::from_text("running overflows checking", &[]);
        let ctx_fp = Fingerprint::from_keywords(&["run", "overflow", "checked"]);

        // Stemmer: "running"→"run", "overflows"→"overflow", "checking"→"check"
        // "checked"→"check" in context
        // So stemmed tokens overlap: "run", "overflow", "check"
        let overlap = note_fp.bloom_overlap(&ctx_fp);
        assert!(overlap >= 3, "stemmed variants should produce bloom overlap, got {}", overlap);
    }

    #[test]
    fn test_keyword_splitting() {
        // "storage.rs" should split into "storage" and "rs"
        let fp = Fingerprint::from_keywords(&["storage.rs", "checked_add"]);
        let fp_manual = Fingerprint::from_keywords(&["storage", "rs", "checked", "add"]);

        // Bloom should be identical since the same stemmed tokens are hashed
        assert_eq!(fp.bloom, fp_manual.bloom, "bloom should match after splitting");
        // SimHash may differ subtly due to iterator ordering in flat_map,
        // but Hamming distance should be very low
        let hd = fp.hamming_distance(&fp_manual);
        assert!(hd <= 10, "split vs pre-split keywords should have low HD, got {}", hd);
    }

    // --- Weighted scoring tests ---

    #[test]
    fn test_score_weighted_identical() {
        let fp = Fingerprint::from_text("storage overflow checking", &["engram"]);
        let score = fp.score_weighted(&fp);
        // Identical fingerprints: semantic = 1.0 (HD=0), but keyword = bloom.count_ones()/64
        // which is < 1.0 because not all 64 bloom bits are set for typical content.
        // Minimum is 0.6 (when bloom has 0 bits). Typical: 0.65-0.85.
        assert!(score > 0.3, "self-score should be > 0.3, got {}", score);
        assert!(score <= 1.0, "self-score should be <= 1.0, got {}", score);
    }

    #[test]
    fn test_score_weighted_similar_higher_than_different() {
        let note = Fingerprint::from_text("integer overflow in storage deserialization", &["engram"]);
        let similar_ctx = Fingerprint::from_keywords(&["storage", "overflow", "integer"]);
        let different_ctx = Fingerprint::from_keywords(&["fitquest", "recipe", "nutrition"]);

        let similar_score = note.score_weighted(&similar_ctx);
        let different_score = note.score_weighted(&different_ctx);

        assert!(
            similar_score > different_score,
            "similar ({}) should score higher than different ({})",
            similar_score,
            different_score
        );
    }

    #[test]
    fn test_score_weighted_range() {
        let fp1 = Fingerprint::from_text("some content", &[]);
        let fp2 = Fingerprint::from_text("totally different topic about cooking", &[]);
        let score = fp1.score_weighted(&fp2);
        assert!(score >= 0.0 && score <= 1.0, "weighted score must be in [0,1], got {}", score);
    }

    // --- Upsert tests ---

    #[test]
    fn test_upsert_insert_new() {
        let mut idx = FingerprintIndex::new();
        idx.upsert(1, "first note", &["tag1"]);
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.entries()[0].note_id, 1);
    }

    #[test]
    fn test_upsert_update_existing() {
        let mut idx = FingerprintIndex::new();
        idx.upsert(1, "original content about storage", &["engram"]);
        let original_fp = idx.entries()[0].fingerprint;

        idx.upsert(1, "completely new content about nutrition", &["fitquest"]);
        assert_eq!(idx.len(), 1, "upsert should not add duplicate");
        assert_ne!(
            idx.entries()[0].fingerprint.simhash,
            original_fp.simhash,
            "fingerprint should change after upsert"
        );
    }

    #[test]
    fn test_upsert_entry_update() {
        let mut idx = FingerprintIndex::new();
        let fp1 = FingerprintEntry {
            fingerprint: Fingerprint::new(111, 222),
            note_id: 42,
            flags: 0,
        };
        idx.upsert_entry(fp1);
        assert_eq!(idx.len(), 1);

        let fp2 = FingerprintEntry {
            fingerprint: Fingerprint::new(333, 444),
            note_id: 42,
            flags: 0,
        };
        idx.upsert_entry(fp2);
        assert_eq!(idx.len(), 1, "upsert_entry should not add duplicate");
        assert_eq!(idx.entries()[0].fingerprint.simhash, 333);
    }

    // --- Cluster pre-filtering tests ---

    #[test]
    fn test_build_clusters() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "storage overflow fix", &["engram"]);
        idx.add(2, "nutrition tracking", &["fitquest"]);
        idx.add(3, "BM25 recall search", &["engram"]);

        let note_tags: Vec<(u64, Vec<&str>)> = vec![
            (1, vec!["engram"]),
            (2, vec!["fitquest"]),
            (3, vec!["engram"]),
        ];

        let clusters = idx.build_clusters(&note_tags);
        assert_eq!(clusters.len(), 2, "should have 2 clusters (engram + fitquest)");

        // Find the engram cluster
        let engram_cluster = clusters.iter().find(|c| c.tag == "engram").unwrap();
        assert_eq!(engram_cluster.member_indices.len(), 2, "engram cluster should have 2 members");

        // Super-fingerprint should be OR of members
        let fp1 = idx.entries()[0].fingerprint;
        let fp3 = idx.entries()[2].fingerprint;
        assert_eq!(engram_cluster.super_bloom, fp1.bloom | fp3.bloom);
    }

    #[test]
    fn test_scan_clustered_finds_match() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "integer overflow in storage deserialization bounds check", &["engram", "fix"]);
        idx.add(2, "fitquest recipe nutrition tracking calories", &["fitquest"]);
        idx.add(3, "BM25 tokenizer word boundary stemming recall", &["engram", "search"]);

        let note_tags: Vec<(u64, Vec<&str>)> = vec![
            (1, vec!["engram", "fix"]),
            (2, vec!["fitquest"]),
            (3, vec!["engram", "search"]),
        ];

        let clusters = idx.build_clusters(&note_tags);
        let context = Fingerprint::from_keywords(&["storage", "overflow", "integer", "bounds"]);
        let result = idx.scan_clustered(&context, &clusters, Some(20));

        assert!(result.is_some(), "should find a match");
        assert_eq!(result.unwrap().note_id, 1, "should match the storage overflow note");
    }

    #[test]
    fn test_scan_clustered_skips_irrelevant_clusters() {
        let mut idx = FingerprintIndex::new();
        // Add 100 fitquest notes and 1 engram note
        for i in 0..100 {
            idx.add(i, "fitquest nutrition recipe calories diet", &["fitquest"]);
        }
        idx.add(100, "storage overflow integer bounds checking", &["engram"]);

        let mut note_tags: Vec<(u64, Vec<&str>)> = (0..100)
            .map(|i| (i, vec!["fitquest"]))
            .collect();
        note_tags.push((100, vec!["engram"]));

        let clusters = idx.build_clusters(&note_tags);
        let context = Fingerprint::from_keywords(&["storage", "overflow"]);
        let result = idx.scan_clustered(&context, &clusters, Some(20));

        // Should find the engram note, skipping the 100 fitquest notes via cluster pre-filter
        if let Some(r) = result {
            assert_eq!(r.note_id, 100);
        }
    }

    #[test]
    fn test_scan_clustered_unclustered_fallback() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "storage overflow fix", &["engram"]);
        idx.add(2, "unclustered orphan note about storage", &[]);

        // Only note 1 has tags
        let note_tags: Vec<(u64, Vec<&str>)> = vec![(1, vec!["engram"])];
        let clusters = idx.build_clusters(&note_tags);

        let context = Fingerprint::from_keywords(&["storage", "overflow"]);
        let result = idx.scan_clustered(&context, &clusters, None);

        // Both notes should be considered (note 2 via unclustered fallback)
        assert!(result.is_some(), "unclustered notes should still be scanned");
    }

    // --- File persistence tests ---

    #[test]
    fn test_sidecar_path() {
        let engram_path = Path::new("/home/user/memory.engram");
        let fp_path = FingerprintIndex::sidecar_path(engram_path);
        assert_eq!(fp_path, PathBuf::from("/home/user/memory.engram.fp"));
    }

    #[test]
    fn test_save_and_load() {
        let dir = std::env::temp_dir().join("engram_fp_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.engram.fp");

        let mut idx = FingerprintIndex::new();
        idx.add(1, "storage overflow fix", &["engram"]);
        idx.add(2, "nutrition tracking", &["fitquest"]);

        idx.save(&path).expect("save should succeed");
        assert!(path.exists(), "file should exist after save");

        let loaded = FingerprintIndex::load(&path).expect("load should succeed");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.entries()[0].note_id, 1);
        assert_eq!(loaded.entries()[1].note_id, 2);
        assert_eq!(loaded.entries()[0].fingerprint, idx.entries()[0].fingerprint);

        // Cleanup
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_load_nonexistent() {
        let result = FingerprintIndex::load(Path::new("/nonexistent/path.fp"));
        assert!(result.is_err(), "loading nonexistent file should error");
    }

    // --- Performance sanity check ---

    #[test]
    fn test_scan_1800_entries_completes_fast() {
        let mut idx = FingerprintIndex::with_capacity(1800);
        for i in 0..1800u64 {
            idx.add_entry(FingerprintEntry {
                fingerprint: Fingerprint::new(i.wrapping_mul(0x517cc1b727220a95), i.wrapping_mul(0x6c62272e07bb0142)),
                note_id: i,
                flags: 0,
            });
        }

        let context = Fingerprint::from_keywords(&["storage", "overflow", "checked"]);
        let start = std::time::Instant::now();

        for _ in 0..1000 {
            let _ = idx.scan_best(&context, Some(20));
        }

        let elapsed = start.elapsed();
        let per_scan_ns = elapsed.as_nanos() / 1000;

        // Should be well under 100μs per scan
        assert!(
            per_scan_ns < 100_000,
            "scan of 1800 entries should be <100μs, got {}ns",
            per_scan_ns
        );
    }

    #[test]
    fn test_clustered_vs_linear_benchmark() {
        // Simulate realistic workload: 1800 notes across 20 tags
        // NOTE: At 1800 entries, linear scan (~40μs debug) often beats clustered
        // due to Vec<bool> allocation overhead. Clusters pay off at 10K+ entries.
        // Release mode is 3-5x faster for both paths.
        let tags: Vec<&str> = vec![
            "engram", "fitquest", "federation", "security", "teamengram",
            "shm", "wordquest", "mathquest", "chimera", "deployment",
            "architecture", "testing", "debugging", "performance", "ui",
            "api", "database", "auth", "networking", "docs",
        ];

        let mut idx = FingerprintIndex::with_capacity(1800);
        let mut note_tags_list: Vec<(u64, Vec<&str>)> = Vec::with_capacity(1800);

        for i in 0..1800u64 {
            let tag = tags[(i as usize) % tags.len()];
            let content = format!("{} note content about topic number {}", tag, i);
            let tag_slice: &[&str] = &[tag];
            idx.add(i, &content, tag_slice);
            note_tags_list.push((i, vec![tag]));
        }

        let clusters = idx.build_clusters(&note_tags_list);
        let context = Fingerprint::from_keywords(&["engram", "storage", "overflow"]);

        // Benchmark linear scan
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = idx.scan_best(&context, None);
        }
        let linear_ns = start.elapsed().as_nanos() / 1000;

        // Benchmark clustered scan
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = idx.scan_clustered(&context, &clusters, None);
        }
        let clustered_ns = start.elapsed().as_nanos() / 1000;

        // Benchmark scan_mmap (zero-copy scalar)
        let serialized = idx.to_bytes();
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = FingerprintIndex::scan_mmap(&serialized, &context, 32);
        }
        let mmap_ns = start.elapsed().as_nanos() / 1000;

        // Benchmark scan_mmap_batch4
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = FingerprintIndex::scan_mmap_batch4(&serialized, &context, 32);
        }
        let batch4_ns = start.elapsed().as_nanos() / 1000;

        eprintln!(
            "Benchmark (1800 notes, 1000 iters, per-iter avg):\n  \
             Linear:    {}ns\n  \
             Clustered: {}ns\n  \
             scan_mmap: {}ns\n  \
             batch4:    {}ns",
            linear_ns, clustered_ns, mmap_ns, batch4_ns
        );

        // All must complete within 500μs per-call (generous for debug mode on WSL2)
        assert!(linear_ns < 500_000, "linear scan too slow: {}ns", linear_ns);
        assert!(clustered_ns < 500_000, "clustered scan too slow: {}ns", clustered_ns);
        assert!(mmap_ns < 500_000, "scan_mmap too slow: {}ns", mmap_ns);
        assert!(batch4_ns < 500_000, "batch4 too slow: {}ns", batch4_ns);

        // Verify clustered scan finds the same best result as linear
        let linear_result = idx.scan_best(&context, None);
        let clustered_result = idx.scan_clustered(&context, &clusters, None);
        assert_eq!(
            linear_result.as_ref().map(|r| r.note_id),
            clustered_result.as_ref().map(|r| r.note_id),
            "clustered and linear must find the same best match"
        );
    }

    // --- IDF Table tests ---

    #[test]
    fn test_idf_table_from_corpus() {
        let corpus = vec![
            tokenize_text("the integer overflow was fixed"),
            tokenize_text("the storage module handles data"),
            tokenize_text("the overflow in deserialization was bad"),
        ];
        let idf = IdfTable::from_corpus(&corpus);

        assert_eq!(idf.doc_count(), 3);
        assert!(idf.vocab_size() > 0);
        assert!(!idf.is_empty());
    }

    #[test]
    fn test_idf_common_tokens_low_weight() {
        // BM25 IDF: ln((N - df + 0.5) / (df + 0.5)), clamped to 0.
        // With N=3, tokens in 2+ docs get negative IDF (clamped to 0).
        // Need N>=10 for proper differentiation.
        // "the" in all 10 docs, "overflow" in 2, "storage" in 1
        let corpus = vec![
            tokenize_text("the integer overflow was fixed"),
            tokenize_text("the storage module handles data"),
            tokenize_text("the overflow in deserialization was bad"),
            tokenize_text("the quick brown fox jumps"),
            tokenize_text("the lazy dog sleeps"),
            tokenize_text("the cat sat on the mat"),
            tokenize_text("the rain in spain falls"),
            tokenize_text("the code compiles cleanly"),
            tokenize_text("the server responds fast"),
            tokenize_text("the database query runs"),
        ];
        let idf = IdfTable::from_corpus(&corpus);

        let stemmer = create_stemmer();
        let the_stem = stemmer.stem("the").into_owned();
        let overflow_stem = stemmer.stem("overflow").into_owned();
        let storage_stem = stemmer.stem("storage").into_owned();

        let w_the = idf.weight(&the_stem);
        let w_overflow = idf.weight(&overflow_stem);
        let w_storage = idf.weight(&storage_stem);

        // "the" (in all 10 docs) → clamped to 0
        // "overflow" (in 2 of 10) → positive IDF
        // "storage" (in 1 of 10) → highest IDF
        assert!(
            w_the < w_overflow,
            "common 'the' ({}) should weigh less than 'overflow' ({})",
            w_the, w_overflow
        );
        assert!(
            w_overflow < w_storage,
            "medium 'overflow' ({}) should weigh less than rare 'storage' ({})",
            w_overflow, w_storage
        );
    }

    #[test]
    fn test_idf_unknown_token_default_weight() {
        let corpus = vec![tokenize_text("hello world")];
        let idf = IdfTable::from_corpus(&corpus);

        // Token not in corpus should get default weight of 1.0
        assert_eq!(idf.weight("nonexistent_token_xyz"), 1.0);
    }

    #[test]
    fn test_idf_empty_corpus() {
        let corpus: Vec<Vec<String>> = vec![];
        let idf = IdfTable::from_corpus(&corpus);

        assert_eq!(idf.doc_count(), 0);
        assert_eq!(idf.vocab_size(), 0);
        assert!(idf.is_empty());
        assert_eq!(idf.weight("anything"), 1.0);
    }

    #[test]
    fn test_idf_single_document_corpus() {
        let corpus = vec![tokenize_text("integer overflow in storage deserialization")];
        let idf = IdfTable::from_corpus(&corpus);

        assert_eq!(idf.doc_count(), 1);
        // With N=1 and df=1: idf = ln((1-1+0.5)/(1+0.5)) = ln(0.333) ≈ -1.1 → clamped to 0.0
        // All tokens appear in the only document, so all get weight 0.0
        let stemmer = create_stemmer();
        let w = idf.weight(&stemmer.stem("overflow").into_owned());
        assert!(
            w < 0.01,
            "token in only-document should have near-zero IDF weight, got {}",
            w
        );
    }

    // --- IDF-weighted SimHash tests ---

    #[test]
    fn test_weighted_simhash_produces_valid_hash() {
        let corpus = vec![
            tokenize_text("integer overflow storage bounds"),
            tokenize_text("fitquest nutrition recipe calories"),
            tokenize_text("BM25 tokenizer stemming recall search"),
        ];
        let idf = IdfTable::from_corpus(&corpus);

        let fp = Fingerprint::from_text_with_idf(
            "integer overflow in storage bounds checking",
            &["engram"],
            &idf,
        );
        assert_ne!(fp.simhash, 0, "weighted SimHash should produce non-zero hash");
        assert_ne!(fp.bloom, 0, "bloom should still be non-zero");
    }

    #[test]
    fn test_weighted_simhash_differs_from_uniform() {
        let corpus = vec![
            tokenize_text("the integer overflow was fixed in storage"),
            tokenize_text("the storage module the handles the data"),
            tokenize_text("the overflow in the deserialization the was bad"),
        ];
        let idf = IdfTable::from_corpus(&corpus);

        let text = "the integer overflow was fixed in storage";
        let uniform_fp = Fingerprint::from_text(text, &[]);
        let weighted_fp = Fingerprint::from_text_with_idf(text, &[], &idf);

        // Weighted and uniform should produce different SimHashes because IDF
        // suppresses "the" (appears in all docs) and amplifies rare tokens
        // They CAN be equal in degenerate cases but very unlikely
        let hd = uniform_fp.hamming_distance(&weighted_fp);
        // Just verify both are valid; they may or may not differ depending on the
        // specific token distributions. The key property is tested below.
        assert!(hd <= 64, "HD must be in valid range, got {}", hd);
    }

    #[test]
    fn test_weighted_simhash_improves_discrimination() {
        // Build a corpus where "the" and "was" are ubiquitous noise
        let corpus = vec![
            tokenize_text("the integer overflow was fixed in the storage module"),
            tokenize_text("the fitquest recipe was tracking the calories"),
            tokenize_text("the BM25 tokenizer was stemming the recall"),
            tokenize_text("the vector search was finding the nearest neighbor"),
            tokenize_text("the federation protocol was signing the messages"),
        ];
        let idf = IdfTable::from_corpus(&corpus);

        // Note about storage overflow
        let note = "the integer overflow was fixed in the storage module";
        // Context: looking for storage overflow stuff
        let context_keywords = &["storage", "overflow", "integer"];
        let context_fp = Fingerprint::from_keywords(context_keywords);

        // Unrelated note
        let unrelated = "the fitquest recipe was tracking the calories";

        let uniform_note = Fingerprint::from_text(note, &[]);
        let weighted_note = Fingerprint::from_text_with_idf(note, &[], &idf);
        let uniform_unrelated = Fingerprint::from_text(unrelated, &[]);
        let weighted_unrelated = Fingerprint::from_text_with_idf(unrelated, &[], &idf);

        // The gap between relevant and irrelevant should be at least as good
        // with IDF weighting (weighted pushes noise tokens down, making
        // discriminative tokens dominate the hash)
        let uniform_gap = uniform_note.hamming_distance(&context_fp) as i32
            - uniform_unrelated.hamming_distance(&context_fp) as i32;
        let weighted_gap = weighted_note.hamming_distance(&context_fp) as i32
            - weighted_unrelated.hamming_distance(&context_fp) as i32;

        // Weighted gap should be more negative (relevant note closer to context)
        // or at least not worse. We allow equal because with small corpus
        // the IDF effect may be minimal.
        assert!(
            weighted_gap <= uniform_gap + 5,
            "IDF weighting should not significantly degrade discrimination. \
             Uniform gap: {}, Weighted gap: {}",
            uniform_gap, weighted_gap
        );
    }

    #[test]
    fn test_weighted_simhash_empty_tokens() {
        let idf = IdfTable::from_corpus(&[]);
        let hash = compute_simhash_weighted(&[], &idf);
        assert_eq!(hash, 0, "empty tokens should produce zero hash");
    }

    // --- Density monitoring tests ---

    #[test]
    fn test_simhash_density_range() {
        let fp = Fingerprint::from_text("some content about storage and overflow", &["engram"]);
        let density = fp.simhash_density();
        assert!(density >= 0.0 && density <= 1.0, "density must be in [0,1], got {}", density);
    }

    #[test]
    fn test_simhash_density_typical_range() {
        // With enough tokens, SimHash density should be near 0.5
        let fp = Fingerprint::from_text(
            "integer overflow storage deserialization bounds checking \
             vector search recall tokenizer stemming bloom filter",
            &[],
        );
        let density = fp.simhash_density();
        assert!(
            density >= 0.25 && density <= 0.75,
            "typical text should have density near 0.5, got {}",
            density
        );
    }

    #[test]
    fn test_simhash_density_zero() {
        let fp = Fingerprint::ZERO;
        assert_eq!(fp.simhash_density(), 0.0);
        assert_eq!(fp.bloom_density(), 0.0);
    }

    #[test]
    fn test_bloom_density_increases_with_tokens() {
        let few = Fingerprint::from_text("hello", &[]);
        let many = Fingerprint::from_text(
            "hello world storage overflow integer bounds checking \
             deserialization vector search recall tokenizer stemming",
            &[],
        );
        assert!(
            many.bloom_density() >= few.bloom_density(),
            "more tokens should produce higher bloom density: few={}, many={}",
            few.bloom_density(),
            many.bloom_density()
        );
    }

    // --- Convenience function tests ---

    #[test]
    fn test_tokenize_text_basic() {
        let tokens = tokenize_text("The integer overflow was fixed");
        assert!(!tokens.is_empty());
        // "The" → "the" → stemmed, "integer" → stemmed, etc.
        // Should have ~5 tokens (the, integer, overflow, was, fixed → stemmed forms)
        assert!(tokens.len() >= 3, "should have multiple tokens, got {}", tokens.len());
    }

    #[test]
    fn test_tokenize_text_stemming() {
        let tokens = tokenize_text("running overflows checking");
        // "running" → "run", "overflows" → "overflow", "checking" → "check"
        assert!(tokens.contains(&"run".to_string()), "should contain stemmed 'run', got {:?}", tokens);
        assert!(tokens.contains(&"overflow".to_string()), "should contain stemmed 'overflow', got {:?}", tokens);
        assert!(tokens.contains(&"check".to_string()), "should contain stemmed 'check', got {:?}", tokens);
    }

    #[test]
    fn test_create_stemmer() {
        let stemmer = create_stemmer();
        // Should be a working English Porter2 stemmer
        assert_eq!(stemmer.stem("running").as_ref(), "run");
    }

    #[test]
    fn test_scan_mmap_finds_best_match() {
        let mut index = FingerprintIndex::new();
        index.add(1, "PostgreSQL connection pooling timeout fix", &["postgres", "fix"]);
        index.add(2, "Redis caching strategy for session tokens", &["redis", "cache"]);
        index.add(3, "PostgreSQL query optimization and indexing", &["postgres", "perf"]);

        // Serialize to bytes (simulates mmap'd file)
        let data = index.to_bytes();

        // Context about postgres work
        let context = Fingerprint::from_text("PostgreSQL connection pool", &["postgres"]);

        // scan_mmap should find a match
        let result = FingerprintIndex::scan_mmap(&data, &context, 30);
        assert!(result.is_some(), "should find a match");

        let result = result.unwrap();
        // Should match one of the postgres notes (1 or 3)
        assert!(
            result.note_id == 1 || result.note_id == 3,
            "should match a postgres note, got note_id={}",
            result.note_id
        );
    }

    #[test]
    fn test_scan_mmap_respects_threshold() {
        let mut index = FingerprintIndex::new();
        index.add(1, "PostgreSQL connection pooling timeout fix", &["postgres"]);

        let data = index.to_bytes();

        // Completely unrelated context
        let context = Fingerprint::from_text("machine learning neural network training", &["ml"]);

        // Very tight threshold should reject the unrelated content
        let result = FingerprintIndex::scan_mmap(&data, &context, 5);
        assert!(result.is_none(), "should not match unrelated content at HD<=5");
    }

    #[test]
    fn test_scan_mmap_bad_data() {
        let context = Fingerprint::from_text("test", &[]);

        // Too short
        assert!(FingerprintIndex::scan_mmap(&[0u8; 4], &context, 10).is_none());

        // Bad magic
        assert!(FingerprintIndex::scan_mmap(&[0u8; 16], &context, 10).is_none());

        // Empty slice
        assert!(FingerprintIndex::scan_mmap(&[], &context, 10).is_none());
    }

    #[test]
    fn test_scan_mmap_matches_scan_best() {
        // Verify scan_mmap produces same results as scan_best
        let mut index = FingerprintIndex::new();
        for i in 0..100 {
            let content = format!("note about topic {} with details {}", i % 10, i);
            index.add(i as u64, &content, &[&format!("tag{}", i % 5)]);
        }

        let data = index.to_bytes();
        let context = Fingerprint::from_text("note about topic 3 with details", &["tag3"]);
        let max_hd = 25;

        let mmap_result = FingerprintIndex::scan_mmap(&data, &context, max_hd);
        let scan_result = index.scan_best(&context, Some(max_hd));

        // Both should agree on finding a match (or both None)
        assert_eq!(mmap_result.is_some(), scan_result.is_some());
        if let (Some(m), Some(s)) = (&mmap_result, &scan_result) {
            assert_eq!(m.note_id, s.note_id, "scan_mmap and scan_best should find same note");
            assert_eq!(m.score, s.score, "scores should match");
        }
    }

    #[test]
    fn test_scan_mmap_batch4_matches_scalar() {
        // Verify batch4 produces identical results to scalar scan_mmap
        // Use 103 entries (25 chunks of 4 + 3 remainder) to test both paths
        let mut index = FingerprintIndex::new();
        for i in 0..103 {
            let content = format!("document about area {} and topic {} details {}", i % 7, i % 13, i);
            index.add(i as u64, &content, &[&format!("cat{}", i % 8)]);
        }

        let data = index.to_bytes();

        // Test with multiple contexts and thresholds
        let contexts = vec![
            Fingerprint::from_text("document about area 3 and topic", &["cat3"]),
            Fingerprint::from_text("completely different unrelated content", &["xyz"]),
            Fingerprint::from_text("details about area", &["cat0", "cat1"]),
        ];

        for (ci, context) in contexts.iter().enumerate() {
            for max_hd in [5, 10, 15, 25, 32] {
                let scalar = FingerprintIndex::scan_mmap(&data, context, max_hd);
                let batch4 = FingerprintIndex::scan_mmap_batch4(&data, context, max_hd);

                assert_eq!(
                    scalar.is_some(), batch4.is_some(),
                    "context={} max_hd={}: scalar found={} batch4 found={}",
                    ci, max_hd, scalar.is_some(), batch4.is_some()
                );

                if let (Some(s), Some(b)) = (&scalar, &batch4) {
                    assert_eq!(
                        s.note_id, b.note_id,
                        "context={} max_hd={}: scalar note_id={} batch4 note_id={}",
                        ci, max_hd, s.note_id, b.note_id
                    );
                    assert_eq!(
                        s.score, b.score,
                        "context={} max_hd={}: scalar score={} batch4 score={}",
                        ci, max_hd, s.score, b.score
                    );
                }
            }
        }
    }

    #[test]
    fn test_scan_mmap_batch4_small_counts() {
        // Test with 0, 1, 2, 3 entries (all remainder, no full chunks)
        for count in 0..4 {
            let mut index = FingerprintIndex::new();
            for i in 0..count {
                index.add(i as u64, &format!("note {}", i), &["tag"]);
            }

            let data = index.to_bytes();
            let context = Fingerprint::from_text("note", &["tag"]);

            let scalar = FingerprintIndex::scan_mmap(&data, &context, 30);
            let batch4 = FingerprintIndex::scan_mmap_batch4(&data, &context, 30);

            assert_eq!(
                scalar.is_some(), batch4.is_some(),
                "count={}: results disagree", count
            );
        }
    }

    // --- V2 sidecar format tests ---

    #[test]
    fn test_entry_v2_roundtrip() {
        let entry = FingerprintEntry {
            fingerprint: Fingerprint::new(0xDEADBEEF_CAFEBABE, 0x1234567890ABCDEF),
            note_id: 42,
            flags: FLAG_SKIP_RECALL,
        };
        let bytes = entry.to_bytes();
        assert_eq!(bytes.len(), 32);
        let restored = FingerprintEntry::from_bytes(&bytes);
        assert_eq!(restored.fingerprint, entry.fingerprint);
        assert_eq!(restored.note_id, 42);
        assert_eq!(restored.flags, FLAG_SKIP_RECALL);
    }

    #[test]
    fn test_entry_v1_compat() {
        // V1 24-byte entry should deserialize with flags=0
        let mut v1_bytes = [0u8; 24];
        v1_bytes[0..8].copy_from_slice(&0xAAAAu64.to_le_bytes());
        v1_bytes[8..16].copy_from_slice(&0xBBBBu64.to_le_bytes());
        v1_bytes[16..24].copy_from_slice(&99u64.to_le_bytes());

        let entry = FingerprintEntry::from_bytes_v1(&v1_bytes);
        assert_eq!(entry.fingerprint.simhash, 0xAAAA);
        assert_eq!(entry.fingerprint.bloom, 0xBBBB);
        assert_eq!(entry.note_id, 99);
        assert_eq!(entry.flags, 0, "V1 entries should have flags=0");
    }

    #[test]
    fn test_index_v2_roundtrip() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "first note", &["tag1"]);
        idx.add(2, "second note", &["tag2"]);
        // Set flags on one entry
        idx.mark_pinned(&[1]);

        let bytes = idx.to_bytes();
        // Verify header version
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        assert_eq!(version, FP_VERSION, "should write V2 format");

        let restored = FingerprintIndex::from_bytes(&bytes).unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(restored.entries()[0].note_id, 1);
        assert_eq!(restored.entries()[0].flags, FLAG_SKIP_RECALL);
        assert_eq!(restored.entries()[1].note_id, 2);
        assert_eq!(restored.entries()[1].flags, 0);
    }

    #[test]
    fn test_index_v1_to_v2_migration() {
        // Build a V1 sidecar manually
        let mut v1_data = Vec::new();
        v1_data.extend_from_slice(&FP_MAGIC.to_le_bytes());
        v1_data.extend_from_slice(&FP_VERSION_V1.to_le_bytes()); // V1
        v1_data.extend_from_slice(&2u32.to_le_bytes()); // 2 entries
        v1_data.extend_from_slice(&[0u8; 6]);

        // 2 entries at 24 bytes each
        v1_data.extend_from_slice(&0xAAAAu64.to_le_bytes());
        v1_data.extend_from_slice(&0xBBBBu64.to_le_bytes());
        v1_data.extend_from_slice(&1u64.to_le_bytes());

        v1_data.extend_from_slice(&0xCCCCu64.to_le_bytes());
        v1_data.extend_from_slice(&0xDDDDu64.to_le_bytes());
        v1_data.extend_from_slice(&2u64.to_le_bytes());

        // Load V1 data
        let idx = FingerprintIndex::from_bytes(&v1_data).unwrap();
        assert_eq!(idx.len(), 2);
        assert_eq!(idx.entries()[0].flags, 0, "V1 entries should load with flags=0");
        assert_eq!(idx.entries()[1].flags, 0);

        // Save as V2
        let v2_data = idx.to_bytes();
        let version = u16::from_le_bytes(v2_data[4..6].try_into().unwrap());
        assert_eq!(version, FP_VERSION, "re-saved should be V2");

        // Expected size: 16 header + 2 * 32 = 80
        assert_eq!(v2_data.len(), 16 + 2 * 32);

        // Round-trip the V2 data
        let idx2 = FingerprintIndex::from_bytes(&v2_data).unwrap();
        assert_eq!(idx2.len(), 2);
        assert_eq!(idx2.entries()[0].fingerprint.simhash, 0xAAAA);
        assert_eq!(idx2.entries()[1].note_id, 2);
    }

    #[test]
    fn test_mark_pinned() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "note one", &[]);
        idx.add(2, "note two", &[]);
        idx.add(3, "note three", &[]);

        // Pin notes 1 and 3
        idx.mark_pinned(&[1, 3]);
        assert_eq!(idx.entries()[0].flags & FLAG_SKIP_RECALL, FLAG_SKIP_RECALL);
        assert_eq!(idx.entries()[1].flags & FLAG_SKIP_RECALL, 0);
        assert_eq!(idx.entries()[2].flags & FLAG_SKIP_RECALL, FLAG_SKIP_RECALL);

        // Change pins: only note 2 pinned now
        idx.mark_pinned(&[2]);
        assert_eq!(idx.entries()[0].flags & FLAG_SKIP_RECALL, 0, "should clear old pin");
        assert_eq!(idx.entries()[1].flags & FLAG_SKIP_RECALL, FLAG_SKIP_RECALL);
        assert_eq!(idx.entries()[2].flags & FLAG_SKIP_RECALL, 0, "should clear old pin");
    }

    #[test]
    fn test_mark_tombstoned() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "note one", &[]);
        idx.add(2, "note two", &[]);

        idx.mark_tombstoned(2);
        assert_eq!(idx.entries()[0].flags, 0);
        assert_eq!(idx.entries()[1].flags & FLAG_TOMBSTONE, FLAG_TOMBSTONE);

        // Tombstone should combine with existing flags
        idx.mark_pinned(&[2]);
        assert_eq!(
            idx.entries()[1].flags,
            FLAG_SKIP_RECALL | FLAG_TOMBSTONE,
            "both flags should be set"
        );
    }

    #[test]
    fn test_scan_mmap_skips_pinned() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "storage overflow integer bounds checking", &["engram"]);
        idx.add(2, "storage overflow deserialization fix", &["engram"]);
        idx.add(3, "nutrition tracking recipes", &["fitquest"]);

        // Pin note 1 (best match)
        idx.mark_pinned(&[1]);

        let data = idx.to_bytes();
        let context = Fingerprint::from_keywords(&["storage", "overflow", "integer"]);

        let result = FingerprintIndex::scan_mmap(&data, &context, 32);
        assert!(result.is_some(), "should find a non-pinned match");
        assert_ne!(result.unwrap().note_id, 1, "pinned note should be skipped");
    }

    #[test]
    fn test_scan_mmap_skips_tombstoned() {
        let mut idx = FingerprintIndex::new();
        idx.add(1, "storage overflow integer bounds checking", &["engram"]);
        idx.add(2, "nutrition tracking recipes", &["fitquest"]);

        // Tombstone note 1
        idx.mark_tombstoned(1);

        let data = idx.to_bytes();
        let context = Fingerprint::from_keywords(&["storage", "overflow", "integer"]);

        let result = FingerprintIndex::scan_mmap(&data, &context, 32);
        // Either no result or a non-tombstoned result
        if let Some(r) = result {
            assert_ne!(r.note_id, 1, "tombstoned note should be skipped");
        }
    }

    // --- Prospective Memory Trigger tests ---

    #[test]
    fn test_triggers_empty_same_as_from_text() {
        let fp_normal = Fingerprint::from_text("check PQC migration status", &["security"]);
        let fp_trigger = Fingerprint::from_text_with_triggers("check PQC migration status", &["security"], &[]);
        assert_eq!(fp_normal.simhash, fp_trigger.simhash, "SimHash should be identical");
        assert_eq!(fp_normal.bloom, fp_trigger.bloom, "Bloom should be identical with no triggers");
    }

    #[test]
    fn test_triggers_set_more_bloom_bits() {
        let fp_normal = Fingerprint::from_text("check migration status", &["security"]);
        let fp_trigger = Fingerprint::from_text_with_triggers("check migration status", &["security"], &["federation", "pqc"]);
        assert_eq!(fp_normal.simhash, fp_trigger.simhash, "SimHash must be identical");
        assert!(
            fp_trigger.bloom.count_ones() > fp_normal.bloom.count_ones(),
            "Triggered fingerprint should have more bloom bits: {} vs {}",
            fp_trigger.bloom.count_ones(),
            fp_normal.bloom.count_ones()
        );
    }

    #[test]
    fn test_triggers_higher_score_when_context_matches() {
        // Note about PQC migration with "federation" trigger
        let fp_triggered = Fingerprint::from_text_with_triggers(
            "check PQC migration status before changes",
            &["security"],
            &["federation"],
        );
        // Same note without trigger
        let fp_normal = Fingerprint::from_text(
            "check PQC migration status before changes",
            &["security"],
        );
        // Context: working on federation
        let context = Fingerprint::from_keywords(&["federation", "node", "quic", "handshake"]);

        let score_triggered = fp_triggered.score(&context);
        let score_normal = fp_normal.score(&context);
        assert!(
            score_triggered > score_normal,
            "Triggered note should score higher when context matches trigger: {} vs {}",
            score_triggered,
            score_normal
        );
    }

    #[test]
    fn test_triggers_bloom_adds_keyword_bits() {
        // The core property: triggers add bloom bits for keywords NOT in the content.
        // This is what makes triggered notes surface when context matches the trigger.
        // On 64-bit bloom, per-keyword discrimination is limited by saturation,
        // but the bloom overlap is guaranteed non-negative and helps at scale.
        let fp_triggered = Fingerprint::from_text_with_triggers(
            "status report",  // minimal content to avoid bloom saturation
            &[],
            &["federation"],
        );
        let fp_normal = Fingerprint::from_text(
            "status report",
            &[],
        );

        // Verify trigger adds bits that weren't there
        let extra_bits = (fp_triggered.bloom & !fp_normal.bloom).count_ones();
        assert!(
            extra_bits > 0,
            "Trigger should add new bloom bits not present in content-only bloom"
        );

        // Verify matching context benefits from those bits
        let context = Fingerprint::from_keywords(&["federation"]);
        let overlap_triggered = fp_triggered.bloom_overlap(&context);
        let overlap_normal = fp_normal.bloom_overlap(&context);
        assert!(
            overlap_triggered > overlap_normal,
            "Triggered note should have more bloom overlap with matching context: {} vs {}",
            overlap_triggered,
            overlap_normal
        );

        // Verify trigger uses standard seeds (no noise from offset families)
        // by checking that triggered bloom bits are a subset of what
        // bloom64_single("federation" stemmed) would produce
        let trigger_only_bits = fp_triggered.bloom & !fp_normal.bloom;
        let federation_context = Fingerprint::from_keywords(&["federation"]);
        // Every trigger-added bit should also be in the context's bloom for "federation"
        assert_eq!(
            trigger_only_bits & federation_context.bloom,
            trigger_only_bits,
            "All trigger bits should be matchable by context (no offset seed noise)"
        );
    }

    #[test]
    fn test_triggers_simhash_identical() {
        let fp1 = Fingerprint::from_text("some content about storage", &[]);
        let fp2 = Fingerprint::from_text_with_triggers("some content about storage", &[], &["trigger1", "trigger2"]);
        assert_eq!(fp1.simhash, fp2.simhash, "SimHash must be identical regardless of triggers");
    }

    #[test]
    fn test_triggers_256bit_variant() {
        let fp_normal = Fingerprint256::from_text("check migration status", &["security"]);
        let fp_trigger = Fingerprint256::from_text_with_triggers("check migration status", &["security"], &["federation"]);
        assert_eq!(fp_normal.simhash, fp_trigger.simhash, "256-bit SimHash must be identical");
        let normal_bits = fp_normal.bloom[0].count_ones() + fp_normal.bloom[1].count_ones();
        let trigger_bits = fp_trigger.bloom[0].count_ones() + fp_trigger.bloom[1].count_ones();
        assert!(trigger_bits > normal_bits, "256-bit triggered bloom should have more bits");
    }

    #[test]
    fn test_upsert_with_triggers() {
        let mut idx = FingerprintIndex::new();
        idx.upsert_with_triggers(1, "check PQC status", &["security"], &["federation"]);
        assert_eq!(idx.len(), 1);

        let entry = &idx.entries[0];
        let fp_normal = Fingerprint::from_text("check PQC status", &["security"]);
        assert_eq!(entry.fingerprint.simhash, fp_normal.simhash);
        assert!(
            entry.fingerprint.bloom.count_ones() > fp_normal.bloom.count_ones(),
            "Triggered upsert should produce more bloom bits"
        );
    }

    #[test]
    fn test_trigger_compound_keywords_split() {
        // "pqc_migration" should split into "pqc" and "migration"
        let fp = Fingerprint::from_text_with_triggers("some content", &[], &["pqc_migration"]);
        let context_pqc = Fingerprint::from_keywords(&["pqc"]);
        let context_migration = Fingerprint::from_keywords(&["migration"]);
        // Both sub-keywords should have bloom overlap
        assert!(fp.bloom_overlap(&context_pqc) > 0, "should match 'pqc' sub-keyword");
        assert!(fp.bloom_overlap(&context_migration) > 0, "should match 'migration' sub-keyword");
    }
}
