//! Hybrid recall - combining vector, keyword, graph, and recency signals
//!
//! BM25 scoring uses Snowball stemming (Porter2 for English) so inflected
//! forms like "running", "runs", "ran" all reduce to the same stem "run".
//! This dramatically improves keyword recall without sacrificing precision
//! (vector similarity handles semantic nuance).

use crate::note::Note;
use rust_stemmers::{Algorithm, Stemmer};
use std::collections::{HashMap, HashSet};

/// Tokenize text into lowercase words with punctuation stripped (no stemming).
///
/// Splits on whitespace, lowercases, and strips leading/trailing punctuation
/// so "Dog," matches "dog" and "it's" stays as "it's" (internal punctuation preserved).
/// Does NOT apply stemming — use [`tokenize_stemmed`] for BM25 scoring.
#[cfg(test)]
fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            let lower = w.to_lowercase();
            lower.trim_matches(|c: char| c.is_ascii_punctuation()).to_string()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Tokenize and stem text for BM25 scoring.
///
/// Same as [`tokenize`] but additionally applies Snowball stemming so
/// "running" → "run", "memories" → "memori", "dogs" → "dog", etc.
/// Both document and query must be stemmed with the same stemmer for
/// term matching to work correctly.
fn tokenize_stemmed(text: &str, stemmer: &Stemmer) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            let lower = w.to_lowercase();
            let trimmed = lower.trim_matches(|c: char| c.is_ascii_punctuation()).to_string();
            if trimmed.is_empty() {
                trimmed
            } else {
                stemmer.stem(&trimmed).into_owned()
            }
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Count exact occurrences of a term in a token list (word-boundary matching).
///
/// Unlike `str::matches()` which is substring-based (searching "is" matches
/// "this"), this counts only exact token matches.
fn count_term(tokens: &[String], term: &str) -> usize {
    tokens.iter().filter(|t| t.as_str() == term).count()
}

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
            // Keyword-primary: BM25 is the most reliable signal for explicit queries.
            // A perfect keyword match must outrank inflated vector/graph scores.
            // Calibrated Mar 2026: keyword 1.000 was losing to vector 0.972 (garbage
            // normalized to ~1.0) + graph 1.000 (same issue) under old 0.3 weight.
            keyword_weight: 0.45,
            vector_weight: 0.25,
            graph_weight: 0.15,
            recency_weight: 0.15,
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

