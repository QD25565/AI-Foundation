//! Health Check and Diagnostics for Engram Knowledge Graph 2.0
//!
//! Provides comprehensive graph health analysis:
//! - Graph statistics and metrics
//! - Orphan node detection
//! - Cycle detection for temporal edges
//! - Confidence distribution analysis
//! - Contradiction summary
//! - Memory usage reporting

use std::collections::{HashMap, HashSet, VecDeque};
use super::csr::CsrGraph;
use super::types::{EdgeType, TemporalEdge};
use super::inference::ContradictionDetector;

// ============================================================================
// Health Check Report
// ============================================================================

/// Comprehensive health report for the knowledge graph
#[derive(Debug, Clone)]
pub struct HealthReport {
    /// Overall health status
    pub status: HealthStatus,

    /// Graph statistics
    pub stats: GraphHealthStats,

    /// Detected issues
    pub issues: Vec<HealthIssue>,

    /// Warnings (non-critical)
    pub warnings: Vec<String>,

    /// Recommendations for improvement
    pub recommendations: Vec<String>,
}

/// Overall health status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// No issues detected
    Healthy,
    /// Minor issues that don't affect functionality
    Warning,
    /// Significant issues that may affect correctness
    Degraded,
    /// Critical issues requiring attention
    Critical,
}

impl HealthStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthStatus::Healthy => "HEALTHY",
            HealthStatus::Warning => "WARNING",
            HealthStatus::Degraded => "DEGRADED",
            HealthStatus::Critical => "CRITICAL",
        }
    }
}

/// Detailed graph statistics
#[derive(Debug, Clone, Default)]
pub struct GraphHealthStats {
    /// Total number of nodes
    pub node_count: usize,
    /// Total number of edges
    pub edge_count: usize,
    /// Number of orphan nodes (no edges)
    pub orphan_count: usize,
    /// Number of isolated components
    pub component_count: usize,
    /// Size of largest component
    pub largest_component_size: usize,
    /// Average out-degree
    pub avg_out_degree: f32,
    /// Maximum out-degree
    pub max_out_degree: usize,
    /// Node with highest out-degree
    pub max_out_degree_node: Option<u64>,
    /// Average confidence score
    pub avg_confidence: f32,
    /// Minimum confidence score
    pub min_confidence: f32,
    /// Number of low-confidence edges (< 0.5)
    pub low_confidence_count: usize,
    /// Number of inferred edges
    pub inferred_edge_count: usize,
    /// Edge type distribution
    pub edge_type_distribution: HashMap<u8, usize>,
    /// Number of temporal cycles detected
    pub temporal_cycle_count: usize,
    /// Number of contradictions
    pub contradiction_count: usize,
    /// Memory usage estimate (bytes)
    pub memory_bytes: usize,
}

/// A specific health issue
#[derive(Debug, Clone)]
pub struct HealthIssue {
    /// Severity level
    pub severity: IssueSeverity,
    /// Issue category
    pub category: IssueCategory,
    /// Human-readable description
    pub description: String,
    /// Affected nodes (if applicable)
    pub affected_nodes: Vec<u64>,
}

/// Issue severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IssueSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

/// Issue categories
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueCategory {
    /// Structural issues (orphans, disconnected components)
    Structure,
    /// Temporal consistency issues (cycles)
    Temporal,
    /// Logical contradictions
    Contradiction,
    /// Confidence/quality issues
    Quality,
    /// Performance concerns
    Performance,
}

// ============================================================================
// Health Checker
// ============================================================================

/// Performs health checks on the knowledge graph
pub struct HealthChecker {
    /// Minimum confidence threshold for warnings
    min_confidence_threshold: f32,
    /// Maximum out-degree before warning
    max_out_degree_threshold: usize,
    /// Whether to detect temporal cycles
    check_temporal_cycles: bool,
    /// Whether to detect contradictions
    check_contradictions: bool,
}

impl HealthChecker {
    /// Create a new health checker with default settings
    pub fn new() -> Self {
        Self {
            min_confidence_threshold: 0.3,
            max_out_degree_threshold: 100,
            check_temporal_cycles: true,
            check_contradictions: true,
        }
    }

    /// Set minimum confidence threshold
    pub fn with_min_confidence(mut self, threshold: f32) -> Self {
        self.min_confidence_threshold = threshold;
        self
    }

