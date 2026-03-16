//! Inference Engine for Engram Knowledge Graph 2.0
//!
//! Provides reasoning capabilities through:
//! - Transitive closure computation (reachability)
//! - Forward chaining inference with rules
//! - Confidence propagation through inference chains
//! - Contradiction detection
//!
//! Based on research:
//! - Italiano's algorithm for incremental transitive closure
//! - Forward chaining (materialization) for fast query response
//! - Confidence decay through inference hops

use std::collections::{HashMap, HashSet, VecDeque};
use super::csr::CsrGraph;
use super::types::{Edge, EdgeType, SemanticEdge, CausalEdge, TemporalEdge};

// ============================================================================
// Transitive Closure
// ============================================================================

/// Transitive closure index for O(1) reachability queries
///
/// Uses a sparse representation to handle large graphs efficiently.
/// For each node, stores the set of all reachable nodes through transitive edges.
#[derive(Debug, Clone)]
pub struct TransitiveClosure {
    /// For each node, the set of nodes reachable via transitive edges
    /// Key: source node, Value: set of reachable nodes with (edge_type, min_hops, confidence)
    reachable: HashMap<u64, HashMap<u64, ReachabilityInfo>>,

    /// Edge types that are transitive
    transitive_types: HashSet<u8>,

    /// Maximum depth for closure computation
    max_depth: usize,
}

/// Information about reachability between two nodes
#[derive(Debug, Clone)]
pub struct ReachabilityInfo {
    /// The edge type of the transitive relationship
    pub edge_type: u8,
    /// Minimum number of hops to reach
    pub min_hops: usize,
    /// Confidence of the transitive relationship (product of edge confidences)
    pub confidence: f32,
    /// Whether this is a direct edge (not inferred)
    pub direct: bool,
}

impl TransitiveClosure {
    /// Create a new transitive closure index
    pub fn new(max_depth: usize) -> Self {
        let mut transitive_types = HashSet::new();

        // Add all transitive edge types
        transitive_types.insert(EdgeType::Semantic(SemanticEdge::IsA).to_byte());
        transitive_types.insert(EdgeType::Semantic(SemanticEdge::PartOf).to_byte());
        transitive_types.insert(EdgeType::Causal(CausalEdge::Causes).to_byte());
        transitive_types.insert(EdgeType::Causal(CausalEdge::Implies).to_byte());
        transitive_types.insert(EdgeType::Causal(CausalEdge::Requires).to_byte());
        transitive_types.insert(EdgeType::Temporal(TemporalEdge::Before).to_byte());
        transitive_types.insert(EdgeType::Temporal(TemporalEdge::After).to_byte());

        Self {
            reachable: HashMap::new(),
            transitive_types,
            max_depth,
        }
    }

    /// Compute transitive closure from a graph
    pub fn compute(graph: &CsrGraph, max_depth: usize) -> Self {
        let mut tc = Self::new(max_depth);
        tc.rebuild(graph);
        tc
    }

    /// Rebuild the transitive closure from scratch
    pub fn rebuild(&mut self, graph: &CsrGraph) {
        self.reachable.clear();

        // For each node, compute reachability
        for node in graph.nodes() {
            self.compute_reachability(graph, node);
        }
    }

