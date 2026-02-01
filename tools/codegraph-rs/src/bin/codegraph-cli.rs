//! CodeGraph CLI - Universal code graph tool for AI codebase understanding
//!
//! Part of AI-Foundation: "Empowering AIs Everywhere, Always"
//!
//! Commands:
//!   parse <path>              Parse a codebase into a graph
//!   compare <src> <target>    Compare two codebases
//!   routes <path>             List all routes in a codebase
//!   lookup <path> <route>     Find which file handles a route

use clap::{Parser as ClapParser, Subcommand};
use codegraph::{CodeGraph, parsers, compare, Parser as GraphParser};
use std::path::PathBuf;
use anyhow::Result;

#[derive(ClapParser)]
#[command(name = "codegraph")]
#[command(author = "AI-Foundation")]
#[command(version = "0.1.0")]
#[command(about = "Universal code graph tool for AI codebase understanding")]
#[command(long_about = "Parse navigation routes from any framework, build relationship graphs, and compare across platforms.\n\nPart of AI-Foundation: Empowering AIs Everywhere, Always")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output format: text, json, or markdown
    #[arg(short, long, default_value = "text")]
    format: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse a codebase into a graph
    #[command(aliases = ["p", "scan"])]
    Parse {
        /// Path to the codebase root
        path: PathBuf,

        /// Name for this graph (e.g., "MyApp Mobile")
        #[arg(short, long)]
        name: Option<String>,

        /// Force specific parser (sveltekit, compose, kotlin-routes)
        #[arg(short = 'p', long)]
        parser: Option<String>,
    },

    /// Compare two codebases for route alignment
    #[command(aliases = ["c", "diff", "gap"])]
    Compare {
        /// Source codebase (reference)
        source: PathBuf,

        /// Target codebase (to check coverage)
        target: PathBuf,

        /// Name for source graph
        #[arg(long, default_value = "Source")]
        source_name: String,

        /// Name for target graph
        #[arg(long, default_value = "Target")]
        target_name: String,
    },

    /// List all routes in a codebase
    #[command(aliases = ["r", "list", "ls"])]
    Routes {
        /// Path to the codebase root
        path: PathBuf,

        /// Filter by category prefix
        #[arg(short, long)]
        category: Option<String>,

        /// Show only route paths (no details)
        #[arg(short, long)]
        quiet: bool,
    },

    /// Look up which file handles a specific route
    #[command(aliases = ["l", "find", "which"])]
    Lookup {
        /// Path to the codebase root
        path: PathBuf,

        /// Route to look up (e.g., "diet_tracker" or "/diet/tracker")
        route: String,
    },

    /// Show supported parsers
    #[command(aliases = ["parsers", "frameworks"])]
    List,

    /// Show graph statistics
    #[command(aliases = ["s", "info"])]
    Stats {
        /// Path to the codebase root
        path: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let registry = parsers::default_registry();

    match cli.command {
        Commands::Parse { path, name, parser } => {
            let graph_name = name.unwrap_or_else(|| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Unknown")
                    .to_string()
            });

            let graph = if let Some(parser_name) = parser {
                // Use specific parser
                match parser_name.as_str() {
                    "sveltekit" => GraphParser::parse(&parsers::SvelteKitParser::new(), &path, &graph_name)?,
                    "compose" => GraphParser::parse(&parsers::ComposeParser::new(), &path, &graph_name)?,
                    "kotlin-routes" | "kotlin" => GraphParser::parse(&parsers::KotlinRoutesParser::new(), &path, &graph_name)?,
                    _ => return Err(anyhow::anyhow!("Unknown parser: {}", parser_name)),
                }
            } else {
                // Auto-detect
                registry.parse(&path, &graph_name)?
            };

            output_graph(&graph, &cli.format)?;
        }

        Commands::Compare { source, target, source_name, target_name } => {
            let source_graph = registry.parse(&source, &source_name)?;
            let target_graph = registry.parse(&target, &target_name)?;

            // Create default mapping for MyApp-style routes
            let mapping = compare::RouteMapping::new()
                .transform_prefix("diet_", "/diet/")
                .transform_prefix("training_", "/training/")
                .transform_prefix("leagues_", "/leagues/")
                .alias("home", &["/", "index"]);

            let result = compare::compare(&source_graph, &target_graph, Some(&mapping));

            match cli.format.as_str() {
                "json" => println!("{}", serde_json::to_string_pretty(&result)?),
                "markdown" | "md" => println!("{}", result.to_report()),
                _ => print_comparison_text(&result),
            }
        }

        Commands::Routes { path, category, quiet } => {
            let name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Unknown");
            let graph = registry.parse(&path, name)?;

            let routes: Vec<_> = graph.routes()
                .filter(|r| {
                    if let Some(ref cat) = category {
                        r.id.contains(cat) || r.name.to_lowercase().contains(&cat.to_lowercase())
                    } else {
                        true
                    }
                })
                .collect();

            if quiet {
                for route in routes {
                    println!("{}", route.id);
                }
            } else {
                println!("|ROUTES|{}", routes.len());
                println!();
                for route in routes {
                    let file = route.file_path.as_deref().unwrap_or("-");
                    println!("  {} → {}", route.id, route.name);
                    println!("    File:{}", file);
                }
            }
        }

        Commands::Lookup { path, route } => {
            let name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Unknown");
            let graph = registry.parse(&path, name)?;

            // Normalize the search route
            let normalized = route.trim_matches('/').replace('_', "-").to_lowercase();

            let found: Vec<_> = graph.routes()
                .filter(|r| {
                    let r_norm = r.id.trim_matches('/').replace('_', "-").to_lowercase();
                    r.id == route || r_norm == normalized || r.name.to_lowercase().contains(&normalized)
                })
                .collect();

            if found.is_empty() {
                println!("Route not found: {}", route);
                println!();
                println!("Hint: Try 'codegraph routes {}' to see all routes", path.display());
            } else {
                for r in found {
                    println!("|FOUND|");
                    println!("  Route:{}", r.id);
                    println!("  Name:{}", r.name);
                    if let Some(ref fp) = r.file_path {
                        println!("  File:{}", fp);
                    }
                    if let Some(ref rp) = r.route_path {
                        println!("  Path:{}", rp);
                    }
                }
            }
        }

        Commands::List => {
            println!("|PARSERS|");
            for framework in registry.frameworks() {
                println!("  {}", framework);
            }
        }

        Commands::Stats { path } => {
            let name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Unknown");
            let graph = registry.parse(&path, name)?;

            println!("|STATS|");
            println!("  Name:{}", graph.name);
            println!("  Framework:{}", graph.framework);
            println!("  Nodes:{}", graph.stats.total_nodes);
            println!("  Edges:{}", graph.stats.total_edges);
            println!("  Routes:{}", graph.stats.routes);
            println!("  Components:{}", graph.stats.components);
            println!("  Stores:{}", graph.stats.stores);
            println!("  Endpoints:{}", graph.stats.endpoints);
        }
    }

    Ok(())
}

