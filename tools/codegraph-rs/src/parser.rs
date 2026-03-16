//! Parser trait for framework-agnostic code graph extraction
//!
//! Each framework parser implements this trait to extract nodes and edges
//! from source files in their specific format.

use crate::graph::{CodeGraph, Node, Edge, EdgeKind};
use std::path::Path;
use anyhow::Result;

/// Framework parser trait - implement for each supported framework
pub trait Parser: Send + Sync {
    /// Framework identifier (e.g., "sveltekit", "compose", "react-router")
    fn framework(&self) -> &str;

    /// File extensions this parser handles
    fn extensions(&self) -> &[&str];

    /// Check if this parser can handle the given project
    fn can_parse(&self, root: &Path) -> bool;

    /// Parse a project directory into a CodeGraph
    fn parse(&self, root: &Path, name: &str) -> Result<CodeGraph>;

    /// Parse a single file (optional, used for incremental updates)
    fn parse_file(&self, path: &Path, graph: &mut CodeGraph) -> Result<()> {
        let _ = (path, graph);
        Ok(())
    }
}

/// Route definition extracted from source
#[derive(Debug, Clone)]
pub struct RouteDefinition {
    /// Route path (e.g., "/diet/tracker", "diet_hub")
    pub path: String,
    /// Display name
    pub name: String,
    /// Source file path
    pub file_path: String,
    /// Line number where defined
    pub line: Option<usize>,
    /// Additional metadata
    pub metadata: std::collections::HashMap<String, String>,
}

impl RouteDefinition {
    pub fn new(path: &str, name: &str, file_path: &str) -> Self {
        Self {
            path: path.to_string(),
            name: name.to_string(),
            file_path: file_path.to_string(),
            line: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    pub fn with_line(mut self, line: usize) -> Self {
        self.line = Some(line);
        self
    }

    pub fn to_node(&self, framework: &str) -> Node {
        Node::route(
            &self.path,
            &self.name,
            &self.path,
            &self.file_path,
            framework,
        )
    }
}

/// Navigation relationship between routes
#[derive(Debug, Clone)]
pub struct NavigationLink {
    /// Source route path
    pub from: String,
    /// Target route path
    pub to: String,
    /// How navigation happens (e.g., "link", "push", "replace")
    pub nav_type: String,
}

impl NavigationLink {
    pub fn new(from: &str, to: &str, nav_type: &str) -> Self {
        Self {
            from: from.to_string(),
            to: to.to_string(),
            nav_type: nav_type.to_string(),
        }
    }

    pub fn to_edge(&self) -> Edge {
        Edge::new(&self.from, &self.to, EdgeKind::NavigatesTo)
            .with_metadata("nav_type", &self.nav_type)
    }
}

/// Parser registry for managing multiple framework parsers
pub struct ParserRegistry {
    parsers: Vec<Box<dyn Parser>>,
}

impl ParserRegistry {
    pub fn new() -> Self {
        Self { parsers: Vec::new() }
    }

    /// Register a parser
    pub fn register<P: Parser + 'static>(&mut self, parser: P) {
        self.parsers.push(Box::new(parser));
    }

    /// Find parser for a project
    pub fn find_parser(&self, root: &Path) -> Option<&dyn Parser> {
        self.parsers.iter()
            .find(|p| p.can_parse(root))
            .map(|p| p.as_ref())
    }

    /// Parse a project with auto-detected parser
    pub fn parse(&self, root: &Path, name: &str) -> Result<CodeGraph> {
        let parser = self.find_parser(root)
            .ok_or_else(|| anyhow::anyhow!("No parser found for project at {:?}", root))?;
        parser.parse(root, name)
    }

    /// List registered frameworks
    pub fn frameworks(&self) -> Vec<&str> {
        self.parsers.iter().map(|p| p.framework()).collect()
    }
}

impl Default for ParserRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_definition() {
        let route = RouteDefinition::new("/diet/tracker", "Diet Tracker", "src/routes/diet/tracker/+page.svelte")
            .with_line(1);

        assert_eq!(route.path, "/diet/tracker");
        assert_eq!(route.line, Some(1));

        let node = route.to_node("sveltekit");
        assert_eq!(node.framework, "sveltekit");
    }
}
