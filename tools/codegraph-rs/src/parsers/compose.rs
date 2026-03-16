//! Jetpack Compose navigation parser
//!
//! Extracts routes from Compose Navigation patterns:
//! - NavHost { composable("route") { Screen() } }
//! - composable(Screen.Route.route) { ... }
//! - navigation(route = "parent") { composable("child") { } }

use crate::graph::{CodeGraph, Edge, EdgeKind};
use crate::parser::{Parser, RouteDefinition};
use std::path::Path;
use std::fs;
use walkdir::WalkDir;
use regex::Regex;
use anyhow::Result;

pub struct ComposeParser {
    /// Regex for composable("route") pattern
    composable_regex: Regex,
    /// Regex for composable(SomeScreen.route) pattern
    composable_ref_regex: Regex,
    /// Regex for navigation(route = "parent") pattern
    navigation_regex: Regex,
    /// Regex for navController.navigate("route") pattern
    navigate_regex: Regex,
    /// Regex for popBackStack()
    #[allow(dead_code)]
    pop_back_regex: Regex,
}

impl ComposeParser {
    pub fn new() -> Self {
        Self {
            composable_regex: Regex::new(
                r#"composable\s*\(\s*['"]([^'"]+)['"]"#
            ).unwrap(),
            composable_ref_regex: Regex::new(
                r#"composable\s*\(\s*(\w+(?:\.\w+)*\.route)"#
            ).unwrap(),
            navigation_regex: Regex::new(
                r#"navigation\s*\([^)]*route\s*=\s*['"]([^'"]+)['"]"#
            ).unwrap(),
            navigate_regex: Regex::new(
                r#"navigate\s*\(\s*['"]([^'"]+)['"]"#
            ).unwrap(),
            pop_back_regex: Regex::new(
                r#"popBackStack\s*\(\s*\)"#
            ).unwrap(),
        }
    }

    /// Extract composable routes from a Kotlin file
    fn extract_composables(&self, content: &str, file_path: &str) -> Vec<RouteDefinition> {
        let mut routes = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        for (line_num, line) in lines.iter().enumerate() {
            // Direct string routes: composable("diet_hub")
            for cap in self.composable_regex.captures_iter(line) {
                let route = &cap[1];
                let display_name = route_to_display_name(route);
                let route_def = RouteDefinition::new(route, &display_name, file_path)
                    .with_line(line_num + 1);
                routes.push(route_def);
            }

            // Reference routes: composable(Screen.Diet.route)
            for cap in self.composable_ref_regex.captures_iter(line) {
                let reference = &cap[1];
                // Extract the screen name from reference
                let parts: Vec<&str> = reference.split('.').collect();
                if parts.len() >= 2 {
                    let screen_name = parts[parts.len() - 2];
                    let display_name = route_to_display_name(screen_name);
                    let mut route_def = RouteDefinition::new(reference, &display_name, file_path)
                        .with_line(line_num + 1);
                    route_def.metadata.insert("is_reference".to_string(), "true".to_string());
                    routes.push(route_def);
                }
            }

            // Navigation groups: navigation(route = "parent")
            for cap in self.navigation_regex.captures_iter(line) {
                let route = &cap[1];
                let display_name = format!("{} (Group)", route_to_display_name(route));
                let mut route_def = RouteDefinition::new(route, &display_name, file_path)
                    .with_line(line_num + 1);
                route_def.metadata.insert("is_group".to_string(), "true".to_string());
                routes.push(route_def);
            }
        }

        routes
    }

    /// Extract navigation calls from a Kotlin file
    fn extract_navigation(&self, content: &str, file_path: &str) -> Vec<(String, String)> {
        let mut navigations = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        // Try to determine current route context from file name or composable
        let current_route = Path::new(file_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.replace("Screen", "").to_lowercase())
            .unwrap_or_default();

        for line in lines.iter() {
            for cap in self.navigate_regex.captures_iter(line) {
                let target = &cap[1];
                navigations.push((current_route.clone(), target.to_string()));
            }
        }

        navigations
    }
}

impl Parser for ComposeParser {
    fn framework(&self) -> &str {
        "compose"
    }

    fn extensions(&self) -> &[&str] {
        &[".kt"]
    }

    fn can_parse(&self, root: &Path) -> bool {
        // Check for Compose/Android project markers
        let has_gradle = root.join("build.gradle").exists()
            || root.join("build.gradle.kts").exists()
            || root.join("app/build.gradle.kts").exists();

        // Look for compose in build files
        if has_gradle {
            if let Ok(content) = fs::read_to_string(root.join("app/build.gradle.kts")) {
                return content.contains("compose") || content.contains("navigation");
            }
            if let Ok(content) = fs::read_to_string(root.join("build.gradle.kts")) {
                return content.contains("compose");
            }
        }

        has_gradle
    }

    fn parse(&self, root: &Path, name: &str) -> Result<CodeGraph> {
        let mut graph = CodeGraph::new(name, "compose", &root.to_string_lossy());

        // Search patterns for navigation files
        let _nav_patterns = [
            "NavHost",
            "NavGraph",
            "Navigation",
            "MainActivity",
        ];

        // First pass: collect all composable routes
        for entry in WalkDir::new(root)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "kt") {
                if let Ok(content) = fs::read_to_string(path) {
                    // Check if file contains NavHost or composable
                    if content.contains("NavHost") || content.contains("composable(") {
                        let file_path_str = path.to_string_lossy().to_string();
                        let routes = self.extract_composables(&content, &file_path_str);

                        for route in routes {
                            graph.add_node(route.to_node("compose"));
                        }
                    }
                }
            }
        }

        // Second pass: extract navigation relationships
        for entry in WalkDir::new(root)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "kt") {
                if let Ok(content) = fs::read_to_string(path) {
                    if content.contains("navigate(") {
                        let file_path_str = path.to_string_lossy().to_string();
                        let navigations = self.extract_navigation(&content, &file_path_str);

                        for (from, to) in navigations {
                            // Only add if target exists
                            if graph.find_node(&to).is_some() {
                                let edge = Edge::new(&from, &to, EdgeKind::NavigatesTo);
                                graph.add_edge(edge);
                            }
                        }
                    }
                }
            }
        }

        Ok(graph)
    }
}

impl Default for ComposeParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert route string to display name
fn route_to_display_name(route: &str) -> String {
    // Handle snake_case
    if route.contains('_') {
        return route.split('_')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
    }

    // Handle kebab-case
    if route.contains('-') {
        return route.split('-')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
    }

    // Handle PascalCase
    let mut result = String::new();
    for (i, c) in route.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            result.push(' ');
        }
        if i == 0 {
            result.push(c.to_ascii_uppercase());
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_composable_extraction() {
        let parser = ComposeParser::new();
        let content = r#"
NavHost(navController = navController, startDestination = "home") {
    composable("home") { HomeScreen() }
    composable("diet_hub") { DietHubScreen() }
    composable(Screen.Training.route) { TrainingScreen() }
    navigation(route = "settings_group") {
        composable("settings") { SettingsScreen() }
    }
}
        "#;

        let routes = parser.extract_composables(content, "MainActivity.kt");
        assert!(routes.len() >= 3);
        assert!(routes.iter().any(|r| r.path == "home"));
        assert!(routes.iter().any(|r| r.path == "diet_hub"));
    }

    #[test]
    fn test_route_to_display_name() {
        assert_eq!(route_to_display_name("diet_hub"), "Diet Hub");
        assert_eq!(route_to_display_name("theme-shop"), "Theme Shop");
        assert_eq!(route_to_display_name("DietHub"), "Diet Hub");
    }
}
