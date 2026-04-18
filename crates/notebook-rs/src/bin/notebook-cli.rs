///! Notebook CLI - High-Performance Local AI Notebook
///!
///! Each AI instance has their own private notebook stored locally.
///! Uses Engram - a purpose-built AI memory database (1000x faster than SQLite).
///!
///! Usage:
///!   notebook-cli remember "Important note" --tags important,feature
///!   notebook-cli recall "search query"
///!   notebook-cli list --limit 10
///!   notebook-cli pin 123
///!   notebook-cli vault set api_token secret123
///!   notebook-cli vault get api_token
///!   notebook-cli stats

// Use mimalloc for 10-30% faster allocations
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use engram::{Engram, graph::EdgeType};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "notebook-cli")]
#[command(about = "High-performance AI notebook powered by Engram (1000x faster than SQLite)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Save a note to memory with optional screenshot capture
    #[command(alias = "save", alias = "note", alias = "mem")]
    Remember {
        /// Note content (e.g., "Fixed login bug, UI now correct")
        #[arg(value_name = "CONTENT")]
        content: String,

        /// Comma-separated tags (e.g., "bug,ui,milestone")
        #[arg(long)]
        tags: Option<String>,

        /// Pin this note for quick access
        #[arg(long)]
        pin: bool,

        /// Path to existing image file (e.g., "screenshot.png")
        #[arg(long, value_name = "IMAGE_PATH")]
        image: Option<String>,

        /// Capture screenshot - window title (e.g., "MyApp") or omit for full screen
        #[arg(long, value_name = "WINDOW", num_args = 0..=1, default_missing_value = "")]
        capture: Option<String>,

        // Hidden aliases for capture
        #[arg(long = "screenshot", hide = true, value_name = "WINDOW", num_args = 0..=1, default_missing_value = "")]
        screenshot: Option<String>,
        #[arg(long = "snap", hide = true, value_name = "WINDOW", num_args = 0..=1, default_missing_value = "")]
        snap: Option<String>,
    },

    /// Save an ephemeral working memory that expires automatically
    #[command(alias = "temp", alias = "ephemeral", alias = "scratch")]
    Work {
        /// Note content (e.g., "Currently debugging auth token refresh")
        #[arg(value_name = "CONTENT")]
        content: String,

        /// Comma-separated tags (e.g., "wip,session,context")
        #[arg(long)]
        tags: Option<String>,

        /// Hours until this note expires and is filtered from recall (default: 24)
        #[arg(long = "ttl", value_name = "HOURS", default_value = "24")]
        ttl_hours: u16,
    },

    /// Search and recall notes
    #[command(alias = "search", alias = "find", alias = "query", alias = "lookup")]
    Recall {
        /// Search query (e.g., "login bug")
        #[arg(value_name = "QUERY")]
        query: Option<String>,

        /// Maximum results (e.g., 10, 20)
        #[arg(value_name = "LIMIT")]
        limit_positional: Option<usize>,

        #[arg(long = "limit", hide = true, default_value = "10")]
        limit: usize,

        /// Only show pinned notes
        #[arg(long)]
        pinned_only: bool,
    },

    /// List recent notes
    #[command(alias = "ls", alias = "recent", alias = "show", alias = "all")]
    List {
        /// Maximum results (e.g., 10, 20)
        #[arg(value_name = "LIMIT")]
        limit_positional: Option<usize>,

        #[arg(long = "limit", hide = true, default_value = "20")]
        limit: usize,

        /// Only show pinned notes
        #[arg(long)]
        pinned_only: bool,

        /// Only show non-pinned notes
        #[arg(long)]
        not_pinned: bool,

        /// Output only note IDs (one per line)
        #[arg(long)]
        ids_only: bool,

        /// Filter by tag (e.g. "ai-foundation")
        #[arg(long)]
        tag: Option<String>,
    },

    /// Pin a note for quick access
    #[command(alias = "star", alias = "mark", alias = "favorite")]
    Pin {
        /// Note ID to pin (e.g., 123)
        #[arg(value_name = "NOTE_ID")]
        note_id_positional: Option<u64>,

        #[arg(long = "note-id", hide = true)]
        note_id: Option<u64>,
    },

    /// Unpin a note
    #[command(alias = "unstar", alias = "unmark", alias = "unfavorite")]
    Unpin {
        /// Note ID to unpin (e.g., 123)
        #[arg(value_name = "NOTE_ID")]
        note_id_positional: Option<u64>,

        #[arg(long = "note-id", hide = true)]
        note_id: Option<u64>,
    },

    /// Get a specific note by ID
    #[command(alias = "read", alias = "view", alias = "fetch")]
    Get {
        /// Note ID (e.g., 123)
        #[arg(value_name = "NOTE_ID")]
        note_id_positional: Option<u64>,

        #[arg(long = "note-id", hide = true)]
        note_id: Option<u64>,
    },

    /// Update a note's content and/or tags
    #[command(alias = "edit", alias = "modify", alias = "change")]
    Update {
        /// Note ID to update (e.g., 123)
        #[arg(value_name = "NOTE_ID")]
        note_id_positional: Option<u64>,

        #[arg(long = "id", hide = true)]
        note_id: Option<u64>,

        /// New content (replaces existing content)
        #[arg(long)]
        content: Option<String>,

        /// New tags (comma-separated, replaces existing tags)
        #[arg(long)]
        tags: Option<String>,
    },

    /// Add tags to an existing note (keeps existing tags)
    #[command(alias = "tag", alias = "add-tag")]
    AddTags {
        /// Note ID (e.g., 123)
        #[arg(value_name = "NOTE_ID")]
        note_id_positional: Option<u64>,

        #[arg(long = "note-id", hide = true)]
        note_id: Option<u64>,

        /// Tags to add (comma-separated)
        #[arg(value_name = "TAGS")]
        tags_positional: Option<String>,

        #[arg(long = "tags", hide = true)]
        tags: Option<String>,
    },

    /// Delete a note permanently
    #[command(alias = "rm", alias = "remove", alias = "trash", alias = "forget")]
    Delete {
        /// Note ID to delete (e.g., 123)
        #[arg(value_name = "NOTE_ID")]
        note_id_positional: Option<u64>,

        #[arg(long = "note-id", hide = true)]
        note_id: Option<u64>,
    },

    /// Vault key-value storage operations
    Vault {
        #[command(subcommand)]
        operation: VaultOps,
    },

    /// Show notebook statistics
    #[command(alias = "stat", alias = "info", alias = "status", alias = "summary")]
    Stats,

    /// List all pinned notes
    #[command(alias = "starred", alias = "favorites", alias = "important")]
    Pinned {
        /// Maximum results (e.g., 10, 20)
        #[arg(value_name = "LIMIT")]
        limit_positional: Option<usize>,

        #[arg(long = "limit", hide = true, default_value = "50")]
        limit: usize,

        /// Output only note IDs (one per line)
        #[arg(long)]
        ids_only: bool,
    },

    /// Start a new session (called by platform hooks)
    #[command(alias = "session", alias = "init")]
    StartSession,

    // ==================== EMBEDDING OPERATIONS ====================

    /// Generate embedding for a specific note
    #[command(alias = "vectorize", alias = "encode", alias = "embed-note", alias = "gen-embedding")]
    Embed {
        /// Note ID to embed (e.g., 123)
        #[arg(value_name = "NOTE_ID")]
        note_id_positional: Option<u64>,

        #[arg(long = "note-id", hide = true)]
        note_id: Option<u64>,

        /// Path to embedding model (e.g., embeddinggemma-300M-Q8_0.gguf)
        #[arg(long = "model")]
        model_path: Option<String>,
    },

    /// Generate embeddings for notes missing them
    #[command(name = "generate-embeddings", alias = "backfill", alias = "backfill-embeddings", alias = "gen-embeddings", alias = "vectorize-all")]
    GenerateEmbeddings {
        /// Maximum notes to process (e.g., 100, 1000)
        #[arg(value_name = "LIMIT")]
        limit_positional: Option<usize>,

        #[arg(long = "limit", hide = true)]
        limit: Option<usize>,

        /// Path to embedding model
        #[arg(long = "model")]
        model_path: Option<String>,

        /// Skip notes that already have embeddings
        #[arg(long = "skip-existing", default_value = "true")]
        skip_existing: bool,
    },

    // ==================== GRAPH MEMORY (Zettelkasten Links) ====================

    /// Link two notes together
    #[command(alias = "connect", alias = "relate", alias = "edge")]
    Link {
        /// Source note ID
        #[arg(value_name = "FROM_ID")]
        from_id: u64,

        /// Target note ID
        #[arg(value_name = "TO_ID")]
        to_id: u64,

        /// Relationship type: semantic, temporal, tag, manual
        #[arg(long = "relationship", default_value = "manual")]
        relationship: String,

        /// Link weight (0.0-1.0, higher = stronger connection)
        #[arg(long = "weight", default_value = "1.0")]
        weight: f32,
    },

    /// Unlink two notes (remove edge)
    #[command(alias = "disconnect", alias = "unrelate", alias = "remove-edge")]
    Unlink {
        /// Source note ID
        #[arg(value_name = "FROM_ID")]
        from_id: u64,

        /// Target note ID
        #[arg(value_name = "TO_ID")]
        to_id: u64,
    },

    /// Invalidate an edge between two notes (removes it from graph scoring)
    #[command(alias = "invalidate-edge", alias = "remove-valid-edge")]
    InvalidateEdge {
        /// Source note ID
        #[arg(value_name = "FROM_ID")]
        from: u64,
        /// Target note ID
        #[arg(value_name = "TO_ID")]
        to: u64,
    },

    /// Get notes linked to a specific note
    #[command(alias = "linked", alias = "connections", alias = "neighbors")]
    GetLinked {
        /// Note ID to get links for
        #[arg(value_name = "NOTE_ID")]
        note_id: u64,
    },

    /// Auto-link notes temporally (within time window)
    #[command(alias = "link-temporal", alias = "temporal-link", alias = "time-link")]
    AutoLinkTemporal {
        /// Note ID to link
        #[arg(value_name = "NOTE_ID")]
        note_id: u64,

        /// Time window in minutes
        #[arg(long = "window", default_value = "30")]
        window_minutes: i64,
    },

    /// Auto-link notes semantically (by similarity)
    #[command(alias = "link-semantic", alias = "semantic-link", alias = "similar-link")]
    AutoLinkSemantic {
        /// Note ID to link
        #[arg(value_name = "NOTE_ID")]
        note_id: u64,

        /// Number of similar notes to link
        #[arg(long = "top-k", default_value = "5")]
        top_k: usize,

        /// Minimum similarity threshold (0.0-1.0)
        #[arg(long = "min-similarity", default_value = "0.65")]
        min_similarity: f32,
    },

    /// Compute importance scores for all notes based on graph connections
    #[command(name = "rank-notes", alias = "pagerank", alias = "rank", alias = "compute-rank", alias = "compute-pagerank")]
    RankNotes,

    /// Persist indexes to disk
    #[command(alias = "persist", alias = "save-indexes", alias = "flush")]
    PersistIndexes,

    /// Verify database integrity
    #[command(alias = "check-db", alias = "integrity", alias = "validate")]
    Verify,

    // ==================== BATCH OPERATIONS ====================

    /// Delete multiple notes by ID
    #[command(alias = "bulk-delete")]
    BatchDelete {
        /// Comma-separated note IDs to delete (e.g., "1,2,3,4")
        note_ids: String,
        /// Skip confirmation
        #[arg(long = "yes")]
        confirm: bool,
    },

    /// Pin multiple notes by ID
    #[command(alias = "bulk-pin")]
    BatchPin {
        /// Comma-separated note IDs to pin (e.g., "1,2,3,4")
        note_ids: String,
    },

    /// Unpin multiple notes by ID
    #[command(alias = "bulk-unpin")]
    BatchUnpin {
        /// Comma-separated note IDs to unpin (e.g., "1,2,3,4")
        note_ids: String,
    },

    /// Export notes to JSON
    #[command(alias = "export-notes")]
    Export {
        /// Output file path (e.g., "backup.json")
        output: String,
        /// Only export pinned notes
        #[arg(long = "pinned-only")]
        pinned_only: bool,
        /// Limit number of notes
        #[arg(long = "limit", default_value = "1000")]
        limit: usize,
    },

    // ==================== TEMPORAL & GRAPH QUERIES (Phase 2) ====================

    /// Show notes chronologically (timeline view)
    #[command(alias = "chrono", alias = "history", alias = "when", alias = "time")]
    Timeline {
        /// Number of notes to show (e.g., 20)
        #[arg(value_name = "LIMIT")]
        limit_positional: Option<usize>,

        #[arg(long = "limit", hide = true, default_value = "20")]
        limit: usize,

        /// Show oldest first (default: newest first)
        #[arg(long = "oldest")]
        oldest_first: bool,

        /// Filter by hours ago (e.g., 24 for last day)
        #[arg(long = "hours")]
        hours_ago: Option<i64>,

        /// Filter by days ago (e.g., 7 for last week)
        #[arg(long = "days")]
        days_ago: Option<i64>,
    },

    /// Find notes created in a specific time range
    #[command(alias = "range", alias = "between", alias = "period")]
    TimeRange {
        /// Start date (YYYY-MM-DD or relative like "7d" for 7 days ago)
        #[arg(value_name = "START")]
        start: String,

        /// End date (YYYY-MM-DD or "now" for current time)
        #[arg(value_name = "END")]
        end: Option<String>,

        /// Maximum results
        #[arg(long = "limit", default_value = "50")]
        limit: usize,
    },

    /// Multi-hop traversal from a note (explore knowledge graph)
    #[command(alias = "explore", alias = "neighbors", alias = "graph-walk")]
    Traverse {
        /// Starting note ID
        #[arg(value_name = "NOTE_ID")]
        note_id: u64,

        /// Maximum hops (depth) from start note
        #[arg(long = "depth", default_value = "2")]
        max_depth: usize,

        /// Edge type filter: all, semantic, temporal, manual, tag
        #[arg(long = "edge-type", default_value = "all")]
        edge_type: String,

        /// Show full content (default: summary)
        #[arg(long = "full")]
        full_content: bool,
    },

    /// Find path between two notes in the knowledge graph
    #[command(alias = "connect", alias = "route", alias = "link-path")]
    Path {
        /// Source note ID
        #[arg(value_name = "FROM_ID")]
        from_id: u64,

        /// Target note ID
        #[arg(value_name = "TO_ID")]
        to_id: u64,

        /// Maximum path length (hops)
        #[arg(long = "max-depth", default_value = "5")]
        max_depth: usize,
    },

    /// Show what notes are related to a given note (by any edge type)
    #[command(alias = "related-to", alias = "connections", alias = "edges")]
    Related {
        /// Note ID to find related notes for
        #[arg(value_name = "NOTE_ID")]
        note_id: u64,

        /// Edge type filter: all, semantic, temporal, manual, tag
        #[arg(long = "edge-type", default_value = "all")]
        edge_type: String,

        /// Show incoming edges (notes that link TO this note)
        #[arg(long = "incoming")]
        incoming: bool,
    },

    // ==================== COGNITIVE MEMORY TYPES (Phase 3) ====================

    /// Classify a note's memory type (semantic/episodic/procedural)
    #[command(alias = "memory-type", alias = "type", alias = "categorize")]
    Classify {
        /// Note ID to classify (or "all" for recent notes)
        #[arg(value_name = "NOTE_ID")]
        note_id: Option<String>,

        /// Number of notes to classify (when using "all")
        #[arg(long = "limit", default_value = "20")]
        limit: usize,
    },

    /// Search notes by memory type
    #[command(alias = "by-type", alias = "filter-type", alias = "memory-search")]
    ByMemoryType {
        /// Memory type to search: semantic, episodic, procedural
        #[arg(value_name = "TYPE")]
        memory_type: String,

        /// Maximum results
        #[arg(value_name = "LIMIT")]
        limit_positional: Option<usize>,

        #[arg(long = "limit", hide = true, default_value = "20")]
        limit: usize,
    },

    /// Show memory type statistics
    #[command(alias = "memory-stats", alias = "type-stats", alias = "cognitive-stats")]
    MemoryStats {
        /// Number of notes to analyze
        #[arg(long = "limit", default_value = "100")]
        limit: usize,
    },

    // ==================== KNOWLEDGE GRAPH 2.0 FEATURES (Phase 4) ====================

    /// Show detailed knowledge graph statistics
    #[command(alias = "graph", alias = "kg-stats", alias = "knowledge-graph")]
    GraphStats,

    /// Explain the connection between two notes (reasoning chain)
    #[command(alias = "why", alias = "connection", alias = "how-connected")]
    Explain {
        /// First note ID
        #[arg(value_name = "FROM_ID")]
        from_id: u64,

        /// Second note ID
        #[arg(value_name = "TO_ID")]
        to_id: u64,

        /// Maximum path depth to search
        #[arg(long = "depth", default_value = "5")]
        max_depth: usize,
    },

    /// Run database health check with suggestions
    #[command(alias = "health", alias = "check", alias = "diagnose")]
    HealthCheck {
        /// Fix issues automatically where possible
        #[arg(long = "fix")]
        auto_fix: bool,
    },

    /// Show top notes by PageRank (most important/connected)
    #[command(alias = "important", alias = "top-notes", alias = "top")]
    TopNotes {
        /// Number of top notes to show
        #[arg(value_name = "LIMIT")]
        limit_positional: Option<usize>,

        #[arg(long = "limit", hide = true, default_value = "10")]
        limit: usize,
    },

    // ==================== AI PROFILE ====================

    /// Manage your AI profile (identity, visuals)
    #[command(alias = "me", alias = "identity", alias = "self")]
    Profile {
        #[command(subcommand)]
        operation: ProfileOps,
    },

    /// List all tags with note counts (sorted by frequency)
    #[command(alias = "tag-list", alias = "list-tags")]
    Tags {
        /// Maximum tags to show (e.g., 20)
        #[arg(value_name = "LIMIT")]
        limit_positional: Option<usize>,

        #[arg(long = "limit", hide = true, default_value = "50")]
        limit: usize,
    },
}

