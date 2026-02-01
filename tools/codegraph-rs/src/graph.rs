//! Core graph data structures for CodeGraph
//!
//! Designed for:
//! - Framework-agnostic representation
//! - Fast querying and comparison
//! - JSON serialization for AI consumption

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use petgraph::graph::{DiGraph, NodeIndex};

/// The kind of node in the code graph
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// A navigable route/screen
    Route,
    /// A UI component
    Component,
    /// A data store/state
    Store,
    /// An API endpoint
    Endpoint,
    /// A utility/helper function
    Utility,
    /// A type/interface definition
    Type,
    /// A configuration file
    Config,
    /// Unknown/other
    Other,
}

/// The kind of edge (relationship) between nodes
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Navigation from one route to another
    NavigatesTo,
    /// Component imports/uses another component
    Uses,
    /// Component renders another component
    Renders,
    /// Calls a function/method
    Calls,
    /// Reads from a store
    ReadsFrom,
    /// Writes to a store
    WritesTo,
    /// Implements an interface/type
    Implements,
    /// Extends/inherits from
    Extends,
    /// Generic dependency
    DependsOn,
}

/// A node in the code graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Unique identifier (typically file path or route path)
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Kind of node
    pub kind: NodeKind,
    /// File path (if applicable)
    pub file_path: Option<String>,
    /// Route path (if applicable, e.g., "/theme-shop")
    pub route_path: Option<String>,
    /// Framework that this node comes from
    pub framework: String,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

/// An edge (relationship) in the code graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// Source node ID
    pub from: String,
    /// Target node ID
    pub to: String,
    /// Kind of relationship
    pub kind: EdgeKind,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

/// The complete code graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeGraph {
    /// Name of this graph (e.g., "MyApp PWA", "MyApp Mobile")
    pub name: String,
    /// Framework/platform this graph represents
    pub framework: String,
    /// Root directory this graph was parsed from
    pub root_path: String,
    /// All nodes in the graph
    pub nodes: Vec<Node>,
    /// All edges in the graph
    pub edges: Vec<Edge>,
    /// Graph statistics
    pub stats: GraphStats,
}

/// Statistics about the graph
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphStats {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub routes: usize,
    pub components: usize,
    pub stores: usize,
    pub endpoints: usize,
}

impl CodeGraph {
    /// Create a new empty graph
    pub fn new(name: &str, framework: &str, root_path: &str) -> Self {
        Self {
            name: name.to_string(),
            framework: framework.to_string(),
            root_path: root_path.to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
            stats: GraphStats::default(),
        }
    }

    /// Add a node to the graph
    pub fn add_node(&mut self, node: Node) {
        self.nodes.push(node);
        self.update_stats();
    }

    /// Add an edge to the graph
    pub fn add_edge(&mut self, edge: Edge) {
        self.edges.push(edge);
        self.update_stats();
    }

    /// Find a node by ID
    pub fn find_node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Find all routes
    pub fn routes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(|n| n.kind == NodeKind::Route)
    }

    /// Find all components
    pub fn components(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(|n| n.kind == NodeKind::Component)
    }

    /// Find edges from a node
    pub fn edges_from<'a>(&'a self, node_id: &'a str) -> impl Iterator<Item = &'a Edge> + 'a {
        self.edges.iter().filter(move |e| e.from == node_id)
    }

    /// Find edges to a node
    pub fn edges_to<'a>(&'a self, node_id: &'a str) -> impl Iterator<Item = &'a Edge> + 'a {
        self.edges.iter().filter(move |e| e.to == node_id)
    }

    /// Update statistics
    fn update_stats(&mut self) {
        self.stats = GraphStats {
            total_nodes: self.nodes.len(),
            total_edges: self.edges.len(),
            routes: self.nodes.iter().filter(|n| n.kind == NodeKind::Route).count(),
            components: self.nodes.iter().filter(|n| n.kind == NodeKind::Component).count(),
            stores: self.nodes.iter().filter(|n| n.kind == NodeKind::Store).count(),
            endpoints: self.nodes.iter().filter(|n| n.kind == NodeKind::Endpoint).count(),
        };
    }

    /// Convert to petgraph for advanced operations
    pub fn to_petgraph(&self) -> DiGraph<&Node, &Edge> {
        let mut graph = DiGraph::new();
        let mut node_indices: HashMap<&str, NodeIndex> = HashMap::new();

        // Add all nodes
        for node in &self.nodes {
            let idx = graph.add_node(node);
            node_indices.insert(&node.id, idx);
        }

        // Add all edges
        for edge in &self.edges {
            if let (Some(&from_idx), Some(&to_idx)) =
                (node_indices.get(edge.from.as_str()), node_indices.get(edge.to.as_str()))
            {
                graph.add_edge(from_idx, to_idx, edge);
            }
        }

        graph
    }
}

impl Node {
    /// Create a new route node
    pub fn route(id: &str, name: &str, route_path: &str, file_path: &str, framework: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            kind: NodeKind::Route,
            file_path: Some(file_path.to_string()),
            route_path: Some(route_path.to_string()),
            framework: framework.to_string(),
            metadata: HashMap::new(),
        }
    }

    /// Create a new component node
    pub fn component(id: &str, name: &str, file_path: &str, framework: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            kind: NodeKind::Component,
            file_path: Some(file_path.to_string()),
            route_path: None,
            framework: framework.to_string(),
            metadata: HashMap::new(),
        }
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }
}

impl Edge {
    /// Create a new edge
    pub fn new(from: &str, to: &str, kind: EdgeKind) -> Self {
        Self {
            from: from.to_string(),
            to: to.to_string(),
            kind,
            metadata: HashMap::new(),
        }
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_graph() {
        let mut graph = CodeGraph::new("Test", "test", "/path");
        graph.add_node(Node::route("home", "Home", "/", "src/home.rs", "test"));
        graph.add_node(Node::route("settings", "Settings", "/settings", "src/settings.rs", "test"));
        graph.add_edge(Edge::new("home", "settings", EdgeKind::NavigatesTo));

        assert_eq!(graph.stats.total_nodes, 2);
        assert_eq!(graph.stats.routes, 2);
        assert_eq!(graph.stats.total_edges, 1);
    }
}
