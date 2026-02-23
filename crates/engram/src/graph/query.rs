//! Query Language and Reasoning Chains for Engram Knowledge Graph 2.0
//!
//! Provides a simple query language for knowledge graph exploration:
//! - Path queries: Find paths between nodes
//! - Neighbor queries: Get related nodes by type
//! - Reasoning chains: Explain why two nodes are connected
//! - Pattern matching: Find nodes matching criteria
//!
//! Query Syntax Examples:
//! - `path(1, 2)` - Find shortest path from node 1 to node 2
//! - `neighbors(1, RelatedTo)` - Get nodes related to node 1
//! - `explain(1, 2)` - Explain connection between nodes
//! - `reachable(1, IsA, 3)` - Check if node 2 is reachable from 1 via IsA edges in 3 hops

use std::collections::HashSet;
use super::csr::CsrGraph;
use super::types::{EdgeType, SemanticEdge, CausalEdge, TemporalEdge, StructuralEdge};
use super::traversal::{Direction, EdgeFilter, Path, bfs, dijkstra, find_all_paths};
use super::inference::TransitiveClosure;

// ============================================================================
// Query Types
// ============================================================================

/// A parsed query
#[derive(Debug, Clone)]
pub enum Query {
    /// Find path between two nodes
    Path {
        source: u64,
        target: u64,
        max_hops: Option<usize>,
        edge_filter: Option<EdgeFilter>,
    },

    /// Find all paths between two nodes
    AllPaths {
        source: u64,
        target: u64,
        max_paths: usize,
        max_depth: usize,
    },

    /// Get neighbors of a node
    Neighbors {
        node: u64,
        edge_type: Option<EdgeType>,
        direction: Direction,
        depth: usize,
    },

    /// Check reachability
    Reachable {
        source: u64,
        target: u64,
        edge_types: Option<Vec<EdgeType>>,
        max_hops: Option<usize>,
    },

    /// Explain connection between nodes
    Explain {
        source: u64,
        target: u64,
    },

    /// Find nodes with specific properties
    Find {
        edge_type: EdgeType,
        direction: Direction,
        from_node: Option<u64>,
    },

    /// Get transitive closure for a node
    Closure {
        node: u64,
        edge_types: Vec<EdgeType>,
    },

    /// Run inference and get new edges
    Infer {
        max_iterations: usize,
    },
}

/// Result of a query execution
#[derive(Debug, Clone)]
pub enum QueryResult {
    /// Path result
    Path(Option<Path>),

    /// Multiple paths
    Paths(Vec<Path>),

    /// List of nodes
    Nodes(Vec<u64>),

    /// Node with edge info
    NodesWithEdges(Vec<(u64, EdgeType, f32)>),

    /// Boolean result
    Bool(bool),

    /// Explanation/reasoning chain
    Explanation(ReasoningChain),

    /// Inference result
    InferenceResult {
        new_edges: usize,
        iterations: usize,
    },

    /// Error
    Error(String),
}

// ============================================================================
// Reasoning Chains
// ============================================================================

/// A step in a reasoning chain
#[derive(Debug, Clone)]
pub struct ReasoningStep {
    /// Source node
    pub from: u64,
    /// Target node
    pub to: u64,
    /// Edge type used
    pub edge_type: EdgeType,
    /// Confidence of this step
    pub confidence: f32,
    /// Whether this edge was inferred
    pub inferred: bool,
    /// Human-readable explanation
    pub explanation: String,
}

/// A complete reasoning chain explaining a connection
#[derive(Debug, Clone)]
pub struct ReasoningChain {
    /// Source of the query
    pub source: u64,
    /// Target of the query
    pub target: u64,
    /// Steps in the chain
    pub steps: Vec<ReasoningStep>,
    /// Overall confidence (product of step confidences)
    pub total_confidence: f32,
    /// Total path length
    pub path_length: usize,
    /// Human-readable summary
    pub summary: String,
}

impl ReasoningChain {
    /// Create a new reasoning chain
    pub fn new(source: u64, target: u64) -> Self {
        Self {
            source,
            target,
            steps: Vec::new(),
            total_confidence: 1.0,
            path_length: 0,
            summary: String::new(),
        }
    }

    /// Add a step to the chain
    pub fn add_step(&mut self, step: ReasoningStep) {
        self.total_confidence *= step.confidence;
        self.path_length += 1;
        self.steps.push(step);
    }

