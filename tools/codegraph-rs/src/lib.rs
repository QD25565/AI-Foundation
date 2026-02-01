//! CodeGraph - Universal code graph tool for AI codebase understanding
//!
//! Provides framework-agnostic parsing and graph operations for:
//! - Navigation routes (SvelteKit, Compose, React Router, etc.)
//! - Component relationships
//! - Cross-platform comparison
//!
//! Part of AI-Foundation: "Empowering AIs Everywhere, Always"

pub mod graph;
pub mod parser;
pub mod parsers;
pub mod compare;

pub use graph::{CodeGraph, Node, Edge, NodeKind, EdgeKind, GraphStats};
pub use parser::{Parser, ParserRegistry, RouteDefinition, NavigationLink};
pub use compare::{compare, ComparisonResult, RouteMapping, CoverageStats};