    /// Compute reachability from a single source node
    fn compute_reachability(&mut self, graph: &CsrGraph, source: u64) {
        let mut reachable_from_source: HashMap<u64, ReachabilityInfo> = HashMap::new();
        let mut queue: VecDeque<(u64, usize, f32, u8)> = VecDeque::new();
        let mut visited: HashSet<u64> = HashSet::new();

        // Start with direct edges
        for (target, edge) in graph.outgoing_edges(source) {
            if self.transitive_types.contains(&edge.edge_type) {
                reachable_from_source.insert(target, ReachabilityInfo {
                    edge_type: edge.edge_type,
                    min_hops: 1,
                    confidence: edge.confidence,
                    direct: true,
                });
                queue.push_back((target, 1, edge.confidence, edge.edge_type));
                visited.insert(target);
            }
        }

        // BFS to find all transitively reachable nodes
        while let Some((current, depth, confidence, edge_type)) = queue.pop_front() {
            if depth >= self.max_depth {
                continue;
            }

            for (next, edge) in graph.outgoing_edges(current) {
                // Only follow edges of the same transitive type
                if edge.edge_type == edge_type && !visited.contains(&next) {
                    visited.insert(next);
                    let new_confidence = confidence * edge.confidence;

                    // Only add if confidence is still meaningful
                    if new_confidence > 0.01 {
                        let info = ReachabilityInfo {
                            edge_type,
                            min_hops: depth + 1,
                            confidence: new_confidence,
                            direct: false,
                        };

                        // Update if this is a better path
                        if let Some(existing) = reachable_from_source.get(&next) {
                            if new_confidence > existing.confidence {
                                reachable_from_source.insert(next, info);
                            }
                        } else {
                            reachable_from_source.insert(next, info);
                        }

                        queue.push_back((next, depth + 1, new_confidence, edge_type));
                    }
                }
            }
        }

        if !reachable_from_source.is_empty() {
            self.reachable.insert(source, reachable_from_source);
        }
    }

    /// Check if target is reachable from source via transitive edges
    pub fn is_reachable(&self, source: u64, target: u64) -> bool {
        self.reachable
            .get(&source)
            .map(|r| r.contains_key(&target))
            .unwrap_or(false)
    }

    /// Get reachability info between two nodes
    pub fn get_reachability(&self, source: u64, target: u64) -> Option<&ReachabilityInfo> {
        self.reachable
            .get(&source)
            .and_then(|r| r.get(&target))
    }

    /// Get all nodes reachable from source
    pub fn reachable_from(&self, source: u64) -> Vec<(u64, &ReachabilityInfo)> {
        self.reachable
            .get(&source)
            .map(|r| r.iter().map(|(&k, v)| (k, v)).collect())
            .unwrap_or_default()
    }

    /// Get all nodes that can reach target
    pub fn reaches_to(&self, target: u64) -> Vec<(u64, &ReachabilityInfo)> {
        let mut result = Vec::new();
        for (&source, reachable) in &self.reachable {
            if let Some(info) = reachable.get(&target) {
                result.push((source, info));
            }
        }
        result
    }

    /// Incrementally update after adding an edge
    pub fn add_edge(&mut self, graph: &CsrGraph, source: u64, _target: u64, edge_type: u8) {
        if !self.transitive_types.contains(&edge_type) {
            return;
        }

        // Recompute reachability for affected nodes
        // This is a simplified version - full Italiano's algorithm would be more efficient
        self.compute_reachability(graph, source);

        // Also update any node that could now reach more nodes through source
        let sources_to_update: Vec<u64> = self.reaches_to(source)
            .into_iter()
            .map(|(s, _)| s)
            .collect();

        for s in sources_to_update {
            self.compute_reachability(graph, s);
        }
    }

    /// Get statistics about the closure
    pub fn stats(&self) -> TransitiveClosureStats {
        let mut total_pairs = 0;
        let mut direct_pairs = 0;
        let mut max_hops = 0;

        for reachable in self.reachable.values() {
            for info in reachable.values() {
                total_pairs += 1;
                if info.direct {
                    direct_pairs += 1;
                }
                max_hops = max_hops.max(info.min_hops);
            }
        }

        TransitiveClosureStats {
            nodes_with_closure: self.reachable.len(),
            total_pairs,
            direct_pairs,
            inferred_pairs: total_pairs - direct_pairs,
            max_hops,
        }
    }
}

/// Statistics about transitive closure
#[derive(Debug, Clone)]
pub struct TransitiveClosureStats {
    pub nodes_with_closure: usize,
    pub total_pairs: usize,
    pub direct_pairs: usize,
    pub inferred_pairs: usize,
    pub max_hops: usize,
}

// ============================================================================
// Inference Rules
// ============================================================================

