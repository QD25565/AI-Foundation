//! Kotlin routes parser
//!
//! Extracts routes from Kotlin route definition objects:
//! - object DietRoutes { const val TRACKER = "diet_tracker" }
//! - sealed class Screen(val route: String) { object Diet : Screen("diet") }

use crate::graph::{CodeGraph, Node, NodeKind};
use crate::parser::{Parser, RouteDefinition};
use std::path::Path;
use std::fs;
use walkdir::WalkDir;
use regex::Regex;
use anyhow::Result;

pub struct KotlinRoutesParser {
    /// Regex for const val patterns: const val NAME = "route"
    const_val_regex: Regex,
    /// Regex for object routes in sealed class: object Name : Screen("route")
    sealed_object_regex: Regex,
    /// Regex for data object routes: data object Name : Screen("route", ...)
    data_object_regex: Regex,
    /// Regex for object declaration: object DietRoutes
    object_decl_regex: Regex,
}

impl KotlinRoutesParser {
    pub fn new() -> Self {
        Self {
            const_val_regex: Regex::new(
                r#"const\s+val\s+(\w+)\s*=\s*['"]([^'"]+)['"]"#
            ).unwrap(),
            sealed_object_regex: Regex::new(
                r#"object\s+(\w+)\s*:\s*\w+\s*\(\s*['"]([^'"]+)['"]"#
            ).unwrap(),
            data_object_regex: Regex::new(
                r#"data\s+object\s+(\w+)\s*:\s*\w+\s*\(\s*['"]([^'"]+)['"]"#
            ).unwrap(),
            object_decl_regex: Regex::new(
                r#"object\s+(\w+Routes)\s*\{"#
            ).unwrap(),
        }
    }

    /// Extract route definitions from a Kotlin file
    fn extract_routes(&self, content: &str, file_path: &str) -> Vec<RouteDefinition> {
        let mut routes = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        // Track current object context for const val
        let mut current_object: Option<String> = None;

        for (line_num, line) in lines.iter().enumerate() {
            // Check for object declaration (e.g., object DietRoutes {)
            if let Some(cap) = self.object_decl_regex.captures(line) {
                current_object = Some(cap[1].to_string());
            }

            // Check for const val patterns
            for cap in self.const_val_regex.captures_iter(line) {
                let name = &cap[1];
                let route = &cap[2];

                // Use object context if available
                let full_name = if let Some(ref obj) = current_object {
                    format!("{}.{}", obj.replace("Routes", ""), name)
                } else {
                    name.to_string()
                };

                let display_name = const_to_display_name(name);
                let mut route_def = RouteDefinition::new(route, &display_name, file_path)
                    .with_line(line_num + 1);
                route_def.metadata.insert("const_name".to_string(), full_name);

                routes.push(route_def);
            }

            // Check for sealed class object patterns
            for cap in self.sealed_object_regex.captures_iter(line) {
                let name = &cap[1];
                let route = &cap[2];

                let display_name = const_to_display_name(name);
                let mut route_def = RouteDefinition::new(route, &display_name, file_path)
                    .with_line(line_num + 1);
                route_def.metadata.insert("sealed_object".to_string(), name.to_string());

                routes.push(route_def);
            }

            // Check for data object patterns
            for cap in self.data_object_regex.captures_iter(line) {
                let name = &cap[1];
                let route = &cap[2];

                let display_name = const_to_display_name(name);
                let mut route_def = RouteDefinition::new(route, &display_name, file_path)
                    .with_line(line_num + 1);
                route_def.metadata.insert("data_object".to_string(), name.to_string());

                routes.push(route_def);
            }

            // Reset context when leaving object
            if line.contains('}') && current_object.is_some() {
                // Simple heuristic - might need refinement for nested braces
                let open_count = content[..content.find(line).unwrap_or(0)].matches('{').count();
                let close_count = content[..content.find(line).unwrap_or(0)].matches('}').count();
                if close_count >= open_count {
                    current_object = None;
                }
            }
        }

        routes
    }
}

impl Parser for KotlinRoutesParser {
    fn framework(&self) -> &str {
        "kotlin-routes"
    }

    fn extensions(&self) -> &[&str] {
        &[".kt"]
    }

    fn can_parse(&self, root: &Path) -> bool {
        // Check for Kotlin/Android project markers
        let has_gradle = root.join("build.gradle").exists()
            || root.join("build.gradle.kts").exists();
        let has_kotlin = root.join("src/main/kotlin").exists()
            || root.join("app/src/main/java").exists();

        has_gradle && has_kotlin
    }

    fn parse(&self, root: &Path, name: &str) -> Result<CodeGraph> {
        let mut graph = CodeGraph::new(name, "kotlin-routes", &root.to_string_lossy());

        // Search for route definition files
        let search_patterns = [
            "Routes.kt",
            "Screen.kt",
            "Screens.kt",
            "Navigation.kt",
            "NavGraph.kt",
        ];

        for entry in WalkDir::new(root)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let file_name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            // Check if file matches our patterns
            let is_route_file = search_patterns.iter().any(|p| file_name.ends_with(p))
                || file_name.contains("Route");

            if is_route_file && path.extension().map_or(false, |e| e == "kt") {
                if let Ok(content) = fs::read_to_string(path) {
                    let file_path_str = path.to_string_lossy().to_string();
                    let routes = self.extract_routes(&content, &file_path_str);

                    for route in routes {
                        graph.add_node(route.to_node("kotlin-routes"));
                    }
                }
            }
        }

        Ok(graph)
    }
}

impl Default for KotlinRoutesParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert SCREAMING_CASE or PascalCase to Display Name
fn const_to_display_name(s: &str) -> String {
    // Handle SCREAMING_CASE
    if s.contains('_') {
        return s.split('_')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => {
                        first.to_uppercase().chain(
                            chars.map(|c| c.to_ascii_lowercase())
                        ).collect()
                    }
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
    }

    // Handle PascalCase - insert space before capitals
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            result.push(' ');
        }
        result.push(c);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_const_val_extraction() {
        let parser = KotlinRoutesParser::new();
        let content = r#"
object DietRoutes {
    const val HUB = "diet_hub"
    const val TRACKER = "diet_tracker"
    const val ANALYSIS = "diet_analysis"
}
        "#;

        let routes = parser.extract_routes(content, "DietRoutes.kt");
        assert_eq!(routes.len(), 3);
        assert!(routes.iter().any(|r| r.path == "diet_hub"));
        assert!(routes.iter().any(|r| r.path == "diet_tracker"));
    }

    #[test]
    fn test_sealed_class_extraction() {
        let parser = KotlinRoutesParser::new();
        let content = r#"
sealed class Screen(val route: String) {
    object Home : Screen("home")
    object Diet : Screen("diet")
    data object Training : Screen("training", "Training Hub")
}
        "#;

        let routes = parser.extract_routes(content, "Screen.kt");
        assert_eq!(routes.len(), 3);
        assert!(routes.iter().any(|r| r.path == "home"));
        assert!(routes.iter().any(|r| r.path == "diet"));
        assert!(routes.iter().any(|r| r.path == "training"));
    }

    #[test]
    fn test_const_to_display_name() {
        assert_eq!(const_to_display_name("DIET_TRACKER"), "Diet Tracker");
        assert_eq!(const_to_display_name("DietTracker"), "Diet Tracker");
        assert_eq!(const_to_display_name("HOME"), "Home");
    }
}