    /// Generate human-readable summary
    pub fn generate_summary(&mut self) {
        if self.steps.is_empty() {
            self.summary = format!("No connection found between {} and {}", self.source, self.target);
            return;
        }

        let mut parts = Vec::new();
        for step in &self.steps {
            parts.push(format!(
                "{} --[{:?}]--> {}",
                step.from,
                step.edge_type,
                step.to
            ));
        }

        self.summary = format!(
            "Connection: {} (confidence: {:.2}, {} hops)",
            parts.join(" => "),
            self.total_confidence,
            self.path_length
        );
    }

    /// Format as detailed explanation
    pub fn to_detailed_string(&self) -> String {
        let mut output = String::new();

        output.push_str(&format!("=== Reasoning Chain: {} -> {} ===\n", self.source, self.target));
        output.push_str(&format!("Total Confidence: {:.2}\n", self.total_confidence));
        output.push_str(&format!("Path Length: {} hops\n\n", self.path_length));

        for (i, step) in self.steps.iter().enumerate() {
            output.push_str(&format!("Step {}: {}\n", i + 1, step.explanation));
            output.push_str(&format!(
                "  {} --[{:?}]--> {} (confidence: {:.2}{})\n",
                step.from,
                step.edge_type,
                step.to,
                step.confidence,
                if step.inferred { ", inferred" } else { "" }
            ));
        }

        output
    }
}

// ============================================================================
// Query Executor
// ============================================================================

/// Executes queries against the knowledge graph
pub struct QueryExecutor<'a> {
    graph: &'a CsrGraph,
}

impl<'a> QueryExecutor<'a> {
    /// Create a new query executor
    pub fn new(graph: &'a CsrGraph) -> Self {
        Self { graph }
    }

    /// Execute a query
    pub fn execute(&self, query: Query) -> QueryResult {
        match query {
            Query::Path { source, target, max_hops, edge_filter } => {
                self.execute_path(source, target, max_hops, edge_filter)
            }
            Query::AllPaths { source, target, max_paths, max_depth } => {
                self.execute_all_paths(source, target, max_paths, max_depth)
            }
            Query::Neighbors { node, edge_type, direction, depth } => {
                self.execute_neighbors(node, edge_type, direction, depth)
            }
            Query::Reachable { source, target, edge_types, max_hops } => {
                self.execute_reachable(source, target, edge_types, max_hops)
            }
            Query::Explain { source, target } => {
                self.execute_explain(source, target)
            }
            Query::Find { edge_type, direction, from_node } => {
                self.execute_find(edge_type, direction, from_node)
            }
            Query::Closure { node, edge_types } => {
                self.execute_closure(node, edge_types)
            }
            Query::Infer { .. } => {
                // Inference requires mutable graph, return error
                QueryResult::Error("Inference requires mutable graph access".to_string())
            }
        }
    }

    /// Execute path query
    fn execute_path(
        &self,
        source: u64,
        target: u64,
        _max_hops: Option<usize>,
        edge_filter: Option<EdgeFilter>,
    ) -> QueryResult {
        let filter = edge_filter.unwrap_or(EdgeFilter::All);

        let path = dijkstra(
            self.graph,
            source,
            target,
            Direction::Outgoing,
            &filter,
        );

        QueryResult::Path(path)
    }

    /// Execute all paths query
    fn execute_all_paths(
        &self,
        source: u64,
        target: u64,
        max_paths: usize,
        max_depth: usize,
    ) -> QueryResult {
        let paths = find_all_paths(
            self.graph,
            source,
            target,
            max_depth,
            Direction::Outgoing,
            &EdgeFilter::All,
            max_paths,
        );

        QueryResult::Paths(paths)
    }