#[derive(Subcommand)]
enum VaultOps {
    /// Store a key-value pair
    #[command(alias = "set", alias = "put", alias = "save")]
    Store {
        /// Key name (e.g., "api_token")
        #[arg(value_name = "KEY")]
        key_positional: Option<String>,

        /// Value to store (e.g., "secret123")
        #[arg(value_name = "VALUE")]
        value_positional: Option<String>,

        #[arg(long = "key", hide = true)]
        key: Option<String>,
        #[arg(long = "value", hide = true)]
        value: Option<String>,
    },

    /// Retrieve a value by key
    #[command(alias = "read", alias = "fetch", alias = "show")]
    Get {
        /// Key name (e.g., "api_token")
        #[arg(value_name = "KEY")]
        key_positional: Option<String>,

        #[arg(long = "key", hide = true)]
        key: Option<String>,
    },

    /// List all keys in vault
    #[command(alias = "ls", alias = "keys", alias = "all")]
    List,
}

#[derive(Subcommand)]
enum ProfileOps {
    /// Set your profile image
    #[command(alias = "image", alias = "avatar", alias = "picture")]
    SetImage {
        /// Image filename (e.g., "lyra3.png")
        #[arg(value_name = "FILENAME")]
        filename: String,
    },

    /// Set your display name
    #[command(alias = "name", alias = "rename")]
    SetName {
        /// Display name
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Show your current profile
    #[command(alias = "view", alias = "me", alias = "whoami")]
    Show,

    /// Clear profile (reset to defaults)
    #[command(alias = "reset", alias = "default")]
    Clear,
}

fn parse_tags(tags_str: Option<String>) -> Vec<String> {
    tags_str
        .map(|s| s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect())
        .unwrap_or_default()
}
/// Capture context automatically at note save time.
/// Passive enrichment - costs nothing cognitively.
/// Natural language format for human-readable episodic grounding.
/// Captures: files being worked on, teammates online (if any), current location.
fn capture_context() -> Option<String> {
    let my_ai_id = std::env::var("AI_ID").unwrap_or_default();

    // Skip context capture for human users — they don't need AI session metadata
    if my_ai_id.starts_with("human-") {
        return None;
    }

    let mut sentences = Vec::new();

    // Find teambook executable
    let teambook = find_teambook_exe();

    if let Some(ref exe) = teambook {
        // 1. AIs present (only if others are online - skip if solo)
        if let Ok(output) = std::process::Command::new(exe)
            .args(["status"])
            .output()
        {
            if output.status.success() {
                let status_out = String::from_utf8_lossy(&output.stdout);
                let mut teammates: Vec<String> = Vec::new();
                for line in status_out.lines() {
                    let line = line.trim();
                    // Parse lines like "[*] alpha-001:working on task"
                    if line.starts_with("[*]") || line.starts_with("[!]") ||
                       line.starts_with("[~]") || line.starts_with("[ ]") {
                        let rest = line[4..].trim();
                        if let Some(colon_pos) = rest.find(':') {
                            let ai_id = &rest[..colon_pos];
                            // Skip self
                            if ai_id != my_ai_id {
                                // Extract name and capitalize: "alpha-001" -> "Sage"
                                let name = ai_id.split('-').next().unwrap_or(ai_id);
                                let capitalized = name.chars().next()
                                    .map(|c| c.to_uppercase().collect::<String>() + &name[1..])
                                    .unwrap_or_else(|| name.to_string());
                                if !teammates.contains(&capitalized) {
                                    teammates.push(capitalized);
                                }
                            }
                        }
                    }
                }
                if !teammates.is_empty() {
                    let team_str = format_natural_list(&teammates);
                    sentences.push(format!("With {} online.", team_str));
                }
            }
        }

        // 2. Recent file actions (what was I working on?)
        if let Ok(output) = std::process::Command::new(exe)
            .args(["file-actions", "5"])
            .output()
        {
            if output.status.success() {
                let actions = String::from_utf8_lossy(&output.stdout);
                let mut seen = std::collections::HashSet::new();
                let files: Vec<String> = actions.lines()
                    .filter(|l| !l.is_empty() && !l.contains("No recent") && !l.starts_with("|FILE"))
                    .take(5)
                    .filter_map(|l| {
                        // Format: ai_id|path|action (3 parts)
                        let parts: Vec<&str> = l.split('|').collect();
                        if parts.len() >= 3 {
                            let path = parts[1];
                            let fname = std::path::Path::new(path)
                                .file_name()
                                .and_then(|f| f.to_str())
                                .unwrap_or(path);
                            if seen.insert(fname.to_string()) {
                                Some(fname.to_string())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .collect();
                if !files.is_empty() {
                    sentences.push(format!("Working on {}.", files.join(", ")));
                }
            }
        }
    }

    // 3. Current location (just the folder name)
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(folder) = cwd.file_name().and_then(|f| f.to_str()) {
            sentences.push(format!("In {}.", folder));
        }
    }

    if sentences.is_empty() {
        None
    } else {
        Some(format!("[{}]", sentences.join(" ")))
    }
}

/// Format a list with natural language: ["A", "B", "C"] -> "A, B, and C"
fn format_natural_list(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} and {}", items[0], items[1]),
        _ => {
            let (last, rest) = items.split_last().unwrap();
            format!("{}, and {}", rest.join(", "), last)
        }
    }
}

/// Find teambook executable
fn find_teambook_exe() -> Option<std::path::PathBuf> {
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let sibling = parent.join("teambook.exe");
            if sibling.exists() {
                return Some(sibling);
            }
        }
    }
    for candidate in ["./bin/teambook.exe", "bin/teambook.exe", "../bin/teambook.exe"] {
        let path = std::path::PathBuf::from(candidate);
        if path.exists() {
            return Some(path);
        }
    }
    None
}


