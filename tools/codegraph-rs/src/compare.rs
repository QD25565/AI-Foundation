//! Graph comparison module for cross-platform analysis
//!
//! Compares two CodeGraphs to find:
//! - Matching routes (aligned features)
//! - Missing routes (gaps in one platform)
//! - Coverage percentage
//!
//! Key use case: Compare mobile Kotlin routes vs PWA SvelteKit routes

use crate::graph::{CodeGraph, Node, NodeKind};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Result of comparing two code graphs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonResult {
    /// Source graph name (e.g., "MyApp Mobile")
    pub source_name: String,
    /// Target graph name (e.g., "MyApp PWA")
    pub target_name: String,
    /// Routes that exist in both (matched)
    pub matched: Vec<RouteMatch>,
    /// Routes only in source (missing from target)
    pub source_only: Vec<String>,
    /// Routes only in target (extra in target)
    pub target_only: Vec<String>,
    /// Coverage statistics
    pub stats: CoverageStats,
}

/// A matched route pair between two platforms
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteMatch {
    /// Route identifier used for matching
    pub route_id: String,
    /// Source platform's route path
    pub source_path: String,
    /// Target platform's route path
    pub target_path: String,
    /// Match confidence (0.0 - 1.0)
    pub confidence: f64,
    /// How the match was determined
    pub match_type: MatchType,
}

/// How a match was determined
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MatchType {
    /// Exact route path match
    Exact,
    /// Normalized path match (snake_case == kebab-case)
    Normalized,
    /// Name-based fuzzy match
    Fuzzy,
    /// Manual mapping provided
    Manual,
}

/// Coverage statistics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CoverageStats {
    /// Total routes in source
    pub source_total: usize,
    /// Total routes in target
    pub target_total: usize,
    /// Number of matched routes
    pub matched_count: usize,
    /// Coverage percentage (matched / source_total)
    pub coverage_percent: f64,
    /// Routes by category
    pub by_category: HashMap<String, CategoryStats>,
}

/// Stats for a category of routes
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CategoryStats {
    pub source_count: usize,
    pub target_count: usize,
    pub matched_count: usize,
    pub coverage_percent: f64,
}

/// Route mapping configuration for cross-platform comparison
#[derive(Debug, Clone, Default)]
pub struct RouteMapping {
    /// Manual mappings: source_route -> target_route
    pub manual_mappings: HashMap<String, String>,
    /// Prefix transformations: (source_prefix, target_prefix)
    pub prefix_transforms: Vec<(String, String)>,
    /// Route aliases: different names for same feature
    pub aliases: HashMap<String, Vec<String>>,
}

impl RouteMapping {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a manual mapping
    pub fn map(mut self, source: &str, target: &str) -> Self {
        self.manual_mappings.insert(source.to_string(), target.to_string());
        self
    }

    /// Add prefix transformation (e.g., "diet_" -> "/diet/")
    pub fn transform_prefix(mut self, from: &str, to: &str) -> Self {
        self.prefix_transforms.push((from.to_string(), to.to_string()));
        self
    }

    /// Add aliases (e.g., "home" = ["index", "/", "main"])
    pub fn alias(mut self, primary: &str, alternatives: &[&str]) -> Self {
        self.aliases.insert(
            primary.to_string(),
            alternatives.iter().map(|s| s.to_string()).collect(),
        );
        self
    }
}

