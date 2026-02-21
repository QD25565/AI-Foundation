//! Engram Knowledge Graph 2.0 - Graph Module
//!
//! This module provides the core graph infrastructure for knowledge representation:
//!
//! - `legacy`: Original HashMap-based GraphIndex (backward compatibility)
//! - `types`: Extended edge taxonomy (structural, semantic, causal, temporal)
//! - `csr`: Cache-friendly CSR (Compressed Sparse Row) graph storage
//!
//! # Architecture
//!
//! The knowledge graph uses a hybrid storage approach:
//! - CSR format for cache-friendly traversal and O(1) neighbor access
//! - Dynamic update buffer for efficient insertions without full rebuild
//! - Bidirectional edge indexing (both outgoing and incoming)
//!
//! # Edge Types
//!
//! The system supports 20+ relationship types across 4 categories:
//!
//! - **Structural**: References, Continues, Supersedes, Contains, DerivedFrom
//! - **Semantic**: IsA, PartOf, RelatedTo, SimilarTo, SynonymOf, AntonymOf, InstanceOf, HasProperty
//! - **Causal**: Causes, Implies, Contradicts, Supports, Prevents, Enables, Requires
//! - **Temporal**: Before, After, During, TriggeredBy, Concurrent, TemporalProximity
//!
//! # Backward Compatibility
//!
//! The legacy `GraphIndex` and `EdgeType` are still available for existing code:
//! ```rust,ignore
//! use engram::graph::{GraphIndex, EdgeType}; // Legacy types
//! ```
//!
//! # New API (Knowledge Graph 2.0)
//!
//! ```rust,ignore
//! use engram::graph::{KnowledgeGraph, Edge as KgEdge};
//! use engram::graph::types::{EdgeType as KgEdgeType, SemanticEdge};
//!
//! // Create a knowledge graph with CSR storage
//! let mut kg = KnowledgeGraph::new();
//!
//! // Add edges with rich type taxonomy
//! kg.add_edge(KgEdge::new(
//!     note_1_id,
//!     note_2_id,
//!     KgEdgeType::Semantic(SemanticEdge::IsA),
//!     0.95,
//! ));
//!
//! // Query edges
//! let neighbors = kg.outgoing(note_1_id);
//! for (target, data) in neighbors {
//!     println!("  -> {} ({:?})", target, data.edge_type());
//! }
//! ```

// Sub-modules
pub mod legacy;
pub mod types;
pub mod csr;
pub mod traversal;
pub mod inference;
pub mod entity;
pub mod health;
pub mod query;

// ============================================================================
// Legacy Re-exports (Backward Compatibility)
// ============================================================================

// Re-export legacy types at module root for backward compatibility
// Existing code using `use engram::graph::{GraphIndex, EdgeType}` will continue to work
pub use legacy::{
    GraphIndex,
    EdgeType,
    Edge,
};

// ============================================================================
// Knowledge Graph 2.0 Re-exports
// ============================================================================

// New edge type taxonomy (use types:: prefix to avoid conflicts with legacy EdgeType)
pub mod kg2 {
    pub use super::types::{
        EdgeType,
        StructuralEdge,
        SemanticEdge,
        CausalEdge,
        TemporalEdge,
        LegacyEdge,
        Edge,
    };
    pub use super::csr::{
        CsrGraph,
        EdgeData,
        CsrMemoryStats,
    };
    pub use super::{
        KnowledgeGraph,
        NodeMetadata,
        GraphStats,
    };
}

// Also export CSR types directly (no conflict with legacy)
pub use csr::{
    CsrGraph,
    EdgeData,
    CsrMemoryStats,
};

// Export traversal types
pub use traversal::{
    Direction,
    EdgeFilter,
    Path,
    TraversalResult,
    // BFS
    bfs,
    shortest_path_bfs,
    // DFS
    dfs,
    find_path_dfs,
    // Dijkstra
    dijkstra,
    dijkstra_by_strength,
    // A*
    astar,
    null_heuristic,
    Heuristic,
    // Multi-path
    find_all_paths,
    n_hop_neighbors,
    nodes_at_distance,
};

// Export inference types
pub use inference::{
    // Transitive closure
    TransitiveClosure,
    TransitiveClosureStats,
    ReachabilityInfo,
    // Inference engine
    InferenceEngine,
    InferenceRule,
    RuleType,
    InferenceResult,
    // Contradiction detection
    ContradictionDetector,
    Contradiction,
};