/// An inference rule that derives new edges from existing patterns
#[derive(Debug, Clone)]
pub struct InferenceRule {
    /// Unique identifier for the rule
    pub id: &'static str,
    /// Human-readable description
    pub description: &'static str,
    /// Priority (higher = applied first)
    pub priority: i32,
    /// The rule logic
    pub rule_type: RuleType,
}

/// Types of inference rules
#[derive(Debug, Clone)]
pub enum RuleType {
    /// If A->B and B->C with edge type T, then A->C with type T
    Transitivity { edge_type: EdgeType },

    /// If A->B with type T1, then B->A with type T2
    Symmetry { forward: EdgeType, backward: EdgeType },

    /// If A IsA B and B HasProperty P, then A HasProperty P
    PropertyInheritance,

    /// If A Causes B and B Causes C, then A indirectly Causes C
    CausalChain,

    /// If A Implies B and A is true, then B is likely true
    ModusPonens,

    /// Custom rule with a function
    Custom {
        /// Function that checks if rule applies and returns new edges
        apply: fn(graph: &CsrGraph, source: u64) -> Vec<Edge>,
    },
}

impl InferenceRule {
    /// Create a transitivity rule for a specific edge type
    pub fn transitivity(id: &'static str, edge_type: EdgeType) -> Self {
        Self {
            id,
            description: "Transitive closure",
            priority: 100,
            rule_type: RuleType::Transitivity { edge_type },
        }
    }

    /// Create a symmetry rule
    pub fn symmetry(id: &'static str, forward: EdgeType, backward: EdgeType) -> Self {
        Self {
            id,
            description: "Symmetric relationship",
            priority: 90,
            rule_type: RuleType::Symmetry { forward, backward },
        }
    }
}

// ============================================================================
// Inference Engine
// ============================================================================

/// Forward-chaining inference engine
///
/// Applies inference rules to derive new edges from existing graph structure.
/// Uses materialization strategy - inferred edges are stored for fast querying.
pub struct InferenceEngine {
    /// Inference rules to apply
    rules: Vec<InferenceRule>,

    /// Inferred edges (not in base graph)
    inferred_edges: Vec<Edge>,

    /// Set of (source, target, type) for quick duplicate checking
    inferred_set: HashSet<(u64, u64, u8)>,

    /// Transitive closure index
    transitive_closure: Option<TransitiveClosure>,

    /// Minimum confidence threshold for inferred edges
    min_confidence: f32,

    /// Maximum inference chain length
    max_chain_length: usize,
}

impl InferenceEngine {
    /// Create a new inference engine
    pub fn new() -> Self {
        Self {
            rules: Self::default_rules(),
            inferred_edges: Vec::new(),
            inferred_set: HashSet::new(),
            transitive_closure: None,
            min_confidence: 0.1,
            max_chain_length: 5,
        }
    }

    /// Create with custom configuration
    pub fn with_config(min_confidence: f32, max_chain_length: usize) -> Self {
        Self {
            rules: Self::default_rules(),
            inferred_edges: Vec::new(),
            inferred_set: HashSet::new(),
            transitive_closure: None,
            min_confidence,
            max_chain_length,
        }
    }

    /// Default inference rules
    fn default_rules() -> Vec<InferenceRule> {
        vec![
            // Transitivity rules
            InferenceRule::transitivity("isa_trans", EdgeType::Semantic(SemanticEdge::IsA)),
            InferenceRule::transitivity("partof_trans", EdgeType::Semantic(SemanticEdge::PartOf)),
            InferenceRule::transitivity("causes_trans", EdgeType::Causal(CausalEdge::Causes)),
            InferenceRule::transitivity("implies_trans", EdgeType::Causal(CausalEdge::Implies)),
            InferenceRule::transitivity("before_trans", EdgeType::Temporal(TemporalEdge::Before)),
            InferenceRule::transitivity("after_trans", EdgeType::Temporal(TemporalEdge::After)),

            // Symmetry rules
            InferenceRule::symmetry(
                "similar_sym",
                EdgeType::Semantic(SemanticEdge::SimilarTo),
                EdgeType::Semantic(SemanticEdge::SimilarTo),
            ),
            InferenceRule::symmetry(
                "related_sym",
                EdgeType::Semantic(SemanticEdge::RelatedTo),
                EdgeType::Semantic(SemanticEdge::RelatedTo),
            ),
            InferenceRule::symmetry(
                "contradicts_sym",
                EdgeType::Causal(CausalEdge::Contradicts),
                EdgeType::Causal(CausalEdge::Contradicts),
            ),

            // Property inheritance
            InferenceRule {
                id: "property_inherit",
                description: "Inherit properties through IsA hierarchy",
                priority: 80,
                rule_type: RuleType::PropertyInheritance,
            },
        ]
    }