/// Compare two code graphs
pub fn compare(
    source: &CodeGraph,
    target: &CodeGraph,
    mapping: Option<&RouteMapping>,
) -> ComparisonResult {
    let empty_mapping = RouteMapping::default();
    let mapping = mapping.unwrap_or(&empty_mapping);

    // Get route nodes from both graphs
    let source_routes: Vec<&Node> = source.nodes.iter()
        .filter(|n| n.kind == NodeKind::Route)
        .collect();

    let target_routes: Vec<&Node> = target.nodes.iter()
        .filter(|n| n.kind == NodeKind::Route)
        .collect();

    // Build normalized lookup for target routes
    let mut target_lookup: HashMap<String, &Node> = HashMap::new();
    for node in &target_routes {
        // Add by original path
        target_lookup.insert(node.id.clone(), *node);
        // Add by normalized path
        target_lookup.insert(normalize_route(&node.id), *node);
        // Add by route_path if different
        if let Some(ref rp) = node.route_path {
            target_lookup.insert(rp.clone(), *node);
            target_lookup.insert(normalize_route(rp), *node);
        }
    }

    // Also add aliases
    for (primary, alternatives) in &mapping.aliases {
        if let Some(node) = target_lookup.get(primary).copied() {
            for alt in alternatives {
                target_lookup.insert(alt.clone(), node);
            }
        }
    }

    let mut matched = Vec::new();
    let mut source_only = Vec::new();
    let mut matched_target_ids: HashSet<String> = HashSet::new();

    // Try to match each source route
    for source_node in &source_routes {
        let source_id = &source_node.id;

        // 1. Check manual mapping first
        if let Some(target_id) = mapping.manual_mappings.get(source_id) {
            if let Some(target_node) = target_lookup.get(target_id) {
                matched.push(RouteMatch {
                    route_id: source_id.clone(),
                    source_path: source_id.clone(),
                    target_path: target_node.id.clone(),
                    confidence: 1.0,
                    match_type: MatchType::Manual,
                });
                matched_target_ids.insert(target_node.id.clone());
                continue;
            }
        }

        // 2. Try exact match
        if let Some(target_node) = target_lookup.get(source_id) {
            matched.push(RouteMatch {
                route_id: source_id.clone(),
                source_path: source_id.clone(),
                target_path: target_node.id.clone(),
                confidence: 1.0,
                match_type: MatchType::Exact,
            });
            matched_target_ids.insert(target_node.id.clone());
            continue;
        }

        // 3. Try normalized match
        let normalized = normalize_route(source_id);
        if let Some(target_node) = target_lookup.get(&normalized) {
            matched.push(RouteMatch {
                route_id: source_id.clone(),
                source_path: source_id.clone(),
                target_path: target_node.id.clone(),
                confidence: 0.9,
                match_type: MatchType::Normalized,
            });
            matched_target_ids.insert(target_node.id.clone());
            continue;
        }

        // 4. Try prefix transforms
        let mut found = false;
        for (from_prefix, to_prefix) in &mapping.prefix_transforms {
            if source_id.starts_with(from_prefix) {
                let transformed = source_id.replace(from_prefix, to_prefix);
                if let Some(target_node) = target_lookup.get(&transformed) {
                    matched.push(RouteMatch {
                        route_id: source_id.clone(),
                        source_path: source_id.clone(),
                        target_path: target_node.id.clone(),
                        confidence: 0.85,
                        match_type: MatchType::Normalized,
                    });
                    matched_target_ids.insert(target_node.id.clone());
                    found = true;
                    break;
                }
            }
        }
        if found {
            continue;
        }

        // 5. Try fuzzy name match
        if let Some((target_node, confidence)) = fuzzy_match_route(source_node, &target_routes) {
            if !matched_target_ids.contains(&target_node.id) {
                matched.push(RouteMatch {
                    route_id: source_id.clone(),
                    source_path: source_id.clone(),
                    target_path: target_node.id.clone(),
                    confidence,
                    match_type: MatchType::Fuzzy,
                });
                matched_target_ids.insert(target_node.id.clone());
                continue;
            }
        }

        // No match found
        source_only.push(source_id.clone());
    }

    // Find routes only in target
    let target_only: Vec<String> = target_routes.iter()
        .filter(|n| !matched_target_ids.contains(&n.id))
        .map(|n| n.id.clone())
        .collect();

    // Calculate stats
    let coverage_percent = if source_routes.is_empty() {
        100.0
    } else {
        (matched.len() as f64 / source_routes.len() as f64) * 100.0
    };

    let stats = CoverageStats {
        source_total: source_routes.len(),
        target_total: target_routes.len(),
        matched_count: matched.len(),
        coverage_percent,
        by_category: calculate_category_stats(&source_routes, &target_routes, &matched),
    };

    ComparisonResult {
        source_name: source.name.clone(),
        target_name: target.name.clone(),
        matched,
        source_only,
        target_only,
        stats,
    }
}

/// Normalize a route for comparison
/// - snake_case to kebab-case
/// - remove leading/trailing slashes
/// - lowercase
fn normalize_route(route: &str) -> String {
    route
        .trim_matches('/')
        .replace('_', "-")
        .to_lowercase()
}

/// Fuzzy match based on route name similarity
fn fuzzy_match_route<'a>(source: &Node, targets: &[&'a Node]) -> Option<(&'a Node, f64)> {
    let source_name = source.name.to_lowercase();

    let mut best_match: Option<(&Node, f64)> = None;

    for target in targets {
        let target_name = target.name.to_lowercase();

        // Calculate similarity
        let similarity = string_similarity(&source_name, &target_name);

        if similarity > 0.7 {
            if best_match.map_or(true, |(_, s)| similarity > s) {
                best_match = Some((*target, similarity));
            }
        }
    }

    best_match
}