    /// Execute neighbors query
    fn execute_neighbors(
        &self,
        node: u64,
        edge_type: Option<EdgeType>,
        direction: Direction,
        depth: usize,
    ) -> QueryResult {
        let mut results: Vec<(u64, EdgeType, f32)> = Vec::new();

        if depth == 1 {
            // Direct neighbors
            let edges = match direction {
                Direction::Outgoing => self.graph.outgoing_edges(node),
                Direction::Incoming => self.graph.incoming_edges(node),
                Direction::Both => {
                    let mut all = self.graph.outgoing_edges(node);
                    all.extend(self.graph.incoming_edges(node));
                    all
                }
            };

            for (neighbor, data) in edges {
                if let Some(ref et) = edge_type {
                    if let Some(actual_type) = EdgeType::from_byte(data.edge_type) {
                        if actual_type == *et {
                            results.push((neighbor, actual_type, data.confidence));
                        }
                    }
                } else if let Some(actual_type) = EdgeType::from_byte(data.edge_type) {
                    results.push((neighbor, actual_type, data.confidence));
                }
            }
        } else {
            // Multi-hop neighbors via BFS
            let filter = match edge_type {
                Some(et) => EdgeFilter::Include(vec![et]),
                None => EdgeFilter::All,
            };

            let traversal = bfs(self.graph, node, depth, direction, &filter);

            for n in traversal.visited {
                if n != node {
                    // Get edge type from path (simplified - just use RelatedTo)
                    results.push((n, EdgeType::Semantic(SemanticEdge::RelatedTo), 0.8));
                }
            }
        }

        QueryResult::NodesWithEdges(results)
    }

    /// Execute reachability query
    fn execute_reachable(
        &self,
        source: u64,
        target: u64,
        edge_types: Option<Vec<EdgeType>>,
        max_hops: Option<usize>,
    ) -> QueryResult {
        let filter = match edge_types {
            Some(types) => EdgeFilter::Include(types),
            None => EdgeFilter::All,
        };
        let max_depth = max_hops.unwrap_or(10);

        let traversal = bfs(self.graph, source, max_depth, Direction::Outgoing, &filter);

        let reachable = traversal.visited.contains(&target);

        QueryResult::Bool(reachable)
    }

    /// Execute explain query - build reasoning chain
    fn execute_explain(&self, source: u64, target: u64) -> QueryResult {
        // Find path first
        let path = dijkstra(
            self.graph,
            source,
            target,
            Direction::Outgoing,
            &EdgeFilter::All,
        );

        let mut chain = ReasoningChain::new(source, target);

        if let Some(p) = path {
            // Build reasoning steps from path
            for i in 0..p.nodes.len().saturating_sub(1) {
                let from = p.nodes[i];
                let to = p.nodes[i + 1];

                // Get edge info
                let edges = self.graph.outgoing_edges(from);
                if let Some((_, data)) = edges.iter().find(|(t, _)| *t == to) {
                    let edge_type = EdgeType::from_byte(data.edge_type)
                        .unwrap_or(EdgeType::Semantic(SemanticEdge::RelatedTo));

                    let explanation = self.explain_edge(from, to, &edge_type, data.inferred);

                    chain.add_step(ReasoningStep {
                        from,
                        to,
                        edge_type,
                        confidence: data.confidence,
                        inferred: data.inferred,
                        explanation,
                    });
                }
            }
        }

        chain.generate_summary();
        QueryResult::Explanation(chain)
    }