/// Resolve the AI identity for notebook path resolution.
///
/// Priority:
///   1. `{CWD}/.claude/settings.json`  — reliable on all platforms, including
///      WSL where Windows .exe files may not inherit Linux env vars
///   2. `$AI_ID` env var               — works when explicitly set by a launcher
///   3. `"unknown"`                     — loud fallback so misconfiguration is visible
fn get_ai_id() -> String {
    // Check settings.json in the current working directory first
    if let Ok(cwd) = std::env::current_dir() {
        let settings = cwd.join(".claude").join("settings.json");
        if let Ok(content) = std::fs::read_to_string(&settings) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(id) = json.get("env")
                    .and_then(|e| e.get("AI_ID"))
                    .and_then(|v| v.as_str())
                {
                    return id.to_string();
                }
            }
        }
    }
    // Fall back to environment variable
    std::env::var("AI_ID").unwrap_or_else(|_| "unknown".to_string())
}

fn get_default_db_path() -> Result<PathBuf> {
    // CENTRALIZED: ~/.ai-foundation/agents/{ai_id}/notebook.engram
    // Memory belongs to the AI identity, not the terminal window.
    // Each AI has ONE notebook that follows them across instances.
    // Per-agent directory groups all agent data (notebook, tasks, config).
    let ai_id = get_ai_id();
    let home = dirs::home_dir()
        .ok_or_else(|| anyhow!("Could not determine home directory"))?;

    // NEW path: ~/.ai-foundation/agents/{ai_id}/notebook.engram
    let new_dir = home.join(".ai-foundation").join("agents").join(&ai_id);
    let new_path = new_dir.join("notebook.engram");

    // OLD path: ~/.ai-foundation/notebook/{ai_id}.engram
    let old_path = home.join(".ai-foundation").join("notebook").join(format!("{}.engram", ai_id));

    // Auto-migrate if old exists but new doesn't
    if old_path.exists() && !new_path.exists() {
        std::fs::create_dir_all(&new_dir)?;
        std::fs::rename(&old_path, &new_path)?;
        eprintln!("[MIGRATED] {} -> {}", old_path.display(), new_path.display());
    } else {
        std::fs::create_dir_all(&new_dir)?;
    }

    Ok(new_path)
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Open Engram database (1000x faster than SQLite)
    let db_path = get_default_db_path()?;

    // Ensure directory exists
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Bind AI_ID into the process env BEFORE opening. Engram's cipher is keyed on the
    // env var at open-time (storage.rs open_existing), so an ad-hoc CLI invocation that
    // inherited no AI_ID (e.g. from a fresh shell) would otherwise open under "default"
    // and fail to decrypt rows written under the real identity. get_ai_id() resolves from
    // {cwd}/.claude/settings.json first, then env — covers both hook and ad-hoc cases.
    let ai_id = get_ai_id();
    std::env::set_var("AI_ID", &ai_id);

    let mut db = Engram::open(&db_path)?;

    match cli.command {
        Commands::Remember { content, tags, pin, image, capture, screenshot, snap } => {
            // Consolidate capture aliases
            let capture_window = capture.or(screenshot).or(snap);
            let mut tags_vec = parse_tags(tags);

            // Handle screenshot capture if requested
            let final_image = if let Some(ref window) = capture_window {
                // Generate timestamp-based filename
                let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
                let images_dir = dirs::home_dir()
                    .map(|h| h.join(".ai-foundation").join("images"))
                    .unwrap_or_else(|| PathBuf::from(".ai-foundation/images"));

                std::fs::create_dir_all(&images_dir).ok();
                let img_path = images_dir.join(format!("capture_{}.png", timestamp));
                let img_path_str = img_path.to_string_lossy().to_string();

                // Screenshot capture not yet implemented
                eprintln!("Error: Screenshot capture is not available.");
                None
            } else {
                image.clone()
            };

            // Build content with image reference if present
            let final_content = if let Some(ref img_path) = final_image {
                if !tags_vec.contains(&"visual".to_string()) {
                    tags_vec.push("visual".to_string());
                }
                format!("{}\n[image: {}]", content, img_path)
            } else {
                content.clone()
            };

            // Capture context automatically (passive enrichment)
            let content_with_context = if let Some(ctx) = capture_context() {
                format!("{} {}", final_content, ctx)
            } else {
                final_content
            };

            // Remember the note
            let tag_refs: Vec<&str> = tags_vec.iter().map(|s| s.as_str()).collect();
            let note_id = db.remember(&content_with_context, &tag_refs)?;

            // Pin if requested
            if pin {
                db.pin(note_id)?;
            }

            // Persist indexes after write operation (critical for pinned/vault/graph persistence)
            db.persist_indexes()?;

            println!("Note saved: ID {}", note_id);
            if !tags_vec.is_empty() {
                println!("Tags: {}", tags_vec.join(", "));
            }
            if pin {
                println!("Pinned: yes");
            }
            if final_image.is_some() {
                println!("Image: attached");
            }
        }

        Commands::Work { content, tags, ttl_hours } => {
            let tags_vec = parse_tags(tags);
            let tag_refs: Vec<&str> = tags_vec.iter().map(|s| s.as_str()).collect();
            let note_id = db.remember_working(&content, &tag_refs, ttl_hours)?;
            db.persist_indexes()?;
            println!("Working note saved: ID {} (expires in {}h)", note_id, ttl_hours);
            if !tags_vec.is_empty() {
                println!("Tags: {}", tags_vec.join(", "));
            }
        }

        Commands::Recall { query, limit_positional, limit, pinned_only } => {
            let final_limit = limit_positional.unwrap_or(limit);

            if pinned_only {
                let pinned = db.pinned()?;
                if pinned.is_empty() {
                    println!("No pinned notes found");
                } else {
                    for note in pinned.iter().take(final_limit) {
                        print_note_row(note);
                    }
                }
            } else if let Some(q) = query {
                // Full hybrid recall: vector + keyword + graph + recency.
                // Loads Gemma embedding model for semantic search; degrades gracefully
                // to keyword+graph+recency if the model file is not present.
                use engram::embedding::{EmbeddingGenerator, EmbeddingConfig};

                let query_embedding = EmbeddingGenerator::find_model("embeddinggemma-300M-Q8_0.gguf")
                    .and_then(|model_path| {
                        let config = EmbeddingConfig::default().with_model(&model_path);
                        EmbeddingGenerator::load(config).ok()
                    })
                    .and_then(|mut gen| gen.embed(&q).ok());

                let results = if let Some(ref emb) = query_embedding {
                    db.recall(&q, Some(emb), final_limit)?
                } else {
                    db.recall_by_keyword(&q, final_limit)?
                };

                if results.is_empty() {
                    // Try tag search
                    let tag_results = db.by_tag(&q)?;
                    if tag_results.is_empty() {
                        println!("No notes found");
                        println!("Hint: Try broader search terms, or 'notebook-cli list' for recent notes");
                    } else {
                        for note in tag_results.iter().take(final_limit) {
                            print_note_row(note);
                        }
                    }
                } else {
                    // Surface similar note pairs as a non-blocking deduplication warning.
                    // Uses pre-computed semantic edges (EdgeType::Semantic, weight >= 0.75)
                    // from AutoLinkSemantic — no extra inference at recall time.
                    let result_ids: HashSet<u64> = results.iter().map(|r| r.note.id).collect();
                    let mut seen_pairs: HashSet<(u64, u64)> = HashSet::new();
                    let mut similar_pairs: Vec<(u64, u64, f32)> = Vec::new();
                    for r in &results {
                        for (neighbor_id, weight, edge_type) in db.get_related(r.note.id) {
                            if edge_type == EdgeType::Semantic
                                && weight >= 0.75
                                && result_ids.contains(&neighbor_id)
                            {
                                let pair = (r.note.id.min(neighbor_id), r.note.id.max(neighbor_id));
                                if seen_pairs.insert(pair) {
                                    similar_pairs.push((pair.0, pair.1, weight));
                                }
                            }
                        }
                    }
                    if !similar_pairs.is_empty() {
                        println!("\n⚠️  SIMILAR NOTES DETECTED");
                        for (a, b, score) in &similar_pairs {
                            println!(
                                "  Notes #{} and #{} are {:.0}% similar — consider: merge, delete, or add tags to distinguish",
                                a, b, score * 100.0
                            );
                        }
                        println!();
                    }
                    // ── BFS graph expansion (depth ≤ 2) ──────────────────────────────────
                    // Expand from each primary recall result up to 2 hops via graph edges.
                    // Surfaces related notes that keyword/semantic search alone would miss.
                    // Weight decays 0.5× per hop so 2-hop results don't drown direct matches.
                    let mut graph_neighbors: Vec<(u64, f32, &str)> = Vec::new();
                    let mut seen_graph_ids: HashSet<u64> = result_ids.clone();
                    // Queue: (note_id, cumulative_weight, hop_depth)
                    let mut bfs_queue: VecDeque<(u64, f32, u8)> =
                        results.iter().map(|r| (r.note.id, 1.0_f32, 0_u8)).collect();

                    while let Some((current_id, parent_weight, depth)) = bfs_queue.pop_front() {
                        for (neighbor_id, edge_weight, edge_type) in db.get_related(current_id) {
                            let include = match edge_type {
                                EdgeType::Semantic => edge_weight >= 0.6,
                                EdgeType::Manual => true,
                                EdgeType::Tag => true,
                                EdgeType::Temporal => false,
                            };
                            if include && seen_graph_ids.insert(neighbor_id) {
                                let type_str = match edge_type {
                                    EdgeType::Semantic => "semantic",
                                    EdgeType::Manual => "manual",
                                    EdgeType::Tag => "tag",
                                    EdgeType::Temporal => "temporal",
                                };
                                // 0.5× decay per hop beyond first
                                let hop_factor = if depth == 0 { 1.0_f32 } else { 0.5_f32 };
                                let score = edge_weight * parent_weight * hop_factor;
                                graph_neighbors.push((neighbor_id, score, type_str));
                                if depth + 1 < 2 {
                                    bfs_queue.push_back((neighbor_id, score, depth + 1));
                                }
                            }
                        }
                    }

                    // Sort by score descending, cap at 5
                    graph_neighbors.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                    graph_neighbors.truncate(5);

                    // Primary results first — rank 1 is the best direct match
                    for r in &results {
                        print_recall_result(r);
                    }

                    if !graph_neighbors.is_empty() {
                        println!("\n↳ Also related (via graph):");
                        for (neighbor_id, score, type_str) in &graph_neighbors {
                            if let Ok(Some(note)) = db.get(*neighbor_id) {
                                let content = note.content.replace('\n', " ");
                                if note.pinned {
                                    println!("{}|via-{}:{:.2}|{}|pinned", neighbor_id, type_str, score, content);
                                } else {
                                    println!("{}|via-{}:{:.2}|{}", neighbor_id, type_str, score, content);
                                }
                            }
                        }
                    }
                }
            } else {
                // No query - return recent
                let notes = db.recent(final_limit)?;
                if notes.is_empty() {
                    println!("No notes found");
                } else {
                    for note in &notes {
                        print_note_row(note);
                    }
                }
            }
        }

        Commands::List { limit_positional, limit, pinned_only, not_pinned, ids_only, tag } => {
            let final_limit = limit_positional.unwrap_or(limit);

            let notes = if let Some(ref t) = tag {
                db.by_tag(t)?
            } else if pinned_only {
                db.pinned()?
            } else {
                db.recent(final_limit * 2)? // Fetch more for filtering
            };

            let filtered: Vec<_> = notes.iter()
                .filter(|n| {
                    if pinned_only { n.pinned }
                    else if not_pinned { !n.pinned }
                    else { true }
                })
                .take(final_limit)
                .collect();

            if filtered.is_empty() {
                if !ids_only {
                    println!("No notes found");
                }
            } else {
                for note in &filtered {
                    if ids_only {
                        println!("{}", note.id);
                    } else {
                        print_note_row(note);
                    }
                }
            }
        }

        Commands::Tags { limit_positional, limit } => {
            let final_limit = limit_positional.unwrap_or(limit);
            let tags = db.all_tags();
            if tags.is_empty() {
                println!("No tags found");
            } else {
                println!("|TAGS|{}", tags.len().min(final_limit));
                for (tag, count) in tags.iter().take(final_limit) {
                    println!("{}|{}", tag, count);
                }
            }
        }

        Commands::Pinned { limit_positional, limit, ids_only } => {
            let final_limit = limit_positional.unwrap_or(limit);
            let notes = db.pinned()?;
            let limited: Vec<_> = notes.iter().take(final_limit).collect();

            if limited.is_empty() {
                if !ids_only {
                    println!("No pinned notes");
                }
            } else {
                if !ids_only {
                    println!("|PINNED|{}", limited.len());
                }
                for note in &limited {
                    if ids_only {
                        println!("{}", note.id);
                    } else {
                        print_note_row(note);
                    }
                }
            }
        }

        Commands::Get { note_id_positional, note_id } => {
            let final_note_id = note_id_positional.or(note_id);

            let Some(nid) = final_note_id else {
                eprintln!("Error: Note ID is required.");
                eprintln!("Hint: notebook-cli get 123");
                eprintln!("      notebook-cli list  # See recent notes");
                return Ok(());
            };

            match db.get(nid)? {
                Some(note) => print_note_full(&note),
                None => {
                    eprintln!("Error: Note #{} not found.", nid);
                    eprintln!("Hint: notebook-cli list  # See recent notes");
                }
            }
        }

        Commands::Pin { note_id_positional, note_id } => {
            let final_note_id = note_id_positional.or(note_id);

            let Some(nid) = final_note_id else {
                eprintln!("Error: Note ID is required.");
                eprintln!("Hint: notebook-cli pin 123");
                eprintln!("      notebook-cli list  # See recent notes");
                return Ok(());
            };

            db.pin(nid)?;
            db.persist_indexes()?;  // Persist pinned state
            println!("Note {} pinned", nid);
        }

        Commands::Unpin { note_id_positional, note_id } => {
            let final_note_id = note_id_positional.or(note_id);

            let Some(nid) = final_note_id else {
                eprintln!("Error: Note ID is required.");
                eprintln!("Hint: notebook-cli unpin 123");
                eprintln!("      notebook-cli recall --pinned-only  # See pinned notes");
                return Ok(());
            };

            db.unpin(nid)?;
            db.persist_indexes()?;  // Persist pinned state
            println!("Note {} unpinned", nid);
        }

        Commands::Update { note_id_positional, note_id, content, tags } => {
            let final_note_id = note_id_positional.or(note_id);

            let Some(nid) = final_note_id else {
                eprintln!("Error: Note ID is required.");
                eprintln!("Hint: notebook-cli update 123 --content \"new content\"");
                eprintln!("      notebook-cli update 123 --tags \"tag1,tag2\"");
                return Ok(());
            };

            if content.is_none() && tags.is_none() {
                eprintln!("Error: At least one of --content or --tags is required.");
                eprintln!("Hint: notebook-cli update {} --content \"new content\"", nid);
                return Ok(());
            }

            let tags_vec: Option<Vec<&str>> = tags.as_ref().map(|t| {
                t.split(',').map(|s| s.trim()).collect()
            });
            let tags_slice: Option<&[&str]> = tags_vec.as_ref().map(|v| v.as_slice());

            db.update(nid, content.as_deref(), tags_slice)?;
            println!("Note {} updated", nid);
        }

        Commands::AddTags { note_id_positional, note_id, tags_positional, tags } => {
            let final_note_id = note_id_positional.or(note_id);
            let final_tags = tags_positional.or(tags);

            let Some(nid) = final_note_id else {
                eprintln!("Error: Note ID is required.");
                eprintln!("Hint: notebook-cli add-tags 123 \"tag1,tag2\"");
                return Ok(());
            };

            let Some(new_tags_str) = final_tags else {
                eprintln!("Error: Tags are required.");
                eprintln!("Hint: notebook-cli add-tags {} \"tag1,tag2\"", nid);
                return Ok(());
            };

            // Get existing note to preserve content and existing tags
            let existing = db.get(nid)?;
            let Some(note) = existing else {
                eprintln!("Error: Note {} not found.", nid);
                return Ok(());
            };

            // Combine existing tags with new tags
            let mut all_tags: Vec<&str> = note.tags.iter().map(|s| s.as_str()).collect();
            let new_tags: Vec<&str> = new_tags_str.split(',').map(|s| s.trim()).collect();
            for tag in new_tags {
                if !all_tags.contains(&tag) {
                    all_tags.push(tag);
                }
            }

            db.update(nid, None, Some(&all_tags))?;
            println!("Added tags to note {}", nid);
        }

Commands::Delete { note_id_positional, note_id } => {
            let final_note_id = note_id_positional.or(note_id);

            let Some(nid) = final_note_id else {
                eprintln!("Error: Note ID is required.");
                eprintln!("Hint: notebook delete 123");
                return Ok(());
            };

            db.forget(nid)?;
            db.persist_indexes()?;
            println!("Note {} deleted", nid);
        }

        Commands::Vault { operation } => {
            match operation {
                VaultOps::Store { key_positional, value_positional, key, value } => {
                    let final_key = key_positional.or(key);
                    let final_value = value_positional.or(value);

                    let Some(k) = final_key else {
                        eprintln!("Error: Key is required.");
                        eprintln!("Hint: notebook-cli vault store api_token \"secret123\"");
                        return Ok(());
                    };

                    let Some(v) = final_value else {
                        eprintln!("Error: Value is required.");
                        eprintln!("Hint: notebook-cli vault store {} \"your_value\"", k);
                        return Ok(());
                    };

                    db.vault_set_string(&k, &v)?;
                    println!("Stored: {}", k);
                }

                VaultOps::Get { key_positional, key } => {
                    let final_key = key_positional.or(key);

                    let Some(k) = final_key else {
                        eprintln!("Error: Key is required.");
                        eprintln!("Hint: notebook-cli vault get api_token");
                        eprintln!("      notebook-cli vault list  # See all keys");
                        return Ok(());
                    };

                    match db.vault_get_string(&k)? {
                        Some(value) => println!("{}", value),
                        None => {
                            eprintln!("Error: Key \"{}\" not found.", k);
                            eprintln!("Hint: notebook-cli vault list  # See all keys");
                        }
                    }
                }

                VaultOps::List => {
                    let keys = db.vault_keys();
                    if keys.is_empty() {
                        println!("vault_keys|0");
                    } else {
                        println!("vault_keys|{}|{}", keys.len(), keys.join("|"));
                    }
                }
            }
        }

        Commands::Profile { operation } => {
            match operation {
                ProfileOps::SetImage { filename } => {
                    // Store in vault with profile: prefix
                    db.vault_set_string("profile:image", &filename)?;
                    println!("Profile image set: {}", filename);
                    println!("Hint: Image should be at AIsVisuals/Images/{}", filename);
                }

                ProfileOps::SetName { name } => {
                    db.vault_set_string("profile:name", &name)?;
                    println!("Display name set: {}", name);
                }

                ProfileOps::Show => {
                    let image = db.vault_get_string("profile:image")?.unwrap_or_else(|| "not set".to_string());
                    let name = db.vault_get_string("profile:name")?.unwrap_or_else(|| ai_id.clone());
                    
                    println!("|AI PROFILE|");
                    println!("Name:{}", name);
                    println!("Image:{}", image);
                    println!("AI_ID:{}", ai_id);
                    
                    // Show image path if set
                    if image != "not set" {
                        println!("Path:AIsVisuals/Images/{}", image);
                    }
                }

                ProfileOps::Clear => {
                    // Remove profile keys from vault
                    // Note: vault doesn't have delete, so we set to empty
                    db.vault_set_string("profile:image", "")?;
                    db.vault_set_string("profile:name", "")?;
                    println!("Profile cleared to defaults");
                }
            }
        }

        Commands::Stats => {
            let stats = db.stats();
            println!("|STATS|");
            println!("Notes:{} Pinned:{} Vault:{}", stats.active_notes, stats.pinned_count, stats.vault_entries);
            println!("AI:{}", ai_id);
        }
        Commands::StartSession => {
            println!("Session started: {}", ai_id);
        }


        // ===== EMBEDDING COMMANDS =====

        Commands::Embed { note_id_positional, note_id, model_path } => {
            let final_note_id = note_id_positional.or(note_id);

            let Some(nid) = final_note_id else {
                eprintln!("Error: Note ID is required.");
                eprintln!("Hint: notebook-cli embed 123");
                eprintln!("      notebook-cli list  # See recent notes");
                return Ok(());
            };

            // Check note exists
            let note = match db.get(nid)? {
                Some(n) => n,
                None => {
                    eprintln!("Error: Note #{} not found.", nid);
                    eprintln!("Hint: notebook-cli list  # See recent notes");
                    return Ok(());
                }
            };

            use engram::embedding::{EmbeddingGenerator, EmbeddingConfig};

            let model_file = model_path.unwrap_or_else(|| "embeddinggemma-300M-Q8_0.gguf".to_string());
            let model_path_resolved = EmbeddingGenerator::find_model(&model_file)
                .ok_or_else(|| anyhow!("Model not found: {}. Place GGUF file in bin/ directory.", model_file))?;

            let config = EmbeddingConfig::default().with_model(&model_path_resolved);
            let mut generator = EmbeddingGenerator::load(config)?;

            let start = std::time::Instant::now();
            let embedding = generator.embed(&note.content)?;
            let elapsed = start.elapsed();

            db.add_embedding(nid, &embedding)?;
            db.persist_indexes()?;

            println!("embedded|{}|{}d|{:.3}s", nid, embedding.len(), elapsed.as_secs_f64());
        }

        Commands::GenerateEmbeddings { limit_positional, limit, model_path, skip_existing } => {
            let final_limit = limit_positional.or(limit).unwrap_or(usize::MAX);

            use engram::embedding::{EmbeddingGenerator, EmbeddingConfig};

            let model_file = model_path.unwrap_or_else(|| "embeddinggemma-300M-Q8_0.gguf".to_string());
            let model_path_resolved = EmbeddingGenerator::find_model(&model_file)
                .ok_or_else(|| anyhow!("Model not found: {}. Place GGUF file in bin/ directory.", model_file))?;

            let config = EmbeddingConfig::default().with_model(&model_path_resolved);
            let mut generator = EmbeddingGenerator::load(config)?;

            let start = std::time::Instant::now();
            let notes = db.recent(final_limit)?;

            let mut processed = 0u64;
            let mut skipped = 0u64;
            let mut embedded = 0u64;
            let mut errors = 0u64;

            for note in &notes {
                processed += 1;

                // Skip if already has embedding
                if skip_existing && db.has_embedding(note.id) {
                    skipped += 1;
                    continue;
                }

                match generator.embed(&note.content) {
                    Ok(embedding) => {
                        if let Err(_) = db.add_embedding(note.id, &embedding) {
                            errors += 1;
                        } else {
                            embedded += 1;
                        }
                    }
                    Err(_) => {
                        errors += 1;
                    }
                }

                // Progress indicator every 10 notes
                if processed % 10 == 0 {
                    eprint!("\rProcessed: {} / {}", processed, notes.len());
                }
            }
            eprintln!(); // Clear progress line

            db.persist_indexes()?;
            let elapsed = start.elapsed();

            println!("backfill|processed:{}|skipped:{}|new:{}|errors:{}|time:{:.1}s",
                processed, skipped, embedded, errors, elapsed.as_secs_f64());
        }

        // ===== GRAPH MEMORY COMMANDS =====

        Commands::Link { from_id, to_id, relationship, weight } => {
            let edge_type = match relationship.as_str() {
                "semantic" => EdgeType::Semantic,
                "temporal" => EdgeType::Temporal,
                "tag" => EdgeType::Tag,
                _ => EdgeType::Manual,
            };

            db.add_edge(from_id, to_id, weight, edge_type);
            db.persist_indexes()?;  // Persist graph state
            println!("linked|{}|{}|{}|{:.2}", from_id, to_id, relationship, weight);
        }


        Commands::Unlink { from_id, to_id } => {
            let removed = db.remove_edge(from_id, to_id);
            if removed {
                db.persist_indexes()?;
                println!("unlinked|{}|{}", from_id, to_id);
            } else {
                println!("error|no_edge|{}|{}", from_id, to_id);
            }
        }

        Commands::InvalidateEdge { from, to } => {
            let removed = db.invalidate_edge(from, to);
            if removed {
                println!("Edge {} → {} invalidated", from, to);
            } else {
                println!("No edge found from {} to {}", from, to);
            }
        }

        Commands::GetLinked { note_id } => {
            let neighbors = db.get_related(note_id);

            if neighbors.is_empty() {
                println!("No notes linked to #{}", note_id);
            } else {
                println!("LINKED TO #{}", note_id);
                for (id, weight, edge_type) in neighbors {
                    if let Ok(Some(note)) = db.get(id) {
                        // NO TRUNCATION per QD policy
                        let preview = note.content.replace('\n', " ");
                        let type_str = match edge_type {
                            EdgeType::Semantic => "semantic",
                            EdgeType::Temporal => "temporal",
                            EdgeType::Tag => "tag",
                            EdgeType::Manual => "manual",
                        };
                        println!("{}|{}|{:.2}|{}", id, type_str, weight, preview);
                    }
                }
            }
        }

        Commands::AutoLinkTemporal { note_id, window_minutes } => {
            let links_created = db.auto_link_temporal(note_id, window_minutes)?;
            db.persist_indexes()?;  // Persist graph state
            println!("auto_link_temporal|{}|window:{}m|links:{}", note_id, window_minutes, links_created);
        }

        Commands::AutoLinkSemantic { note_id, top_k, min_similarity } => {
            let links_created = db.auto_link_semantic(note_id, min_similarity, top_k)?;
            db.persist_indexes()?;  // Persist graph state
            println!("auto_link_semantic|{}|top_k:{}|min_sim:{:.2}|links:{}", note_id, top_k, min_similarity, links_created);
        }

        Commands::RankNotes => {
            db.compute_pagerank();
            db.persist_indexes()?;  // Persist pagerank scores
            println!("notes_ranked|pagerank_computed");
        }

        Commands::PersistIndexes => {
            db.persist_indexes()?;
            println!("indexes_persisted");
        }

        Commands::Verify => {
            // Get stats as basic verification
            let stats = db.stats();
            println!("verify|ok|notes:{}|vectors:{}|edges:{}",
                stats.active_notes, stats.vector_count, stats.edge_count);
        }

        // ===== BATCH COMMANDS =====

        Commands::BatchDelete { note_ids, confirm } => {
            let ids: Vec<u64> = note_ids
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();

            if ids.is_empty() {
                eprintln!("Error: No valid note IDs provided");
                eprintln!("Example: notebook-cli batch-delete \"1,2,3,4\"");
            } else if !confirm {
                println!("About to delete {} notes: {:?}", ids.len(), ids);
                println!("Use -y flag to confirm deletion");
            } else {
                let mut deleted = 0;
                let mut failed = 0;
                for id in &ids {
                    match db.forget(*id) {
                        Ok(_) => deleted += 1,
                        Err(_) => failed += 1,
                    }
                }
                println!("batch_delete|deleted:{}|failed:{}", deleted, failed);
            }
        }

        Commands::BatchPin { note_ids } => {
            let ids: Vec<u64> = note_ids
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();

            if ids.is_empty() {
                eprintln!("Error: No valid note IDs provided");
            } else {
                let mut pinned = 0;
                let mut failed = 0;
                for id in &ids {
                    match db.pin(*id) {
                        Ok(_) => pinned += 1,
                        Err(_) => failed += 1,
                    }
                }
                db.persist_indexes()?;  // Persist pinned state
                println!("batch_pin|pinned:{}|failed:{}", pinned, failed);
            }
        }

        Commands::BatchUnpin { note_ids } => {
            let ids: Vec<u64> = note_ids
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();

            if ids.is_empty() {
                eprintln!("Error: No valid note IDs provided");
            } else {
                let mut unpinned = 0;
                let mut failed = 0;
                for id in &ids {
                    match db.unpin(*id) {
                        Ok(_) => unpinned += 1,
                        Err(_) => failed += 1,
                    }
                }
                db.persist_indexes()?;  // Persist pinned state
                println!("batch_unpin|unpinned:{}|failed:{}", unpinned, failed);
            }
        }

        Commands::Export { output, pinned_only, limit } => {
            let notes = if pinned_only {
                db.pinned()?
            } else {
                db.recent(limit)?
            };

            // Convert to JSON-serializable format
            let export_data: Vec<_> = notes.iter().map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "content": n.content,
                    "tags": n.tags,
                    "pinned": n.pinned,
                    "pagerank": n.pagerank
                })
            }).collect();

            let json = serde_json::to_string_pretty(&export_data)?;
            std::fs::write(&output, json)?;
            println!("exported|{}|{} notes", output, notes.len());
        }

        // ===== TEMPORAL & GRAPH QUERIES (Phase 2) =====

        Commands::Timeline { limit_positional, limit, oldest_first, hours_ago, days_ago } => {
            let final_limit = limit_positional.unwrap_or(limit);
            let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

            // Calculate time filter if specified
            let time_filter = if let Some(hours) = hours_ago {
                Some(now - (hours * 3600 * 1_000_000_000))
            } else if let Some(days) = days_ago {
                Some(now - (days * 86400 * 1_000_000_000))
            } else {
                None
            };

            // Get notes with optional time filter
            let notes = if let Some(cutoff) = time_filter {
                db.temporal_range(cutoff, now)?
            } else {
                db.recent(final_limit * 2)?
            };

            // Sort by timestamp
            let mut sorted_notes = notes;
            if oldest_first {
                sorted_notes.sort_by_key(|n| n.timestamp);
            } else {
                sorted_notes.sort_by_key(|n| std::cmp::Reverse(n.timestamp));
            }

            if sorted_notes.is_empty() {
                println!("No notes found in timeline");
            } else {
                println!("TIMELINE ({})", if oldest_first { "oldest first" } else { "newest first" });
                for note in sorted_notes.iter().take(final_limit) {
                    let time_str = format_timestamp(note.timestamp);
                    // NO TRUNCATION per QD policy
                    let content = note.content.replace('\n', " ");
                    println!("{}|{}|{}", note.id, time_str, content);
                }
            }
        }

        Commands::TimeRange { start, end, limit } => {
            let now = chrono::Utc::now();
            let now_nanos = now.timestamp_nanos_opt().unwrap_or(0);

            // Parse start time
            let start_nanos = parse_time_spec(&start, now_nanos)?;

            // Parse end time (default to now)
            let end_nanos = match end.as_deref() {
                Some("now") | None => now_nanos,
                Some(e) => parse_time_spec(e, now_nanos)?,
            };

            let notes = db.temporal_range(start_nanos, end_nanos)?;

            if notes.is_empty() {
                println!("No notes found in range");
            } else {
                println!("NOTES IN RANGE ({} found)", notes.len());
                for note in notes.iter().take(limit) {
                    let time_str = format_timestamp(note.timestamp);
                    // NO TRUNCATION per QD policy
                    let content = note.content.replace('\n', " ");
                    println!("{}|{}|{}", note.id, time_str, content);
                }
            }
        }

        Commands::Traverse { note_id, max_depth, edge_type, full_content } => {
            // Get the starting note
            let start_note = match db.get(note_id)? {
                Some(n) => n,
                None => {
                    eprintln!("Error: Note #{} not found", note_id);
                    return Ok(());
                }
            };

            println!("TRAVERSAL from #{} (depth {})", note_id, max_depth);
            println!("Start: {}", truncate_content(&start_note.content, 60));

            // BFS traversal through the graph
            let mut visited = std::collections::HashSet::new();
            let mut queue = std::collections::VecDeque::new();
            visited.insert(note_id);
            queue.push_back((note_id, 0));

            let edge_filter = edge_type.as_str();

            while let Some((current_id, depth)) = queue.pop_front() {
                if depth >= max_depth {
                    continue;
                }

                let neighbors = db.get_related(current_id);
                for (neighbor_id, weight, etype) in neighbors {
                    // Apply edge type filter
                    let type_str = match etype {
                        EdgeType::Semantic => "semantic",
                        EdgeType::Temporal => "temporal",
                        EdgeType::Tag => "tag",
                        EdgeType::Manual => "manual",
                    };

                    if edge_filter != "all" && type_str != edge_filter {
                        continue;
                    }

                    if !visited.contains(&neighbor_id) {
                        visited.insert(neighbor_id);
                        queue.push_back((neighbor_id, depth + 1));

                        if let Ok(Some(note)) = db.get(neighbor_id) {
                            let indent = "  ".repeat(depth + 1);
                            let content = if full_content {
                                note.content.replace('\n', " ")
                            } else {
                                truncate_content(&note.content, 60)
                            };
                            println!("{}→ {}|{}|{:.2}|{}", indent, neighbor_id, type_str, weight, content);
                        }
                    }
                }
            }

            println!("Visited {} notes", visited.len());
        }

        Commands::Path { from_id, to_id, max_depth } => {
            // Verify both notes exist
            if db.get(from_id)?.is_none() {
                eprintln!("Error: Source note #{} not found", from_id);
                return Ok(());
            }
            if db.get(to_id)?.is_none() {
                eprintln!("Error: Target note #{} not found", to_id);
                return Ok(());
            }

            // BFS to find shortest path
            let mut visited = std::collections::HashMap::new();
            let mut queue = std::collections::VecDeque::new();
            visited.insert(from_id, (0u64, EdgeType::Manual)); // parent, edge_type
            queue.push_back((from_id, 0));

            let mut found = false;

            while let Some((current_id, depth)) = queue.pop_front() {
                if current_id == to_id {
                    found = true;
                    break;
                }

                if depth >= max_depth {
                    continue;
                }

                let neighbors = db.get_related(current_id);
                for (neighbor_id, _, etype) in neighbors {
                    if !visited.contains_key(&neighbor_id) {
                        visited.insert(neighbor_id, (current_id, etype));
                        queue.push_back((neighbor_id, depth + 1));
                    }
                }
            }

            if !found {
                println!("No path found from #{} to #{} within {} hops", from_id, to_id, max_depth);
            } else {
                // Reconstruct path
                let mut path = vec![to_id];
                let mut current = to_id;
                while current != from_id {
                    if let Some(&(parent, _)) = visited.get(&current) {
                        if parent == 0 && current != from_id {
                            break; // Safety check
                        }
                        path.push(parent);
                        current = parent;
                    } else {
                        break;
                    }
                }
                path.reverse();

                println!("PATH from #{} to #{} ({} hops)", from_id, to_id, path.len() - 1);
                for (i, &nid) in path.iter().enumerate() {
                    if let Ok(Some(note)) = db.get(nid) {
                        let prefix = if i == 0 { "START" } else if i == path.len() - 1 { "END" } else { "→" };
                        let content = truncate_content(&note.content, 60);
                        println!("  {} {}|{}", prefix, nid, content);
                    }
                }
            }
        }

        Commands::Related { note_id, edge_type, incoming: _ } => {
            // Verify note exists
            if db.get(note_id)?.is_none() {
                eprintln!("Error: Note #{} not found", note_id);
                return Ok(());
            }

            let neighbors = db.get_related(note_id);
            let edge_filter = edge_type.as_str();

            // Filter by edge type
            let filtered: Vec<_> = neighbors.iter()
                .filter(|(_, _, etype)| {
                    if edge_filter == "all" {
                        return true;
                    }
                    let type_str = match etype {
                        EdgeType::Semantic => "semantic",
                        EdgeType::Temporal => "temporal",
                        EdgeType::Tag => "tag",
                        EdgeType::Manual => "manual",
                    };
                    type_str == edge_filter
                })
                .collect();

            if filtered.is_empty() {
                println!("No related notes found for #{}", note_id);
            } else {
                println!("RELATED TO #{} ({} connections)", note_id, filtered.len());
                for (neighbor_id, weight, etype) in &filtered {
                    let type_str = match etype {
                        EdgeType::Semantic => "semantic",
                        EdgeType::Temporal => "temporal",
                        EdgeType::Tag => "tag",
                        EdgeType::Manual => "manual",
                    };
                    if let Ok(Some(note)) = db.get(*neighbor_id) {
                        let content = truncate_content(&note.content, 60);
                        println!("{}|{}|{:.2}|{}", neighbor_id, type_str, weight, content);
                    }
                }
            }
        }

        // ===== COGNITIVE MEMORY TYPES (Phase 3) =====

        Commands::Classify { note_id, limit } => {
            use engram::cognitive::{classify_content, MemoryType};

            match note_id.as_deref() {
                Some("all") | None => {
                    // Classify recent notes
                    let notes = db.recent(limit)?;
                    if notes.is_empty() {
                        println!("No notes to classify");
                    } else {
                        println!("MEMORY TYPE CLASSIFICATION");
                        for note in &notes {
                            let mem_type = classify_content(&note.content);
                            let content = truncate_content(&note.content, 50);
                            println!("{}|{}|{}", note.id, mem_type.name(), content);
                        }
                    }
                }
                Some(id_str) => {
                    // Classify specific note
                    let nid: u64 = id_str.parse().map_err(|_| anyhow!("Invalid note ID: {}", id_str))?;
                    let note = match db.get(nid)? {
                        Some(n) => n,
                        None => {
                            eprintln!("Error: Note #{} not found", nid);
                            return Ok(());
                        }
                    };

                    let mem_type = classify_content(&note.content);
                    println!("Note #{}: {}", nid, mem_type.name());
                    println!();
                    println!("Classification: {}", match mem_type {
                        MemoryType::Semantic => "SEMANTIC - Facts, concepts, definitions",
                        MemoryType::Episodic => "EPISODIC - Events, experiences, specific instances",
                        MemoryType::Procedural => "PROCEDURAL - How-to instructions, processes",
                    });
                    println!();
                    println!("Content preview: {}", truncate_content(&note.content, 100));
                }
            }
        }

        Commands::ByMemoryType { memory_type, limit_positional, limit } => {
            use engram::cognitive::{classify_content, MemoryType};

            let final_limit = limit_positional.unwrap_or(limit);
            let target_type = match MemoryType::from_str(&memory_type) {
                Some(t) => t,
                None => {
                    eprintln!("Error: Unknown memory type '{}'. Use: semantic, episodic, procedural", memory_type);
                    return Ok(());
                }
            };

            // Get recent notes and filter by memory type
            let notes = db.recent(final_limit * 5)?; // Fetch more to filter
            let matching: Vec<_> = notes.iter()
                .filter(|n| classify_content(&n.content) == target_type)
                .take(final_limit)
                .collect();

            if matching.is_empty() {
                println!("No {} notes found", target_type.name());
            } else {
                println!("{} NOTES ({} found)", target_type.name().to_uppercase(), matching.len());
                for note in &matching {
                    let content = truncate_content(&note.content, 70);
                    println!("{}|{}", note.id, content);
                }
            }
        }

        Commands::MemoryStats { limit } => {
            use engram::cognitive::{classify_content, MemoryTypeStats};

            let notes = db.recent(limit)?;
            let mut stats = MemoryTypeStats::default();

            for note in &notes {
                let mem_type = classify_content(&note.content);
                stats.increment(mem_type);
            }

            println!("MEMORY TYPE DISTRIBUTION ({} notes analyzed)", stats.total());
            println!("  Semantic:   {:4} ({:.1}%) - Facts, concepts, definitions",
                stats.semantic_count, stats.percentage(engram::cognitive::MemoryType::Semantic));
            println!("  Episodic:   {:4} ({:.1}%) - Events, experiences, specific instances",
                stats.episodic_count, stats.percentage(engram::cognitive::MemoryType::Episodic));
            println!("  Procedural: {:4} ({:.1}%) - How-to instructions, processes",
                stats.procedural_count, stats.percentage(engram::cognitive::MemoryType::Procedural));
        }

        // ===== KNOWLEDGE GRAPH 2.0 FEATURES (Phase 4) =====

        Commands::GraphStats => {
            let stats = db.stats();

            println!("|GRAPH STATS|");
            println!("Notes:{} Active:{} Tombstoned:{}",
                stats.note_count, stats.active_notes, stats.tombstone_count);
            println!("Pinned:{}", stats.pinned_count);
            let embed_pct = if stats.active_notes > 0 { stats.vector_count as f64 / stats.active_notes as f64 * 100.0 } else { 0.0 };
            println!("Vectors:{} Coverage:{:.1}%", stats.vector_count, embed_pct);
            println!("Edges:{}", stats.edge_count);
            println!("Tags:{}", stats.tag_count);
            println!("Vault:{}", stats.vault_entries);
            println!("Size:{}bytes {:.2}MB", stats.file_size, stats.file_size as f64 / (1024.0 * 1024.0));

            // Cache stats
            let (hits, misses, rate) = db.cache_stats();
            if hits + misses > 0 {
                println!("Cache:{}hits {}misses {:.1}%rate", hits, misses, rate * 100.0);
            }

            // Graph density
            if stats.active_notes > 1 {
                let max_edges = stats.active_notes * (stats.active_notes - 1);
                let density = stats.edge_count as f64 / max_edges as f64 * 100.0;
                println!("Density:{:.4}%", density);
            }
        }

        Commands::Explain { from_id, to_id, max_depth } => {
            // Verify both notes exist
            let from_note = match db.get(from_id)? {
                Some(n) => n,
                None => {
                    eprintln!("Error: Note #{} not found", from_id);
                    return Ok(());
                }
            };
            let to_note = match db.get(to_id)? {
                Some(n) => n,
                None => {
                    eprintln!("Error: Note #{} not found", to_id);
                    return Ok(());
                }
            };

            // BFS to find path with edge info
            let mut visited = std::collections::HashMap::new();
            let mut queue = std::collections::VecDeque::new();
            visited.insert(from_id, (0u64, EdgeType::Manual, 0.0f32));
            queue.push_back((from_id, 0));

            let mut found = false;

            while let Some((current_id, depth)) = queue.pop_front() {
                if current_id == to_id {
                    found = true;
                    break;
                }

                if depth >= max_depth {
                    continue;
                }

                let neighbors = db.get_related(current_id);
                for (neighbor_id, weight, etype) in neighbors {
                    if !visited.contains_key(&neighbor_id) {
                        visited.insert(neighbor_id, (current_id, etype, weight));
                        queue.push_back((neighbor_id, depth + 1));
                    }
                }
            }

            if !found {
                println!("NO CONNECTION found between #{} and #{}", from_id, to_id);
                println!();
                println!("From: {}", truncate_content(&from_note.content, 60));
                println!("To:   {}", truncate_content(&to_note.content, 60));
                println!();
                println!("The notes are not connected within {} hops.", max_depth);
                println!("Try: auto-link-semantic {} or auto-link-temporal {}", from_id, from_id);
            } else {
                // Reconstruct path with edge types
                let mut path = vec![(to_id, EdgeType::Manual, 0.0f32)];
                let mut current = to_id;
                while current != from_id {
                    if let Some(&(parent, etype, weight)) = visited.get(&current) {
                        if parent == 0 && current != from_id {
                            break;
                        }
                        path.push((parent, etype, weight));
                        current = parent;
                    } else {
                        break;
                    }
                }
                path.reverse();

                println!("|CONNECTION|{}|{}", from_id, to_id);

                for (i, &(nid, etype, weight)) in path.iter().enumerate() {
                    if let Ok(Some(note)) = db.get(nid) {
                        let content = truncate_content(&note.content, 50);
                        let type_str = match etype {
                            EdgeType::Semantic => "semantic",
                            EdgeType::Temporal => "temporal",
                            EdgeType::Tag => "tag",
                            EdgeType::Manual => "manual",
                        };
                        if i == 0 {
                            println!("  start|#{}|{}", nid, content);
                        } else if i == path.len() - 1 {
                            println!("  {}|{:.2}|#{}|{}", type_str, weight, nid, content);
                            println!("  end|#{}|{}", nid, content);
                        } else {
                            println!("  {}|{:.2}|#{}|{}", type_str, weight, nid, content);
                        }
                    }
                }

                println!();
                println!("Hops:{}", path.len() - 1);
            }
        }

        Commands::HealthCheck { auto_fix } => {
            println!("|HEALTH CHECK|");

            let mut issues = Vec::new();
            let mut warnings = Vec::new();
            let stats = db.stats();

            // Check 1: Verify integrity
            let verify_result = db.verify()?;
            if !verify_result.is_valid {
                for err in &verify_result.errors {
                    issues.push(format!("INTEGRITY:{}", err));
                }
            }
            for warn in &verify_result.warnings {
                warnings.push(warn.clone());
            }

            // Check 2: Embedding coverage
            let embedding_coverage = if stats.active_notes > 0 {
                stats.vector_count as f64 / stats.active_notes as f64 * 100.0
            } else {
                100.0
            };
            if embedding_coverage < 50.0 {
                warnings.push(format!("LOW_EMBEDDING_COVERAGE:{:.1}%", embedding_coverage));
            }

            // Check 3: Graph connectivity
            if stats.edge_count == 0 && stats.active_notes > 5 {
                warnings.push("NO_GRAPH_EDGES".to_string());
            }

            // Check 4: Pinned notes ratio
            let pinned_ratio = if stats.active_notes > 0 {
                stats.pinned_count as f64 / stats.active_notes as f64 * 100.0
            } else {
                0.0
            };
            if pinned_ratio > 30.0 {
                warnings.push(format!("HIGH_PIN_RATIO:{:.1}%", pinned_ratio));
            }

            // Print results
            if issues.is_empty() && warnings.is_empty() {
                println!("Status:healthy");
            } else {
                if !issues.is_empty() {
                    println!("|ISSUES|{}", issues.len());
                    for issue in &issues {
                        println!("  {}", issue);
                    }
                }
                if !warnings.is_empty() {
                    println!("|WARNINGS|{}", warnings.len());
                    for warning in &warnings {
                        println!("  {}", warning);
                    }
                }
            }

            // Auto-fix if requested
            if auto_fix && !warnings.is_empty() {
                println!();
                println!("|AUTO-FIX|");
                if embedding_coverage < 50.0 {
                    println!("  run:backfill");
                }
                if stats.edge_count == 0 {
                    println!("  run:compute-pagerank");
                }
            }

            println!();
            println!("Summary:{}notes {}vectors {}edges {:.2}MB",
                stats.active_notes, stats.vector_count, stats.edge_count,
                stats.file_size as f64 / (1024.0 * 1024.0));
        }

        Commands::TopNotes { limit_positional, limit } => {
            let final_limit = limit_positional.unwrap_or(limit);

            // Get recent notes and sort by pagerank
            let notes = db.recent(final_limit * 5)?;
            let mut with_pagerank: Vec<_> = notes.iter()
                .map(|n| (n, db.get_pagerank(n.id)))
                .collect();

            with_pagerank.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            if with_pagerank.is_empty() {
                println!("No notes found");
            } else {
                println!("TOP NOTES BY PAGERANK");
                for (i, (note, rank)) in with_pagerank.iter().take(final_limit).enumerate() {
                    let content = truncate_content(&note.content, 60);
                    let pin_marker = if note.pinned { "📌" } else { "  " };
                    println!("{:2}. {} #{} (rank: {:.4}) {}", i + 1, pin_marker, note.id, rank, content);
                }
            }
        }
    }

    Ok(())
}

