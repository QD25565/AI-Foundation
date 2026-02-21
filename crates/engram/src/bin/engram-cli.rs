//! Engram CLI - Command line interface for the Engram database

use clap::{Parser, Subcommand};
use engram::{Engram, EngramStats};
use engram::embedding::{EmbeddingGenerator, EmbeddingConfig};
use std::path::PathBuf;

/// Get the default database path based on AI_ID
/// Uses centralized location: ~/.ai-foundation/agents/{ai_id}/notebook.engram
/// This matches the MCP server path for CLI/MCP parity.
/// Per-agent directory groups all agent data (notebook, tasks, config).
fn get_default_database_path() -> PathBuf {
    let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| "default".to_string());

    // Use ~/.ai-foundation/agents/{ai_id}/notebook.engram (same as MCP server)
    if let Some(home) = dirs::home_dir() {
        // NEW path: ~/.ai-foundation/agents/{ai_id}/notebook.engram
        let new_dir = home.join(".ai-foundation").join("agents").join(&ai_id);
        let new_path = new_dir.join("notebook.engram");

        // OLD path: ~/.ai-foundation/notebook/{ai_id}.engram
        let old_path = home.join(".ai-foundation").join("notebook").join(format!("{}.engram", ai_id));

        // Auto-migrate if old exists but new doesn't
        if old_path.exists() && !new_path.exists() {
            let _ = std::fs::create_dir_all(&new_dir);
            if std::fs::rename(&old_path, &new_path).is_ok() {
                eprintln!("[MIGRATED] {} -> {}", old_path.display(), new_path.display());
            }
        } else {
            let _ = std::fs::create_dir_all(&new_dir);
        }

        new_path
    } else {
        // Fallback to current directory if no home dir
        PathBuf::from(format!("{}.engram", ai_id))
    }
}

#[derive(Parser)]
#[command(name = "engram")]
#[command(version = "0.1.0")]
#[command(about = "Purpose-built AI memory database", long_about = None)]
struct Cli {
    /// Path to the Engram database file (default: ~/.ai-foundation/notebook/{AI_ID}.engram)
    #[arg(short, long)]
    database: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Store a new memory
    #[command(alias = "add", alias = "store", alias = "save")]
    Remember {
        /// Content to remember
        #[arg(value_name = "CONTENT")]
        content: String,

        /// Tags (comma-separated)
        #[arg(short, long, value_delimiter = ',')]
        tags: Vec<String>,
    },

    /// Retrieve a memory by ID
    #[command(alias = "read", alias = "fetch")]
    Get {
        /// Note ID
        #[arg(value_name = "ID")]
        id: u64,
    },

    /// Delete a memory
    #[command(alias = "delete", alias = "remove", alias = "rm")]
    Forget {
        /// Note ID to forget
        #[arg(value_name = "ID")]
        id: u64,
    },

    /// List recent memories
    #[command(alias = "ls", alias = "recent")]
    List {
        /// Maximum number of notes to show
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },

    /// Search memories by tag
    #[command(alias = "search", alias = "find")]
    ByTag {
        /// Tag to search for
        #[arg(value_name = "TAG")]
        tag: String,
    },

    /// Hybrid search (keyword + semantic + graph + recency) - EXCEPTIONAL RECALL
    #[command(alias = "search-smart", alias = "query")]
    Recall {
        /// Search query
        #[arg(value_name = "QUERY")]
        query: String,

        /// Maximum results
        #[arg(short, long, default_value = "10")]
        limit: usize,

        /// Path to embedding model (default: ~/.ai-foundation/models/embeddinggemma-300M-Q8_0.gguf)
        #[arg(short, long)]
        model: Option<PathBuf>,

        /// Skip semantic search (keyword + graph + recency only)
        #[arg(long)]
        no_semantic: bool,
    },

    /// Pin a note
    Pin {
        /// Note ID to pin
        #[arg(value_name = "ID")]
        id: u64,
    },

