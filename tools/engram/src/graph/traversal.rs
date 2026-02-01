//! Graph Traversal Algorithms for Engram Knowledge Graph 2.0
//!
//! Provides multi-hop traversal with edge type filtering:
//! - BFS (Breadth-First Search) - Find shortest path by hop count
//! - DFS (Depth-First Search) - Explore deeply, good for finding any path
//! - Dijkstra - Weighted shortest path using edge weights
//! - A* - Heuristic-guided search for large graphs
//!
//! All algorithms support:
//! - Edge type filtering (only traverse specific relationship types)
//! - Maximum depth/hop limits
//! - Direction control (outgoing, incoming, both)
//! - Path reconstruction

use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::cmp::Ordering;
use super::csr::{CsrGraph, EdgeData};
use super::types::EdgeType;

// ============================================================================
// Traversal Direction
// ============================================================================

/// Direction for graph traversal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Follow outgoing edges only
    Outgoing,
    /// Follow incoming edges only
    Incoming,
    /// Follow both directions
    Both,
}

// ============================================================================
// Edge Type Filter
// ============================================================================

/// Filter for selecting which edge types to traverse
#[derive(Debug, Clone)]
pub enum EdgeFilter {
    /// Accept all edge types
    All,
    /// Accept only these specific types
    Include(Vec<EdgeType>),
    /// Accept all except these types
    Exclude(Vec<EdgeType>),
    /// Custom filter function (edge type byte -> bool)
    Custom(fn(u8) -> bool),
}

impl EdgeFilter {
    /// Check if an edge type passes the filter
    pub fn accepts(&self, edge_type: u8) -> bool {
        match self {
            EdgeFilter::All => true,
            EdgeFilter::Include(types) => {
                types.iter().any(|t| t.to_byte() == edge_type)
            }
            EdgeFilter::Exclude(types) => {
                !types.iter().any(|t| t.to_byte() == edge_type)
            }
            EdgeFilter::Custom(f) => f(edge_type),
        }
    }

    /// Create a filter for semantic edges only
    pub fn semantic_only() -> Self {
        EdgeFilter::Custom(|b| b >= 10 && b < 20)
    }

    /// Create a filter for causal edges only
    pub fn causal_only() -> Self {
        EdgeFilter::Custom(|b| b >= 20 && b < 30)
    }

    /// Create a filter for transitive edges only
    pub fn transitive_only() -> Self {
        EdgeFilter::Custom(|b| {
            // IsA, PartOf, Causes, Implies, Requires, Before, After
            matches!(b, 10 | 11 | 20 | 21 | 26 | 30 | 31)
        })
    }
}

impl Default for EdgeFilter {
    fn default() -> Self {
        EdgeFilter::All
    }
}

// ============================================================================
// Traversal Result
// ============================================================================

/// A path through the graph
#[derive(Debug, Clone)]
pub struct Path {
    /// Sequence of node IDs from start to end
    pub nodes: Vec<u64>,
    /// Edge data for each hop (length = nodes.len() - 1)
    pub edges: Vec<EdgeData>,
    /// Total path weight (sum of edge weights)
    pub total_weight: f32,
    /// Total confidence (product of edge confidences)
    pub total_confidence: f32,
}

impl Path {
    /// Create an empty path starting at a node
    pub fn new(start: u64) -> Self {
        Self {
            nodes: vec![start],
            edges: Vec::new(),
            total_weight: 0.0,
            total_confidence: 1.0,
        }
    }

    /// Extend the path with a new edge
    pub fn extend(&mut self, target: u64, edge: EdgeData) {
        self.total_weight += edge.weight;
        self.total_confidence *= edge.confidence;
        self.nodes.push(target);
        self.edges.push(edge);
    }

    /// Get the number of hops in the path
    pub fn hop_count(&self) -> usize {
        self.edges.len()
    }

    /// Get the start node
    pub fn start(&self) -> u64 {
        self.nodes[0]
    }

