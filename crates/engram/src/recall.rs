//! Hybrid recall - combining vector, keyword, graph, and recency signals

use crate::note::Note;
use std::collections::{HashMap, HashSet};

/// Recall configuration
#[derive(Debug, Clone)]
pub struct RecallConfig {
    /// Weight for vector similarity
    pub vector_weight: f32,
    /// Weight for keyword/BM25 score
    pub keyword_weight: f32,
    /// Weight for graph/PageRank score
    pub graph_weight: f32,
    /// Weight for recency
    pub recency_weight: f32,
    /// Recency decay half-life in hours
    pub recency_half_life_hours: f32,
}

impl Default for RecallConfig {
    fn default() -> Self {
        Self {
            vector_weight: 0.4,
            keyword_weight: 0.3,
            graph_weight: 0.2,
            recency_weight: 0.1,
            recency_half_life_hours: 24.0,
        }
    }
}

/// A recall result with scores
#[derive(Debug, Clone)]
pub struct RecallResult {
    pub note: Note,
    pub vector_score: f32,
    pub keyword_score: f32,
    pub graph_score: f32,
    pub recency_score: f32,
    pub final_score: f32,
}

/// Compute BM25 score for a document against a query
pub fn bm25_score(doc: &str, query: &str, k1: f32, b: f32, avgdl: f32) -> f32 {
    let doc_lower = doc.to_lowercase();
    let query_lower = query.to_lowercase();
    let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

    let doc_len = doc_lower.split_whitespace().count() as f32;

    let mut score = 0.0;

    for term in &query_terms {
        // Term frequency in document
        let tf = doc_lower.matches(term).count() as f32;

        if tf > 0.0 {
            // BM25 formula
            let idf = 1.0; // Simplified - would need corpus stats for real IDF
            let numerator = tf * (k1 + 1.0);
            let denominator = tf + k1 * (1.0 - b + b * doc_len / avgdl);
            score += idf * numerator / denominator;
        }
    }

    score
}

/// Precomputed BM25 corpus statistics for IDF-weighted keyword scoring.
///
/// Build once from the candidate document set, then call [`BM25Corpus::score`]
/// for each document. IDF suppresses common terms (high document frequency)
/// and amplifies rare terms — without it BM25 degenerates to TF-only.
///
/// # Example
/// ```ignore
/// let contents: Vec<&str> = notes.iter().map(|n| n.content.as_str()).collect();
/// let corpus = BM25Corpus::new(&contents);
/// let score = corpus.score(&note.content, query, 1.2, 0.75);
/// ```
pub struct BM25Corpus {
    /// IDF value for each term: ln((N - df + 0.5) / (df + 0.5) + 1)
    idf: HashMap<String, f32>,
    /// Average document length in words (precomputed from corpus)
    pub avgdl: f32,
}

impl BM25Corpus {
    /// Build corpus statistics from a slice of document strings.
    ///
    /// Uses the BM25+ IDF formula which is always non-negative:
    /// `IDF(t) = ln((N − df(t) + 0.5) / (df(t) + 0.5) + 1)`
    ///
    /// Each term is counted once per document for IDF purposes (not by
    /// frequency), matching standard BM25 semantics.
    pub fn new(docs: &[&str]) -> Self {
        let n_docs = docs.len();
        let mut df: HashMap<String, usize> = HashMap::new();
        let mut total_len = 0usize;

        for doc in docs {
            let doc_lower = doc.to_lowercase();
            total_len += doc_lower.split_whitespace().count();

            // Count each term once per document (document frequency, not term frequency)
            let terms_in_doc: HashSet<String> = doc_lower
                .split_whitespace()
                .map(|t| t.to_string())
                .collect();
            for term in terms_in_doc {
                *df.entry(term).or_insert(0) += 1;
            }
        }

        let avgdl = if n_docs > 0 {
            total_len as f32 / n_docs as f32
        } else {
            50.0 // reasonable fallback when no corpus
        };

        let n = n_docs as f32;
        let idf = df
            .into_iter()
            .map(|(term, freq)| {
                let d = freq as f32;
                // BM25+ IDF — always >= 0, approaches 0 as df → N
                let val = ((n - d + 0.5) / (d + 0.5) + 1.0).ln();
                (term, val.max(0.0))
            })
            .collect();

        Self { idf, avgdl }
    }

