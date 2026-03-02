//! Enrichment library for hook-bulletin associative recall.
//!
//! Provides keyword extraction from tool events, context fingerprint accumulation,
//! inline fingerprint scanning (no engram dependency), and recently-recalled dedup.
//!
//! # Architecture
//!
//! ```text
//! Tool Event (stdin JSON)
//!   → extract_keywords()        ~200ns  string ops
//!   → ContextAccumulator.push() ~500ns  SimHash + Bloom recompute
//!   → ContextWriter.update()    ~200ns  seqlock write to SHM
//!   → scan_fp_bytes()           ~600ns  POPCNT scan of .engram.fp
//!   → RecentlyRecalled.check()  ~10ns   dedup against last 5
//!   → format recall injection   ~50ns   string format
//! Total: ~1.5μs
//! ```
//!
//! # Usage from hook-bulletin.rs
//!
//! ```rust,ignore
//! use shm::enrichment::{extract_keywords, ContextAccumulator, scan_fp_bytes, RecentlyRecalled};
//!
//! let keywords = extract_keywords(&tool_name, &tool_input);
//! accumulator.push_keywords(&keywords);
//! let (simhash, bloom) = accumulator.fingerprint();
//! // Write to context SHM...
//! if let Some(hit) = scan_fp_bytes(&fp_data, simhash, bloom, 16) {
//!     if !recently_recalled.contains(hit.note_id) {
//!         recently_recalled.add(hit.note_id);
//!         // Inject [recall: #ID score:X] into output
//!     }
//! }
//! ```

use rust_stemmers::{Algorithm, Stemmer};
use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3;

// ============================================================================
// Keyword extraction from tool events
// ============================================================================

/// Extract search-relevant keywords from a tool call event.
///
/// Parses the tool name and input to find file paths, grep patterns,
/// command keywords, and edit content tokens. Returns lowercased,
/// split-on-punctuation tokens suitable for fingerprint computation.
///
/// # Performance
/// ~200ns for typical tool events (string splitting, no allocations beyond Vec).
pub fn extract_keywords(tool_name: &str, tool_input: &serde_json::Value) -> Vec<String> {
    let mut keywords = Vec::with_capacity(16);

    match tool_name {
        // File operations: extract path components
        "Read" | "ReadFile" | "Edit" | "EditFile" | "Write" | "WriteFile" => {
            if let Some(path) = tool_input
                .get("file_path")
                .or_else(|| tool_input.get("path"))
                .and_then(|v| v.as_str())
            {
                keywords.extend(split_path_keywords(path));
            }
            // For edits, extract tokens from old_string and new_string
            if let Some(old) = tool_input.get("old_string").and_then(|v| v.as_str()) {
                keywords.extend(extract_code_tokens(old, 10));
            }
            if let Some(new) = tool_input.get("new_string").and_then(|v| v.as_str()) {
                keywords.extend(extract_code_tokens(new, 10));
            }
        }

        // Search operations: extract pattern and path
        "Grep" | "Search" => {
            if let Some(pattern) = tool_input.get("pattern").and_then(|v| v.as_str()) {
                keywords.extend(split_to_tokens(pattern));
            }
            if let Some(path) = tool_input.get("path").and_then(|v| v.as_str()) {
                keywords.extend(split_path_keywords(path));
            }
            if let Some(glob) = tool_input.get("glob").and_then(|v| v.as_str()) {
                keywords.extend(split_to_tokens(glob));
            }
        }

        // Glob: extract pattern keywords
        "Glob" => {
            if let Some(pattern) = tool_input.get("pattern").and_then(|v| v.as_str()) {
                keywords.extend(split_to_tokens(pattern));
            }
            if let Some(path) = tool_input.get("path").and_then(|v| v.as_str()) {
                keywords.extend(split_path_keywords(path));
            }
        }

        // Shell commands: extract command keywords
        "Bash" | "Shell" | "Execute" => {
            if let Some(cmd) = tool_input.get("command").and_then(|v| v.as_str()) {
                keywords.extend(extract_command_keywords(cmd));
            }
        }

        // Agent/subagent: extract description and prompt keywords
        "Agent" => {
            if let Some(desc) = tool_input.get("description").and_then(|v| v.as_str()) {
                keywords.extend(split_to_tokens(desc));
            }
            if let Some(prompt) = tool_input.get("prompt").and_then(|v| v.as_str()) {
                // Only take first 100 chars of prompt to avoid noise
                let truncated = if prompt.len() > 100 { &prompt[..100] } else { prompt };
                keywords.extend(extract_code_tokens(truncated, 8));
            }
        }

        // Notebook operations: extract query/content
        "notebook_recall" | "notebook_remember" => {
            if let Some(query) = tool_input.get("query").and_then(|v| v.as_str()) {
                keywords.extend(split_to_tokens(query));
            }
            if let Some(content) = tool_input.get("content").and_then(|v| v.as_str()) {
                keywords.extend(extract_code_tokens(content, 10));
            }
        }

        // Default: try common field names
        _ => {
            for key in &["file_path", "path", "pattern", "query", "command", "description"] {
                if let Some(val) = tool_input.get(*key).and_then(|v| v.as_str()) {
                    keywords.extend(split_to_tokens(val));
                }
            }
        }
    }

    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    keywords.retain(|k| seen.insert(k.clone()));

    keywords
}

/// Split a file path into keyword tokens.
/// "/mnt/c/Users/.../engram/src/fingerprint.rs" → ["engram", "src", "fingerprint", "rs"]
fn split_path_keywords(path: &str) -> Vec<String> {
    path.split(|c: char| c == '/' || c == '\\' || c == '.')
        .filter(|s| !s.is_empty() && s.len() > 1)
        // Skip common path prefixes that add no signal
        .filter(|s| !matches!(*s, "mnt" | "Users" | "Desktop" | "home" | "src" | "bin" | "lib"))
        .map(|s| s.to_lowercase())
        .collect()
}

/// Split a string into tokens on non-alphanumeric boundaries.
fn split_to_tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty() && s.len() > 1)
        .map(|s| s.to_lowercase())
        .collect()
}

/// Extract meaningful tokens from code/text content. Returns at most `max` tokens.
fn extract_code_tokens(content: &str, max: usize) -> Vec<String> {
    content
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty() && s.len() > 2)
        // Skip very common code tokens
        .filter(|s| {
            !matches!(
                *s,
                "let" | "mut" | "pub" | "fn" | "use" | "mod" | "impl"
                    | "self" | "str" | "the" | "and" | "for" | "was"
                    | "with" | "that" | "this" | "from" | "into"
            )
        })
        .take(max)
        .map(|s| s.to_lowercase())
        .collect()
}

/// Extract keywords from a shell command.
/// "cargo test fingerprint --release" → ["cargo", "test", "fingerprint"]
fn extract_command_keywords(cmd: &str) -> Vec<String> {
    cmd.split_whitespace()
        .filter(|s| !s.starts_with('-') && !s.starts_with('|') && !s.starts_with('>'))
        .flat_map(|s| split_to_tokens(s))
        .filter(|s| {
            !matches!(
                s.as_str(),
                "cmd" | "exe" | "2>&1" | "dev" | "null" | "echo" | "cd"
            )
        })
        .collect()
}

