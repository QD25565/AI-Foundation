//! Legacy graph index for backward compatibility
//!
//! This module contains the original GraphIndex implementation that uses
//! HashMap-based storage. It remains for backward compatibility with existing
//! code and serialized data. New code should use the KnowledgeGraph and CSR storage.

use std::collections::HashMap;

/// Legacy edge type between notes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeType {
    /// High cosine similarity between embeddings
    Semantic,
    /// Created within temporal window
    Temporal,
    /// Explicitly linked by user/AI
    Manual,
    /// Shared tag
    Tag,
}

impl EdgeType {
    /// Convert to byte for serialization
    pub fn to_byte(&self) -> u8 {
        match self {
            EdgeType::Semantic => 0,
            EdgeType::Temporal => 1,
            EdgeType::Manual => 2,
            EdgeType::Tag => 3,
        }
    }

    /// Parse from byte
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => EdgeType::Semantic,
            1 => EdgeType::Temporal,
            2 => EdgeType::Manual,
            _ => EdgeType::Tag,
        }
    }
}

/// A weighted edge (legacy format)
#[derive(Debug, Clone)]
pub struct Edge {
    pub target: u64,
    pub weight: f32,
    pub edge_type: EdgeType,
}

/// Legacy graph index for note relationships
///
/// Uses HashMap-based storage. For new code, prefer `KnowledgeGraph` with CSR storage.
pub struct GraphIndex {
    /// Forward edges: source -> [edges]
    forward: HashMap<u64, Vec<Edge>>,

    /// Reverse edges: target -> [source_ids]
    reverse: HashMap<u64, Vec<u64>>,

    /// Precomputed PageRank scores
    pagerank: HashMap<u64, f32>,
}

impl GraphIndex {
    /// Create a new empty graph index
    pub fn new() -> Self {
        Self {
            forward: HashMap::new(),
            reverse: HashMap::new(),
            pagerank: HashMap::new(),
        }
    }

    /// Add an edge between notes
    pub fn add_edge(&mut self, from: u64, to: u64, weight: f32, edge_type: EdgeType) {
        // Forward edge
        self.forward
            .entry(from)
            .or_insert_with(Vec::new)
            .push(Edge {
                target: to,
                weight,
                edge_type,
            });

        // Reverse edge
        self.reverse
            .entry(to)
            .or_insert_with(Vec::new)
            .push(from);
    }

    /// Invalidate (soft-delete) an edge, removing it from graph scoring
    ///
    /// For the legacy GraphIndex layer, invalidation is equivalent to removal.
    /// Callers should follow up with `compute_pagerank()` and `persist_indexes()`
    /// to propagate the structural change.
    ///
    /// The CsrGraph layer (KG 2.0) uses proper t_invalid timestamps instead.
    pub fn invalidate_edge(&mut self, from: u64, to: u64) -> bool {
        self.remove_edge(from, to)
    }

    /// Remove an edge between notes
    pub fn remove_edge(&mut self, from: u64, to: u64) -> bool {
        let mut removed = false;

        // Remove forward edge
        if let Some(edges) = self.forward.get_mut(&from) {
            let len_before = edges.len();
            edges.retain(|e| e.target != to);
            removed = edges.len() < len_before;
        }

        // Remove reverse edge
        if let Some(sources) = self.reverse.get_mut(&to) {
            sources.retain(|&s| s != from);
        }

        removed
    }