    /// Generate explanation for an edge
    fn explain_edge(&self, from: u64, to: u64, edge_type: &EdgeType, inferred: bool) -> String {
        let inference_note = if inferred { " (inferred)" } else { "" };

        match edge_type {
            EdgeType::Semantic(se) => match se {
                SemanticEdge::IsA => format!("Node {} is a type of {}{}", from, to, inference_note),
                SemanticEdge::PartOf => format!("Node {} is part of {}{}", from, to, inference_note),
                SemanticEdge::RelatedTo => format!("Node {} is related to {}{}", from, to, inference_note),
                SemanticEdge::SimilarTo => format!("Node {} is similar to {}{}", from, to, inference_note),
                SemanticEdge::SynonymOf => format!("Node {} is a synonym of {}{}", from, to, inference_note),
                SemanticEdge::AntonymOf => format!("Node {} is an antonym of {}{}", from, to, inference_note),
                SemanticEdge::InstanceOf => format!("Node {} is an instance of {}{}", from, to, inference_note),
                SemanticEdge::HasProperty => format!("Node {} has property {}{}", from, to, inference_note),
            },
            EdgeType::Causal(ce) => match ce {
                CausalEdge::Causes => format!("Node {} causes {}{}", from, to, inference_note),
                CausalEdge::Implies => format!("Node {} implies {}{}", from, to, inference_note),
                CausalEdge::Contradicts => format!("Node {} contradicts {}{}", from, to, inference_note),
                CausalEdge::Supports => format!("Node {} supports {}{}", from, to, inference_note),
                CausalEdge::Prevents => format!("Node {} prevents {}{}", from, to, inference_note),
                CausalEdge::Enables => format!("Node {} enables {}{}", from, to, inference_note),
                CausalEdge::Requires => format!("Node {} requires {}{}", from, to, inference_note),
            },
            EdgeType::Temporal(te) => match te {
                TemporalEdge::Before => format!("Node {} occurred before {}{}", from, to, inference_note),
                TemporalEdge::After => format!("Node {} occurred after {}{}", from, to, inference_note),
                TemporalEdge::During => format!("Node {} occurred during {}{}", from, to, inference_note),
                TemporalEdge::TriggeredBy => format!("Node {} was triggered by {}{}", from, to, inference_note),
                TemporalEdge::Concurrent => format!("Node {} is concurrent with {}{}", from, to, inference_note),
                TemporalEdge::TemporalProximity => format!("Node {} is temporally close to {}{}", from, to, inference_note),
            },
            EdgeType::Structural(st) => match st {
                StructuralEdge::References => format!("Node {} references {}{}", from, to, inference_note),
                StructuralEdge::Continues => format!("Node {} continues {}{}", from, to, inference_note),
                StructuralEdge::Supersedes => format!("Node {} supersedes {}{}", from, to, inference_note),
                StructuralEdge::Contains => format!("Node {} contains {}{}", from, to, inference_note),
                StructuralEdge::DerivedFrom => format!("Node {} is derived from {}{}", from, to, inference_note),
            },
            EdgeType::Legacy(_) => format!("Node {} is connected to {}{}", from, to, inference_note),
        }
    }

    /// Execute find query
    fn execute_find(
        &self,
        edge_type: EdgeType,
        direction: Direction,
        from_node: Option<u64>,
    ) -> QueryResult {
        let mut results = Vec::new();

        match from_node {
            Some(node) => {
                // Find nodes connected to specific node with edge type
                let edges = match direction {
                    Direction::Outgoing => self.graph.outgoing_edges_of_type(node, edge_type),
                    Direction::Incoming => self.graph.incoming_edges(node)
                        .into_iter()
                        .filter(|(_, data)| EdgeType::from_byte(data.edge_type) == Some(edge_type))
                        .collect(),
                    Direction::Both => {
                        let mut all = self.graph.outgoing_edges_of_type(node, edge_type);
                        all.extend(
                            self.graph.incoming_edges(node)
                                .into_iter()
                                .filter(|(_, data)| EdgeType::from_byte(data.edge_type) == Some(edge_type))
                        );
                        all
                    }
                };

                for (target, _) in edges {
                    results.push(target);
                }
            }
            None => {
                // Find all edges of this type
                for (source, target, data) in self.graph.iter_edges() {
                    if EdgeType::from_byte(data.edge_type) == Some(edge_type) {
                        match direction {
                            Direction::Outgoing => results.push(target),
                            Direction::Incoming => results.push(source),
                            Direction::Both => {
                                results.push(source);
                                results.push(target);
                            }
                        }
                    }
                }
            }
        }

        // Deduplicate
        let unique: HashSet<_> = results.into_iter().collect();
        QueryResult::Nodes(unique.into_iter().collect())
    }

    /// Execute transitive closure query
    fn execute_closure(&self, node: u64, _edge_types: Vec<EdgeType>) -> QueryResult {
        let tc = TransitiveClosure::compute(self.graph, 10);

        let reachable: Vec<u64> = tc.reachable_from(node)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        QueryResult::Nodes(reachable)
    }
}

// ============================================================================
// Query Builder (Fluent API)
// ============================================================================

/// Fluent query builder
pub struct QueryBuilder {
    query: Option<Query>,
}

impl QueryBuilder {
    /// Create a new query builder
    pub fn new() -> Self {
        Self { query: None }
    }

    /// Find path between nodes
    pub fn path(mut self, source: u64, target: u64) -> Self {
        self.query = Some(Query::Path {
            source,
            target,
            max_hops: None,
            edge_filter: None,
        });
        self
    }

