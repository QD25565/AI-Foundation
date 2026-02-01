//! Hybrid recall - combining vector, keyword, graph, and recency signals

use crate::note::Note;

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

/// Compute recency score (exponential decay)
pub fn recency_score(timestamp: i64, half_life_hours: f32) -> f32 {
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let age_nanos = (now - timestamp).max(0) as f64;
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
}