// ============================================================================
// Context accumulator (keyword ring buffer → fingerprint)
// ============================================================================

/// Maximum keywords used for Bloom computation.
///
/// 64-bit Bloom with k=5: at 50 keywords (250 bit-sets), fill = 98% → bloom
/// becomes useless (every note passes). At 8 keywords (40 bit-sets), fill = 47%
/// → meaningful discrimination. SimHash uses all keywords (counter-based, no saturation).
const BLOOM_KEYWORD_CAP: usize = 8;

/// Ring buffer of recent keywords that computes a context fingerprint.
///
/// Maintains the last N keywords across tool calls. Each push recomputes
/// the SimHash + Bloom fingerprint from all buffered keywords. The fingerprint
/// represents "what the AI is currently working on" as a 128-bit value.
///
/// Serializable to/from JSON for persistence in the hook state file.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ContextAccumulator {
    /// Ring buffer of stemmed keywords (oldest at front, newest at back)
    keywords: Vec<String>,
    /// Maximum number of keywords to keep
    capacity: usize,
    /// Cached SimHash (recomputed on push)
    #[serde(default)]
    cached_simhash: u64,
    /// Cached Bloom (recomputed on push)
    #[serde(default)]
    cached_bloom: u64,
}

impl ContextAccumulator {
    /// Create a new accumulator with the given capacity.
    ///
    /// Capacity of 50 keywords (~10 tool calls × 5 keywords each) provides
    /// good coverage of the current working context without noise from
    /// older operations.
    pub fn new(capacity: usize) -> Self {
        Self {
            keywords: Vec::with_capacity(capacity),
            capacity,
            cached_simhash: 0,
            cached_bloom: 0,
        }
    }

    /// Push new keywords into the ring buffer and recompute fingerprint.
    ///
    /// If the buffer is full, oldest keywords are evicted first.
    /// Keywords are stemmed before storage for morphological matching.
    pub fn push_keywords(&mut self, keywords: &[String]) {
        if keywords.is_empty() {
            return;
        }

        let stemmer = Stemmer::create(Algorithm::English);

        for kw in keywords {
            let stemmed = stemmer.stem(kw).into_owned();
            if !stemmed.is_empty() {
                self.keywords.push(stemmed);
            }
        }

        // Trim to capacity (drop oldest)
        if self.keywords.len() > self.capacity {
            let excess = self.keywords.len() - self.capacity;
            self.keywords.drain(..excess);
        }

        // Recompute fingerprint from all keywords
        self.recompute();
    }

    /// Get the current context fingerprint (simhash, bloom).
    #[inline]
    pub fn fingerprint(&self) -> (u64, u64) {
        (self.cached_simhash, self.cached_bloom)
    }

    /// Number of keywords currently buffered.
    pub fn len(&self) -> usize {
        self.keywords.len()
    }

    /// Whether the accumulator has any keywords.
    pub fn is_empty(&self) -> bool {
        self.keywords.is_empty()
    }

    /// Get the keywords (for debugging/testing).
    pub fn keywords(&self) -> &[String] {
        &self.keywords
    }

    /// Recompute SimHash and Bloom from all buffered keywords.
    fn recompute(&mut self) {
        if self.keywords.is_empty() {
            self.cached_simhash = 0;
            self.cached_bloom = 0;
            return;
        }

        // SimHash: Charikar accumulation
        let mut counters = [0i32; 64];
        for token in &self.keywords {
            let h = xxh3::xxh3_64(token.as_bytes());
            for i in 0..64 {
                if (h >> i) & 1 == 1 {
                    counters[i] += 1;
                } else {
                    counters[i] -= 1;
                }
            }
        }
        let mut simhash: u64 = 0;
        for i in 0..64 {
            if counters[i] > 0 {
                simhash |= 1u64 << i;
            }
        }

        // Bloom64: k=5 hash functions via xxh3
        // Cap to last BLOOM_KEYWORD_CAP keywords to avoid saturating the 64-bit filter.
        // At 50 keywords × k=5 = 250 bit-sets → 98% fill (useless).
        // At 8 keywords × k=5 = 40 bit-sets → 47% fill (meaningful discrimination).
        let mut bloom: u64 = 0;
        let bloom_start = self.keywords.len().saturating_sub(BLOOM_KEYWORD_CAP);
        for token in &self.keywords[bloom_start..] {
            let bytes = token.as_bytes();
            for seed in 0..5u64 {
                let h = xxh3::xxh3_64_with_seed(bytes, seed);
                bloom |= 1u64 << (h % 64);
            }
        }

        self.cached_simhash = simhash;
        self.cached_bloom = bloom;
    }
}

impl Default for ContextAccumulator {
    fn default() -> Self {
        Self::new(50)
    }
}

// ============================================================================
// Inline fingerprint scanning (no engram dependency)
// ============================================================================

/// Result of a fingerprint scan hit.
#[derive(Debug, Clone)]
pub struct RecallHit {
    /// Note ID of the matching note
    pub note_id: u64,
    /// Combined score: (64 - hamming_distance) + bloom_overlap
    pub score: u32,
    /// Hamming distance between SimHashes
    pub hamming_distance: u32,
    /// Bloom filter overlap count
    pub bloom_overlap: u32,
}

/// Fingerprint index file constants (must match engram/src/fingerprint.rs)
const FP_MAGIC: u32 = 0x454E4650; // "ENFP"
const FP_HEADER_SIZE: usize = 16;
/// V2 entry size: simhash(8) + bloom(8) + note_id(8) + flags(1) + reserved(7) = 32
const FP_ENTRY_SIZE: usize = 32;
/// V1 entry size: simhash(8) + bloom(8) + note_id(8) = 24
const FP_ENTRY_SIZE_V1: usize = 24;
/// V1 format version
const FP_VERSION_V1: u16 = 1;
/// V2 format version
const FP_VERSION_V2: u16 = 2;
/// V3 format version (256-bit: 128-bit SimHash + 128-bit Bloom, 48-byte entries)
const FP_VERSION_V3: u16 = 3;
/// V3 entry size: simhash[0](8) + simhash[1](8) + bloom[0](8) + bloom[1](8) + note_id(8) + flags(1) + reserved(7) = 48
const FP_ENTRY_SIZE_V3: usize = 48;
/// Skip this entry during recall (e.g., pinned note already in context)
const FLAG_SKIP_RECALL: u8 = 0x01;
/// Entry is tombstoned (note deleted)
const FLAG_TOMBSTONE: u8 = 0x02;

/// Adaptive Hamming distance threshold for 64-bit SimHash.
///
/// As corpus grows, random collisions at lower HD increase (birthday paradox).
/// This table keeps expected false matches < 0.1.
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

/// Adaptive Hamming distance threshold for 128-bit SimHash.
///
/// 128-bit SimHash: birthday threshold N~28,500 (vs N~896 for 64-bit).
pub fn adaptive_max_hd_128(corpus_size: u32) -> u32 {
    match corpus_size {
        0..=5000 => 42,
        5001..=10000 => 38,
        10001..=50000 => 36,
        _ => 34,
    }
}