/// Simple string similarity (Jaccard on words)
fn string_similarity(a: &str, b: &str) -> f64 {
    let a_words: HashSet<&str> = a.split_whitespace().collect();
    let b_words: HashSet<&str> = b.split_whitespace().collect();

    if a_words.is_empty() && b_words.is_empty() {
        return 1.0;
    }

    let intersection = a_words.intersection(&b_words).count();
    let union = a_words.union(&b_words).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Calculate per-category statistics
fn calculate_category_stats(
    source: &[&Node],
    target: &[&Node],
    matched: &[RouteMatch],
) -> HashMap<String, CategoryStats> {
    let mut stats: HashMap<String, CategoryStats> = HashMap::new();

    // Extract category from route (first segment)
    let get_category = |id: &str| -> String {
        let normalized = id.trim_matches('/').replace('_', "/");
        normalized.split('/').next().unwrap_or("root").to_string()
    };

    // Count source routes by category
    for node in source {
        let cat = get_category(&node.id);
        stats.entry(cat).or_default().source_count += 1;
    }

    // Count target routes by category
    for node in target {
        let cat = get_category(&node.id);
        stats.entry(cat).or_default().target_count += 1;
    }

    // Count matched by category
    for m in matched {
        let cat = get_category(&m.source_path);
        stats.entry(cat).or_default().matched_count += 1;
    }

    // Calculate coverage per category
    for (_, stat) in stats.iter_mut() {
        stat.coverage_percent = if stat.source_count == 0 {
            100.0
        } else {
            (stat.matched_count as f64 / stat.source_count as f64) * 100.0
        };
    }

    stats
}

impl ComparisonResult {
    /// Format as a human-readable report
    pub fn to_report(&self) -> String {
        let mut report = String::new();

        report.push_str(&format!("# Route Comparison: {} → {}\n\n", self.source_name, self.target_name));

        // Summary
        report.push_str("## Summary\n\n");
        report.push_str(&format!("- Source routes: {}\n", self.stats.source_total));
        report.push_str(&format!("- Target routes: {}\n", self.stats.target_total));
        report.push_str(&format!("- Matched: {}\n", self.stats.matched_count));
        report.push_str(&format!("- Coverage: {:.1}%\n\n", self.stats.coverage_percent));

        // Matched routes
        if !self.matched.is_empty() {
            report.push_str("## Matched Routes ✓\n\n");
            for m in &self.matched {
                let match_icon = match m.match_type {
                    MatchType::Exact => "=",
                    MatchType::Normalized => "≈",
                    MatchType::Fuzzy => "~",
                    MatchType::Manual => "→",
                };
                report.push_str(&format!("- {} {} {} ({})\n",
                    m.source_path, match_icon, m.target_path,
                    format!("{:.0}%", m.confidence * 100.0)
                ));
            }
            report.push('\n');
        }

        // Missing from target
        if !self.source_only.is_empty() {
            report.push_str("## Missing from Target ✗\n\n");
            for route in &self.source_only {
                report.push_str(&format!("- {}\n", route));
            }
            report.push('\n');
        }

        // Extra in target
        if !self.target_only.is_empty() {
            report.push_str("## Extra in Target (+)\n\n");
            for route in &self.target_only {
                report.push_str(&format!("- {}\n", route));
            }
            report.push('\n');
        }

        // Category breakdown
        if !self.stats.by_category.is_empty() {
            report.push_str("## By Category\n\n");
            let mut categories: Vec<_> = self.stats.by_category.iter().collect();
            categories.sort_by(|a, b| a.0.cmp(b.0));

            for (cat, stat) in categories {
                let status = if stat.coverage_percent >= 100.0 {
                    "✓"
                } else if stat.coverage_percent >= 50.0 {
                    "◐"
                } else {
                    "✗"
                };
                report.push_str(&format!(
                    "- {} {}: {}/{} ({:.0}%)\n",
                    status, cat, stat.matched_count, stat.source_count, stat.coverage_percent
                ));
            }
        }

        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Node;

    fn make_route_graph(name: &str, routes: &[&str]) -> CodeGraph {
        let mut graph = CodeGraph::new(name, "test", "/test");
        for route in routes {
            graph.add_node(Node::route(route, route, route, "test.kt", "test"));
        }
        graph
    }

    #[test]
    fn test_compare_exact_match() {
        let source = make_route_graph("Mobile", &["home", "diet", "training"]);
        let target = make_route_graph("PWA", &["home", "diet", "training"]);

        let result = compare(&source, &target, None);

        assert_eq!(result.matched.len(), 3);
        assert!(result.source_only.is_empty());
        assert!(result.target_only.is_empty());
        assert_eq!(result.stats.coverage_percent, 100.0);
    }

    #[test]
    fn test_compare_with_gaps() {
        let source = make_route_graph("Mobile", &["home", "diet", "training", "leagues"]);
        let target = make_route_graph("PWA", &["home", "diet"]);

        let result = compare(&source, &target, None);

        assert_eq!(result.matched.len(), 2);
        assert_eq!(result.source_only.len(), 2);
        assert_eq!(result.stats.coverage_percent, 50.0);
    }

    #[test]
    fn test_normalize_route() {
        assert_eq!(normalize_route("diet_tracker"), "diet-tracker");
        assert_eq!(normalize_route("/Diet/Tracker/"), "diet-tracker");
        assert_eq!(normalize_route("DIET_HUB"), "diet-hub");
    }
}