    /// IDF value for a specific term (lowercased). Returns 0.0 for unseen terms.
    pub fn idf_for(&self, term: &str) -> f32 {
        self.idf.get(&term.to_lowercase()).copied().unwrap_or(0.0)
    }

    /// Score a single document against a query using BM25 with real IDF.
    ///
    /// Terms absent from the corpus get IDF 0 and contribute nothing to the
    /// score — safe because if no candidate contains a term it cannot help rank.
    pub fn score(&self, doc: &str, query: &str, k1: f32, b: f32) -> f32 {
        let doc_lower = doc.to_lowercase();
        let query_lower = query.to_lowercase();
        let query_terms: Vec<&str> = query_lower.split_whitespace().collect();
        let doc_len = doc_lower.split_whitespace().count() as f32;

        let mut score = 0.0f32;
        for term in &query_terms {
            let tf = doc_lower.matches(term).count() as f32;
            if tf > 0.0 {
                let idf = self.idf.get(*term).copied().unwrap_or(0.0);
                if idf > 0.0 {
                    let numerator = tf * (k1 + 1.0);
                    let denominator = tf + k1 * (1.0 - b + b * doc_len / self.avgdl);
                    score += idf * numerator / denominator;
                }
            }
        }
        score
    }
}

/// Compute recency score (exponential decay)
pub fn recency_score(timestamp: i64, half_life_hours: f32) -> f32 {
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    recency_score_at(timestamp, half_life_hours, now)
}

/// Compute recency score with a pre-computed `now` timestamp (nanoseconds).
///
/// Use this in batch scoring to avoid calling `chrono::Utc::now()` per note.
/// Compute `now` once, then call this for each candidate.
#[inline]
pub fn recency_score_at(timestamp: i64, half_life_hours: f32, now_nanos: i64) -> f32 {
    let age_nanos = (now_nanos - timestamp).max(0) as f64;
    let age_hours = age_nanos / (1_000_000_000.0 * 3600.0);

    // Exponential decay: score = 0.5^(age / half_life)
    let decay = (-age_hours / half_life_hours as f64 * 0.693).exp();
    decay as f32
}

/// Reciprocal Rank Fusion - combine multiple ranked lists
pub fn rrf_fusion(rankings: &[Vec<(u64, f32)>], k: f32) -> Vec<(u64, f32)> {
    use std::collections::HashMap;

    let mut scores: HashMap<u64, f32> = HashMap::new();

    for ranking in rankings {
        for (rank, (id, _score)) in ranking.iter().enumerate() {
            let rrf_score = 1.0 / (k + rank as f32 + 1.0);
            *scores.entry(*id).or_insert(0.0) += rrf_score;
        }
    }

    let mut results: Vec<(u64, f32)> = scores.into_iter().collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results
}