fn output_graph(graph: &CodeGraph, format: &str) -> Result<()> {
    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(graph)?);
        }
        _ => {
            println!("|GRAPH|{}", graph.name);
            println!("  Framework:{}", graph.framework);
            println!("  Root:{}", graph.root_path);
            println!();
            println!("|NODES|{}", graph.nodes.len());
            for node in &graph.nodes {
                let kind = format!("{:?}", node.kind).to_lowercase();
                println!("  [{}] {} → {}", kind, node.id, node.name);
            }
            if !graph.edges.is_empty() {
                println!();
                println!("|EDGES|{}", graph.edges.len());
                for edge in &graph.edges {
                    let kind = format!("{:?}", edge.kind);
                    println!("  {} --{}-> {}", edge.from, kind, edge.to);
                }
            }
        }
    }
    Ok(())
}

fn print_comparison_text(result: &compare::ComparisonResult) {
    println!("|COMPARISON|{} → {}", result.source_name, result.target_name);
    println!();
    println!("|COVERAGE|{:.1}%", result.stats.coverage_percent);
    println!("  Source:{}", result.stats.source_total);
    println!("  Target:{}", result.stats.target_total);
    println!("  Matched:{}", result.stats.matched_count);
    println!();

    if !result.matched.is_empty() {
        println!("|MATCHED|{}", result.matched.len());
        for m in &result.matched {
            let symbol = match m.match_type {
                compare::MatchType::Exact => "=",
                compare::MatchType::Normalized => "≈",
                compare::MatchType::Fuzzy => "~",
                compare::MatchType::Manual => "→",
            };
            println!("  ✓ {} {} {}", m.source_path, symbol, m.target_path);
        }
        println!();
    }

    if !result.source_only.is_empty() {
        println!("|MISSING|{}", result.source_only.len());
        for route in &result.source_only {
            println!("  ✗ {}", route);
        }
        println!();
    }

    if !result.target_only.is_empty() {
        println!("|EXTRA|{}", result.target_only.len());
        for route in &result.target_only {
            println!("  + {}", route);
        }
    }
}