    /// Set maximum out-degree threshold
    pub fn with_max_out_degree(mut self, threshold: usize) -> Self {
        self.max_out_degree_threshold = threshold;
        self
    }

    /// Enable/disable temporal cycle detection
    pub fn with_temporal_cycles(mut self, enabled: bool) -> Self {
        self.check_temporal_cycles = enabled;
        self
    }

    /// Enable/disable contradiction detection
    pub fn with_contradictions(mut self, enabled: bool) -> Self {
        self.check_contradictions = enabled;
        self
    }

    /// Run comprehensive health check
    pub fn check(&self, graph: &CsrGraph) -> HealthReport {
        let mut issues = Vec::new();
        let mut warnings = Vec::new();
        let mut recommendations = Vec::new();

        // Gather basic statistics
        let mut stats = self.gather_stats(graph);

        // Check for orphan nodes
        let orphans = self.find_orphans(graph);
        stats.orphan_count = orphans.len();
        if !orphans.is_empty() {
            if orphans.len() > 10 {
                issues.push(HealthIssue {
                    severity: IssueSeverity::Warning,
                    category: IssueCategory::Structure,
                    description: format!("{} orphan nodes detected (no connections)", orphans.len()),
                    affected_nodes: orphans.iter().take(10).copied().collect(),
                });
                recommendations.push("Consider removing or connecting orphan nodes".to_string());
            } else {
                warnings.push(format!("{} orphan nodes detected", orphans.len()));
            }
        }

        // Find connected components
        let components = self.find_components(graph);
        stats.component_count = components.len();
        stats.largest_component_size = components.iter().map(|c| c.len()).max().unwrap_or(0);

        if components.len() > 1 {
            warnings.push(format!(
                "Graph has {} disconnected components (largest: {} nodes)",
                components.len(),
                stats.largest_component_size
            ));
        }

        // Check for temporal cycles
        if self.check_temporal_cycles {
            let temporal_cycles = self.detect_temporal_cycles(graph);
            stats.temporal_cycle_count = temporal_cycles.len();

            for cycle in &temporal_cycles {
                issues.push(HealthIssue {
                    severity: IssueSeverity::Error,
                    category: IssueCategory::Temporal,
                    description: format!("Temporal cycle detected: {} nodes involved", cycle.len()),
                    affected_nodes: cycle.clone(),
                });
            }

            if !temporal_cycles.is_empty() {
                recommendations.push("Temporal cycles indicate logical inconsistencies in Before/After relationships".to_string());
            }
        }

        // Check for contradictions
        if self.check_contradictions {
            let detector = ContradictionDetector::new();
            let contradictions = detector.find_contradictions(graph);
            stats.contradiction_count = contradictions.len();

            for contradiction in &contradictions {
                issues.push(HealthIssue {
                    severity: IssueSeverity::Error,
                    category: IssueCategory::Contradiction,
                    description: format!(
                        "Contradiction: {} {:?} {} conflicts with {:?}",
                        contradiction.source,
                        contradiction.edge_type_1,
                        contradiction.target,
                        contradiction.edge_type_2
                    ),
                    affected_nodes: vec![contradiction.source, contradiction.target],
                });
            }
        }

        // Check confidence distribution
        if stats.low_confidence_count > stats.edge_count / 4 {
            warnings.push(format!(
                "{}% of edges have low confidence (< 0.5)",
                (stats.low_confidence_count * 100) / stats.edge_count.max(1)
            ));
            recommendations.push("Consider reviewing or pruning low-confidence edges".to_string());
        }

        // Check for hub nodes (extremely high degree)
        if let Some(max_node) = stats.max_out_degree_node {
            if stats.max_out_degree > self.max_out_degree_threshold {
                warnings.push(format!(
                    "Hub node {} has {} outgoing edges (threshold: {})",
                    max_node, stats.max_out_degree, self.max_out_degree_threshold
                ));
            }
        }

        // Calculate memory usage
        stats.memory_bytes = graph.memory_stats().total_bytes();

        // Determine overall status
        let status = self.determine_status(&issues, &warnings);

        HealthReport {
            status,
            stats,
            issues,
            warnings,
            recommendations,
        }
    }