// Export entity types
pub use entity::{
    EntityType,
    Entity,
    EntityExtractor,
    EntityIndex,
    EntityIndexStats,
    AutoLinker,
};

// Export health check types
pub use health::{
    HealthChecker,
    HealthReport,
    HealthStatus,
    GraphHealthStats,
    HealthIssue,
    IssueSeverity,
    IssueCategory,
    quick_summary,
    detailed_report,
};

// Export query types
pub use query::{
    Query,
    QueryResult,
    QueryExecutor,
    QueryBuilder,
    ReasoningChain,
    ReasoningStep,
};

// ============================================================================
// Knowledge Graph Integration
// ============================================================================

use std::collections::HashMap;

// Use the new types for KnowledgeGraph (not the legacy re-exports)
use types::Edge as KgEdge;
use types::EdgeType as KgEdgeType;

/// Knowledge Graph with CSR storage and advanced features
///
/// This is the high-level interface that will be used by Engram for
/// note relationships, inference, and querying.
pub struct KnowledgeGraph {
    /// Core CSR storage
    graph: CsrGraph,

    /// Node metadata (maps node ID to additional info)
    node_metadata: HashMap<u64, NodeMetadata>,

    /// Statistics for optimization
    stats: GraphStats,
}

/// Metadata about a node in the knowledge graph
#[derive(Debug, Clone, Default)]
pub struct NodeMetadata {
    /// PageRank score (for importance ranking)
    pub pagerank: f32,
    /// Number of times this node has been accessed
    pub access_count: u64,
    /// Last access timestamp
    pub last_accessed: i64,
    /// Whether this node represents a pinned note
    pub pinned: bool,
    /// Tags associated with this node
    pub tags: Vec<String>,
}

/// Graph statistics for optimization and monitoring
#[derive(Debug, Clone, Default)]
pub struct GraphStats {
    /// Total nodes
    pub node_count: usize,
    /// Total edges
    pub edge_count: usize,
    /// Edges by type
    pub edge_type_counts: HashMap<u8, usize>,
    /// Average out-degree
    pub avg_out_degree: f32,
    /// Maximum out-degree
    pub max_out_degree: usize,
    /// Number of inferred edges
    pub inferred_edge_count: usize,
}

impl KnowledgeGraph {
    /// Create a new empty knowledge graph
    pub fn new() -> Self {
        Self {
            graph: CsrGraph::new(),
            node_metadata: HashMap::new(),
            stats: GraphStats::default(),
        }
    }

    /// Create from existing edges
    pub fn from_edges(edges: &[KgEdge]) -> Self {
        let graph = CsrGraph::from_edges(edges);
        let mut kg = Self {
            graph,
            node_metadata: HashMap::new(),
            stats: GraphStats::default(),
        };
        kg.update_stats();
        kg
    }

    /// Add an edge to the graph
    pub fn add_edge(&mut self, edge: KgEdge) {
        // Ensure nodes exist in metadata
        self.ensure_node(edge.source);
        self.ensure_node(edge.target);

        // Add to CSR
        self.graph.add_edge(edge);

        // Update stats
        self.stats.edge_count += 1;
    }

    /// Add multiple edges
    pub fn add_edges(&mut self, edges: impl IntoIterator<Item = KgEdge>) {
        for edge in edges {
            self.add_edge(edge);
        }
    }

    /// Ensure a node exists in metadata
    fn ensure_node(&mut self, node_id: u64) {
        self.node_metadata.entry(node_id).or_insert_with(|| {
            self.stats.node_count += 1;
            NodeMetadata::default()
        });
    }

    /// Get outgoing edges from a node
    pub fn outgoing(&self, node_id: u64) -> Vec<(u64, EdgeData)> {
        self.graph.outgoing_edges(node_id)
    }

    /// Get incoming edges to a node
    pub fn incoming(&self, node_id: u64) -> Vec<(u64, EdgeData)> {
        self.graph.incoming_edges(node_id)
    }

    /// Get all neighbors (bidirectional)
    pub fn neighbors(&self, node_id: u64) -> Vec<u64> {
        self.graph.neighbors(node_id)
    }

    /// Get outgoing edges of a specific type
    pub fn outgoing_of_type(&self, node_id: u64, edge_type: KgEdgeType) -> Vec<(u64, EdgeData)> {
        self.graph.outgoing_edges_of_type(node_id, edge_type)
    }

