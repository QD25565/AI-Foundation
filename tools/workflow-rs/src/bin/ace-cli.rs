///! ACE Playbooks CLI - Generate, Reflect, Curate
///!
///! Manual interface for Stanford ACE framework

use anyhow::Result;
use clap::{Parser, Subcommand};
use workflow_rs::{PersonalPlaybook, Strategy, Insight, Pattern};

#[derive(Parser)]
#[command(name = "ace-cli")]
#[command(about = "ACE Playbooks - Self-improving AI context management", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new strategy to your playbook
    #[command(alias = "strategy", alias = "add-strat", alias = "new-strategy")]
    AddStrategy {
        /// Strategy title (e.g., "Use hybrid retrieval for better recall")
        title: String,
        /// Context where this applies (e.g., "When searching memory")
        context: String,
        /// The approach to use (e.g., "Combine vector + keyword + graph")
        approach: String,
        /// Comma-separated tags (e.g., "search,memory,performance")
        #[arg(long)]
        tags: Option<String>,
    },

    /// Add an insight or learning to your playbook
    #[command(alias = "insight", alias = "learn", alias = "note-insight")]
    AddInsight {
        /// The discovery or insight (e.g., "Recency bias improves relevance")
        discovery: String,
        /// Confidence 0.0-1.0
        #[arg(default_value = "0.7")]
        confidence: f64,
        /// Comma-separated tags (e.g., "memory,learning")
        #[arg(long)]
        tags: Option<String>,
    },

    /// Add a pattern observation to your playbook
    #[command(alias = "pattern", alias = "observe", alias = "note-pattern")]
    AddPattern {
        /// Situation description (e.g., "When user reports slow queries")
        situation: String,
        /// Pattern observed (e.g., "Missing database indexes")
        pattern: String,
        /// Strength 0.0-1.0
        #[arg(default_value = "0.5")]
        strength: f64,
        /// Comma-separated tags (e.g., "performance,database")
        #[arg(long)]
        tags: Option<String>,
    },

    /// Record outcome of using a strategy
    #[command(alias = "outcome", alias = "result", alias = "record")]
    RecordOutcome {
        /// Strategy ID (e.g., "strat-a88d9521")
        strategy_id: String,
        /// Was it successful? (true/false, yes/no, 1/0)
        success: String,
    },

    /// List all strategies in your playbook
    #[command(alias = "strategies", alias = "strats", alias = "list-strats")]
    ListStrategies {
        /// Filter by comma-separated tags (e.g., "search,performance")
        #[arg(long)]
        tags: Option<String>,
        /// Sort by: effectiveness, recent, usage
        #[arg(long, default_value = "effectiveness")]
        sort: String,
    },

    /// List all insights in your playbook
    #[command(alias = "insights", alias = "learnings", alias = "list-insights")]
    ListInsights {
        /// Filter by comma-separated tags (e.g., "memory,learning")
        #[arg(long)]
        tags: Option<String>,
    },

    /// List all patterns in your playbook
    #[command(alias = "patterns", alias = "observations", alias = "list-patterns")]
    ListPatterns {
        /// Filter by comma-separated tags (e.g., "performance,bugs")
        #[arg(long)]
        tags: Option<String>,
    },

    /// Show playbook statistics and summary
    #[command(alias = "statistics", alias = "info", alias = "summary")]
    Stats,

    /// Curate playbook by removing ineffective entries
    #[command(alias = "clean", alias = "prune", alias = "cleanup")]
    Curate {
        /// Minimum effectiveness threshold (default: 0.3)
        #[arg(long, default_value = "0.3")]
        min_effectiveness: f64,
        /// Maximum age in days (default: 90)
        #[arg(long, default_value = "90")]
        max_age_days: i64,
    },

    /// Get relevant context for current task
    #[command(alias = "context", alias = "get", alias = "fetch-context")]
    GetContext {
        /// Comma-separated tags to match (e.g., "search,performance")
        #[arg(long)]
        tags: Option<String>,
        /// Maximum number of strategies to return
        #[arg(long, default_value = "5")]
        limit: usize,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let ai_id = std::env::var("AI_ID")
        .unwrap_or_else(|_| "unknown".to_string());

    let pb = PersonalPlaybook::new(&ai_id)?;

    match cli.command {
        Commands::AddStrategy { title, context, approach, tags } => {
            let tags_vec = tags.map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
            let id = pb.add_strategy(&title, &context, &approach, None, tags_vec)?;
            println!("✅ Strategy added: {}", id);
            println!("   Title: {}", title);
            println!("   Context: {}", context);
            println!("   Approach: {}", approach);
        }

        Commands::AddInsight { discovery, confidence, tags } => {
            let tags_vec = tags.map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
            let id = pb.add_insight(&discovery, None, confidence, tags_vec)?;
            println!("✅ Insight added: {}", id);
            println!("   Discovery: {}", discovery);
            println!("   Confidence: {:.0}%", confidence * 100.0);
        }

        Commands::AddPattern { situation, pattern, strength, tags } => {
            let tags_vec = tags.map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
            let id = pb.add_pattern(&situation, &pattern, None, strength, tags_vec)?;
            println!("✅ Pattern added: {}", id);
            println!("   Situation: {}", situation);
            println!("   Pattern: {}", pattern);
            println!("   Strength: {:.0}%", strength * 100.0);
        }

        Commands::RecordOutcome { strategy_id, success } => {
            let success_bool = match success.to_lowercase().as_str() {
                "true" | "yes" | "1" | "success" => true,
                "false" | "no" | "0" | "failure" => false,
                _ => {
                    eprintln!("❌ Invalid success value. Use: true/false, yes/no, 1/0");
                    std::process::exit(1);
                }
            };

            pb.record_strategy_outcome(&strategy_id, success_bool)?;

            if success_bool {
                println!("✅ Recorded SUCCESS for strategy: {}", strategy_id);
            } else {
                println!("📉 Recorded FAILURE for strategy: {}", strategy_id);
            }
        }

        Commands::ListStrategies { tags, sort } => {
            let tags_vec = tags.map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
            let mut strategies = pb.get_strategies(tags_vec)?;

            // Sort strategies
            match sort.as_str() {
                "effectiveness" => strategies.sort_by(|a, b| b.effectiveness.partial_cmp(&a.effectiveness).unwrap()),
                "recent" => strategies.sort_by(|a, b| b.last_used.cmp(&a.last_used)),
                "usage" => strategies.sort_by(|a, b| b.use_count.cmp(&a.use_count)),
                _ => {}
            }

            if strategies.is_empty() {
                println!("No strategies found");
                return Ok(());
            }

            println!("╔══════════════════════════════════════════════════════════════╗");
            println!("║  🎯 STRATEGIES ({})                                          ", strategies.len());
            println!("╠══════════════════════════════════════════════════════════════╣");

            for s in strategies {
                println!("║");
                println!("║  [{}]", s.id);
                println!("║  Title: {}", s.title);
                println!("║  Effectiveness: {:.0}% | Used: {} times | Success Rate: {:.0}%",
                    s.effectiveness * 100.0, s.use_count, s.success_rate * 100.0);
                println!("║  Context: {}", s.context);
                println!("║  Approach: {}", s.approach);
                if !s.tags.is_empty() {
                    println!("║  Tags: {}", s.tags.join(", "));
                }
            }

            println!("╚══════════════════════════════════════════════════════════════╝");
        }

        Commands::ListInsights { tags } => {
            let tags_vec = tags.map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
            let insights = pb.get_insights(tags_vec)?;

            if insights.is_empty() {
                println!("No insights found");
                return Ok(());
            }

            println!("╔══════════════════════════════════════════════════════════════╗");
            println!("║  💡 INSIGHTS ({})                                            ", insights.len());
            println!("╠══════════════════════════════════════════════════════════════╣");

            for i in insights {
                println!("║");
                println!("║  [{}]", i.id);
                println!("║  Discovery: {}", i.discovery);
                println!("║  Confidence: {:.0}%", i.confidence * 100.0);
                if !i.tags.is_empty() {
                    println!("║  Tags: {}", i.tags.join(", "));
                }
            }

            println!("╚══════════════════════════════════════════════════════════════╝");
        }

        Commands::ListPatterns { tags } => {
            let tags_vec = tags.map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
            let patterns = pb.get_patterns(tags_vec)?;

            if patterns.is_empty() {
                println!("No patterns found");
                return Ok(());
            }

            println!("╔══════════════════════════════════════════════════════════════╗");
            println!("║  📊 PATTERNS ({})                                            ", patterns.len());
            println!("╠══════════════════════════════════════════════════════════════╣");

            for p in patterns {
                println!("║");
                println!("║  [{}]", p.id);
                println!("║  Situation: {}", p.situation);
                println!("║  Pattern: {}", p.pattern);
                println!("║  Strength: {:.0}%", p.strength * 100.0);
                if !p.tags.is_empty() {
                    println!("║  Tags: {}", p.tags.join(", "));
                }
            }

            println!("╚══════════════════════════════════════════════════════════════╝");
        }

        Commands::Stats => {
            let strategies = pb.get_strategies(None)?;
            let insights = pb.get_insights(None)?;
            let patterns = pb.get_patterns(None)?;

            let total = strategies.len() + insights.len() + patterns.len();
            let avg_effectiveness = if !strategies.is_empty() {
                strategies.iter().map(|s| s.effectiveness).sum::<f64>() / strategies.len() as f64
            } else {
                0.0
            };

            let most_used = strategies.iter()
                .max_by_key(|s| s.use_count)
                .map(|s| (s.title.clone(), s.use_count));

            println!("╔══════════════════════════════════════════════════════════════╗");
            println!("║  📈 PLAYBOOK STATISTICS                                       ║");
            println!("╠══════════════════════════════════════════════════════════════╣");
            println!("║  Total Entries: {}", total);
            println!("║    - Strategies: {}", strategies.len());
            println!("║    - Insights: {}", insights.len());
            println!("║    - Patterns: {}", patterns.len());
            println!("║");
            println!("║  Avg Effectiveness: {:.0}%", avg_effectiveness * 100.0);

            if let Some((title, count)) = most_used {
                println!("║  Most Used: {} ({} times)", title, count);
            }

            println!("╚══════════════════════════════════════════════════════════════╝");
        }

        Commands::Curate { min_effectiveness, max_age_days } => {
            println!("🧹 Curating playbook...");
            println!("   Min effectiveness: {:.0}%", min_effectiveness * 100.0);
            println!("   Max age: {} days", max_age_days);

            let removed = pb.curate_playbook(min_effectiveness, max_age_days)?;

            if removed > 0 {
                println!("✅ Removed {} ineffective entries", removed);
            } else {
                println!("✅ No entries removed - playbook is clean");
            }
        }

        Commands::GetContext { tags, limit } => {
            let tags_vec = tags.map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
            let mut strategies = pb.get_strategies(tags_vec)?;

            // Sort by effectiveness and limit
            strategies.sort_by(|a, b| b.effectiveness.partial_cmp(&a.effectiveness).unwrap());
            strategies.truncate(limit);

            if strategies.is_empty() {
                println!("No relevant strategies found");
                return Ok(());
            }

            println!("ACE PLAYBOOK CONTEXT:");
            println!("======================");
            for (i, s) in strategies.iter().enumerate() {
                println!("\n{}. {} (effectiveness: {:.0}%)", i + 1, s.title, s.effectiveness * 100.0);
                println!("   Context: {}", s.context);
                println!("   Approach: {}", s.approach);
            }
        }
    }

    Ok(())
}