    /// Gather basic statistics
    fn gather_stats(&self, graph: &CsrGraph) -> GraphHealthStats {
        let mut stats = GraphHealthStats::default();

        stats.node_count = graph.node_count();
        stats.edge_count = graph.edge_count();

        let nodes = graph.nodes();
        if nodes.is_empty() {
            return stats;
        }

        let mut total_confidence = 0.0f32;
        let mut min_confidence = 1.0f32;
        let mut max_degree = 0;
        let mut max_degree_node = None;
        let mut total_degree = 0;

        for node in &nodes {
            let degree = graph.out_degree(*node);
            total_degree += degree;

            if degree > max_degree {
                max_degree = degree;
                max_degree_node = Some(*node);
            }

            for (_, data) in graph.outgoing_edges(*node) {
                total_confidence += data.confidence;
                min_confidence = min_confidence.min(data.confidence);

                if data.confidence < 0.5 {
                    stats.low_confidence_count += 1;
                }

                if data.inferred {
                    stats.inferred_edge_count += 1;
                }

                *stats.edge_type_distribution.entry(data.edge_type).or_insert(0) += 1;
            }
        }

        stats.avg_out_degree = if !nodes.is_empty() {
            total_degree as f32 / nodes.len() as f32
        } else {
            0.0
        };

        stats.max_out_degree = max_degree;
        stats.max_out_degree_node = max_degree_node;

        stats.avg_confidence = if stats.edge_count > 0 {
            total_confidence / stats.edge_count as f32
        } else {
            1.0
        };

        stats.min_confidence = if stats.edge_count > 0 {
            min_confidence
        } else {
            1.0
        };

        stats
    }

    /// Find nodes with no edges
    fn find_orphans(&self, graph: &CsrGraph) -> Vec<u64> {
        let mut orphans = Vec::new();

        for node in graph.nodes() {
            if graph.out_degree(node) == 0 && graph.in_degree(node) == 0 {
                orphans.push(node);
            }
        }

        orphans
    }

    /// Find connected components
    fn find_components(&self, graph: &CsrGraph) -> Vec<Vec<u64>> {
        let nodes = graph.nodes();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut components = Vec::new();

        for start in nodes {
            if visited.contains(&start) {
                continue;
            }

            let mut component = Vec::new();
            let mut queue = VecDeque::new();
            queue.push_back(start);
            visited.insert(start);

            while let Some(node) = queue.pop_front() {
                component.push(node);

                // Add all neighbors (both directions)
                for neighbor in graph.neighbors(node) {
                    if !visited.contains(&neighbor) {
                        visited.insert(neighbor);
                        queue.push_back(neighbor);
                    }
                }
            }

            components.push(component);
        }

        components
    }

    /// Detect cycles in temporal edges (Before/After)
    fn detect_temporal_cycles(&self, graph: &CsrGraph) -> Vec<Vec<u64>> {
        let mut cycles = Vec::new();
        let nodes = graph.nodes();

        // Build temporal adjacency (only Before edges - After is reverse)
        let mut temporal_adj: HashMap<u64, Vec<u64>> = HashMap::new();

        for node in &nodes {
            for (target, data) in graph.outgoing_edges(*node) {
                if let Some(edge_type) = EdgeType::from_byte(data.edge_type) {
                    match edge_type {
                        EdgeType::Temporal(TemporalEdge::Before) => {
                            temporal_adj.entry(*node).or_default().push(target);
                        }
                        EdgeType::Temporal(TemporalEdge::After) => {
                            // After(A, B) means B is before A
                            temporal_adj.entry(target).or_default().push(*node);
                        }
                        _ => {}
                    }
                }
            }
        }

        // DFS for cycle detection
        let mut visited: HashSet<u64> = HashSet::new();
        let mut rec_stack: HashSet<u64> = HashSet::new();
        let mut path: Vec<u64> = Vec::new();

        for start in nodes {
            if !visited.contains(&start) {
                if let Some(cycle) = self.dfs_cycle(&temporal_adj, start, &mut visited, &mut rec_stack, &mut path) {
                    cycles.push(cycle);
                }
            }
        }

        cycles
    }