    /// Find all paths between nodes
    pub fn all_paths(mut self, source: u64, target: u64) -> Self {
        self.query = Some(Query::AllPaths {
            source,
            target,
            max_paths: 10,
            max_depth: 5,
        });
        self
    }

    /// Get neighbors of a node
    pub fn neighbors(mut self, node: u64) -> Self {
        self.query = Some(Query::Neighbors {
            node,
            edge_type: None,
            direction: Direction::Both,
            depth: 1,
        });
        self
    }

    /// Check reachability
    pub fn reachable(mut self, source: u64, target: u64) -> Self {
        self.query = Some(Query::Reachable {
            source,
            target,
            edge_types: None,
            max_hops: None,
        });
        self
    }

    /// Explain connection
    pub fn explain(mut self, source: u64, target: u64) -> Self {
        self.query = Some(Query::Explain { source, target });
        self
    }

    /// Set max hops
    pub fn max_hops(mut self, hops: usize) -> Self {
        if let Some(Query::Path { ref mut max_hops, .. }) = self.query {
            *max_hops = Some(hops);
        }
        if let Some(Query::Reachable { ref mut max_hops, .. }) = self.query {
            *max_hops = Some(hops);
        }
        if let Some(Query::AllPaths { ref mut max_depth, .. }) = self.query {
            *max_depth = hops;
        }
        self
    }

    /// Set edge type filter
    pub fn with_edge_type(mut self, edge_type: EdgeType) -> Self {
        if let Some(Query::Neighbors { edge_type: ref mut et, .. }) = self.query {
            *et = Some(edge_type);
        }
        if let Some(Query::Reachable { ref mut edge_types, .. }) = self.query {
            *edge_types = Some(vec![edge_type]);
        }
        if let Some(Query::Path { ref mut edge_filter, .. }) = self.query {
            *edge_filter = Some(EdgeFilter::Include(vec![edge_type]));
        }
        self
    }

    /// Set direction
    pub fn direction(mut self, dir: Direction) -> Self {
        if let Some(Query::Neighbors { ref mut direction, .. }) = self.query {
            *direction = dir;
        }
        self
    }

    /// Set depth for neighbor search
    pub fn depth(mut self, d: usize) -> Self {
        if let Some(Query::Neighbors { ref mut depth, .. }) = self.query {
            *depth = d;
        }
        self
    }

    /// Build the query
    pub fn build(self) -> Option<Query> {
        self.query
    }

    /// Execute the query directly
    pub fn execute(self, graph: &CsrGraph) -> QueryResult {
        match self.query {
            Some(q) => QueryExecutor::new(graph).execute(q),
            None => QueryResult::Error("No query specified".to_string()),
        }
    }
}

impl Default for QueryBuilder {
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
    use crate::graph::types::Edge;

    fn create_test_graph() -> CsrGraph {
        let mut graph = CsrGraph::new();
        // Create a simple graph: 1 -> 2 -> 3 -> 4, with branch 2 -> 5
        graph.add_edge(Edge::new(1, 2, EdgeType::Semantic(SemanticEdge::IsA), 0.9));
        graph.add_edge(Edge::new(2, 3, EdgeType::Semantic(SemanticEdge::PartOf), 0.8));
        graph.add_edge(Edge::new(3, 4, EdgeType::Causal(CausalEdge::Causes), 0.7));
        graph.add_edge(Edge::new(2, 5, EdgeType::Semantic(SemanticEdge::RelatedTo), 0.6));
        graph.compact();
        graph
    }

    #[test]
    fn test_path_query() {
        let graph = create_test_graph();
        let executor = QueryExecutor::new(&graph);

        let result = executor.execute(Query::Path {
            source: 1,
            target: 4,
            max_hops: None,
            edge_filter: None,
        });

        match result {
            QueryResult::Path(Some(path)) => {
                assert_eq!(path.nodes.len(), 4);
                assert_eq!(path.nodes[0], 1);
                assert_eq!(path.nodes[3], 4);
            }
            _ => panic!("Expected path result"),
        }
    }