    /// Add a custom rule
    pub fn add_rule(&mut self, rule: InferenceRule) {
        self.rules.push(rule);
        self.rules.sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    /// Run inference on the graph
    pub fn run_inference(&mut self, graph: &CsrGraph) -> InferenceResult {
        let start = std::time::Instant::now();
        let mut iterations = 0;
        let mut new_edges_total = 0;

        // Clear previous inferences
        self.inferred_edges.clear();
        self.inferred_set.clear();

        // Build transitive closure
        self.transitive_closure = Some(TransitiveClosure::compute(graph, self.max_chain_length));

        // Fixed-point iteration
        loop {
            let new_edges = self.apply_rules_once(graph);
            if new_edges == 0 {
                break;
            }
            new_edges_total += new_edges;
            iterations += 1;

            // Safety limit
            if iterations > 100 {
                break;
            }
        }

        InferenceResult {
            edges_inferred: new_edges_total,
            iterations,
            duration_ms: start.elapsed().as_millis() as u64,
            rules_applied: self.rules.len(),
        }
    }

    /// Apply all rules once and return number of new edges
    fn apply_rules_once(&mut self, graph: &CsrGraph) -> usize {
        let mut new_edges = Vec::new();

        for node in graph.nodes() {
            for rule in &self.rules {
                let edges = self.apply_rule(graph, node, rule);
                new_edges.extend(edges);
            }
        }

        // Add new edges that pass threshold and aren't duplicates
        let mut added = 0;
        for edge in new_edges {
            if edge.confidence >= self.min_confidence {
                let key = (edge.source, edge.target, edge.edge_type.to_byte());
                if !self.inferred_set.contains(&key) && !self.edge_exists(graph, &edge) {
                    self.inferred_set.insert(key);
                    self.inferred_edges.push(edge);
                    added += 1;
                }
            }
        }

        added
    }

    /// Check if edge already exists in base graph
    fn edge_exists(&self, graph: &CsrGraph, edge: &Edge) -> bool {
        graph.has_edge_of_type(edge.source, edge.target, edge.edge_type)
    }

    /// Apply a single rule from a source node
    fn apply_rule(&self, graph: &CsrGraph, source: u64, rule: &InferenceRule) -> Vec<Edge> {
        match &rule.rule_type {
            RuleType::Transitivity { edge_type } => {
                self.apply_transitivity(graph, source, *edge_type)
            }
            RuleType::Symmetry { forward, backward } => {
                self.apply_symmetry(graph, source, *forward, *backward)
            }
            RuleType::PropertyInheritance => {
                self.apply_property_inheritance(graph, source)
            }
            RuleType::CausalChain => {
                self.apply_causal_chain(graph, source)
            }
            RuleType::ModusPonens => {
                Vec::new() // Would require truth values
            }
            RuleType::Custom { apply } => {
                apply(graph, source)
            }
        }
    }

    /// Apply transitivity rule
    fn apply_transitivity(&self, graph: &CsrGraph, source: u64, edge_type: EdgeType) -> Vec<Edge> {
        let mut new_edges = Vec::new();
        let type_byte = edge_type.to_byte();

        // Get direct edges of this type
        for (mid, edge1) in graph.outgoing_edges(source) {
            if edge1.edge_type != type_byte {
                continue;
            }

            // Get edges from mid node
            for (target, edge2) in graph.outgoing_edges(mid) {
                if edge2.edge_type != type_byte {
                    continue;
                }

                // Skip if source == target
                if source == target {
                    continue;
                }

                // Create transitive edge with decayed confidence
                let confidence = edge1.confidence * edge2.confidence * edge_type.confidence_factor();

                new_edges.push(Edge::inferred(
                    source,
                    target,
                    edge_type,
                    (edge1.weight + edge2.weight) / 2.0,
                    confidence,
                    vec![source, mid, target],
                ));
            }
        }

        new_edges
    }

    /// Apply symmetry rule
    fn apply_symmetry(&self, graph: &CsrGraph, source: u64, forward: EdgeType, backward: EdgeType) -> Vec<Edge> {
        let mut new_edges = Vec::new();
        let forward_byte = forward.to_byte();

        for (target, edge) in graph.outgoing_edges(source) {
            if edge.edge_type == forward_byte {
                // Check if reverse edge exists
                if !graph.has_edge_of_type(target, source, backward) {
                    new_edges.push(Edge::inferred(
                        target,
                        source,
                        backward,
                        edge.weight,
                        edge.confidence * backward.confidence_factor(),
                        vec![target, source],
                    ));
                }
            }
        }

        new_edges
    }

    /// Apply property inheritance through IsA hierarchy
    fn apply_property_inheritance(&self, graph: &CsrGraph, source: u64) -> Vec<Edge> {
        let mut new_edges = Vec::new();
        let isa_byte = EdgeType::Semantic(SemanticEdge::IsA).to_byte();
        let hasprop_byte = EdgeType::Semantic(SemanticEdge::HasProperty).to_byte();

        // Find what source IsA
        for (parent, isa_edge) in graph.outgoing_edges(source) {
            if isa_edge.edge_type != isa_byte {
                continue;
            }

            // Find properties of parent
            for (prop, prop_edge) in graph.outgoing_edges(parent) {
                if prop_edge.edge_type != hasprop_byte {
                    continue;
                }

                // Inherit property with decayed confidence
                let confidence = isa_edge.confidence * prop_edge.confidence * 0.9;

                new_edges.push(Edge::inferred(
                    source,
                    prop,
                    EdgeType::Semantic(SemanticEdge::HasProperty),
                    prop_edge.weight,
                    confidence,
                    vec![source, parent, prop],
                ));
            }
        }

        new_edges
    }

    /// Apply causal chain inference
    fn apply_causal_chain(&self, graph: &CsrGraph, source: u64) -> Vec<Edge> {
        let mut new_edges = Vec::new();
        let causes_byte = EdgeType::Causal(CausalEdge::Causes).to_byte();

        for (mid, edge1) in graph.outgoing_edges(source) {
            if edge1.edge_type != causes_byte {
                continue;
            }

            for (target, edge2) in graph.outgoing_edges(mid) {
                if edge2.edge_type != causes_byte {
                    continue;
                }

                if source == target {
                    continue;
                }

                // Causal chains have significant confidence decay
                let confidence = edge1.confidence * edge2.confidence * 0.7;

                new_edges.push(Edge::inferred(
                    source,
                    target,
                    EdgeType::Causal(CausalEdge::Causes),
                    (edge1.weight + edge2.weight) / 2.0,
                    confidence,
                    vec![source, mid, target],
                ));
            }
        }

        new_edges
    }

    /// Get all inferred edges
    pub fn inferred_edges(&self) -> &[Edge] {
        &self.inferred_edges
    }

    /// Get inferred edges from a specific source
    pub fn inferred_from(&self, source: u64) -> Vec<&Edge> {
        self.inferred_edges
            .iter()
            .filter(|e| e.source == source)
            .collect()
    }

    /// Get the transitive closure
    pub fn transitive_closure(&self) -> Option<&TransitiveClosure> {
        self.transitive_closure.as_ref()
    }

    /// Check if there's a transitive path between nodes
    pub fn has_transitive_path(&self, source: u64, target: u64) -> bool {
        self.transitive_closure
            .as_ref()
            .map(|tc| tc.is_reachable(source, target))
            .unwrap_or(false)
    }
}

impl Default for InferenceEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of running inference
#[derive(Debug, Clone)]
pub struct InferenceResult {
    pub edges_inferred: usize,
    pub iterations: usize,
    pub duration_ms: u64,
    pub rules_applied: usize,
}

// ============================================================================
// Contradiction Detection
// ============================================================================

/// Detects contradictions in the knowledge graph
pub struct ContradictionDetector {
    /// Pairs of edge types that are contradictory
    contradictory_pairs: Vec<(EdgeType, EdgeType)>,
}

impl ContradictionDetector {
    pub fn new() -> Self {
        Self {
            contradictory_pairs: vec![
                // Before/After are contradictory
                (
                    EdgeType::Temporal(TemporalEdge::Before),
                    EdgeType::Temporal(TemporalEdge::After),
                ),
                // Causes/Prevents are contradictory
                (
                    EdgeType::Causal(CausalEdge::Causes),
                    EdgeType::Causal(CausalEdge::Prevents),
                ),
                // Supports/Contradicts are contradictory
                (
                    EdgeType::Causal(CausalEdge::Supports),
                    EdgeType::Causal(CausalEdge::Contradicts),
                ),
                // Synonym/Antonym are contradictory
                (
                    EdgeType::Semantic(SemanticEdge::SynonymOf),
                    EdgeType::Semantic(SemanticEdge::AntonymOf),
                ),
            ],
        }
    }