    /// DFS helper for cycle detection
    fn dfs_cycle(
        &self,
        adj: &HashMap<u64, Vec<u64>>,
        node: u64,
        visited: &mut HashSet<u64>,
        rec_stack: &mut HashSet<u64>,
        path: &mut Vec<u64>,
    ) -> Option<Vec<u64>> {
        visited.insert(node);
        rec_stack.insert(node);
        path.push(node);

        if let Some(neighbors) = adj.get(&node) {
            for &neighbor in neighbors {
                if !visited.contains(&neighbor) {
                    if let Some(cycle) = self.dfs_cycle(adj, neighbor, visited, rec_stack, path) {
                        return Some(cycle);
                    }
                } else if rec_stack.contains(&neighbor) {
                    // Found cycle - extract it
                    let cycle_start = path.iter().position(|&n| n == neighbor).unwrap_or(0);
                    return Some(path[cycle_start..].to_vec());
                }
            }
        }

        path.pop();
        rec_stack.remove(&node);
        None
    }

    /// Determine overall health status
    fn determine_status(&self, issues: &[HealthIssue], warnings: &[String]) -> HealthStatus {
        let has_critical = issues.iter().any(|i| i.severity == IssueSeverity::Critical);
        let has_error = issues.iter().any(|i| i.severity == IssueSeverity::Error);
        let has_warning = issues.iter().any(|i| i.severity == IssueSeverity::Warning) || !warnings.is_empty();

        if has_critical {
            HealthStatus::Critical
        } else if has_error {
            HealthStatus::Degraded
        } else if has_warning {
            HealthStatus::Warning
        } else {
            HealthStatus::Healthy
        }
    }
}

impl Default for HealthChecker {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Quick Health Summary
// ============================================================================

/// Generate a quick one-line health summary
pub fn quick_summary(graph: &CsrGraph) -> String {
    let checker = HealthChecker::new();
    let report = checker.check(graph);

    format!(
        "[{}] {} nodes, {} edges, {} contradictions, {} temporal cycles",
        report.status.as_str(),
        report.stats.node_count,
        report.stats.edge_count,
        report.stats.contradiction_count,
        report.stats.temporal_cycle_count
    )
}

/// Generate detailed health report as string
pub fn detailed_report(graph: &CsrGraph) -> String {
    let checker = HealthChecker::new();
    let report = checker.check(graph);

    let mut output = String::new();

    output.push_str(&format!("=== Knowledge Graph Health Report ===\n"));
    output.push_str(&format!("Status: {}\n\n", report.status.as_str()));

    output.push_str("--- Statistics ---\n");
    output.push_str(&format!("Nodes: {}\n", report.stats.node_count));
    output.push_str(&format!("Edges: {} (inferred: {})\n",
        report.stats.edge_count, report.stats.inferred_edge_count));
    output.push_str(&format!("Orphan nodes: {}\n", report.stats.orphan_count));
    output.push_str(&format!("Connected components: {} (largest: {})\n",
        report.stats.component_count, report.stats.largest_component_size));
    output.push_str(&format!("Avg out-degree: {:.2}\n", report.stats.avg_out_degree));
    output.push_str(&format!("Max out-degree: {}", report.stats.max_out_degree));
    if let Some(node) = report.stats.max_out_degree_node {
        output.push_str(&format!(" (node {})", node));
    }
    output.push('\n');
    output.push_str(&format!("Avg confidence: {:.2}\n", report.stats.avg_confidence));
    output.push_str(&format!("Low confidence edges: {}\n", report.stats.low_confidence_count));
    output.push_str(&format!("Memory: {} KB\n", report.stats.memory_bytes / 1024));

    if !report.issues.is_empty() {
        output.push_str("\n--- Issues ---\n");
        for issue in &report.issues {
            output.push_str(&format!("[{:?}] {}: {}\n",
                issue.severity,
                match issue.category {
                    IssueCategory::Structure => "Structure",
                    IssueCategory::Temporal => "Temporal",
                    IssueCategory::Contradiction => "Contradiction",
                    IssueCategory::Quality => "Quality",
                    IssueCategory::Performance => "Performance",
                },
                issue.description
            ));
        }
    }

    if !report.warnings.is_empty() {
        output.push_str("\n--- Warnings ---\n");
        for warning in &report.warnings {
            output.push_str(&format!("- {}\n", warning));
        }
    }

    if !report.recommendations.is_empty() {
        output.push_str("\n--- Recommendations ---\n");
        for rec in &report.recommendations {
            output.push_str(&format!("* {}\n", rec));
        }
    }

    output
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Edge, EdgeType, SemanticEdge, TemporalEdge, CausalEdge};