    /// Get neighbors of a note
    pub fn neighbors(&self, id: u64) -> &[Edge] {
        self.forward.get(&id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Get PageRank score for a note
    pub fn get_pagerank(&self, id: u64) -> f32 {
        self.pagerank.get(&id).copied().unwrap_or(0.0)
    }

    /// Set PageRank score directly
    pub fn set_pagerank(&mut self, id: u64, score: f32) {
        self.pagerank.insert(id, score);
    }

    /// Compute PageRank scores
    pub fn compute_pagerank(&mut self, iterations: usize, damping: f32) {
        let nodes: Vec<u64> = self.forward.keys().chain(self.reverse.keys()).copied().collect();
        let n = nodes.len();

        if n == 0 {
            return;
        }

        // Initialize scores
        let initial = 1.0 / n as f32;
        for &node in &nodes {
            self.pagerank.insert(node, initial);
        }

        // Iterate
        for _ in 0..iterations {
            let mut new_scores = HashMap::new();

            for &node in &nodes {
                let mut score = (1.0 - damping) / n as f32;

                // Sum contributions from incoming edges
                if let Some(sources) = self.reverse.get(&node) {
                    for &source in sources {
                        let source_score = self.pagerank.get(&source).copied().unwrap_or(0.0);
                        let out_degree = self.forward.get(&source).map(|v| v.len()).unwrap_or(1);
                        score += damping * source_score / out_degree as f32;
                    }
                }

                new_scores.insert(node, score);
            }

            self.pagerank = new_scores;
        }
    }

    /// Number of edges
    pub fn edge_count(&self) -> usize {
        self.forward.values().map(|v| v.len()).sum()
    }

    /// Number of nodes
    pub fn node_count(&self) -> usize {
        let mut nodes: std::collections::HashSet<u64> = self.forward.keys().copied().collect();
        nodes.extend(self.reverse.keys().copied());
        nodes.len()
    }

    /// Get sample of node IDs that have outgoing edges
    pub fn sample_nodes(&self, limit: usize) -> Vec<u64> {
        self.forward.keys().take(limit).copied().collect()
    }

    /// Remove all edges involving a note
    pub fn remove_node(&mut self, id: u64) {
        self.forward.remove(&id);
        self.reverse.remove(&id);
        self.pagerank.remove(&id);

        // Remove edges pointing to this node
        for edges in self.forward.values_mut() {
            edges.retain(|e| e.target != id);
        }
        for sources in self.reverse.values_mut() {
            sources.retain(|&s| s != id);
        }
    }

    /// Serialize graph for persistence
    pub fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::new();

        // Write edge count
        let edge_count = self.edge_count() as u64;
        data.extend_from_slice(&edge_count.to_le_bytes());

        // Write all edges (from, to, weight, type)
        for (&from, edges) in &self.forward {
            for edge in edges {
                data.extend_from_slice(&from.to_le_bytes());
                data.extend_from_slice(&edge.target.to_le_bytes());
                data.extend_from_slice(&edge.weight.to_le_bytes());
                data.push(edge.edge_type.to_byte());
            }
        }

        // Write pagerank scores
        let pr_count = self.pagerank.len() as u64;
        data.extend_from_slice(&pr_count.to_le_bytes());
        for (&id, &score) in &self.pagerank {
            data.extend_from_slice(&id.to_le_bytes());
            data.extend_from_slice(&score.to_le_bytes());
        }

        data
    }

    /// Deserialize graph from persisted data
    pub fn deserialize(&mut self, data: &[u8]) -> Result<(), &'static str> {
        if data.len() < 8 {
            return Ok(()); // Empty is valid
        }

        let mut offset = 0;

        // Read edge count
        let edge_count = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap()) as usize;
        offset += 8;

        // Read edges
        for _ in 0..edge_count {
            if offset + 21 > data.len() {
                return Err("Graph data truncated");
            }

            let from = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
            offset += 8;
            let to = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
            offset += 8;
            let weight = f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
            offset += 4;
            let edge_type = EdgeType::from_byte(data[offset]);
            offset += 1;

            self.add_edge(from, to, weight, edge_type);
        }

        // Read pagerank scores
        if offset + 8 > data.len() {
            return Ok(()); // Pagerank optional
        }
        let pr_count = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap()) as usize;
        offset += 8;

        for _ in 0..pr_count {
            if offset + 12 > data.len() {
                break;
            }
            let id = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
            offset += 8;
            let score = f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
            offset += 4;
            self.pagerank.insert(id, score);
        }

        Ok(())
    }

    /// Clear all data
    pub fn clear(&mut self) {
        self.forward.clear();
        self.reverse.clear();
        self.pagerank.clear();
    }
}

impl Default for GraphIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_edge() {
        let mut graph = GraphIndex::new();
        graph.add_edge(1, 2, 0.5, EdgeType::Semantic);

        let neighbors = graph.neighbors(1);
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].target, 2);
        assert_eq!(neighbors[0].weight, 0.5);
    }

    #[test]
    fn test_pagerank() {
        let mut graph = GraphIndex::new();

        // Simple graph: 1 -> 2 -> 3 -> 1
        graph.add_edge(1, 2, 1.0, EdgeType::Manual);
        graph.add_edge(2, 3, 1.0, EdgeType::Manual);
        graph.add_edge(3, 1, 1.0, EdgeType::Manual);

        graph.compute_pagerank(20, 0.85);

        // All nodes should have similar PageRank in a cycle
        let pr1 = graph.get_pagerank(1);
        let pr2 = graph.get_pagerank(2);
        let pr3 = graph.get_pagerank(3);

        assert!((pr1 - pr2).abs() < 0.01);
        assert!((pr2 - pr3).abs() < 0.01);
    }

    #[test]
    fn test_serialization() {
        let mut graph = GraphIndex::new();
        graph.add_edge(1, 2, 0.8, EdgeType::Semantic);
        graph.add_edge(2, 3, 0.6, EdgeType::Temporal);
        graph.compute_pagerank(5, 0.85);

        let data = graph.serialize();

        let mut loaded = GraphIndex::new();
        loaded.deserialize(&data).unwrap();

        assert_eq!(graph.edge_count(), loaded.edge_count());
    }
}