/// Normalize scores to [0, 1] range
pub fn normalize_scores(scores: &mut [(u64, f32)]) {
    if scores.is_empty() {
        return;
    }

    let max = scores.iter().map(|(_, s)| *s).fold(0.0f32, f32::max);
    let min = scores.iter().map(|(_, s)| *s).fold(f32::MAX, f32::min);

    let range = max - min;
    if range > 0.0 {
        for (_, score) in scores.iter_mut() {
            *score = (*score - min) / range;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bm25() {
        let doc = "The quick brown fox jumps over the lazy dog";
        let query = "quick fox";

        let score = bm25_score(doc, query, 1.2, 0.75, 10.0);
        assert!(score > 0.0);
    }

    #[test]
    fn test_bm25_no_match() {
        let doc = "Hello world";
        let query = "goodbye moon";

        let score = bm25_score(doc, query, 1.2, 0.75, 10.0);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_recency_score() {
        // Recent timestamp should have high score
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let score = recency_score(now, 24.0);
        assert!(score > 0.99);

        // Old timestamp should have lower score
        let old = now - (48 * 3600 * 1_000_000_000); // 48 hours ago
        let old_score = recency_score(old, 24.0);
        assert!(old_score < 0.3);
    }

    #[test]
    fn test_rrf_fusion() {
        let ranking1 = vec![(1, 0.9), (2, 0.8), (3, 0.7)];
        let ranking2 = vec![(2, 0.95), (1, 0.85), (4, 0.6)];

        let fused = rrf_fusion(&[ranking1, ranking2], 60.0);

        // ID 2 appears high in both rankings, should be first or second
        assert!(fused.iter().take(2).any(|(id, _)| *id == 2));
    }

    #[test]
    fn test_normalize_scores() {
        let mut scores = vec![(1, 10.0), (2, 20.0), (3, 30.0)];
        normalize_scores(&mut scores);

        assert_eq!(scores[0].1, 0.0);  // Min becomes 0
        assert_eq!(scores[2].1, 1.0);  // Max becomes 1
        assert!((scores[1].1 - 0.5).abs() < 0.01); // Middle is 0.5
    }

    // --- BM25Corpus tests ---

    #[test]
    fn test_bm25_corpus_idf_rare_beats_common() {
        // "rust" appears in all 3 docs → high df → low IDF
        // "lifetime" appears in only 1 doc → low df → high IDF
        let docs = [
            "rust is a systems programming language with memory safety",
            "rust prevents data races and memory bugs",
            "rust ownership lifetime borrow checker prevents bugs",
        ];
        let corpus = BM25Corpus::new(&docs);

        let idf_rust = corpus.idf_for("rust");
        let idf_lifetime = corpus.idf_for("lifetime");

        assert!(idf_rust > 0.0, "rust should have positive IDF");
        assert!(idf_lifetime > 0.0, "lifetime should have positive IDF");
        assert!(
            idf_lifetime > idf_rust,
            "rare term 'lifetime' (df=1) should have higher IDF than ubiquitous 'rust' (df=3): lifetime={idf_lifetime:.4} rust={idf_rust:.4}"
        );
    }

    #[test]
    fn test_bm25_corpus_scores_rare_match_higher() {
        // "lifetime" appears in only 1 of the 3 corpus docs
        // querying "lifetime" should score that doc higher than a doc with no match
        let corpus_docs = [
            "rust is a systems programming language",
            "rust ownership lifetime borrow checker",
            "rust prevents data races and memory bugs",
        ];
        let corpus = BM25Corpus::new(&corpus_docs);

        let score_no_match = corpus.score("rust is a systems programming language", "lifetime", 1.2, 0.75);
        let score_match = corpus.score("rust ownership lifetime borrow checker", "lifetime", 1.2, 0.75);

        assert_eq!(score_no_match, 0.0, "doc without 'lifetime' should score 0");
        assert!(score_match > 0.0, "doc with 'lifetime' should score > 0");
    }

    #[test]
    fn test_bm25_corpus_universal_term_scores_zero() {
        // A term that appears in EVERY doc has minimal discriminating power.
        // With BM25+ IDF = ln((N-N+0.5)/(N+0.5)+1) = ln(1.5/N+1) → approaches 0 as N grows.
        // With N=3, df=3: idf = ln((3-3+0.5)/(3+0.5)+1) = ln(0.5/3.5+1) = ln(1.143) ≈ 0.134
        // It's small but not zero — BM25+ ensures non-negative IDF.
        let docs = [
            "common term and other words",
            "common term with different words",
            "common term yet more words",
        ];
        let corpus = BM25Corpus::new(&docs);
        let idf_common = corpus.idf_for("common");
        let idf_unique = corpus.idf_for("different"); // df=1

        assert!(idf_unique > idf_common, "unique term should beat common term");
    }

    #[test]
    fn test_bm25_corpus_empty_corpus() {
        let corpus = BM25Corpus::new(&[]);
        let score = corpus.score("some document content", "query", 1.2, 0.75);
        assert_eq!(score, 0.0, "empty corpus should always score 0");
        assert!((corpus.avgdl - 50.0).abs() < 0.01, "empty corpus avgdl should be 50.0 fallback");
    }

    #[test]
    fn test_bm25_corpus_avgdl() {
        let docs = [
            "one two three",        // 3 words
            "four five six seven",  // 4 words
            "eight nine",           // 2 words
        ];
        let corpus = BM25Corpus::new(&docs);
        // avgdl = (3 + 4 + 2) / 3 = 3.0
        assert!((corpus.avgdl - 3.0).abs() < 0.01, "avgdl should be 3.0, got {}", corpus.avgdl);
    }

    #[test]
    fn test_bm25_corpus_no_match_returns_zero() {
        let docs = ["hello world", "foo bar baz"];
        let corpus = BM25Corpus::new(&docs);
        let score = corpus.score("hello world", "zzz_nonexistent", 1.2, 0.75);
        assert_eq!(score, 0.0);
    }
}