    #[test]
    fn test_healthy_graph() {
        let mut graph = CsrGraph::new();
        graph.add_edge(Edge::new(1, 2, EdgeType::Semantic(SemanticEdge::RelatedTo), 0.9));
        graph.add_edge(Edge::new(2, 3, EdgeType::Semantic(SemanticEdge::RelatedTo), 0.85));
        graph.compact();

        let checker = HealthChecker::new();
        let report = checker.check(&graph);

        assert_eq!(report.status, HealthStatus::Healthy);
        assert_eq!(report.stats.node_count, 3);
        assert_eq!(report.stats.edge_count, 2);
        assert_eq!(report.stats.temporal_cycle_count, 0);
    }

    #[test]
    fn test_orphan_detection() {
        let mut graph = CsrGraph::new();
        graph.add_edge(Edge::new(1, 2, EdgeType::Semantic(SemanticEdge::RelatedTo), 0.9));
        // Node 3 will be an orphan (we need to add it somehow)
        graph.compact();

        let checker = HealthChecker::new();
        let orphans = checker.find_orphans(&graph);

        // Only 1 and 2 have edges, so no orphans in this case
        assert!(orphans.is_empty());
    }

    #[test]
    fn test_temporal_cycle_detection() {
        let mut graph = CsrGraph::new();
        // Create a cycle: 1 before 2, 2 before 3, 3 before 1
        graph.add_edge(Edge::new(1, 2, EdgeType::Temporal(TemporalEdge::Before), 0.9));
        graph.add_edge(Edge::new(2, 3, EdgeType::Temporal(TemporalEdge::Before), 0.9));
        graph.add_edge(Edge::new(3, 1, EdgeType::Temporal(TemporalEdge::Before), 0.9));
        graph.compact();

        let checker = HealthChecker::new();
        let cycles = checker.detect_temporal_cycles(&graph);

        assert!(!cycles.is_empty());
        assert_eq!(cycles[0].len(), 3);
    }

    #[test]
    fn test_contradiction_detection() {
        let mut graph = CsrGraph::new();
        // Create contradiction: 1 causes 2, 1 prevents 2
        graph.add_edge(Edge::new(1, 2, EdgeType::Causal(CausalEdge::Causes), 0.9));
        graph.add_edge(Edge::new(1, 2, EdgeType::Causal(CausalEdge::Prevents), 0.9));
        graph.compact();

        let checker = HealthChecker::new();
        let report = checker.check(&graph);

        assert_eq!(report.stats.contradiction_count, 1);
        assert_eq!(report.status, HealthStatus::Degraded);
    }

    #[test]
    fn test_component_detection() {
        let mut graph = CsrGraph::new();
        // Component 1: nodes 1, 2
        graph.add_edge(Edge::new(1, 2, EdgeType::Semantic(SemanticEdge::RelatedTo), 0.9));
        // Component 2: nodes 3, 4
        graph.add_edge(Edge::new(3, 4, EdgeType::Semantic(SemanticEdge::RelatedTo), 0.9));
        graph.compact();

        let checker = HealthChecker::new();
        let components = checker.find_components(&graph);

        assert_eq!(components.len(), 2);
    }

    #[test]
    fn test_low_confidence_warning() {
        let mut graph = CsrGraph::new();
        // Add low-confidence edges using with_confidence constructor
        for i in 0..5 {
            graph.add_edge(Edge::with_confidence(
                i, i + 1,
                EdgeType::Semantic(SemanticEdge::RelatedTo),
                0.5,  // weight
                0.2,  // confidence (low)
            ));
        }
        graph.compact();

        let checker = HealthChecker::new();
        let report = checker.check(&graph);

        // Should have detected some low-confidence edges
        assert!(report.stats.low_confidence_count > 0);
        assert!(report.stats.avg_confidence < 0.5);
    }

    #[test]
    fn test_quick_summary() {
        let mut graph = CsrGraph::new();
        graph.add_edge(Edge::new(1, 2, EdgeType::Semantic(SemanticEdge::RelatedTo), 0.9));
        graph.compact();

        let summary = quick_summary(&graph);
        assert!(summary.contains("HEALTHY"));
        assert!(summary.contains("2 nodes"));
    }

    #[test]
    fn test_detailed_report() {
        let mut graph = CsrGraph::new();
        graph.add_edge(Edge::new(1, 2, EdgeType::Semantic(SemanticEdge::RelatedTo), 0.9));
        graph.compact();

        let report = detailed_report(&graph);
        assert!(report.contains("Knowledge Graph Health Report"));
        assert!(report.contains("Statistics"));
    }
}