/// Scan a serialized .engram.fp file for the best match against a context fingerprint.
///
/// Zero-copy, zero-allocation scan. Supports both V1 (24-byte) and V2 (32-byte)
/// entry formats. V2 entries with FLAG_SKIP_RECALL or FLAG_TOMBSTONE are skipped
/// automatically — no exclude_ids list needed.
///
/// # Arguments
/// * `data` — Raw bytes of the .engram.fp file (header + entries)
/// * `context_simhash` — SimHash of the current context
/// * `context_bloom` — Bloom filter of the current context
/// * `max_hd` — Maximum Hamming distance to accept (e.g., 16)
///
/// # Returns
/// The best matching entry if any passes the threshold, or None.
///
/// # Performance
/// ~5ns per entry. V2's 32-byte alignment = exactly 2 entries per 64-byte cache line.
pub fn scan_fp_bytes(
    data: &[u8],
    context_simhash: u64,
    context_bloom: u64,
    max_hd: u32,
) -> Option<RecallHit> {
    if data.len() < FP_HEADER_SIZE {
        return None;
    }

    // Validate magic
    let magic = u32::from_le_bytes(data[0..4].try_into().ok()?);
    if magic != FP_MAGIC {
        return None;
    }

    // Detect version and entry size
    let version = u16::from_le_bytes(data[4..6].try_into().ok()?);
    let entry_size = match version {
        FP_VERSION_V1 => FP_ENTRY_SIZE_V1,
        FP_VERSION_V2 => FP_ENTRY_SIZE,
        _ => return None,
    };

    // Read entry count
    let count = u32::from_le_bytes(data[6..10].try_into().ok()?) as usize;
    let expected_size = FP_HEADER_SIZE + count * entry_size;
    if data.len() < expected_size {
        return None;
    }

    let mut best: Option<RecallHit> = None;

    for i in 0..count {
        let offset = FP_HEADER_SIZE + i * entry_size;

        // V2: check flags byte — single-byte skip before any POPCNT work
        if version >= FP_VERSION_V2 && data[offset + 24] & (FLAG_SKIP_RECALL | FLAG_TOMBSTONE) != 0 {
            continue;
        }

        // Read entry fields directly from bytes
        let simhash = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
        let bloom = u64::from_le_bytes(data[offset + 8..offset + 16].try_into().ok()?);

        // Bloom pre-filter: skip if zero keyword overlap
        let overlap = (bloom & context_bloom).count_ones();
        if overlap == 0 && context_bloom != 0 {
            continue;
        }

        // SimHash Hamming distance
        let hd = (simhash ^ context_simhash).count_ones();
        if hd > max_hd {
            continue;
        }

        let note_id = u64::from_le_bytes(data[offset + 16..offset + 24].try_into().ok()?);
        let score = overlap * 3 + (64 - hd);
        let dominated = best.as_ref().map_or(false, |b| score <= b.score);
        if !dominated {
            best = Some(RecallHit {
                note_id,
                score,
                hamming_distance: hd,
                bloom_overlap: overlap,
            });
        }
    }

    best
}

// ============================================================================
// Recently-recalled dedup
// ============================================================================

/// Tracks recently recalled note IDs to avoid repeating the same note.
///
/// Keeps the last N note IDs. If a note was recalled in the last N tool calls,
/// it won't be surfaced again (even if it's still the best match).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RecentlyRecalled {
    /// Ring buffer of recently recalled note IDs
    note_ids: Vec<u64>,
    /// Maximum entries to track
    capacity: usize,
}

impl RecentlyRecalled {
    pub fn new(capacity: usize) -> Self {
        Self {
            note_ids: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Check if a note was recently recalled.
    pub fn contains(&self, note_id: u64) -> bool {
        self.note_ids.contains(&note_id)
    }

    /// Record a note as recently recalled.
    pub fn add(&mut self, note_id: u64) {
        if self.note_ids.len() >= self.capacity {
            self.note_ids.remove(0);
        }
        self.note_ids.push(note_id);
    }

    /// Number of recently recalled notes tracked.
    pub fn len(&self) -> usize {
        self.note_ids.len()
    }

    /// Whether any notes are tracked.
    pub fn is_empty(&self) -> bool {
        self.note_ids.is_empty()
    }
}

impl Default for RecentlyRecalled {
    fn default() -> Self {
        Self::new(5)
    }
}

// ============================================================================
// Urgency gradients — priority markers for incoming messages
// ============================================================================

/// A file claim owned by this AI, used for urgency scoring.
#[derive(Debug, Clone)]
pub struct OwnedClaim {
    /// File path or resource identifier
    pub path: String,
    /// Seconds since the claim was made/refreshed
    pub age_secs: u64,
}

/// Compute an urgency score for an incoming message.
///
/// Scoring heuristics (from ENRICHMENT-ARCHITECTURE.md §3.3):
/// - Message mentions my AI_ID:        +3
/// - Message mentions my claimed file:  +2 * recency_weight
/// - Message is a reply to me:          +2
/// - Message mentions my active task:   +1
///
/// Recency weight for claims:
///   < 5m:  1.0
///   < 30m: 0.7
///   < 2h:  0.3
///   > 2h:  0.1
///
/// Returns the integer urgency score. If >= 3, the message warrants a `[!]` marker.
///
/// # Performance
/// Pure string matching. <100ns per message for typical inputs.
pub fn compute_urgency(
    message: &str,
    my_ai_id: &str,
    my_claims: &[OwnedClaim],
    is_reply_to_me: bool,
    my_active_task: Option<&str>,
) -> u32 {
    let mut score: u32 = 0;
    let message_lower = message.to_lowercase();
    let my_id_lower = my_ai_id.to_lowercase();

    // +3 if message mentions my AI_ID
    if message_lower.contains(&my_id_lower) {
        score += 3;
    }

    // +2 * recency_weight if message mentions my claimed file
    for claim in my_claims {
        let claim_name = extract_claim_keyword(&claim.path);
        if !claim_name.is_empty() && message_lower.contains(&claim_name) {
            let weight = claim_recency_weight(claim.age_secs);
            // Multiply by 10 then divide to get integer math: 2.0 * 0.7 = 1.4 → 1
            score += ((20.0 * weight) as u32) / 10;
            break; // Only count the best claim match once
        }
    }

    // +2 if this is a reply to one of my messages
    if is_reply_to_me {
        score += 2;
    }

    // +1 if message mentions my active task
    if let Some(task) = my_active_task {
        let task_keywords: Vec<String> = split_to_tokens(task);
        // Require at least 2 task keyword matches to avoid false positives
        let matches = task_keywords
            .iter()
            .filter(|kw| kw.len() > 2 && message_lower.contains(kw.as_str()))
            .count();
        if matches >= 2 {
            score += 1;
        }
    }

    score
}

/// Threshold for adding [!] urgency marker.
pub const URGENCY_THRESHOLD: u32 = 3;

/// Check if a message is urgent (score >= threshold).
#[inline]
pub fn is_urgent(score: u32) -> bool {
    score >= URGENCY_THRESHOLD
}

/// Recency weight for a claim based on its age.
fn claim_recency_weight(age_secs: u64) -> f32 {
    const FIVE_MIN: u64 = 5 * 60;
    const THIRTY_MIN: u64 = 30 * 60;
    const TWO_HOURS: u64 = 2 * 60 * 60;

    if age_secs < FIVE_MIN {
        1.0
    } else if age_secs < THIRTY_MIN {
        0.7
    } else if age_secs < TWO_HOURS {
        0.3
    } else {
        0.1
    }
}

/// Extract the most distinguishing part of a file path for matching.
/// "/mnt/c/.../engram/src/storage.rs" → "storage"
fn extract_claim_keyword(path: &str) -> String {
    // Take the filename without extension
    path.rsplit(|c: char| c == '/' || c == '\\')
        .next()
        .unwrap_or("")
        .rsplit('.')
        .last()
        .unwrap_or("")
        .to_lowercase()
}

// ============================================================================
// Anomaly Pulse — error spiral detection
// ============================================================================

/// Size of the outcome ring buffer.
const OUTCOME_RING_SIZE: usize = 10;

/// Minimum errors in the ring to trigger an anomaly pulse.
const ANOMALY_ERROR_THRESHOLD: usize = 3;

/// Minimum filled entries before anomaly detection activates.
/// Prevents false positives during session startup.
const ANOMALY_MIN_FILLED: usize = 5;

/// Outcome of a single tool call.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolOutcome {
    Ok,
    Err,
}

impl Default for ToolOutcome {
    fn default() -> Self {
        ToolOutcome::Ok
    }
}

/// Fixed-size ring buffer tracking the last 10 tool call outcomes.
///
/// ~20ns hot path: array write + modular increment + count.
/// Zero heap allocation — fixed `[ToolOutcome; 10]` array.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OutcomeRing {
    outcomes: [ToolOutcome; OUTCOME_RING_SIZE],
    pos: usize,
    filled: usize,
}