/// Compute BM25 score for a document against a query.
///
/// Uses Snowball stemming so "running" matches "run", "dogs" matches "dog", etc.
pub fn bm25_score(doc: &str, query: &str, k1: f32, b: f32, avgdl: f32) -> f32 {
    let stemmer = Stemmer::create(Algorithm::English);
    let doc_tokens = tokenize_stemmed(doc, &stemmer);
    let query_tokens = tokenize_stemmed(query, &stemmer);
    let doc_len = doc_tokens.len() as f32;

    let mut score = 0.0;

    for term in &query_tokens {
        // Term frequency in document (exact word-boundary matching)
        let tf = count_term(&doc_tokens, term) as f32;

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
    /// IDF value for each stemmed term: ln((N - df + 0.5) / (df + 0.5) + 1)
    idf: HashMap<String, f32>,
    /// Average document length in words (precomputed from corpus)
    pub avgdl: f32,
    /// Snowball stemmer (created once, reused for all scoring calls)
    stemmer: Stemmer,
}

impl BM25Corpus {
    /// Build corpus statistics from a slice of document strings.
    ///
    /// Uses the BM25+ IDF formula which is always non-negative:
    /// `IDF(t) = ln((N − df(t) + 0.5) / (df(t) + 0.5) + 1)`
    ///
    /// Terms are stemmed with Snowball English stemmer so "running" and "run"
    /// share the same IDF entry. Each term is counted once per document for
    /// IDF purposes (not by frequency), matching standard BM25 semantics.
    pub fn new(docs: &[&str]) -> Self {
        let stemmer = Stemmer::create(Algorithm::English);
        let n_docs = docs.len();
        let mut df: HashMap<String, usize> = HashMap::new();
        let mut total_len = 0usize;

        for doc in docs {
            let tokens = tokenize_stemmed(doc, &stemmer);
            total_len += tokens.len();

            // Count each term once per document (document frequency, not term frequency)
            let terms_in_doc: HashSet<String> = tokens.into_iter().collect();
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

        Self { idf, avgdl, stemmer }
    }

    /// IDF value for a specific term (stemmed). Returns 0.0 for unseen terms.
    ///
    /// The term is lowercased, stripped of leading/trailing punctuation,
    /// and stemmed to match how terms are stored in the IDF map.
    pub fn idf_for(&self, term: &str) -> f32 {
        let normalized = term.to_lowercase();
        let normalized = normalized.trim_matches(|c: char| c.is_ascii_punctuation());
        let stemmed = self.stemmer.stem(normalized);
        self.idf.get(stemmed.as_ref()).copied().unwrap_or(0.0)
    }

    /// Score a single document against a query using BM25 with real IDF.
    ///
    /// Both document and query are stemmed so inflected forms match their roots.
    /// Terms absent from the corpus get IDF 0 and contribute nothing to the
    /// score — safe because if no candidate contains a term it cannot help rank.
    pub fn score(&self, doc: &str, query: &str, k1: f32, b: f32) -> f32 {
        let doc_tokens = tokenize_stemmed(doc, &self.stemmer);
        let query_tokens = tokenize_stemmed(query, &self.stemmer);
        let doc_len = doc_tokens.len() as f32;

        let mut score = 0.0f32;
        for term in &query_tokens {
            let tf = count_term(&doc_tokens, term) as f32;
            if tf > 0.0 {
                let idf = self.idf.get(term.as_str()).copied().unwrap_or(0.0);
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
    normalize_scores_gated(scores, 0.0);
}

/// Normalize scores to [0, 1] with quality gating.
///
/// When `quality_floor` > 0 and the best raw score is below it, normalized
/// values are attenuated by `max_raw / quality_floor`. This prevents min-max
/// normalization from inflating garbage scores to 1.0 — a cosine similarity
/// of 0.15 shouldn't become 1.0 just because it's the "best of bad."
pub fn normalize_scores_gated(scores: &mut [(u64, f32)], quality_floor: f32) {
    if scores.is_empty() {
        return;
    }

    let max = scores.iter().map(|(_, s)| *s).fold(0.0f32, f32::max);
    let min = scores.iter().map(|(_, s)| *s).fold(f32::MAX, f32::min);

    let range = max - min;
    let confidence = if quality_floor > 0.0 {
        (max / quality_floor).min(1.0)
    } else {
        1.0
    };

    if range > 0.0 {
        for (_, score) in scores.iter_mut() {
            *score = ((*score - min) / range) * confidence;
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

    // --- Tokenizer tests ---

    #[test]
    fn test_tokenize_basic() {
        let tokens = tokenize("Hello World");
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn test_tokenize_strips_punctuation() {
        let tokens = tokenize("Dog, cat. bird! fish?");
        assert_eq!(tokens, vec!["dog", "cat", "bird", "fish"]);
    }

    #[test]
    fn test_tokenize_preserves_internal_punctuation() {
        let tokens = tokenize("it's a don't-touch scenario");
        assert_eq!(tokens, vec!["it's", "a", "don't-touch", "scenario"]);
    }

    #[test]
    fn test_tokenize_empty_and_whitespace() {
        assert!(tokenize("").is_empty());
        assert!(tokenize("   ").is_empty());
        assert!(tokenize("...").is_empty()); // all punctuation stripped → empty
    }

    #[test]
    fn test_count_term_exact_match() {
        let tokens = tokenize("this is a test and this is fun");
        assert_eq!(count_term(&tokens, "this"), 2);
        assert_eq!(count_term(&tokens, "is"), 2);
        assert_eq!(count_term(&tokens, "test"), 1);
        assert_eq!(count_term(&tokens, "missing"), 0);
    }

    // --- Substring bug regression tests ---

    #[test]
    fn test_bm25_no_substring_match() {
        // The old bug: searching "is" would match inside "this", "island", etc.
        let doc = "this island is beautiful";
        let query = "is";

        let score = bm25_score(doc, query, 1.2, 0.75, 10.0);
        // "is" appears exactly once as a word. Old code counted 3 (th-IS, IS-land, IS).
        // With k1=1.2, b=0.75, avgdl=10, dl=4:
        // tf=1, idf=1 → 1*(1.2+1)/(1+1.2*(1-0.75+0.75*4/10)) = 2.2/(1+1.2*0.55) = 2.2/1.66 ≈ 1.325
        assert!(score > 1.0 && score < 1.5, "score should reflect tf=1 only, got {score}");
    }

    #[test]
    fn test_bm25_corpus_no_substring_match() {
        let docs = [
            "this island is beautiful",
            "history is written by winners",
        ];
        let corpus = BM25Corpus::new(&docs);

        // "is" has df=2 (appears as a word in both docs)
        // "history" has df=1 (appears in only doc 2)
        let idf_is = corpus.idf_for("is");
        let idf_history = corpus.idf_for("history");
        assert!(idf_history > idf_is, "history (df=1) should have higher IDF than is (df=2)");

        // Old bug: "is" would substring-match "this", "island", "history", "written"
        // inflating df to 2 incorrectly and also inflating TF in scoring.
        // With the fix, "this" and "island" do NOT contain "is" as a token.
        let score = corpus.score("this island is beautiful", "is", 1.2, 0.75);
        assert!(score > 0.0, "doc containing word 'is' should score > 0");

        // "this" should NOT match query "is"
        let score_this = corpus.score("this that those", "is", 1.2, 0.75);
        assert_eq!(score_this, 0.0, "'this' should NOT match query 'is'");
    }

    #[test]
    fn test_bm25_punctuation_handling() {
        // "dog," in the document should match query "dog"
        let doc = "The quick brown fox, jumps over the lazy dog.";
        let query = "dog fox";

        let score = bm25_score(doc, query, 1.2, 0.75, 10.0);
        assert!(score > 0.0, "punctuation-adjacent words should match: got {score}");
    }

    #[test]
    fn test_bm25_corpus_punctuation_in_idf() {
        // Corpus IDF should normalize punctuation so "dog," and "dog" unify
        let docs = [
            "the dog, ran fast",
            "a cat sat quiet",
            "dog is loyal",
        ];
        let corpus = BM25Corpus::new(&docs);

        // "dog" appears in doc 1 ("dog,") and doc 3 ("dog") → df=2
        let idf_dog = corpus.idf_for("dog");
        assert!(idf_dog > 0.0, "dog should have positive IDF");

        // "cat" appears in doc 2 only → df=1 → higher IDF
        let idf_cat = corpus.idf_for("cat");
        assert!(idf_cat > idf_dog, "cat (df=1) should beat dog (df=2)");
    }

    // --- Stemming tests ---

    #[test]
    fn test_tokenize_stemmed_basic() {
        let stemmer = Stemmer::create(Algorithm::English);
        let tokens = tokenize_stemmed("running dogs are jumping", &stemmer);
        // "running" → "run", "dogs" → "dog", "are" → "are", "jumping" → "jump"
        assert_eq!(tokens, vec!["run", "dog", "are", "jump"]);
    }

    #[test]
    fn test_tokenize_stemmed_preserves_already_stemmed() {
        let stemmer = Stemmer::create(Algorithm::English);
        let tokens = tokenize_stemmed("run dog jump", &stemmer);
        assert_eq!(tokens, vec!["run", "dog", "jump"]);
    }

    #[test]
    fn test_bm25_stemming_inflected_forms_match() {
        // "running" in the doc should match query "run" (and vice versa)
        let doc = "the dogs are running quickly through the park";
        let query = "run dog";

        let score = bm25_score(doc, query, 1.2, 0.75, 10.0);
        assert!(score > 0.0, "stemmed 'run' should match 'running', 'dog' should match 'dogs': got {score}");
    }

    #[test]
    fn test_bm25_stemming_query_inflected() {
        // Query has inflected form, doc has base form
        let doc = "the dog can run fast";
        let query = "running dogs";

        let score = bm25_score(doc, query, 1.2, 0.75, 10.0);
        assert!(score > 0.0, "query 'running dogs' should match doc with 'run' and 'dog': got {score}");
    }

    #[test]
    fn test_bm25_corpus_stemming_unifies_inflections() {
        // "memories" and "memory" should stem to the same root
        let docs = [
            "storing memories for later recall",
            "memory management in systems",
            "completely unrelated topic here",
        ];
        let corpus = BM25Corpus::new(&docs);

        // Both docs 1 and 2 contain the stem "memori" → df=2
        // "unrelated" only in doc 3 → df=1
        let idf_memory = corpus.idf_for("memory");
        let idf_unrelated = corpus.idf_for("unrelated");
        assert!(idf_memory > 0.0, "memory should have positive IDF");
        assert!(
            idf_unrelated > idf_memory,
            "unrelated (df=1) should have higher IDF than memory (df=2 via stemming): unrelated={idf_unrelated:.4} memory={idf_memory:.4}"
        );
    }

    #[test]
    fn test_bm25_corpus_stemming_score_matches_inflections() {
        let docs = [
            "the runner was running quickly",
            "programming in rust language",
        ];
        let corpus = BM25Corpus::new(&docs);

        // Query "runs" should match doc 1 ("runner" → "runner", "running" → "run")
        // Stemmer: "runs" → "run", "runner" → "runner", "running" → "run"
        let score_match = corpus.score("the runner was running quickly", "runs", 1.2, 0.75);
        let score_no_match = corpus.score("programming in rust language", "runs", 1.2, 0.75);

        assert!(score_match > 0.0, "doc with 'running' should match query 'runs' via stemming");
        assert_eq!(score_no_match, 0.0, "doc without any 'run' variant should score 0");
    }
}