/// Format timestamp as human-readable relative time
fn format_timestamp(timestamp_nanos: i64) -> String {
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let diff_secs = (now - timestamp_nanos) / 1_000_000_000;

    if diff_secs < 60 {
        format!("{}s ago", diff_secs)
    } else if diff_secs < 3600 {
        format!("{}m ago", diff_secs / 60)
    } else if diff_secs < 86400 {
        format!("{}h ago", diff_secs / 3600)
    } else {
        format!("{}d ago", diff_secs / 86400)
    }
}

/// Parse time specification (relative like "7d" or absolute YYYY-MM-DD)
fn parse_time_spec(spec: &str, now_nanos: i64) -> anyhow::Result<i64> {
    // Handle relative time specs like "7d", "24h", "30m"
    if spec.ends_with('d') {
        let days: i64 = spec[..spec.len()-1].parse()?;
        return Ok(now_nanos - (days * 86400 * 1_000_000_000));
    }
    if spec.ends_with('h') {
        let hours: i64 = spec[..spec.len()-1].parse()?;
        return Ok(now_nanos - (hours * 3600 * 1_000_000_000));
    }
    if spec.ends_with('m') {
        let minutes: i64 = spec[..spec.len()-1].parse()?;
        return Ok(now_nanos - (minutes * 60 * 1_000_000_000));
    }

    // Try to parse as YYYY-MM-DD
    if let Ok(date) = chrono::NaiveDate::parse_from_str(spec, "%Y-%m-%d") {
        let datetime = date.and_hms_opt(0, 0, 0).unwrap();
        return Ok(datetime.and_utc().timestamp_nanos_opt().unwrap_or(0));
    }

    Err(anyhow!("Invalid time specification: {}. Use 7d, 24h, 30m, or YYYY-MM-DD", spec))
}