impl Default for OutcomeRing {
    fn default() -> Self {
        Self {
            outcomes: [ToolOutcome::Ok; OUTCOME_RING_SIZE],
            pos: 0,
            filled: 0,
        }
    }
}

impl OutcomeRing {
    /// Push a new outcome, evicting the oldest if full.
    #[inline]
    pub fn push(&mut self, outcome: ToolOutcome) {
        self.outcomes[self.pos] = outcome;
        self.pos = (self.pos + 1) % OUTCOME_RING_SIZE;
        if self.filled < OUTCOME_RING_SIZE {
            self.filled += 1;
        }
    }

    /// Count errors in the current ring window.
    #[inline]
    pub fn error_count(&self) -> usize {
        self.outcomes[..self.filled]
            .iter()
            .filter(|o| **o == ToolOutcome::Err)
            .count()
    }

    /// Check if we're in an anomalous error spiral.
    ///
    /// Requires at least 5 entries (startup guard) and 3+ errors.
    #[inline]
    pub fn is_anomaly(&self) -> bool {
        self.filled >= ANOMALY_MIN_FILLED && self.error_count() >= ANOMALY_ERROR_THRESHOLD
    }

    /// Number of entries currently in the ring.
    pub fn filled(&self) -> usize {
        self.filled
    }
}