    /// Get the end node
    pub fn end(&self) -> u64 {
        *self.nodes.last().unwrap()
    }

    /// Clone and extend with a new edge
    pub fn with_edge(&self, target: u64, edge: EdgeData) -> Self {
        let mut new_path = self.clone();
        new_path.extend(target, edge);
        new_path
    }
}

/// Result of a multi-hop traversal
#[derive(Debug, Clone)]
pub struct TraversalResult {
    /// All nodes visited during traversal
    pub visited: HashSet<u64>,
    /// Nodes at each depth level
    pub by_depth: HashMap<usize, Vec<u64>>,
    /// Parent node for each visited node (for path reconstruction)
    pub parents: HashMap<u64, (u64, EdgeData)>,
    /// Starting node
    pub start: u64,
}

impl TraversalResult {
    /// Create a new traversal result
    fn new(start: u64) -> Self {
        let mut visited = HashSet::new();
        visited.insert(start);
        let mut by_depth = HashMap::new();
        by_depth.insert(0, vec![start]);

        Self {
            visited,
            by_depth,
            parents: HashMap::new(),
            start,
        }
    }

    /// Reconstruct path from start to target
    pub fn path_to(&self, target: u64) -> Option<Path> {
        if !self.visited.contains(&target) {
            return None;
        }

        if target == self.start {
            return Some(Path::new(self.start));
        }

        // Trace back from target to start
        let mut path_nodes = vec![target];
        let mut path_edges = Vec::new();
        let mut current = target;

        while let Some((parent, edge)) = self.parents.get(&current) {
            path_nodes.push(*parent);
            path_edges.push(edge.clone());
            current = *parent;
            if current == self.start {
                break;
            }
        }

        // Reverse to get start -> target order
        path_nodes.reverse();
        path_edges.reverse();

        let mut path = Path::new(self.start);
        for (i, edge) in path_edges.into_iter().enumerate() {
            path.extend(path_nodes[i + 1], edge);
        }

        Some(path)
    }

