//! Framework-specific parsers for code graph extraction
//!
//! Each parser implements the Parser trait to extract routes and relationships
//! from a specific framework's codebase.

pub mod sveltekit;
pub mod compose;
pub mod kotlin_routes;

pub use sveltekit::SvelteKitParser;
pub use compose::ComposeParser;
pub use kotlin_routes::KotlinRoutesParser;

use crate::parser::{Parser, ParserRegistry};

/// Create a registry with all built-in parsers
pub fn default_registry() -> ParserRegistry {
    let mut registry = ParserRegistry::new();
    registry.register(SvelteKitParser::new());
    registry.register(ComposeParser::new());
    registry.register(KotlinRoutesParser::new());
    registry
}