    /// Unpin a note
    Unpin {
        /// Note ID to unpin
        #[arg(value_name = "ID")]
        id: u64,
    },

    /// Show pinned notes
    Pinned,

    /// Vault operations (encrypted key-value store)
    #[command(subcommand)]
    Vault(VaultCommands),

    /// Graph operations
    #[command(subcommand)]
    Graph(GraphCommands),

    /// Show database statistics
    #[command(alias = "info", alias = "status")]
    Stats,

    /// Verify database integrity
    #[command(alias = "check")]
    Verify,

    /// Persist indexes for O(1) startup
    #[command(alias = "save-indexes")]
    PersistIndexes,

    /// Sync and update memory map
    Sync,

    /// Backfill embeddings for notes that don't have them
    /// Foundational for exceptional recall (keyword + semantic + graph)
    #[command(alias = "embed-all", alias = "generate-embeddings")]
    Backfill {
        /// Path to embedding model (default: ~/.ai-foundation/models/embeddinggemma-300M-Q8_0.gguf)
        #[arg(short, long)]
        model: Option<PathBuf>,

        /// Batch size for progress reporting
        #[arg(short, long, default_value = "10")]
        batch_size: usize,
    },
}

#[derive(Subcommand)]
enum VaultCommands {
    /// Set a secret value
    Set {
        /// Key name
        #[arg(value_name = "KEY")]
        key: String,

        /// Value to store
        #[arg(value_name = "VALUE")]
        value: String,
    },

    /// Get a secret value
    Get {
        /// Key name
        #[arg(value_name = "KEY")]
        key: String,
    },

    /// List all keys in the vault
    #[command(alias = "ls", alias = "keys")]
    List,

    /// Delete a key from the vault
    #[command(alias = "rm", alias = "remove")]
    Delete {
        /// Key to delete
        #[arg(value_name = "KEY")]
        key: String,
    },
}

#[derive(Subcommand)]
enum GraphCommands {
    /// Show related notes
    Related {
        /// Note ID
        #[arg(value_name = "ID")]
        id: u64,
    },

    /// Compute PageRank scores
    #[command(alias = "pr")]
    Pagerank,

    /// Get PageRank score for a note
    Score {
        /// Note ID
        #[arg(value_name = "ID")]
        id: u64,
    },

    /// Auto-link a note (semantic + temporal edges)
    Autolink {
        /// Note ID
        #[arg(value_name = "ID")]
        id: u64,
    },

    /// Dump graph structure (debug)
    Dump,

    /// Rebuild graph edges from existing embeddings
    /// Creates semantic + temporal edges for all notes with embeddings
    #[command(alias = "rebuild-all")]
    Rebuild,
}

fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = Cli::parse();

    // Use provided database path or compute default from AI_ID
    // Default: ~/.ai-foundation/notebook/{ai_id}.engram (matches MCP server)
    let database = cli.database.unwrap_or_else(get_default_database_path);

    match cli.command {
        Commands::Remember { content, tags } => {
            let mut db = Engram::open(&database)?;
            let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
            let id = db.remember(&content, &tag_refs)?;

            // Generate embedding for the new note (foundational for exceptional recall)
            if let Some(model_path) = EmbeddingGenerator::find_model("embeddinggemma-300M-Q8_0.gguf") {
                let config = EmbeddingConfig::default().with_model(&model_path);
                if let Ok(mut embedder) = EmbeddingGenerator::load(config) {
                    if let Ok(embedding) = embedder.embed(&content) {
                        let _ = db.add_embedding(id, &embedding);
                    }
                }
            }

            // Auto-link: semantic + temporal edges + PageRank (30-minute session window)
            // This ensures notes within a "session" are connected in the knowledge graph
            let _ = db.auto_link(id);

            db.persist_indexes()?;  // Persist all changes

            println!("remember/ok/{}", id);
        }

        Commands::Get { id } => {
            let mut db = Engram::open(&database)?;
            match db.get(id)? {
                Some(note) => {
                    println!("note/{}/{}", note.id, note.content.len());
                    println!("{}", note.content);
                    if !note.tags.is_empty() {
                        println!("tags/{}", note.tags.join(","));
                    }
                }
                None => {
                    eprintln!("Error: Note {} not found", id);
                    std::process::exit(1);
                }
            }
        }

        Commands::Forget { id } => {
            let mut db = Engram::open(&database)?;
            db.forget(id)?;
            println!("forget/ok/{}", id);
        }

        Commands::List { limit } => {
            let mut db = Engram::open(&database)?;
            let notes = db.list(limit)?;
            println!("notes/{}", notes.len());
            for note in notes {
                let pinned = if note.pinned { "[P]" } else { "" };
                let tags = if note.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", note.tags.join(","))
                };
                // Truncate content for display
                let preview: String = note.content.chars().take(80).collect();
                let preview = preview.replace('\n', " ");
                println!("{}|{}|{}{}{}", note.id, note.age_string(), pinned, preview, tags);
            }
        }

        Commands::ByTag { tag } => {
            let mut db = Engram::open(&database)?;
            let notes = db.by_tag(&tag)?;
            println!("tag/{}|{}", tag, notes.len());
            for note in notes {
                let preview: String = note.content.chars().take(60).collect();
                println!("{}|{}", note.id, preview.replace('\n', " "));
            }
        }

        Commands::Recall { query, limit, model, no_semantic } => {
            let mut db = Engram::open(&database)?;

            // Generate query embedding for full semantic search (unless --no-semantic)
            let query_embedding = if no_semantic {
                None
            } else {
                // Find and load embedding model
                let model_file = if let Some(path) = model {
                    Some(path)
                } else {
                    EmbeddingGenerator::find_model("embeddinggemma-300M-Q8_0.gguf")
                };

                if let Some(model_path) = model_file {
                    let config = EmbeddingConfig::default().with_model(&model_path);
                    match EmbeddingGenerator::load(config) {
                        Ok(mut embedder) => {
                            match embedder.embed(&query) {
                                Ok(emb) => Some(emb),
                                Err(e) => {
                                    eprintln!("Warning: Failed to generate query embedding: {}. Using keyword-only.", e);
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Warning: Failed to load embedding model: {}. Using keyword-only.", e);
                            None
                        }
                    }
                } else {
                    eprintln!("Warning: Embedding model not found. Using keyword-only search.");
                    None
                }
            };

            let results = db.recall(&query, query_embedding.as_deref(), limit)?;
            println!("recall/{}", results.len());
            for result in results {
                let preview: String = result.note.content.chars().take(60).collect();
                println!("{}|{:.3}|{:.3}|{:.3}|{:.3}|{:.3}|{}",
                    result.note.id,
                    result.final_score,
                    result.vector_score,
                    result.keyword_score,
                    result.graph_score,
                    result.recency_score,
                    preview.replace('\n', " ")
                );
            }
        }

        Commands::Pin { id } => {
            let mut db = Engram::open(&database)?;
            db.pin(id)?;
            println!("pin/ok/{}", id);
        }

        Commands::Unpin { id } => {
            let mut db = Engram::open(&database)?;
            db.unpin(id)?;
            println!("unpin/ok/{}", id);
        }

        Commands::Pinned => {
            let mut db = Engram::open(&database)?;
            let notes = db.pinned()?;
            println!("pinned/{}", notes.len());
            for note in notes {
                let preview: String = note.content.chars().take(60).collect();
                println!("{}|{}", note.id, preview.replace('\n', " "));
            }
        }

        Commands::Vault(vault_cmd) => {
            handle_vault_command(&database, vault_cmd)?;
        }

        Commands::Graph(graph_cmd) => {
            handle_graph_command(&database, graph_cmd)?;
        }

        Commands::Stats => {
            let mut db = Engram::open(&database)?;
            let stats = db.stats();
            print_stats(&stats);
        }

        Commands::Verify => {
            let mut db = Engram::open(&database)?;
            let result = db.verify()?;
            if result.is_valid {
                println!("verify/ok");
            } else {
                println!("verify/failed");
                for error in &result.errors {
                    eprintln!("ERROR: {}", error);
                }
            }
            for warning in &result.warnings {
                eprintln!("WARNING: {}", warning);
            }
        }

        Commands::PersistIndexes => {
            let mut db = Engram::open(&database)?;
            db.persist_indexes()?;
            println!("persist-indexes/ok");
        }

        Commands::Sync => {
            let mut db = Engram::open(&database)?;
            db.sync()?;
            println!("sync/ok");
        }

        Commands::Backfill { model, batch_size } => {
            handle_backfill(&database, model, batch_size)?;
        }
    }

    Ok(())
}