    /// Get all nodes at a specific depth
    pub fn at_depth(&self, depth: usize) -> &[u64] {
        self.by_depth.get(&depth).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Get maximum depth reached
    pub fn max_depth(&self) -> usize {
        self.by_depth.keys().max().copied().unwrap_or(0)
    }
}

// ============================================================================
// BFS (Breadth-First Search)
// ============================================================================

/// Perform BFS traversal from a starting node
///
/// Returns all reachable nodes within max_depth hops, following edges
/// that pass the filter in the specified direction.
pub fn bfs(
    graph: &CsrGraph,
    start: u64,
    max_depth: usize,
    direction: Direction,
    filter: &EdgeFilter,
) -> TraversalResult {
    let mut result = TraversalResult::new(start);
    let mut queue: VecDeque<(u64, usize)> = VecDeque::new();
    queue.push_back((start, 0));

    while let Some((node, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        // Get neighbors based on direction
        let neighbors = get_neighbors(graph, node, direction, filter);

        for (neighbor, edge) in neighbors {
            if !result.visited.contains(&neighbor) {
                result.visited.insert(neighbor);
                result.parents.insert(neighbor, (node, edge));
                result.by_depth.entry(depth + 1).or_insert_with(Vec::new).push(neighbor);
                queue.push_back((neighbor, depth + 1));
            }
        }
    }

    result
}

/// Find shortest path (by hop count) using BFS
pub fn shortest_path_bfs(
    graph: &CsrGraph,
    start: u64,
    end: u64,
    max_depth: usize,
    direction: Direction,
    filter: &EdgeFilter,
) -> Option<Path> {
    if start == end {
        return Some(Path::new(start));
    }

    let mut visited = HashSet::new();
    visited.insert(start);
    let mut parents: HashMap<u64, (u64, EdgeData)> = HashMap::new();
    let mut queue: VecDeque<(u64, usize)> = VecDeque::new();
    queue.push_back((start, 0));

    while let Some((node, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        let neighbors = get_neighbors(graph, node, direction, filter);

        for (neighbor, edge) in neighbors {
            if !visited.contains(&neighbor) {
                visited.insert(neighbor);
                parents.insert(neighbor, (node, edge));

                if neighbor == end {
                    // Found! Reconstruct path
                    return Some(reconstruct_path(start, end, &parents));
                }

                queue.push_back((neighbor, depth + 1));
            }
        }
    }

    None
}

// ============================================================================
// DFS (Depth-First Search)
// ============================================================================

/// Perform DFS traversal from a starting node
pub fn dfs(
    graph: &CsrGraph,
    start: u64,
    max_depth: usize,
    direction: Direction,
    filter: &EdgeFilter,
) -> TraversalResult {
    let mut result = TraversalResult::new(start);
    dfs_recursive(graph, start, 0, max_depth, direction, filter, &mut result);
    result
}

fn dfs_recursive(
    graph: &CsrGraph,
    node: u64,
    depth: usize,
    max_depth: usize,
    direction: Direction,
    filter: &EdgeFilter,
    result: &mut TraversalResult,
) {
    if depth >= max_depth {
        return;
    }

    let neighbors = get_neighbors(graph, node, direction, filter);

    for (neighbor, edge) in neighbors {
        if !result.visited.contains(&neighbor) {
            result.visited.insert(neighbor);
            result.parents.insert(neighbor, (node, edge));
            result.by_depth.entry(depth + 1).or_insert_with(Vec::new).push(neighbor);
            dfs_recursive(graph, neighbor, depth + 1, max_depth, direction, filter, result);
        }
    }
}

/// Find any path using DFS (not necessarily shortest)
pub fn find_path_dfs(
    graph: &CsrGraph,
    start: u64,
    end: u64,
    max_depth: usize,
    direction: Direction,
    filter: &EdgeFilter,
) -> Option<Path> {
    if start == end {
        return Some(Path::new(start));
    }

    let mut visited = HashSet::new();
    visited.insert(start);
    let mut path = Path::new(start);

    if dfs_find_path_recursive(graph, start, end, 0, max_depth, direction, filter, &mut visited, &mut path) {
        Some(path)
    } else {
        None
    }
}

fn dfs_find_path_recursive(
    graph: &CsrGraph,
    current: u64,
    end: u64,
    depth: usize,
    max_depth: usize,
    direction: Direction,
    filter: &EdgeFilter,
    visited: &mut HashSet<u64>,
    path: &mut Path,
) -> bool {
    if depth >= max_depth {
        return false;
    }

    let neighbors = get_neighbors(graph, current, direction, filter);

    for (neighbor, edge) in neighbors {
        if !visited.contains(&neighbor) {
            visited.insert(neighbor);
            path.extend(neighbor, edge);

            if neighbor == end {
                return true;
            }

            if dfs_find_path_recursive(graph, neighbor, end, depth + 1, max_depth, direction, filter, visited, path) {
                return true;
            }

            // Backtrack
            path.nodes.pop();
            path.edges.pop();
        }
    }

    false
}

// ============================================================================
// Dijkstra's Algorithm (Weighted Shortest Path)
// ============================================================================

/// Node in Dijkstra's priority queue
#[derive(Clone)]
struct DijkstraNode {
    node_id: u64,
    distance: f32,
}

impl Eq for DijkstraNode {}

impl PartialEq for DijkstraNode {
    fn eq(&self, other: &Self) -> bool {
        self.node_id == other.node_id
    }
}

impl Ord for DijkstraNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap
        other.distance.partial_cmp(&self.distance).unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for DijkstraNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Find shortest weighted path using Dijkstra's algorithm
///
/// Uses edge weights for distance calculation. Lower weight = shorter distance.
pub fn dijkstra(
    graph: &CsrGraph,
    start: u64,
    end: u64,
    direction: Direction,
    filter: &EdgeFilter,
) -> Option<Path> {
    if start == end {
        return Some(Path::new(start));
    }

    let mut distances: HashMap<u64, f32> = HashMap::new();
    let mut parents: HashMap<u64, (u64, EdgeData)> = HashMap::new();
    let mut heap = BinaryHeap::new();

    distances.insert(start, 0.0);
    heap.push(DijkstraNode { node_id: start, distance: 0.0 });

    while let Some(DijkstraNode { node_id, distance }) = heap.pop() {
        // Skip if we've found a better path
        if let Some(&best) = distances.get(&node_id) {
            if distance > best {
                continue;
            }
        }

        if node_id == end {
            return Some(reconstruct_path(start, end, &parents));
        }

        let neighbors = get_neighbors(graph, node_id, direction, filter);

        for (neighbor, edge) in neighbors {
            // Use 1.0 - weight as distance (higher weight = shorter distance)
            // Or invert: lower weight edges are "cheaper" to traverse
            let edge_distance = 1.0 - edge.weight.min(0.999);
            let new_distance = distance + edge_distance;

            let current_best = distances.get(&neighbor).copied().unwrap_or(f32::INFINITY);
            if new_distance < current_best {
                distances.insert(neighbor, new_distance);
                parents.insert(neighbor, (node_id, edge));
                heap.push(DijkstraNode { node_id: neighbor, distance: new_distance });
            }
        }
    }

    None
}

/// Find shortest weighted path considering confidence
///
/// Uses effective strength (weight * confidence * type_factor) for distance.
pub fn dijkstra_by_strength(
    graph: &CsrGraph,
    start: u64,
    end: u64,
    direction: Direction,
    filter: &EdgeFilter,
) -> Option<Path> {
    if start == end {
        return Some(Path::new(start));
    }

    let mut distances: HashMap<u64, f32> = HashMap::new();
    let mut parents: HashMap<u64, (u64, EdgeData)> = HashMap::new();
    let mut heap = BinaryHeap::new();

    distances.insert(start, 0.0);
    heap.push(DijkstraNode { node_id: start, distance: 0.0 });

    while let Some(DijkstraNode { node_id, distance }) = heap.pop() {
        if let Some(&best) = distances.get(&node_id) {
            if distance > best {
                continue;
            }
        }

        if node_id == end {
            return Some(reconstruct_path(start, end, &parents));
        }

        let neighbors = get_neighbors(graph, node_id, direction, filter);

        for (neighbor, edge) in neighbors {
            // Higher effective strength = shorter distance
            let strength = edge.effective_strength();
            let edge_distance = 1.0 - strength.min(0.999);
            let new_distance = distance + edge_distance;

            let current_best = distances.get(&neighbor).copied().unwrap_or(f32::INFINITY);
            if new_distance < current_best {
                distances.insert(neighbor, new_distance);
                parents.insert(neighbor, (node_id, edge));
                heap.push(DijkstraNode { node_id: neighbor, distance: new_distance });
            }
        }
    }

    None
}

// ============================================================================
// A* Search (Heuristic-Guided)
// ============================================================================

/// Heuristic function type for A* search
pub type Heuristic = fn(current: u64, goal: u64) -> f32;

/// Default heuristic: returns 0 (degenerates to Dijkstra)
pub fn null_heuristic(_current: u64, _goal: u64) -> f32 {
    0.0
}

/// Node in A* priority queue
#[derive(Clone)]
struct AStarNode {
    node_id: u64,
    g_score: f32,  // Cost from start
    f_score: f32,  // g_score + heuristic
}

impl Eq for AStarNode {}

impl PartialEq for AStarNode {
    fn eq(&self, other: &Self) -> bool {
        self.node_id == other.node_id
    }
}

impl Ord for AStarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f_score.partial_cmp(&self.f_score).unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for AStarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Find shortest path using A* with a heuristic
///
/// The heuristic should be admissible (never overestimates) for optimal paths.
pub fn astar(
    graph: &CsrGraph,
    start: u64,
    end: u64,
    direction: Direction,
    filter: &EdgeFilter,
    heuristic: Heuristic,
) -> Option<Path> {
    if start == end {
        return Some(Path::new(start));
    }

    let mut g_scores: HashMap<u64, f32> = HashMap::new();
    let mut parents: HashMap<u64, (u64, EdgeData)> = HashMap::new();
    let mut open_set = BinaryHeap::new();
    let mut closed_set = HashSet::new();

    g_scores.insert(start, 0.0);
    let h = heuristic(start, end);
    open_set.push(AStarNode {
        node_id: start,
        g_score: 0.0,
        f_score: h,
    });

    while let Some(current) = open_set.pop() {
        if current.node_id == end {
            return Some(reconstruct_path(start, end, &parents));
        }

        if closed_set.contains(&current.node_id) {
            continue;
        }
        closed_set.insert(current.node_id);

        let neighbors = get_neighbors(graph, current.node_id, direction, filter);

        for (neighbor, edge) in neighbors {
            if closed_set.contains(&neighbor) {
                continue;
            }

            let edge_cost = 1.0 - edge.effective_strength().min(0.999);
            let tentative_g = current.g_score + edge_cost;

            let current_g = g_scores.get(&neighbor).copied().unwrap_or(f32::INFINITY);
            if tentative_g < current_g {
                g_scores.insert(neighbor, tentative_g);
                parents.insert(neighbor, (current.node_id, edge));

                let h = heuristic(neighbor, end);
                open_set.push(AStarNode {
                    node_id: neighbor,
                    g_score: tentative_g,
                    f_score: tentative_g + h,
                });
            }
        }
    }

    None
}

// ============================================================================
// Multi-Path Finding
// ============================================================================

/// Find all paths between two nodes up to max_depth
pub fn find_all_paths(
    graph: &CsrGraph,
    start: u64,
    end: u64,
    max_depth: usize,
    direction: Direction,
    filter: &EdgeFilter,
    max_paths: usize,
) -> Vec<Path> {
    let mut results = Vec::new();
    let mut visited = HashSet::new();
    visited.insert(start);
    let mut path = Path::new(start);

    find_all_paths_recursive(
        graph, start, end, max_depth, direction, filter,
        &mut visited, &mut path, &mut results, max_paths,
    );

    results
}

fn find_all_paths_recursive(
    graph: &CsrGraph,
    current: u64,
    end: u64,
    max_depth: usize,
    direction: Direction,
    filter: &EdgeFilter,
    visited: &mut HashSet<u64>,
    current_path: &mut Path,
    results: &mut Vec<Path>,
    max_paths: usize,
) {
    if results.len() >= max_paths {
        return;
    }

    if current_path.hop_count() >= max_depth {
        return;
    }

    let neighbors = get_neighbors(graph, current, direction, filter);

    for (neighbor, edge) in neighbors {
        if neighbor == end {
            let mut final_path = current_path.clone();
            final_path.extend(neighbor, edge);
            results.push(final_path);
            if results.len() >= max_paths {
                return;
            }
        } else if !visited.contains(&neighbor) {
            visited.insert(neighbor);
            current_path.extend(neighbor, edge.clone());

            find_all_paths_recursive(
                graph, neighbor, end, max_depth, direction, filter,
                visited, current_path, results, max_paths,
            );

            // Backtrack
            current_path.nodes.pop();
            current_path.edges.pop();
            visited.remove(&neighbor);
        }
    }
}

// ============================================================================
// N-Hop Neighbors
// ============================================================================

/// Get all nodes within N hops
pub fn n_hop_neighbors(
    graph: &CsrGraph,
    start: u64,
    n: usize,
    direction: Direction,
    filter: &EdgeFilter,
) -> HashSet<u64> {
    let result = bfs(graph, start, n, direction, filter);
    result.visited
}

/// Get nodes at exactly N hops (not closer)
pub fn nodes_at_distance(
    graph: &CsrGraph,
    start: u64,
    n: usize,
    direction: Direction,
    filter: &EdgeFilter,
) -> Vec<u64> {
    let result = bfs(graph, start, n, direction, filter);
    result.at_depth(n).to_vec()
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Get neighbors based on direction and filter
fn get_neighbors(
    graph: &CsrGraph,
    node: u64,
    direction: Direction,
    filter: &EdgeFilter,
) -> Vec<(u64, EdgeData)> {
    let mut neighbors = Vec::new();

    match direction {
        Direction::Outgoing => {
            for (target, edge) in graph.outgoing_edges(node) {
                if filter.accepts(edge.edge_type) {
                    neighbors.push((target, edge));
                }
            }
        }
        Direction::Incoming => {
            for (source, edge) in graph.incoming_edges(node) {
                if filter.accepts(edge.edge_type) {
                    neighbors.push((source, edge));
                }
            }
        }
        Direction::Both => {
            for (target, edge) in graph.outgoing_edges(node) {
                if filter.accepts(edge.edge_type) {
                    neighbors.push((target, edge));
                }
            }
            for (source, edge) in graph.incoming_edges(node) {
                if filter.accepts(edge.edge_type) {
                    neighbors.push((source, edge));
                }
            }
        }
    }

    neighbors
}

/// Reconstruct path from parent map
fn reconstruct_path(start: u64, end: u64, parents: &HashMap<u64, (u64, EdgeData)>) -> Path {
    let mut path_nodes = vec![end];
    let mut path_edges = Vec::new();
    let mut current = end;

    while let Some((parent, edge)) = parents.get(&current) {
        path_nodes.push(*parent);
        path_edges.push(edge.clone());
        current = *parent;
        if current == start {
            break;
        }
    }

    path_nodes.reverse();
    path_edges.reverse();

    let mut path = Path::new(start);
    for (i, edge) in path_edges.into_iter().enumerate() {
        path.extend(path_nodes[i + 1], edge);
    }

    path
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Edge, EdgeType as KgEdgeType, SemanticEdge, CausalEdge};

    fn create_test_graph() -> CsrGraph {
        // Create a graph:
        //   1 --IsA--> 2 --IsA--> 3
        //   |                     ^
        //   +---Causes---> 4 -----+
        //                  |
        //                  v
        //                  5
        let edges = vec![
            Edge::new(1, 2, KgEdgeType::Semantic(SemanticEdge::IsA), 0.9),
            Edge::new(2, 3, KgEdgeType::Semantic(SemanticEdge::IsA), 0.8),
            Edge::new(1, 4, KgEdgeType::Causal(CausalEdge::Causes), 0.7),
            Edge::new(4, 3, KgEdgeType::Semantic(SemanticEdge::RelatedTo), 0.6),
            Edge::new(4, 5, KgEdgeType::Causal(CausalEdge::Enables), 0.5),
        ];
        CsrGraph::from_edges(&edges)
    }

    #[test]
    fn test_bfs_basic() {
        let graph = create_test_graph();
        let result = bfs(&graph, 1, 3, Direction::Outgoing, &EdgeFilter::All);

        assert!(result.visited.contains(&1));
        assert!(result.visited.contains(&2));
        assert!(result.visited.contains(&3));
        assert!(result.visited.contains(&4));
        assert!(result.visited.contains(&5));
        assert_eq!(result.visited.len(), 5);
    }

    #[test]
    fn test_bfs_depth_limit() {
        let graph = create_test_graph();
        let result = bfs(&graph, 1, 1, Direction::Outgoing, &EdgeFilter::All);

        assert!(result.visited.contains(&1));
        assert!(result.visited.contains(&2));
        assert!(result.visited.contains(&4));
        assert!(!result.visited.contains(&3)); // 2 hops away
        assert!(!result.visited.contains(&5)); // 2 hops away
    }

    #[test]
    fn test_bfs_with_filter() {
        let graph = create_test_graph();
        let filter = EdgeFilter::semantic_only();
        let result = bfs(&graph, 1, 3, Direction::Outgoing, &filter);

        assert!(result.visited.contains(&1));
        assert!(result.visited.contains(&2));
        assert!(result.visited.contains(&3));
        assert!(!result.visited.contains(&4)); // Only causal edge
        assert!(!result.visited.contains(&5)); // Beyond causal edge
    }

    #[test]
    fn test_shortest_path() {
        let graph = create_test_graph();
        let path = shortest_path_bfs(&graph, 1, 3, 5, Direction::Outgoing, &EdgeFilter::All);

        assert!(path.is_some());
        let path = path.unwrap();
        assert_eq!(path.start(), 1);
        assert_eq!(path.end(), 3);
        assert_eq!(path.hop_count(), 2); // 1 -> 2 -> 3
    }

    #[test]
    fn test_dfs_basic() {
        let graph = create_test_graph();
        let result = dfs(&graph, 1, 3, Direction::Outgoing, &EdgeFilter::All);

        assert_eq!(result.visited.len(), 5);
    }

    #[test]
    fn test_find_path_dfs() {
        let graph = create_test_graph();
        let path = find_path_dfs(&graph, 1, 5, 5, Direction::Outgoing, &EdgeFilter::All);

        assert!(path.is_some());
        let path = path.unwrap();
        assert_eq!(path.start(), 1);
        assert_eq!(path.end(), 5);
    }

    #[test]
    fn test_dijkstra() {
        let graph = create_test_graph();
        let path = dijkstra(&graph, 1, 3, Direction::Outgoing, &EdgeFilter::All);

        assert!(path.is_some());
        let path = path.unwrap();
        assert_eq!(path.start(), 1);
        assert_eq!(path.end(), 3);
    }

    #[test]
    fn test_astar() {
        let graph = create_test_graph();
        let path = astar(&graph, 1, 3, Direction::Outgoing, &EdgeFilter::All, null_heuristic);

        assert!(path.is_some());
        let path = path.unwrap();
        assert_eq!(path.start(), 1);
        assert_eq!(path.end(), 3);
    }

    #[test]
    fn test_find_all_paths() {
        let graph = create_test_graph();
        let paths = find_all_paths(&graph, 1, 3, 3, Direction::Outgoing, &EdgeFilter::All, 10);

        // Should find two paths: 1->2->3 and 1->4->3
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn test_n_hop_neighbors() {
        let graph = create_test_graph();
        let neighbors = n_hop_neighbors(&graph, 1, 2, Direction::Outgoing, &EdgeFilter::All);

        assert!(neighbors.contains(&1));
        assert!(neighbors.contains(&2));
        assert!(neighbors.contains(&3));
        assert!(neighbors.contains(&4));
        assert!(neighbors.contains(&5));
    }

    #[test]
    fn test_nodes_at_distance() {
        let graph = create_test_graph();
        let nodes = nodes_at_distance(&graph, 1, 2, Direction::Outgoing, &EdgeFilter::All);

        // At exactly 2 hops: 3, 5
        assert!(nodes.contains(&3));
        assert!(nodes.contains(&5));
        assert!(!nodes.contains(&1));
        assert!(!nodes.contains(&2));
        assert!(!nodes.contains(&4));
    }

    #[test]
    fn test_bidirectional_traversal() {
        let graph = create_test_graph();
        let result = bfs(&graph, 3, 2, Direction::Incoming, &EdgeFilter::All);

        // Should find 2 (direct incoming) and 4 (incoming)
        assert!(result.visited.contains(&2));
        assert!(result.visited.contains(&4));
    }

    #[test]
    fn test_path_confidence() {
        let graph = create_test_graph();
        let path = shortest_path_bfs(&graph, 1, 3, 5, Direction::Outgoing, &EdgeFilter::All).unwrap();

        // Path 1->2->3 with confidences 1.0 * 1.0 = 1.0 (default confidences)
        assert!(path.total_confidence > 0.0);
    }
}