/// Classify a tool call event as success or error.
///
/// Checks three signals in priority order:
/// 1. Explicit `is_error` field in tool result
/// 2. Error patterns in first 500 chars of output text
/// 3. Non-zero exit code
///
/// # Performance
/// ~10ns — string prefix scan on bounded input.
pub fn classify_outcome(event: &serde_json::Value) -> ToolOutcome {
    // Signal 1: explicit is_error field
    if let Some(is_error) = event
        .get("tool_result")
        .and_then(|r| r.get("is_error"))
        .and_then(|v| v.as_bool())
    {
        if is_error {
            return ToolOutcome::Err;
        }
    }

    // Signal 2: error patterns in output text (bounded to first 500 chars)
    let output_text = event
        .get("tool_result")
        .and_then(|r| r.get("output"))
        .or_else(|| event.get("tool_output"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let check_len = output_text.len().min(500);
    let prefix = &output_text[..check_len];
    // Case-sensitive checks for common error markers
    if prefix.contains("Error")
        || prefix.contains("ERROR")
        || prefix.contains("FAILED")
        || prefix.contains("panic")
        || prefix.contains("error:")
        || prefix.contains("error[")
    {
        return ToolOutcome::Err;
    }

    // Signal 3: non-zero exit code
    if let Some(exit_code) = event
        .get("tool_result")
        .and_then(|r| r.get("exit_code"))
        .and_then(|v| v.as_i64())
    {
        if exit_code != 0 {
            return ToolOutcome::Err;
        }
    }

    ToolOutcome::Ok
}

/// Format an anomaly pulse string for injection into hook output.
///
/// Returns `|PULSE|error_spike(N/M)` where N=errors, M=filled.
pub fn format_anomaly_pulse(ring: &OutcomeRing) -> String {
    format!(
        "|PULSE|error_spike({}/{})",
        ring.error_count(),
        ring.filled()
    )
}

// ============================================================================
// Engram .fp file path helper
// ============================================================================

/// Get the path to the .engram.fp sidecar file for this AI.
///
/// Looks for the notebook database at the standard AI-Foundation data directory,
/// then appends ".fp" for the fingerprint sidecar.
pub fn engram_fp_path(ai_id: &str) -> Option<std::path::PathBuf> {
    let home = dirs::home_dir()?;
    let base = home.join(".ai-foundation");

    // Current standard: ~/.ai-foundation/agents/{ai_id}/notebook.engram.fp
    let agents_path = base.join("agents").join(ai_id).join("notebook.engram.fp");
    if agents_path.exists() {
        return Some(agents_path);
    }

    // Legacy: ~/.ai-foundation/notebook/{ai_id}.engram.fp
    let legacy_path = base.join("notebook").join(format!("{}.engram.fp", ai_id));
    if legacy_path.exists() {
        return Some(legacy_path);
    }

    // AppData fallback: %LOCALAPPDATA%/.ai-foundation/data/{ai_id}/notebook.engram.fp
    if let Some(data_dir) = dirs::data_local_dir() {
        let data_path = data_dir.join(".ai-foundation").join("data").join(ai_id).join("notebook.engram.fp");
        if data_path.exists() {
            return Some(data_path);
        }
    }

    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- Keyword extraction tests ---

    #[test]
    fn test_extract_keywords_read() {
        let input = serde_json::json!({
            "file_path": "/mnt/c/Users/Alquado-PC/Desktop/TestingMCPTools/All Tools/tools/engram/src/fingerprint.rs"
        });
        let kw = extract_keywords("Read", &input);
        assert!(kw.contains(&"engram".to_string()), "should extract 'engram', got {:?}", kw);
        assert!(kw.contains(&"fingerprint".to_string()), "should extract 'fingerprint', got {:?}", kw);
        assert!(kw.contains(&"rs".to_string()), "should extract 'rs', got {:?}", kw);
        // Should NOT contain common path prefixes
        assert!(!kw.contains(&"mnt".to_string()), "should skip 'mnt'");
        assert!(!kw.contains(&"Users".to_string()) && !kw.contains(&"users".to_string()), "should skip 'Users'");
    }

    #[test]
    fn test_extract_keywords_grep() {
        let input = serde_json::json!({
            "pattern": "pub fn remember",
            "path": "/tools/engram/src/storage.rs"
        });
        let kw = extract_keywords("Grep", &input);
        assert!(kw.contains(&"pub".to_string()) || kw.contains(&"remember".to_string()),
            "should extract grep pattern tokens, got {:?}", kw);
        assert!(kw.contains(&"engram".to_string()), "should extract path tokens, got {:?}", kw);
    }

    #[test]
    fn test_extract_keywords_bash() {
        let input = serde_json::json!({
            "command": "cargo test fingerprint --release"
        });
        let kw = extract_keywords("Bash", &input);
        assert!(kw.contains(&"cargo".to_string()), "got {:?}", kw);
        assert!(kw.contains(&"test".to_string()), "got {:?}", kw);
        assert!(kw.contains(&"fingerprint".to_string()), "got {:?}", kw);
        // Should skip flags
        assert!(!kw.contains(&"--release".to_string()), "should skip flags");
    }

    #[test]
    fn test_extract_keywords_edit() {
        let input = serde_json::json!({
            "file_path": "/tools/engram/src/recall.rs",
            "old_string": "fn compute_simhash(tokens: &[String]) -> u64",
            "new_string": "fn compute_simhash_weighted(tokens: &[String], idf: &IdfTable) -> u64"
        });
        let kw = extract_keywords("Edit", &input);
        assert!(kw.contains(&"engram".to_string()), "should have path keywords, got {:?}", kw);
        assert!(kw.contains(&"compute_simhash".to_string()) || kw.contains(&"simhash".to_string()),
            "should have code tokens, got {:?}", kw);
    }

    #[test]
    fn test_extract_keywords_unknown_tool() {
        let input = serde_json::json!({
            "file_path": "/some/path.rs",
            "pattern": "search_term"
        });
        let kw = extract_keywords("SomeUnknownTool", &input);
        assert!(!kw.is_empty(), "should extract from common fields for unknown tools");
    }

    #[test]
    fn test_extract_keywords_empty_input() {
        let input = serde_json::json!({});
        let kw = extract_keywords("Read", &input);
        assert!(kw.is_empty(), "empty input should produce no keywords");
    }

    #[test]
    fn test_extract_keywords_dedup() {
        let input = serde_json::json!({
            "file_path": "/engram/engram/engram.rs"
        });
        let kw = extract_keywords("Read", &input);
        let engram_count = kw.iter().filter(|k| k.as_str() == "engram").count();
        assert_eq!(engram_count, 1, "should deduplicate keywords, got {:?}", kw);
    }

    // --- Context accumulator tests ---

    #[test]
    fn test_accumulator_basic() {
        let mut acc = ContextAccumulator::new(10);
        assert!(acc.is_empty());
        assert_eq!(acc.fingerprint(), (0, 0));

        acc.push_keywords(&["storage".to_string(), "overflow".to_string()]);
        assert_eq!(acc.len(), 2);
        let (simhash, bloom) = acc.fingerprint();
        assert_ne!(simhash, 0, "simhash should be non-zero");
        assert_ne!(bloom, 0, "bloom should be non-zero");
    }

    #[test]
    fn test_accumulator_ring_buffer_eviction() {
        let mut acc = ContextAccumulator::new(5);
        acc.push_keywords(&["a".to_string(), "b".to_string(), "c".to_string(), "d".to_string(), "e".to_string()]);
        assert_eq!(acc.len(), 5);

        // Push more — should evict oldest
        acc.push_keywords(&["f".to_string(), "g".to_string()]);
        assert_eq!(acc.len(), 5, "should stay at capacity");

        // Oldest keywords should be gone
        let keywords = acc.keywords();
        assert!(!keywords.contains(&"a".to_string()), "oldest should be evicted");
    }

    #[test]
    fn test_accumulator_fingerprint_changes_with_context() {
        let mut acc = ContextAccumulator::new(20);
        acc.push_keywords(&["storage".to_string(), "overflow".to_string(), "integer".to_string()]);
        let fp1 = acc.fingerprint();

        acc.push_keywords(&["fitquest".to_string(), "nutrition".to_string(), "recipe".to_string()]);
        let fp2 = acc.fingerprint();

        // Fingerprint should change as context shifts
        assert_ne!(fp1, fp2, "fingerprint should change when context shifts");
    }

    #[test]
    fn test_accumulator_stemming() {
        let mut acc = ContextAccumulator::new(10);
        acc.push_keywords(&["running".to_string(), "overflows".to_string()]);

        // The stemmer should reduce "running" → "run", "overflows" → "overflow"
        let keywords = acc.keywords();
        assert!(
            keywords.contains(&"run".to_string()) || keywords.contains(&"running".to_string()),
            "should stem keywords, got {:?}",
            keywords
        );
    }

    #[test]
    fn test_accumulator_serialization_round_trip() {
        let mut acc = ContextAccumulator::new(10);
        acc.push_keywords(&["storage".to_string(), "overflow".to_string()]);
        let (sh1, bl1) = acc.fingerprint();

        let json = serde_json::to_string(&acc).unwrap();
        let restored: ContextAccumulator = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), acc.len());
        assert_eq!(restored.fingerprint(), (sh1, bl1));
    }

    // --- Fingerprint scan tests ---

    #[test]
    fn test_scan_fp_bytes_empty() {
        assert!(scan_fp_bytes(&[], 0, 0, 32).is_none());
    }

    #[test]
    fn test_scan_fp_bytes_bad_magic() {
        let data = [0u8; 40];
        assert!(scan_fp_bytes(&data, 0, 0, 32).is_none());
    }

    #[test]
    fn test_scan_fp_bytes_valid() {
        // Build a minimal .engram.fp file: header + 1 entry
        let mut data = Vec::new();

        // Header: magic(4) + version(2) + count(4) + reserved(6)
        data.extend_from_slice(&FP_MAGIC.to_le_bytes()); // 4
        data.extend_from_slice(&1u16.to_le_bytes()); // version = 1
        data.extend_from_slice(&1u32.to_le_bytes()); // count = 1
        data.extend_from_slice(&[0u8; 6]); // reserved

        // Entry: simhash(8) + bloom(8) + note_id(8)
        let entry_simhash: u64 = 0xDEADBEEF_CAFEBABE;
        let entry_bloom: u64 = 0b1010101010101010;
        let entry_note_id: u64 = 42;
        data.extend_from_slice(&entry_simhash.to_le_bytes());
        data.extend_from_slice(&entry_bloom.to_le_bytes());
        data.extend_from_slice(&entry_note_id.to_le_bytes());

        // Query with the same fingerprint — should match perfectly
        let result = scan_fp_bytes(&data, entry_simhash, entry_bloom, 32);
        assert!(result.is_some(), "should find a match");
        let hit = result.unwrap();
        assert_eq!(hit.note_id, 42);
        assert_eq!(hit.hamming_distance, 0, "identical simhash should have HD=0");
        assert!(hit.bloom_overlap > 0, "identical bloom should have overlap");
    }

    #[test]
    fn test_scan_fp_bytes_threshold_rejects() {
        // Build file with 1 entry
        let mut data = Vec::new();
        data.extend_from_slice(&FP_MAGIC.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 6]);

        let entry_simhash: u64 = 0;
        let entry_bloom: u64 = u64::MAX; // All bits set (will pass bloom filter)
        data.extend_from_slice(&entry_simhash.to_le_bytes());
        data.extend_from_slice(&entry_bloom.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());

        // Query with all-ones simhash — HD = 64 (maximum distance)
        let result = scan_fp_bytes(&data, u64::MAX, u64::MAX, 10);
        assert!(result.is_none(), "HD=64 should be rejected by max_hd=10 threshold");
    }

    #[test]
    fn test_scan_fp_bytes_bloom_prefilter() {
        // Build file with 1 entry that has no bloom overlap with context
        let mut data = Vec::new();
        data.extend_from_slice(&FP_MAGIC.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 6]);

        let entry_simhash: u64 = 0x1234567890ABCDEF;
        let entry_bloom: u64 = 0b0000_0000_0000_1111; // Low bits
        data.extend_from_slice(&entry_simhash.to_le_bytes());
        data.extend_from_slice(&entry_bloom.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());

        // Context with non-overlapping bloom (high bits only)
        let context_bloom: u64 = 0b1111_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000;
        let result = scan_fp_bytes(&data, entry_simhash, context_bloom, 64);
        assert!(result.is_none(), "zero bloom overlap should be filtered out");
    }

    #[test]
    fn test_scan_fp_bytes_multiple_entries_best_wins() {
        let mut data = Vec::new();
        data.extend_from_slice(&FP_MAGIC.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&3u32.to_le_bytes()); // 3 entries
        data.extend_from_slice(&[0u8; 6]);

        let context_simhash: u64 = 0xAAAAAAAAAAAAAAAA;
        let context_bloom: u64 = 0xFF;

        // Entry 1: mediocre match (HD ~32)
        data.extend_from_slice(&0x5555555555555555u64.to_le_bytes()); // Opposite bits
        data.extend_from_slice(&0xFFu64.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());

        // Entry 2: perfect match (HD = 0)
        data.extend_from_slice(&context_simhash.to_le_bytes());
        data.extend_from_slice(&context_bloom.to_le_bytes());
        data.extend_from_slice(&2u64.to_le_bytes());

        // Entry 3: decent match (HD ~16)
        data.extend_from_slice(&(context_simhash ^ 0xFFFF).to_le_bytes());
        data.extend_from_slice(&0xFFu64.to_le_bytes());
        data.extend_from_slice(&3u64.to_le_bytes());

        let result = scan_fp_bytes(&data, context_simhash, context_bloom, 64);
        assert!(result.is_some());
        assert_eq!(result.unwrap().note_id, 2, "best match (HD=0) should win");
    }

    #[test]
    fn test_scan_fp_bytes_v2_flags_skip_pinned() {
        // Build V2 file with 3 entries (32 bytes each)
        let mut data = Vec::new();
        data.extend_from_slice(&FP_MAGIC.to_le_bytes());
        data.extend_from_slice(&FP_VERSION_V2.to_le_bytes()); // V2
        data.extend_from_slice(&3u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 6]);

        let context_simhash: u64 = 0xAAAAAAAAAAAAAAAA;
        let context_bloom: u64 = 0xFF;

        // Entry 1: mediocre match, no flags
        data.extend_from_slice(&0x5555555555555555u64.to_le_bytes()); // simhash
        data.extend_from_slice(&0xFFu64.to_le_bytes());               // bloom
        data.extend_from_slice(&1u64.to_le_bytes());                   // note_id
        data.push(0x00);                                                // flags = 0
        data.extend_from_slice(&[0u8; 7]);                             // reserved

        // Entry 2: perfect match (HD = 0), FLAG_SKIP_RECALL (pinned)
        data.extend_from_slice(&context_simhash.to_le_bytes());
        data.extend_from_slice(&context_bloom.to_le_bytes());
        data.extend_from_slice(&2u64.to_le_bytes());
        data.push(FLAG_SKIP_RECALL);                                    // pinned
        data.extend_from_slice(&[0u8; 7]);

        // Entry 3: decent match (HD ~16), no flags
        data.extend_from_slice(&(context_simhash ^ 0xFFFF).to_le_bytes());
        data.extend_from_slice(&0xFFu64.to_le_bytes());
        data.extend_from_slice(&3u64.to_le_bytes());
        data.push(0x00);
        data.extend_from_slice(&[0u8; 7]);

        // Pinned entry (note 2) should be skipped, note 3 should win
        let result = scan_fp_bytes(&data, context_simhash, context_bloom, 64);
        assert!(result.is_some(), "should find a match after skipping pinned");
        assert_eq!(result.unwrap().note_id, 3, "note 3 should win when note 2 is pinned");
    }

    #[test]
    fn test_scan_fp_bytes_v2_flags_skip_tombstoned() {
        // Build V2 file with 2 entries
        let mut data = Vec::new();
        data.extend_from_slice(&FP_MAGIC.to_le_bytes());
        data.extend_from_slice(&FP_VERSION_V2.to_le_bytes());
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 6]);

        let context_simhash: u64 = 0xAAAAAAAAAAAAAAAA;
        let context_bloom: u64 = 0xFF;

        // Entry 1: perfect match, tombstoned
        data.extend_from_slice(&context_simhash.to_le_bytes());
        data.extend_from_slice(&context_bloom.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());
        data.push(FLAG_TOMBSTONE);
        data.extend_from_slice(&[0u8; 7]);

        // Entry 2: decent match, clean
        data.extend_from_slice(&(context_simhash ^ 0xFFFF).to_le_bytes());
        data.extend_from_slice(&0xFFu64.to_le_bytes());
        data.extend_from_slice(&2u64.to_le_bytes());
        data.push(0x00);
        data.extend_from_slice(&[0u8; 7]);

        let result = scan_fp_bytes(&data, context_simhash, context_bloom, 64);
        assert!(result.is_some());
        assert_eq!(result.unwrap().note_id, 2, "tombstoned entry should be skipped");
    }

    #[test]
    fn test_scan_fp_bytes_v1_compat() {
        // V1 files (24-byte entries, no flags) should still work
        let mut data = Vec::new();
        data.extend_from_slice(&FP_MAGIC.to_le_bytes());
        data.extend_from_slice(&FP_VERSION_V1.to_le_bytes()); // V1
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 6]);

        let entry_simhash: u64 = 0xDEADBEEF_CAFEBABE;
        let entry_bloom: u64 = 0b1010101010101010;
        data.extend_from_slice(&entry_simhash.to_le_bytes());
        data.extend_from_slice(&entry_bloom.to_le_bytes());
        data.extend_from_slice(&42u64.to_le_bytes());
        // No flags byte — V1 format

        let result = scan_fp_bytes(&data, entry_simhash, entry_bloom, 32);
        assert!(result.is_some(), "V1 sidecar should still work");
        assert_eq!(result.unwrap().note_id, 42);
    }

    // --- RecentlyRecalled tests ---

    #[test]
    fn test_recently_recalled_basic() {
        let mut rr = RecentlyRecalled::new(3);
        assert!(!rr.contains(1));

        rr.add(1);
        assert!(rr.contains(1));
        assert!(!rr.contains(2));
    }

    #[test]
    fn test_recently_recalled_eviction() {
        let mut rr = RecentlyRecalled::new(3);
        rr.add(1);
        rr.add(2);
        rr.add(3);
        assert!(rr.contains(1));

        rr.add(4); // Should evict 1
        assert!(!rr.contains(1), "oldest should be evicted");
        assert!(rr.contains(4));
    }

    #[test]
    fn test_recently_recalled_serialization() {
        let mut rr = RecentlyRecalled::new(5);
        rr.add(10);
        rr.add(20);

        let json = serde_json::to_string(&rr).unwrap();
        let restored: RecentlyRecalled = serde_json::from_str(&json).unwrap();
        assert!(restored.contains(10));
        assert!(restored.contains(20));
        assert!(!restored.contains(30));
    }

    // --- Urgency gradient tests ---

    #[test]
    fn test_urgency_ai_id_mention() {
        let score = compute_urgency(
            "Hey cascade-230, can you review this?",
            "cascade-230",
            &[],
            false,
            None,
        );
        assert_eq!(score, 3, "AI_ID mention should score 3");
        assert!(is_urgent(score));
    }

    #[test]
    fn test_urgency_ai_id_case_insensitive() {
        let score = compute_urgency(
            "CASCADE-230 needs to check this",
            "cascade-230",
            &[],
            false,
            None,
        );
        assert_eq!(score, 3, "AI_ID match should be case-insensitive");
    }

    #[test]
    fn test_urgency_no_mention() {
        let score = compute_urgency(
            "Fixed the storage bug in engram",
            "cascade-230",
            &[],
            false,
            None,
        );
        assert_eq!(score, 0, "no relevant mention should score 0");
        assert!(!is_urgent(score));
    }

    #[test]
    fn test_urgency_claimed_file_recent() {
        let claims = vec![OwnedClaim {
            path: "/tools/engram/src/storage.rs".to_string(),
            age_secs: 120, // 2 minutes — fresh claim
        }];
        let score = compute_urgency(
            "I'm about to modify storage, heads up",
            "cascade-230",
            &claims,
            false,
            None,
        );
        // claim match: 2 * 1.0 = 2
        assert_eq!(score, 2, "recent claim mention should score 2");
    }

    #[test]
    fn test_urgency_claimed_file_stale() {
        let claims = vec![OwnedClaim {
            path: "/tools/engram/src/storage.rs".to_string(),
            age_secs: 8000, // ~2.2 hours — stale claim
        }];
        let score = compute_urgency(
            "I'm about to modify storage, heads up",
            "cascade-230",
            &claims,
            false,
            None,
        );
        // claim match: floor(20 * 0.1 / 10) = floor(0.2) = 0
        assert_eq!(score, 0, "stale claim should have near-zero weight");
    }

    #[test]
    fn test_urgency_reply_to_me() {
        let score = compute_urgency(
            "Good point, I'll fix that approach",
            "cascade-230",
            &[],
            true,
            None,
        );
        assert_eq!(score, 2, "reply should score 2");
    }

    #[test]
    fn test_urgency_active_task_match() {
        let score = compute_urgency(
            "The fingerprint backfill migration is failing on large databases",
            "cascade-230",
            &[],
            false,
            Some("fingerprint backfill migration"),
        );
        // At least 2 task keywords match ("fingerprint", "backfill", "migration")
        assert_eq!(score, 1, "active task keyword match should score 1");
    }

    #[test]
    fn test_urgency_active_task_single_keyword_insufficient() {
        let score = compute_urgency(
            "The fingerprint looks correct",
            "cascade-230",
            &[],
            false,
            Some("fingerprint backfill migration"),
        );
        // Only 1 keyword match ("fingerprint") — below threshold of 2
        assert_eq!(score, 0, "single task keyword match shouldn't score");
    }

    #[test]
    fn test_urgency_compound_high_score() {
        let claims = vec![OwnedClaim {
            path: "/tools/engram/src/storage.rs".to_string(),
            age_secs: 60,
        }];
        let score = compute_urgency(
            "cascade-230: storage.rs has a bug in the backfill migration path",
            "cascade-230",
            &claims,
            true,
            Some("backfill migration path"),
        );
        // AI_ID: +3, claim (storage, fresh): +2, reply: +2, task (backfill+migration): +1 = 8
        assert_eq!(score, 8, "compound urgency should stack");
        assert!(is_urgent(score));
    }

    #[test]
    fn test_urgency_threshold_boundary() {
        assert!(!is_urgent(2), "score 2 should not be urgent");
        assert!(is_urgent(3), "score 3 should be urgent");
        assert!(is_urgent(10), "score 10 should be urgent");
    }

    #[test]
    fn test_claim_recency_weight_boundaries() {
        assert_eq!(claim_recency_weight(0), 1.0);
        assert_eq!(claim_recency_weight(299), 1.0);  // Just under 5 min
        assert_eq!(claim_recency_weight(300), 0.7);  // Exactly 5 min
        assert_eq!(claim_recency_weight(1799), 0.7); // Just under 30 min
        assert_eq!(claim_recency_weight(1800), 0.3); // Exactly 30 min
        assert_eq!(claim_recency_weight(7199), 0.3); // Just under 2 hours
        assert_eq!(claim_recency_weight(7200), 0.1); // Exactly 2 hours
        assert_eq!(claim_recency_weight(86400), 0.1); // 24 hours
    }

    #[test]
    fn test_extract_claim_keyword() {
        assert_eq!(extract_claim_keyword("/tools/engram/src/storage.rs"), "storage");
        assert_eq!(extract_claim_keyword("C:\\Users\\test\\fingerprint.rs"), "fingerprint");
        assert_eq!(extract_claim_keyword("/hook-bulletin.rs"), "hook-bulletin");
        assert_eq!(extract_claim_keyword(""), "");
    }

    // --- Path splitting tests ---

    #[test]
    fn test_split_path_keywords() {
        let kw = split_path_keywords("/mnt/c/Users/Alquado-PC/Desktop/TestingMCPTools/All Tools/tools/engram/src/fingerprint.rs");
        assert!(kw.contains(&"engram".to_string()), "got {:?}", kw);
        assert!(kw.contains(&"fingerprint".to_string()), "got {:?}", kw);
        assert!(kw.contains(&"tools".to_string()), "got {:?}", kw);
        assert!(!kw.contains(&"mnt".to_string()), "should skip 'mnt'");
    }

    #[test]
    fn test_accumulator_bloom_not_saturated() {
        // With 50 diverse keywords, bloom should NOT be saturated (< 75% fill).
        // Before the BLOOM_KEYWORD_CAP fix, 50 keywords × k=5 = 250 bit-sets
        // would fill 98% of the 64-bit bloom, making it useless for discrimination.
        let mut acc = ContextAccumulator::new(50);
        let keywords: Vec<String> = (0..50)
            .map(|i| format!("keyword_{}", i))
            .collect();
        acc.push_keywords(&keywords);

        let (_, bloom) = acc.fingerprint();
        let bits_set = bloom.count_ones();

        // With BLOOM_KEYWORD_CAP=8, we expect ~30 bits (47% fill).
        // Allow up to 48 bits (75%) — anything above means the cap isn't working.
        assert!(
            bits_set <= 48,
            "Bloom is over-saturated: {}/64 bits set ({}%). \
             BLOOM_KEYWORD_CAP should prevent this.",
            bits_set,
            bits_set * 100 / 64
        );

        // Should still be non-trivial (at least 10 bits from 8 keywords)
        assert!(
            bits_set >= 10,
            "Bloom too sparse: only {}/64 bits set. Should have ~30.",
            bits_set
        );
    }

    #[test]
    fn test_accumulator_bloom_uses_recent_keywords() {
        // Bloom should reflect the MOST RECENT keywords, not old ones.
        let mut acc = ContextAccumulator::new(50);

        // Push 40 "old" keywords
        let old_kw: Vec<String> = (0..40).map(|i| format!("old_{}", i)).collect();
        acc.push_keywords(&old_kw);
        let (_, bloom_old) = acc.fingerprint();

        // Push 8 "new" keywords — these should dominate the bloom
        let new_kw: Vec<String> = (0..8).map(|i| format!("fresh_{}", i)).collect();
        acc.push_keywords(&new_kw);
        let (_, bloom_new) = acc.fingerprint();

        // Bloom should change because it now uses the 8 most recent keywords
        assert_ne!(
            bloom_old, bloom_new,
            "Bloom should change when recent keywords change"
        );
    }

    #[test]
    fn test_split_path_keywords_windows() {
        let kw = split_path_keywords("C:\\Users\\Test\\engram\\storage.rs");
        assert!(kw.contains(&"engram".to_string()), "got {:?}", kw);
        assert!(kw.contains(&"storage".to_string()), "got {:?}", kw);
    }

    #[test]
    fn test_extract_command_keywords() {
        let kw = extract_command_keywords("cargo test fingerprint --release 2>&1");
        assert!(kw.contains(&"cargo".to_string()), "got {:?}", kw);
        assert!(kw.contains(&"test".to_string()), "got {:?}", kw);
        assert!(kw.contains(&"fingerprint".to_string()), "got {:?}", kw);
    }

    // --- Anomaly Pulse tests ---

    #[test]
    fn test_outcome_ring_empty_no_anomaly() {
        let ring = OutcomeRing::default();
        assert_eq!(ring.error_count(), 0);
        assert_eq!(ring.filled(), 0);
        assert!(!ring.is_anomaly());
    }

    #[test]
    fn test_outcome_ring_below_threshold() {
        let mut ring = OutcomeRing::default();
        // 8 OK + 2 Err = 2/10 errors (below threshold of 3)
        for _ in 0..8 {
            ring.push(ToolOutcome::Ok);
        }
        ring.push(ToolOutcome::Err);
        ring.push(ToolOutcome::Err);
        assert_eq!(ring.error_count(), 2);
        assert_eq!(ring.filled(), 10);
        assert!(!ring.is_anomaly(), "2/10 errors should not trigger anomaly");
    }

    #[test]
    fn test_outcome_ring_at_threshold() {
        let mut ring = OutcomeRing::default();
        // 7 OK + 3 Err = 3/10 errors (at threshold)
        for _ in 0..7 {
            ring.push(ToolOutcome::Ok);
        }
        for _ in 0..3 {
            ring.push(ToolOutcome::Err);
        }
        assert_eq!(ring.error_count(), 3);
        assert!(ring.is_anomaly(), "3/10 errors should trigger anomaly");
    }

    #[test]
    fn test_outcome_ring_startup_guard() {
        let mut ring = OutcomeRing::default();
        // 3 errors in 3 calls (filled < 5) — should NOT trigger
        ring.push(ToolOutcome::Err);
        ring.push(ToolOutcome::Err);
        ring.push(ToolOutcome::Err);
        assert_eq!(ring.filled(), 3);
        assert_eq!(ring.error_count(), 3);
        assert!(!ring.is_anomaly(), "filled < 5 should not trigger (startup guard)");
    }

    #[test]
    fn test_outcome_ring_eviction_clears_anomaly() {
        let mut ring = OutcomeRing::default();
        // Fill with 5 OK + 5 Err = anomaly
        for _ in 0..5 {
            ring.push(ToolOutcome::Ok);
        }
        for _ in 0..5 {
            ring.push(ToolOutcome::Err);
        }
        assert!(ring.is_anomaly());

        // Push 3 more OK — evicts 3 oldest (which were OK), ring is now 2 OK + 5 Err + 3 OK
        // Wait, ring is fixed at 10. After 10 entries, new pushes overwrite oldest.
        // Current ring: [OK,OK,OK,OK,OK,Err,Err,Err,Err,Err], pos=0
        // Push 3 OK: overwrites pos 0,1,2 → [OK,OK,OK,OK,OK,Err,Err,Err,Err,Err] →
        // Actually pos wraps. Let me think: after 10 pushes, pos=0, filled=10.
        // Push OK at pos 0 → [OK,...], pos=1. Push OK at pos 1, pos=2. Push OK at pos 2, pos=3.
        // Ring: [OK,OK,OK,OK,OK,Err,Err,Err,Err,Err] — we overwrote the first 3 OKs with OKs.
        // Error count still 5. Need to push more to evict errors.
        for _ in 0..5 {
            ring.push(ToolOutcome::Ok);
        }
        // Now we've overwritten positions 0-4 (the old OKs) with OKs. Still 5 errors.
        // Push 3 more to start overwriting the errors at positions 5,6,7
        for _ in 0..3 {
            ring.push(ToolOutcome::Ok);
        }
        // Errors remaining: positions 8,9 = 2 errors
        assert_eq!(ring.error_count(), 2);
        assert!(!ring.is_anomaly(), "errors should evict from ring, clearing anomaly");
    }

    #[test]
    fn test_outcome_ring_serde_roundtrip() {
        let mut ring = OutcomeRing::default();
        ring.push(ToolOutcome::Ok);
        ring.push(ToolOutcome::Err);
        ring.push(ToolOutcome::Ok);

        let json = serde_json::to_string(&ring).unwrap();
        let restored: OutcomeRing = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.filled(), 3);
        assert_eq!(restored.error_count(), 1);
        assert!(!restored.is_anomaly());
    }

    #[test]
    fn test_classify_outcome_explicit_error() {
        let event = serde_json::json!({
            "tool_name": "Bash",
            "tool_result": { "is_error": true, "output": "something" }
        });
        assert_eq!(classify_outcome(&event), ToolOutcome::Err);
    }

    #[test]
    fn test_classify_outcome_error_in_output() {
        let event = serde_json::json!({
            "tool_name": "Bash",
            "tool_result": { "output": "error[E0308]: mismatched types" }
        });
        assert_eq!(classify_outcome(&event), ToolOutcome::Err);
    }

    #[test]
    fn test_classify_outcome_nonzero_exit() {
        let event = serde_json::json!({
            "tool_name": "Bash",
            "tool_result": { "exit_code": 1, "output": "compilation stopped" }
        });
        assert_eq!(classify_outcome(&event), ToolOutcome::Err);
    }

    #[test]
    fn test_classify_outcome_success() {
        let event = serde_json::json!({
            "tool_name": "Read",
            "tool_result": { "output": "fn main() {\n    println!(\"hello\");\n}" }
        });
        assert_eq!(classify_outcome(&event), ToolOutcome::Ok);
    }

    #[test]
    fn test_format_anomaly_pulse() {
        let mut ring = OutcomeRing::default();
        for _ in 0..7 {
            ring.push(ToolOutcome::Ok);
        }
        for _ in 0..3 {
            ring.push(ToolOutcome::Err);
        }
        let pulse = format_anomaly_pulse(&ring);
        assert_eq!(pulse, "|PULSE|error_spike(3/10)");
    }
}