    /// Find all contradictions in the graph
    pub fn find_contradictions(&self, graph: &CsrGraph) -> Vec<Contradiction> {
        let mut contradictions = Vec::new();
        let mut seen: HashSet<(u64, u64, u8, u8)> = HashSet::new();

        for node in graph.nodes() {
            let edges: Vec<_> = graph.outgoing_edges(node);

            for i in 0..edges.len() {
                let (target1, ref edge1) = edges[i];
                let type1 = EdgeType::from_byte(edge1.edge_type);

                if let Some(type1) = type1 {
                    for j in (i + 1)..edges.len() {
                        let (target2, ref edge2) = edges[j];

                        // Only check edges to the same target
                        if target1 != target2 {
                            continue;
                        }

                        let type2 = EdgeType::from_byte(edge2.edge_type);
                        if let Some(type2) = type2 {
                            if self.are_contradictory(type1, type2) {
                                // Normalize the key to avoid duplicates
                                let key = if edge1.edge_type < edge2.edge_type {
                                    (node, target1, edge1.edge_type, edge2.edge_type)
                                } else {
                                    (node, target1, edge2.edge_type, edge1.edge_type)
                                };

                                if !seen.contains(&key) {
                                    seen.insert(key);
                                    contradictions.push(Contradiction {
                                        source: node,
                                        target: target1,
                                        edge_type_1: type1,
                                        edge_type_2: type2,
                                        confidence_1: edge1.confidence,
                                        confidence_2: edge2.confidence,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        contradictions
    }

    /// Check if two edge types are contradictory
    fn are_contradictory(&self, type1: EdgeType, type2: EdgeType) -> bool {
        for (a, b) in &self.contradictory_pairs {
            if (type1 == *a && type2 == *b) || (type1 == *b && type2 == *a) {
                return true;
            }
        }
        false
    }
}

impl Default for ContradictionDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// A contradiction found in the graph
#[derive(Debug, Clone)]
pub struct Contradiction {
    pub source: u64,
    pub target: u64,
    pub edge_type_1: EdgeType,
    pub edge_type_2: EdgeType,
    pub confidence_1: f32,
    pub confidence_2: f32,
}

impl Contradiction {
    /// Get the "winning" edge type based on confidence
    pub fn winner(&self) -> EdgeType {
        if self.confidence_1 >= self.confidence_2 {
            self.edge_type_1
        } else {
            self.edge_type_2
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_graph() -> CsrGraph {
        // Hierarchy: Dog IsA Animal, Cat IsA Animal
        // Animal HasProperty Breathes
        // Dog Causes Barking
        let edges = vec![
            Edge::new(1, 2, EdgeType::Semantic(SemanticEdge::IsA), 0.95),      // Dog IsA Animal
            Edge::new(3, 2, EdgeType::Semantic(SemanticEdge::IsA), 0.95),      // Cat IsA Animal
            Edge::new(2, 4, EdgeType::Semantic(SemanticEdge::HasProperty), 1.0), // Animal HasProperty Breathes
            Edge::new(1, 5, EdgeType::Causal(CausalEdge::Causes), 0.8),        // Dog Causes Barking
            Edge::new(10, 11, EdgeType::Semantic(SemanticEdge::SimilarTo), 0.9), // 10 SimilarTo 11
        ];
        CsrGraph::from_edges(&edges)
    }

    #[test]
    fn test_transitive_closure_basic() {
        let graph = create_test_graph();
        let tc = TransitiveClosure::compute(&graph, 5);

        // Dog IsA Animal is direct
        assert!(tc.is_reachable(1, 2));
        let info = tc.get_reachability(1, 2).unwrap();
        assert!(info.direct);
        assert_eq!(info.min_hops, 1);
    }

    #[test]
    fn test_transitive_closure_stats() {
        let graph = create_test_graph();
        let tc = TransitiveClosure::compute(&graph, 5);
        let stats = tc.stats();

        assert!(stats.total_pairs > 0);
    }

    #[test]
    fn test_inference_engine_basic() {
        let graph = create_test_graph();
        let mut engine = InferenceEngine::new();
        let result = engine.run_inference(&graph);

        // Should run at least one iteration and infer at least one edge
        assert!(result.iterations > 0 && !engine.inferred_edges().is_empty());
    }

    #[test]
    fn test_symmetry_inference() {
        let graph = create_test_graph();
        let mut engine = InferenceEngine::new();
        engine.run_inference(&graph);

        // Should infer 11 SimilarTo 10 from 10 SimilarTo 11
        let reverse_exists = engine.inferred_edges().iter().any(|e| {
            e.source == 11 && e.target == 10 &&
            e.edge_type == EdgeType::Semantic(SemanticEdge::SimilarTo)
        });
        assert!(reverse_exists);
    }

    #[test]
    fn test_property_inheritance() {
        let graph = create_test_graph();
        let mut engine = InferenceEngine::new();
        engine.run_inference(&graph);

        // Dog should inherit HasProperty Breathes from Animal
        let dog_breathes = engine.inferred_edges().iter().any(|e| {
            e.source == 1 && e.target == 4 &&
            e.edge_type == EdgeType::Semantic(SemanticEdge::HasProperty)
        });
        assert!(dog_breathes);
    }

    #[test]
    fn test_contradiction_detection() {
        // Create a graph with contradictions
        let edges = vec![
            Edge::new(1, 2, EdgeType::Temporal(TemporalEdge::Before), 0.9),
            Edge::new(1, 2, EdgeType::Temporal(TemporalEdge::After), 0.8),
        ];
        let graph = CsrGraph::from_edges(&edges);

        let detector = ContradictionDetector::new();
        let contradictions = detector.find_contradictions(&graph);

        assert_eq!(contradictions.len(), 1);
        assert_eq!(contradictions[0].source, 1);
        assert_eq!(contradictions[0].target, 2);
    }

    #[test]
    fn test_contradiction_winner() {
        let contradiction = Contradiction {
            source: 1,
            target: 2,
            edge_type_1: EdgeType::Temporal(TemporalEdge::Before),
            edge_type_2: EdgeType::Temporal(TemporalEdge::After),
            confidence_1: 0.9,
            confidence_2: 0.8,
        };

        assert_eq!(contradiction.winner(), EdgeType::Temporal(TemporalEdge::Before));
    }

    #[test]
    fn test_inference_confidence_decay() {
        let graph = create_test_graph();
        let mut engine = InferenceEngine::new();
        engine.run_inference(&graph);

        // Inferred edges should have lower confidence than source edges
        for edge in engine.inferred_edges() {
            assert!(edge.confidence <= 1.0);
            if edge.inference_chain.is_some() {
                // Multi-hop inferences should have decayed confidence
                assert!(edge.confidence < 0.95);
            }
        }
    }
}