    #[test]
    fn test_neighbors_query() {
        let graph = create_test_graph();
        let executor = QueryExecutor::new(&graph);

        let result = executor.execute(Query::Neighbors {
            node: 2,
            edge_type: None,
            direction: Direction::Outgoing,
            depth: 1,
        });

        match result {
            QueryResult::NodesWithEdges(nodes) => {
                assert_eq!(nodes.len(), 2); // 3 and 5
                let node_ids: Vec<_> = nodes.iter().map(|(n, _, _)| *n).collect();
                assert!(node_ids.contains(&3));
                assert!(node_ids.contains(&5));
            }
            _ => panic!("Expected nodes result"),
        }
    }

    #[test]
    fn test_reachable_query() {
        let graph = create_test_graph();
        let executor = QueryExecutor::new(&graph);

        // 1 should be able to reach 4
        let result = executor.execute(Query::Reachable {
            source: 1,
            target: 4,
            edge_types: None,
            max_hops: None,
        });

        assert!(matches!(result, QueryResult::Bool(true)));

        // 4 should NOT be able to reach 1 (directed graph)
        let result = executor.execute(Query::Reachable {
            source: 4,
            target: 1,
            edge_types: None,
            max_hops: None,
        });

        assert!(matches!(result, QueryResult::Bool(false)));
    }

    #[test]
    fn test_explain_query() {
        let graph = create_test_graph();
        let executor = QueryExecutor::new(&graph);

        let result = executor.execute(Query::Explain {
            source: 1,
            target: 4,
        });

        match result {
            QueryResult::Explanation(chain) => {
                assert_eq!(chain.source, 1);
                assert_eq!(chain.target, 4);
                assert_eq!(chain.steps.len(), 3);
                assert!(chain.total_confidence > 0.0);
            }
            _ => panic!("Expected explanation result"),
        }
    }

    #[test]
    fn test_query_builder() {
        let graph = create_test_graph();

        let result = QueryBuilder::new()
            .path(1, 4)
            .max_hops(5)
            .execute(&graph);

        match result {
            QueryResult::Path(Some(path)) => {
                assert!(path.nodes.len() > 0);
            }
            _ => panic!("Expected path result"),
        }
    }

    #[test]
    fn test_neighbors_with_type() {
        let graph = create_test_graph();
        let executor = QueryExecutor::new(&graph);

        let result = executor.execute(Query::Neighbors {
            node: 2,
            edge_type: Some(EdgeType::Semantic(SemanticEdge::PartOf)),
            direction: Direction::Outgoing,
            depth: 1,
        });

        match result {
            QueryResult::NodesWithEdges(nodes) => {
                assert_eq!(nodes.len(), 1);
                assert_eq!(nodes[0].0, 3);
            }
            _ => panic!("Expected nodes result"),
        }
    }

    #[test]
    fn test_all_paths() {
        let graph = create_test_graph();
        let executor = QueryExecutor::new(&graph);

        let result = executor.execute(Query::AllPaths {
            source: 1,
            target: 4,
            max_paths: 10,
            max_depth: 5,
        });

        match result {
            QueryResult::Paths(paths) => {
                assert!(!paths.is_empty());
            }
            _ => panic!("Expected paths result"),
        }
    }

    #[test]
    fn test_find_by_edge_type() {
        let graph = create_test_graph();
        let executor = QueryExecutor::new(&graph);

        let result = executor.execute(Query::Find {
            edge_type: EdgeType::Causal(CausalEdge::Causes),
            direction: Direction::Outgoing,
            from_node: None,
        });

        match result {
            QueryResult::Nodes(nodes) => {
                assert!(nodes.contains(&4)); // 3 causes 4
            }
            _ => panic!("Expected nodes result"),
        }
    }

    #[test]
    fn test_reasoning_chain_to_string() {
        let mut chain = ReasoningChain::new(1, 3);
        chain.add_step(ReasoningStep {
            from: 1,
            to: 2,
            edge_type: EdgeType::Semantic(SemanticEdge::IsA),
            confidence: 0.9,
            inferred: false,
            explanation: "Node 1 is a type of 2".to_string(),
        });
        chain.add_step(ReasoningStep {
            from: 2,
            to: 3,
            edge_type: EdgeType::Semantic(SemanticEdge::PartOf),
            confidence: 0.8,
            inferred: false,
            explanation: "Node 2 is part of 3".to_string(),
        });
        chain.generate_summary();

        let detailed = chain.to_detailed_string();
        assert!(detailed.contains("Reasoning Chain"));
        assert!(detailed.contains("Step 1"));
        assert!(detailed.contains("Step 2"));
    }
}