fn handle_backfill(database: &PathBuf, model_path: Option<PathBuf>, batch_size: usize) -> anyhow::Result<()> {
    let start = std::time::Instant::now();

    // Find embedding model
    let model_file = if let Some(path) = model_path {
        path
    } else {
        // Try common locations
        let model_name = "embeddinggemma-300M-Q8_0.gguf";
        EmbeddingGenerator::find_model(model_name)
            .ok_or_else(|| anyhow::anyhow!(
                "Embedding model not found. Expected at ~/.ai-foundation/models/{}\n\
                 Download from: https://huggingface.co/ggml-org/embeddinggemma-300M-GGUF",
                model_name
            ))?
    };

    println!("backfill/loading_model");
    eprintln!("Loading embedding model: {:?}", model_file);

    // Load embedding model
    let config = EmbeddingConfig::default().with_model(&model_file);
    let mut embedder = EmbeddingGenerator::load(config)?;

    println!("backfill/model_loaded");

    // Open database
    let mut db = Engram::open(database)?;
    let stats = db.stats();

    println!("backfill/scanning|notes:{}", stats.active_notes);

    // Get all notes that don't have embeddings
    let mut processed = 0u64;
    let mut embedded = 0u64;
    let mut skipped = 0u64;
    let mut errors = 0u64;

    // Get all note IDs
    let notes = db.list(stats.active_notes as usize)?;
    let total = notes.len();

    println!("backfill/starting|total:{}", total);

    for (i, note) in notes.iter().enumerate() {
        // Check if already has embedding
        if db.has_embedding(note.id) {
            skipped += 1;
            continue;
        }

        // Generate embedding
        match embedder.embed(&note.content) {
            Ok(embedding) => {
                // Add to database
                if let Err(e) = db.add_embedding(note.id, &embedding) {
                    eprintln!("Error adding embedding for note {}: {}", note.id, e);
                    errors += 1;
                } else {
                    embedded += 1;

                    // Also create graph edges (semantic + temporal)
                    if let Err(e) = db.auto_link(note.id) {
                        eprintln!("Error auto-linking note {}: {}", note.id, e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Error generating embedding for note {}: {}", note.id, e);
                errors += 1;
            }
        }

        processed += 1;

        // Progress report
        if (i + 1) % batch_size == 0 || i + 1 == total {
            let pct = ((i + 1) as f64 / total as f64 * 100.0) as u32;
            eprintln!("Progress: {}/{} ({}%) - embedded: {}, skipped: {}, errors: {}",
                i + 1, total, pct, embedded, skipped, errors);
        }
    }

    // Persist indexes
    db.persist_indexes()?;

    let duration_ms = start.elapsed().as_millis();

    println!("backfill/complete|processed:{}|embedded:{}|skipped:{}|errors:{}|duration_ms:{}",
        processed, embedded, skipped, errors, duration_ms);

    // Show updated stats
    let new_stats = db.stats();
    println!("backfill/vectors_now:{}", new_stats.vector_count);

    Ok(())
}

fn handle_vault_command(database: &PathBuf, cmd: VaultCommands) -> anyhow::Result<()> {
    let mut db = Engram::open(database)?;

    match cmd {
        VaultCommands::Set { key, value } => {
            db.vault_set_string(&key, &value)?;
            println!("vault/set/{}", key);
        }

        VaultCommands::Get { key } => {
            match db.vault_get_string(&key)? {
                Some(value) => {
                    println!("vault/get/{}", key);
                    println!("{}", value);
                }
                None => {
                    eprintln!("Error: Key '{}' not found in vault", key);
                    std::process::exit(1);
                }
            }
        }

        VaultCommands::List => {
            let keys = db.vault_keys();
            println!("vault/keys/{}", keys.len());
            for key in keys {
                println!("{}", key);
            }
        }

        VaultCommands::Delete { key } => {
            if db.vault_delete(&key) {
                println!("vault/delete/{}", key);
            } else {
                eprintln!("Error: Key '{}' not found", key);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn handle_graph_command(database: &PathBuf, cmd: GraphCommands) -> anyhow::Result<()> {
    let mut db = Engram::open(database)?;

    match cmd {
        GraphCommands::Related { id } => {
            let related = db.get_related(id);
            println!("related/{}/{}", id, related.len());
            for (target_id, weight, edge_type) in related {
                println!("{}|{:.3}|{:?}", target_id, weight, edge_type);
            }
        }

        GraphCommands::Pagerank => {
            db.compute_pagerank();
            db.persist_indexes()?;  // Must persist after computing PageRank!
            println!("pagerank/computed");
        }

        GraphCommands::Score { id } => {
            let score = db.get_pagerank(id);
            println!("pagerank/{}|{:.6}", id, score);
        }

        GraphCommands::Autolink { id } => {
            let (semantic, temporal) = db.auto_link(id)?;
            println!("autolink/{}|semantic:{}|temporal:{}", id, semantic, temporal);
        }

        GraphCommands::Dump => {
            let info = db.dump_graph_info();
            println!("graph/dump");
            println!("edge_count:{}", info.0);
            println!("node_count:{}", info.1);
            println!("sample_nodes:{:?}", info.2);
        }

        GraphCommands::Rebuild => {
            println!("graph/rebuild/starting");

            // Get all notes
            let stats = db.stats();
            let notes = db.list(stats.active_notes as usize)?;
            let total = notes.len();

            let mut semantic_total = 0usize;
            let mut temporal_total = 0usize;
            let mut linked = 0usize;
            let mut skipped = 0usize;

            for (i, note) in notes.iter().enumerate() {
                // Only auto_link notes that have embeddings
                if !db.has_embedding(note.id) {
                    skipped += 1;
                    continue;
                }

                match db.auto_link(note.id) {
                    Ok((semantic, temporal)) => {
                        semantic_total += semantic;
                        temporal_total += temporal;
                        linked += 1;
                    }
                    Err(e) => {
                        eprintln!("Error linking note {}: {}", note.id, e);
                    }
                }

                // Progress report every 100 notes
                if (i + 1) % 100 == 0 || i + 1 == total {
                    let pct = ((i + 1) as f64 / total as f64 * 100.0) as u32;
                    eprintln!("Progress: {}/{} ({}%) - linked: {}, skipped: {}",
                        i + 1, total, pct, linked, skipped);
                }
            }

            // Persist the graph
            db.persist_indexes()?;

            // Final stats
            let new_info = db.dump_graph_info();
            println!("graph/rebuild/complete");
            println!("linked:{}|skipped:{}|semantic:{}|temporal:{}",
                linked, skipped, semantic_total, temporal_total);
            println!("edges:{}|nodes:{}", new_info.0, new_info.1);
        }
    }

    Ok(())
}

fn print_stats(stats: &EngramStats) {
    println!("stats/engram");
    println!("notes/{}|active/{}|tombstones/{}",
        stats.note_count, stats.active_notes, stats.tombstone_count);
    println!("pinned/{}|tags/{}|vectors/{}",
        stats.pinned_count, stats.tag_count, stats.vector_count);
    println!("edges/{}|vault/{}", stats.edge_count, stats.vault_entries);
    println!("file_size/{}", human_size(stats.file_size));
}

fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