/// Clean content for display (NO TRUNCATION per QD policy)
/// QD explicitly stated truncation degrades tool functionality from ~90% to ~20%
fn truncate_content(content: &str, _max_len: usize) -> String {
    // Ignore max_len - NO TRUNCATION
    content.replace('\n', " ")
}

fn print_note_row(note: &engram::Note) {
    // NO TRUNCATION - full content preserves context
    let content = note.content.replace('\n', " ");
    if note.pinned {
        println!("{}|{}|pinned", note.id, content);
    } else {
        println!("{}|{}", note.id, content);
    }
}

fn print_recall_result(r: &engram::recall::RecallResult) {
    // NO TRUNCATION - full content preserves context and AI collaboration effectiveness
    // QD explicitly stated truncation degrades tool functionality from ~90% to ~20%
    let content = r.note.content.replace('\n', " ");
    // Score transparency: show per-signal breakdown so AIs understand WHY a result ranked
    // Format: ID|FINAL [vector:V keyword:K graph:G recency:R]|content
    let scores = format!(
        "{:.2} [vector:{:.2} keyword:{:.2} graph:{:.2} recency:{:.2}]",
        r.final_score, r.vector_score, r.keyword_score, r.graph_score, r.recency_score
    );
    if r.note.pinned {
        println!("{}|{}|{}|pinned", r.note.id, scores, content);
    } else {
        println!("{}|{}|{}", r.note.id, scores, content);
    }
}

fn print_note_full(note: &engram::Note) {
    println!("Note ID: {}", note.id);
    if note.pinned {
        println!("Pinned: yes");
    }
    if !note.tags.is_empty() {
        println!("Tags: {}", note.tags.join(", "));
    }
    println!("PageRank: {:.4}", note.pagerank);
    println!("\n{}", note.content);
}