    /// Check if edge exists
    pub fn has_edge(&self, source: u64, target: u64) -> bool {
        self.graph.has_edge(source, target)
    }

    /// Check if typed edge exists
    pub fn has_edge_of_type(&self, source: u64, target: u64, edge_type: KgEdgeType) -> bool {
        self.graph.has_edge_of_type(source, target, edge_type)
    }

    /// Get node metadata
    pub fn get_metadata(&self, node_id: u64) -> Option<&NodeMetadata> {
        self.node_metadata.get(&node_id)
    }

    /// Update node metadata
    pub fn update_metadata(&mut self, node_id: u64, metadata: NodeMetadata) {
        self.node_metadata.insert(node_id, metadata);
    }

    /// Set PageRank for a node
    pub fn set_pagerank(&mut self, node_id: u64, score: f32) {
        self.ensure_node(node_id);
        if let Some(meta) = self.node_metadata.get_mut(&node_id) {
            meta.pagerank = score;
        }
    }

    /// Get PageRank for a node
    pub fn get_pagerank(&self, node_id: u64) -> f32 {
        self.node_metadata.get(&node_id)
            .map(|m| m.pagerank)
            .unwrap_or(0.0)
    }

    /// Record node access (for access tracking)
    pub fn record_access(&mut self, node_id: u64) {
        self.ensure_node(node_id);
        if let Some(meta) = self.node_metadata.get_mut(&node_id) {
            meta.access_count += 1;
            meta.last_accessed = chrono::Utc::now().timestamp();
        }
    }

    /// Set pinned status
    pub fn set_pinned(&mut self, node_id: u64, pinned: bool) {
        self.ensure_node(node_id);
        if let Some(meta) = self.node_metadata.get_mut(&node_id) {
            meta.pinned = pinned;
        }
    }

    /// Get all pinned nodes
    pub fn pinned_nodes(&self) -> Vec<u64> {
        self.node_metadata.iter()
            .filter(|(_, m)| m.pinned)
            .map(|(&id, _)| id)
            .collect()
    }

    /// Compact the underlying CSR storage
    pub fn compact(&mut self) {
        self.graph.compact();
        self.update_stats();
    }

    /// Update statistics
    fn update_stats(&mut self) {
        self.stats.node_count = self.graph.node_count();
        self.stats.edge_count = self.graph.edge_count();

        // Calculate edge type distribution
        self.stats.edge_type_counts.clear();
        for (_, _, data) in self.graph.iter_edges() {
            *self.stats.edge_type_counts.entry(data.edge_type).or_insert(0) += 1;
            if data.inferred {
                self.stats.inferred_edge_count += 1;
            }
        }

        // Calculate degree stats
        let nodes = self.graph.nodes();
        if !nodes.is_empty() {
            let mut total_degree = 0;
            let mut max_degree = 0;
            for node in &nodes {
                let degree = self.graph.out_degree(*node);
                total_degree += degree;
                max_degree = max_degree.max(degree);
            }
            self.stats.avg_out_degree = total_degree as f32 / nodes.len() as f32;
            self.stats.max_out_degree = max_degree;
        }
    }

    /// Get graph statistics
    pub fn stats(&self) -> &GraphStats {
        &self.stats
    }

    /// Get memory statistics
    pub fn memory_stats(&self) -> CsrMemoryStats {
        self.graph.memory_stats()
    }

    /// Get the number of nodes
    pub fn node_count(&self) -> usize {
        self.stats.node_count
    }

    /// Get the number of edges
    pub fn edge_count(&self) -> usize {
        self.stats.edge_count
    }

    /// Iterate over all nodes
    pub fn nodes(&self) -> Vec<u64> {
        self.graph.nodes()
    }

