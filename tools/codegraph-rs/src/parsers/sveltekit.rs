//! SvelteKit route parser
//!
//! Extracts routes from SvelteKit's file-based routing system:
//! - /src/routes/+page.svelte → /
//! - /src/routes/diet/+page.svelte → /diet
//! - /src/routes/diet/tracker/+page.svelte → /diet/tracker
//! - /src/routes/[slug]/+page.svelte → /[slug] (dynamic)

use crate::graph::{CodeGraph, Node};
use crate::parser::{Parser, NavigationLink};
use std::path::Path;
use std::fs;
use walkdir::WalkDir;
use regex::Regex;
use anyhow::Result;

pub struct SvelteKitParser {
    /// Regex for extracting goto() navigation
    goto_regex: Regex,
    /// Regex for extracting <a href="..."> links
    href_regex: Regex,
    /// Regex for extracting $app/navigation imports
    #[allow(dead_code)]
    nav_import_regex: Regex,
}

impl SvelteKitParser {
    pub fn new() -> Self {
        Self {
            goto_regex: Regex::new(r#"goto\s*\(\s*['"`]([^'"`]+)['"`]"#).unwrap(),
            href_regex: Regex::new(r#"href\s*=\s*['"`]([^'"`]+)['"`]"#).unwrap(),
            nav_import_regex: Regex::new(r#"from\s+['"](\$app/navigation)['"]"#).unwrap(),
        }
    }

    /// Convert file path to route path
    /// /src/routes/diet/tracker/+page.svelte → /diet/tracker
    fn file_to_route(&self, file_path: &Path, routes_dir: &Path) -> Option<String> {
        let relative = file_path.strip_prefix(routes_dir).ok()?;
        let parent = relative.parent()?;

        // Build route path from directory structure
        let mut route = String::from("/");
        for component in parent.components() {
            let part = component.as_os_str().to_string_lossy();
            // Skip group folders like (auth), (main)
            if part.starts_with('(') && part.ends_with(')') {
                continue;
            }
            if !route.ends_with('/') {
                route.push('/');
            }
            route.push_str(&part);
        }

        // Clean up trailing slash for non-root
        if route.len() > 1 && route.ends_with('/') {
            route.pop();
        }

        Some(route)
    }

    /// Extract route name from path
    fn route_to_name(&self, route: &str) -> String {
        if route == "/" {
            return "Home".to_string();
        }

        // Get last segment and convert to title case
        let last = route.rsplit('/').next().unwrap_or(route);

        // Handle dynamic segments like [slug]
        if last.starts_with('[') && last.ends_with(']') {
            let inner = &last[1..last.len()-1];
            return format!("{} (Dynamic)", to_title_case(inner));
        }

        to_title_case(last)
    }

    /// Extract navigation links from a Svelte file
    fn extract_links(&self, content: &str, from_route: &str) -> Vec<NavigationLink> {
        let mut links = Vec::new();

        // Extract goto() calls
        for cap in self.goto_regex.captures_iter(content) {
            if let Some(target) = cap.get(1) {
                let to = target.as_str();
                // Skip external URLs and template literals with variables
                if !to.starts_with("http") && !to.contains("${") {
                    links.push(NavigationLink::new(from_route, to, "goto"));
                }
            }
        }

        // Extract href attributes (internal links only)
        for cap in self.href_regex.captures_iter(content) {
            if let Some(target) = cap.get(1) {
                let to = target.as_str();
                // Only internal links starting with /
                if to.starts_with('/') && !to.starts_with("//") {
                    links.push(NavigationLink::new(from_route, to, "link"));
                }
            }
        }

        links
    }
}

impl Parser for SvelteKitParser {
    fn framework(&self) -> &str {
        "sveltekit"
    }

    fn extensions(&self) -> &[&str] {
        &[".svelte", ".ts", ".js"]
    }

    fn can_parse(&self, root: &Path) -> bool {
        // Check for SvelteKit markers
        let has_svelte_config = root.join("svelte.config.js").exists()
            || root.join("svelte.config.ts").exists();
        let has_routes = root.join("src/routes").exists();

        has_svelte_config && has_routes
    }

    fn parse(&self, root: &Path, name: &str) -> Result<CodeGraph> {
        let mut graph = CodeGraph::new(name, "sveltekit", &root.to_string_lossy());
        let routes_dir = root.join("src/routes");

        if !routes_dir.exists() {
            return Err(anyhow::anyhow!("No src/routes directory found"));
        }

        // First pass: collect all routes
        let mut routes: Vec<(String, String, String)> = Vec::new(); // (route_path, name, file_path)

        for entry in WalkDir::new(&routes_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            let file_name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            // Only process +page.svelte files (routes)
            if file_name == "+page.svelte" {
                if let Some(route_path) = self.file_to_route(path, &routes_dir) {
                    let route_name = self.route_to_name(&route_path);
                    let file_path = path.to_string_lossy().to_string();
                    routes.push((route_path, route_name, file_path));
                }
            }
        }

        // Add route nodes
        for (route_path, route_name, file_path) in &routes {
            let node = Node::route(route_path, route_name, route_path, file_path, "sveltekit");
            graph.add_node(node);
        }

        // Second pass: extract navigation links
        for (route_path, _, file_path) in &routes {
            if let Ok(content) = fs::read_to_string(file_path) {
                let links = self.extract_links(&content, route_path);
                for link in links {
                    // Only add edge if target route exists in our graph
                    if graph.find_node(&link.to).is_some() {
                        graph.add_edge(link.to_edge());
                    }
                }
            }
        }

        // Also check +layout.svelte files for navigation structure
        for entry in WalkDir::new(&routes_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            let file_name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            if file_name == "+layout.svelte" {
                if let Ok(content) = fs::read_to_string(path) {
                    // Layout affects all child routes - extract links
                    if let Some(layout_route) = self.file_to_route(path, &routes_dir) {
                        let links = self.extract_links(&content, &layout_route);
                        for link in links {
                            if graph.find_node(&link.to).is_some() {
                                graph.add_edge(link.to_edge());
                            }
                        }
                    }
                }
            }
        }

        Ok(graph)
    }
}

impl Default for SvelteKitParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert kebab-case or snake_case to Title Case
fn to_title_case(s: &str) -> String {
    s.split(|c| c == '-' || c == '_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_file_to_route() {
        let parser = SvelteKitParser::new();
        let routes_dir = PathBuf::from("/project/src/routes");

        // Root
        let path = PathBuf::from("/project/src/routes/+page.svelte");
        assert_eq!(parser.file_to_route(&path, &routes_dir), Some("/".to_string()));

        // Nested
        let path = PathBuf::from("/project/src/routes/diet/tracker/+page.svelte");
        assert_eq!(parser.file_to_route(&path, &routes_dir), Some("/diet/tracker".to_string()));

        // With group folder
        let path = PathBuf::from("/project/src/routes/(main)/diet/+page.svelte");
        assert_eq!(parser.file_to_route(&path, &routes_dir), Some("/diet".to_string()));
    }

    #[test]
    fn test_route_to_name() {
        let parser = SvelteKitParser::new();

        assert_eq!(parser.route_to_name("/"), "Home");
        assert_eq!(parser.route_to_name("/diet"), "Diet");
        assert_eq!(parser.route_to_name("/diet/tracker"), "Tracker");
        assert_eq!(parser.route_to_name("/theme-shop"), "Theme Shop");
        assert_eq!(parser.route_to_name("/[slug]"), "Slug (Dynamic)");
    }

    #[test]
    fn test_extract_links() {
        let parser = SvelteKitParser::new();
        let content = r#"
            <a href="/diet">Go to Diet</a>
            <button on:click={() => goto('/training')}>Train</button>
            <a href="https://example.com">External</a>
        "#;

        let links = parser.extract_links(content, "/home");
        assert_eq!(links.len(), 2);
        assert!(links.iter().any(|l| l.to == "/diet" && l.nav_type == "link"));
        assert!(links.iter().any(|l| l.to == "/training" && l.nav_type == "goto"));
    }
}