    /// Iterate over all edges
    pub fn iter_edges(&self) -> impl Iterator<Item = (u64, u64, EdgeData)> + '_ {
        self.graph.iter_edges()
    }

    // ========================================================================
    // Serialization
    // ========================================================================

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // CSR graph
        let graph_bytes = self.graph.to_bytes();
        bytes.extend_from_slice(&(graph_bytes.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&graph_bytes);

        // Node metadata count
        bytes.extend_from_slice(&(self.node_metadata.len() as u64).to_le_bytes());

        // Each node's metadata
        for (&node_id, meta) in &self.node_metadata {
            bytes.extend_from_slice(&node_id.to_le_bytes());
            bytes.extend_from_slice(&meta.pagerank.to_le_bytes());
            bytes.extend_from_slice(&meta.access_count.to_le_bytes());
            bytes.extend_from_slice(&meta.last_accessed.to_le_bytes());
            bytes.push(if meta.pinned { 1 } else { 0 });

            // Tags
            bytes.extend_from_slice(&(meta.tags.len() as u32).to_le_bytes());
            for tag in &meta.tags {
                let tag_bytes = tag.as_bytes();
                bytes.extend_from_slice(&(tag_bytes.len() as u16).to_le_bytes());
                bytes.extend_from_slice(tag_bytes);
            }
        }

        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 16 {
            return None;
        }

        let mut offset = 0;

        // CSR graph
        let graph_len = u64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?) as usize;
        offset += 8;
        let graph = CsrGraph::from_bytes(&bytes[offset..offset+graph_len])?;
        offset += graph_len;

        // Node metadata count
        let meta_count = u64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?) as usize;
        offset += 8;

        // Read metadata
        let mut node_metadata = HashMap::with_capacity(meta_count);
        for _ in 0..meta_count {
            if offset + 29 > bytes.len() {
                break;
            }

            let node_id = u64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?);
            offset += 8;
            let pagerank = f32::from_le_bytes(bytes[offset..offset+4].try_into().ok()?);
            offset += 4;
            let access_count = u64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?);
            offset += 8;
            let last_accessed = i64::from_le_bytes(bytes[offset..offset+8].try_into().ok()?);
            offset += 8;
            let pinned = bytes[offset] != 0;
            offset += 1;

            // Tags
            let tag_count = u32::from_le_bytes(bytes[offset..offset+4].try_into().ok()?) as usize;
            offset += 4;
            let mut tags = Vec::with_capacity(tag_count);
            for _ in 0..tag_count {
                if offset + 2 > bytes.len() {
                    break;
                }
                let tag_len = u16::from_le_bytes(bytes[offset..offset+2].try_into().ok()?) as usize;
                offset += 2;
                if offset + tag_len <= bytes.len() {
                    if let Ok(tag) = String::from_utf8(bytes[offset..offset+tag_len].to_vec()) {
                        tags.push(tag);
                    }
                    offset += tag_len;
                }
            }

            node_metadata.insert(node_id, NodeMetadata {
                pagerank,
                access_count,
                last_accessed,
                pinned,
                tags,
            });
        }

        let mut kg = Self {
            graph,
            node_metadata,
            stats: GraphStats::default(),
        };
        kg.update_stats();

        Some(kg)
    }
}

impl Default for KnowledgeGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use super::types::{Edge as KgEdge, EdgeType as KgEdgeType, SemanticEdge, CausalEdge};

    #[test]
    fn test_knowledge_graph_basic() {
        let mut kg = KnowledgeGraph::new();

        kg.add_edge(KgEdge::new(1, 2, KgEdgeType::Semantic(SemanticEdge::IsA), 0.9));
        kg.add_edge(KgEdge::new(1, 3, KgEdgeType::Semantic(SemanticEdge::RelatedTo), 0.8));

        assert_eq!(kg.node_count(), 3);
        assert_eq!(kg.edge_count(), 2);

        let out = kg.outgoing(1);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn test_metadata() {
        let mut kg = KnowledgeGraph::new();
        kg.add_edge(KgEdge::new(1, 2, KgEdgeType::Semantic(SemanticEdge::SimilarTo), 1.0));

        kg.set_pagerank(1, 0.85);
        kg.set_pinned(1, true);
        kg.record_access(1);

        let meta = kg.get_metadata(1).unwrap();
        assert_eq!(meta.pagerank, 0.85);
        assert!(meta.pinned);
        assert_eq!(meta.access_count, 1);
    }

    #[test]
    fn test_serialization() {
        let mut kg = KnowledgeGraph::new();
        kg.add_edge(KgEdge::new(100, 200, KgEdgeType::Causal(CausalEdge::Causes), 0.7));
        kg.set_pagerank(100, 0.5);
        kg.compact();

        let bytes = kg.to_bytes();
        let recovered = KnowledgeGraph::from_bytes(&bytes).expect("Should deserialize");

        assert_eq!(kg.node_count(), recovered.node_count());
        assert_eq!(kg.edge_count(), recovered.edge_count());
        assert!(recovered.has_edge(100, 200));
    }
}
