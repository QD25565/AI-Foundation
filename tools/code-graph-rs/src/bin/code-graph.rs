//! Code Graph CLI - Multi-language code analysis using tree-sitter
//!
//! Provides call graph analysis, impact analysis, and code navigation
//! for Python, JavaScript, TypeScript, Rust, Go, Java, C, and C++.
//!
//! Usage:
//!   code-graph index ./my-project
//!   code-graph callers "function_name"
//!   code-graph callees "function_name"
//!   code-graph impact ./src/file.py
//!   code-graph stats

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// ============================================================================
// DATA STRUCTURES
// ============================================================================

#[derive(Debug, Clone)]
struct Symbol {
    name: String,
    kind: SymbolKind,
    file_path: String,
    line: usize,
    column: usize,
    end_line: usize,
    parent: Option<String>,  // For methods: class name
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
enum SymbolKind {
    Function,
    Method,
    Class,
    Module,
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SymbolKind::Function => write!(f, "function"),
            SymbolKind::Method => write!(f, "method"),
            SymbolKind::Class => write!(f, "class"),
            SymbolKind::Module => write!(f, "module"),
        }
    }
}

#[derive(Debug, Clone)]
struct Call {
    caller: String,
    callee: String,
    file_path: String,
    line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Language {
    Python,
    JavaScript,
    TypeScript,
    Rust,
    Go,
    Java,
    C,
    Cpp,
    Unknown,
}

impl Language {
    fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "py" => Language::Python,
            "js" | "mjs" | "cjs" => Language::JavaScript,
            "ts" | "tsx" => Language::TypeScript,
            "rs" => Language::Rust,
            "go" => Language::Go,
            "java" => Language::Java,
            "c" | "h" => Language::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" => Language::Cpp,
            _ => Language::Unknown,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Language::Python => "Python",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Rust => "Rust",
            Language::Go => "Go",
            Language::Java => "Java",
            Language::C => "C",
            Language::Cpp => "C++",
            Language::Unknown => "Unknown",
        }
    }
}

// ============================================================================
// CLI
// ============================================================================

#[derive(Parser)]
#[command(name = "code-graph")]
#[command(about = "Multi-language code graph analysis", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Index a directory (builds the code graph)
    Index {
        /// Directory to index
        path: PathBuf,

        /// Languages to index (comma-separated, e.g., "python,rust,java")
        #[arg(long)]
        languages: Option<String>,

        /// Exclude patterns (comma-separated)
        #[arg(long, default_value = "node_modules,target,.git,__pycache__,venv")]
        exclude: String,
    },

    /// Find all callers of a function/method
    Callers {
        /// Function or method name
        name: String,

        /// Limit results
        #[arg(long, default_value = "20")]
        limit: i64,
    },

    /// Find all functions called by a function/method
    Callees {
        /// Function or method name
        name: String,

        /// Limit results
        #[arg(long, default_value = "20")]
        limit: i64,
    },

    /// Impact analysis: what might break if this file changes
    Impact {
        /// File path to analyze
        path: PathBuf,

        /// Depth of analysis (how many levels of callers)
        #[arg(long, default_value = "3")]
        depth: usize,
    },

    /// Find symbols by name pattern
    Find {
        /// Name pattern (supports wildcards: * and ?)
        pattern: String,

        /// Limit results
        #[arg(long, default_value = "20")]
        limit: i64,
    },

    /// Show code graph statistics
    Stats,

    /// Clear the index
    Clear,
}

// ============================================================================
// DATABASE
// ============================================================================

fn get_db_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join(".ai-foundation");
    std::fs::create_dir_all(&dir).ok();
    dir.join("code_graph.db")
}

fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS symbols (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            kind TEXT NOT NULL,
            file_path TEXT NOT NULL,
            line INTEGER NOT NULL,
            column INTEGER NOT NULL,
            end_line INTEGER NOT NULL,
            parent TEXT,
            language TEXT NOT NULL,
            indexed_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS calls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            caller TEXT NOT NULL,
            callee TEXT NOT NULL,
            file_path TEXT NOT NULL,
            line INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS indexed_files (
            file_path TEXT PRIMARY KEY,
            language TEXT NOT NULL,
            indexed_at TEXT NOT NULL,
            file_hash TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
        CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_path);
        CREATE INDEX IF NOT EXISTS idx_calls_caller ON calls(caller);
        CREATE INDEX IF NOT EXISTS idx_calls_callee ON calls(callee);
        "#,
    )?;
    Ok(())
}

fn open_db() -> Result<Connection> {
    let conn = Connection::open(get_db_path())?;
    init_db(&conn)?;
    Ok(conn)
}

// ============================================================================
// PARSING
// ============================================================================

fn get_parser(lang: Language) -> Option<tree_sitter::Parser> {
    let mut parser = tree_sitter::Parser::new();

    let language = match lang {
        Language::Python => tree_sitter_python::LANGUAGE,
        Language::JavaScript => tree_sitter_javascript::LANGUAGE,
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        Language::Rust => tree_sitter_rust::LANGUAGE,
        Language::Go => tree_sitter_go::LANGUAGE,
        Language::Java => tree_sitter_java::LANGUAGE,
        Language::C => tree_sitter_c::LANGUAGE,
        Language::Cpp => tree_sitter_cpp::LANGUAGE,
        Language::Unknown => return None,
    };

    parser.set_language(&language.into()).ok()?;
    Some(parser)
}

fn extract_symbols_and_calls(
    tree: &tree_sitter::Tree,
    source: &str,
    file_path: &str,
    lang: Language,
) -> (Vec<Symbol>, Vec<Call>) {
    let mut symbols = Vec::new();
    let mut calls = Vec::new();
    let mut cursor = tree.walk();

    // Query patterns differ by language
    extract_recursive(&mut cursor, source, file_path, lang, &mut symbols, &mut calls, None);

    (symbols, calls)
}

fn extract_recursive(
    cursor: &mut tree_sitter::TreeCursor,
    source: &str,
    file_path: &str,
    lang: Language,
    symbols: &mut Vec<Symbol>,
    calls: &mut Vec<Call>,
    current_function: Option<&str>,
) {
    let node = cursor.node();
    let kind = node.kind();

    // Extract function/method definitions based on language
    let is_function_def = match lang {
        Language::Python => kind == "function_definition",
        Language::JavaScript | Language::TypeScript => {
            kind == "function_declaration" || kind == "method_definition" || kind == "arrow_function"
        }
        Language::Rust => kind == "function_item",
        Language::Go => kind == "function_declaration" || kind == "method_declaration",
        Language::Java => kind == "method_declaration",
        Language::C | Language::Cpp => kind == "function_definition",
        Language::Unknown => false,
    };

    let is_class_def = match lang {
        Language::Python => kind == "class_definition",
        Language::JavaScript | Language::TypeScript => kind == "class_declaration",
        Language::Rust => kind == "impl_item" || kind == "struct_item",
        Language::Go => kind == "type_declaration",
        Language::Java => kind == "class_declaration",
        Language::Cpp => kind == "class_specifier",
        _ => false,
    };

    let is_call = match lang {
        Language::Python => kind == "call",
        Language::JavaScript | Language::TypeScript => kind == "call_expression",
        Language::Rust => kind == "call_expression",
        Language::Go => kind == "call_expression",
        Language::Java => kind == "method_invocation",
        Language::C | Language::Cpp => kind == "call_expression",
        Language::Unknown => false,
    };

    // Extract symbol name
    if is_function_def || is_class_def {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = &source[name_node.byte_range()];
            let symbol_kind = if is_class_def {
                SymbolKind::Class
            } else {
                SymbolKind::Function
            };

            symbols.push(Symbol {
                name: name.to_string(),
                kind: symbol_kind,
                file_path: file_path.to_string(),
                line: node.start_position().row + 1,
                column: node.start_position().column,
                end_line: node.end_position().row + 1,
                parent: None,
            });

            // Recurse with this function as context
            if cursor.goto_first_child() {
                loop {
                    extract_recursive(
                        cursor,
                        source,
                        file_path,
                        lang,
                        symbols,
                        calls,
                        Some(name),
                    );
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
                cursor.goto_parent();
            }
            return;
        }
    }

    // Extract function calls
    if is_call {
        if let Some(callee_name) = extract_call_name(&node, source, lang) {
            if let Some(caller) = current_function {
                calls.push(Call {
                    caller: caller.to_string(),
                    callee: callee_name,
                    file_path: file_path.to_string(),
                    line: node.start_position().row + 1,
                });
            }
        }
    }

    // Recurse into children
    if cursor.goto_first_child() {
        loop {
            extract_recursive(cursor, source, file_path, lang, symbols, calls, current_function);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

fn extract_call_name(node: &tree_sitter::Node, source: &str, lang: Language) -> Option<String> {
    // Get the function/method being called
    let func_node = match lang {
        Language::Python => node.child_by_field_name("function"),
        Language::JavaScript | Language::TypeScript => node.child_by_field_name("function"),
        Language::Rust => node.child_by_field_name("function"),
        Language::Go => node.child_by_field_name("function"),
        Language::Java => node.child_by_field_name("name"),
        Language::C | Language::Cpp => node.child_by_field_name("function"),
        Language::Unknown => None,
    }?;

    // Handle different call patterns (simple name, method call, etc.)
    let text = &source[func_node.byte_range()];

    // For method calls like obj.method(), extract just "method"
    if let Some(pos) = text.rfind('.') {
        Some(text[pos + 1..].to_string())
    } else {
        Some(text.to_string())
    }
}

// ============================================================================
// INDEXING
// ============================================================================

fn index_file(conn: &Connection, path: &Path, lang: Language) -> Result<(usize, usize)> {
    let source = fs::read_to_string(path)?;
    let file_path = path.to_string_lossy().replace('\\', "/");

    let mut parser = get_parser(lang).context("Failed to create parser")?;
    let tree = parser.parse(&source, None).context("Failed to parse file")?;

    let (symbols, calls) = extract_symbols_and_calls(&tree, &source, &file_path, lang);

    // Clear existing data for this file
    conn.execute("DELETE FROM symbols WHERE file_path = ?", params![&file_path])?;
    conn.execute("DELETE FROM calls WHERE file_path = ?", params![&file_path])?;

    let now = Utc::now().to_rfc3339();

    // Insert symbols
    for symbol in &symbols {
        conn.execute(
            "INSERT INTO symbols (name, kind, file_path, line, column, end_line, parent, language, indexed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                symbol.name,
                symbol.kind.to_string(),
                symbol.file_path,
                symbol.line,
                symbol.column,
                symbol.end_line,
                symbol.parent,
                lang.name(),
                now
            ],
        )?;
    }

    // Insert calls
    for call in &calls {
        conn.execute(
            "INSERT INTO calls (caller, callee, file_path, line) VALUES (?, ?, ?, ?)",
            params![call.caller, call.callee, call.file_path, call.line],
        )?;
    }

    // Update indexed files
    conn.execute(
        "INSERT OR REPLACE INTO indexed_files (file_path, language, indexed_at) VALUES (?, ?, ?)",
        params![file_path, lang.name(), now],
    )?;

    Ok((symbols.len(), calls.len()))
}

// ============================================================================
fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        return path.to_string();
    }
    let parts: Vec<&str> = path.split(['/', '\\']).collect();
    if parts.len() <= 2 {
        return format!("...{}", &path[path.len().saturating_sub(max_len - 3)..]);
    }
    format!(".../{}", parts[parts.len() - 2..].join("/"))
}

// ============================================================================
// COMMANDS
// ============================================================================

fn cmd_index(path: &Path, languages: Option<&str>, exclude: &str) -> Result<()> {
    let conn = open_db()?;

    let exclude_patterns: HashSet<_> = exclude.split(',').map(|s| s.trim()).collect();
    let lang_filter: Option<HashSet<_>> = languages.map(|l| {
        l.split(',').map(|s| s.trim().to_lowercase()).collect()
    });

    println!("=== INDEXING ===");
    println!("Path: {}", path.display());

    let mut total_files = 0;
    let mut total_symbols = 0;
    let mut total_calls = 0;
    let mut lang_counts: HashMap<&str, usize> = HashMap::new();

    for entry in WalkDir::new(path)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !exclude_patterns.iter().any(|p| name.contains(p))
        })
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let ext = entry.path().extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = Language::from_extension(ext);

        if lang == Language::Unknown {
            continue;
        }

        // Filter by language if specified
        if let Some(ref filter) = lang_filter {
            if !filter.contains(&lang.name().to_lowercase()) {
                continue;
            }
        }

        match index_file(&conn, entry.path(), lang) {
            Ok((symbols, calls)) => {
                total_files += 1;
                total_symbols += symbols;
                total_calls += calls;
                *lang_counts.entry(lang.name()).or_insert(0) += 1;

                if total_files % 50 == 0 {
                    print!("\rIndexed {} files...", total_files);
                }
            }
            Err(e) => {
                eprintln!("\nWarning: Failed to index {}: {}", entry.path().display(), e);
            }
        }
    }

    println!("\r=== COMPLETE ===");
    println!("Files indexed: {}", total_files);
    println!("Symbols found: {}", total_symbols);
    println!("Calls mapped: {}", total_calls);
    println!();
    println!("By language:");
    for (lang, count) in lang_counts.iter() {
        println!("  {}: {} files", lang, count);
    }

    Ok(())
}

fn cmd_callers(name: &str, limit: i64) -> Result<()> {
    let conn = open_db()?;

    let mut stmt = conn.prepare(
        "SELECT DISTINCT c.caller, c.file_path, c.line
         FROM calls c
         WHERE c.callee = ?
         ORDER BY c.file_path, c.line
         LIMIT ?",
    )?;

    let callers: Vec<_> = stmt
        .query_map(params![name, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    if callers.is_empty() {
        println!("No callers found for '{}'", name);
        return Ok(());
    }

    println!("=== CALLERS OF '{}' ({}) ===", name, callers.len());
    for (caller, file, line) in callers {
        let short_path = truncate_path(&file, 40);
        println!("  {} | {}:{}", caller, short_path, line);
    }

    Ok(())
}

fn cmd_callees(name: &str, limit: i64) -> Result<()> {
    let conn = open_db()?;

    let mut stmt = conn.prepare(
        "SELECT DISTINCT c.callee, c.file_path, c.line
         FROM calls c
         WHERE c.caller = ?
         ORDER BY c.line
         LIMIT ?",
    )?;

    let callees: Vec<_> = stmt
        .query_map(params![name, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    if callees.is_empty() {
        println!("No callees found for '{}'", name);
        return Ok(());
    }

    println!("=== CALLEES OF '{}' ({}) ===", name, callees.len());
    for (callee, file, line) in callees {
        let short_path = truncate_path(&file, 40);
        println!("  {} | {}:{}", callee, short_path, line);
    }

    Ok(())
}

fn cmd_impact(path: &Path, depth: usize) -> Result<()> {
    let conn = open_db()?;
    let file_path = path.to_string_lossy().replace('\\', "/");

    // Get all functions defined in this file
    let mut stmt = conn.prepare(
        "SELECT name FROM symbols WHERE file_path = ? AND kind IN ('function', 'method')",
    )?;

    let functions: Vec<String> = stmt
        .query_map(params![file_path], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    if functions.is_empty() {
        println!("No functions found in {}", path.display());
        return Ok(());
    }

    println!("=== IMPACT ANALYSIS ===");
    println!("File: {}", path.display());
    println!("Functions in file: {}", functions.len());
    println!();

    // Find transitive callers
    let mut all_impacted: HashSet<String> = HashSet::new();
    let mut current_level: HashSet<String> = functions.iter().cloned().collect();

    for level in 0..depth {
        let mut next_level = HashSet::new();

        for func in &current_level {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT caller, file_path FROM calls WHERE callee = ?",
            )?;

            let callers: Vec<(String, String)> = stmt
                .query_map(params![func], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();

            for (caller, file) in callers {
                if !all_impacted.contains(&caller) {
                    all_impacted.insert(caller.clone());
                    next_level.insert(caller.clone());

                    if level == 0 {
                        let short_path = truncate_path(&file, 40);
                        println!("  [direct] {} | {}", caller, short_path);
                    }
                }
            }
        }

        if next_level.is_empty() {
            break;
        }

        if level > 0 {
            println!("  [level {}] {} functions", level + 1, next_level.len());
        }

        current_level = next_level;
    }

    println!();
    println!("Total impact: {} functions may be affected", all_impacted.len());

    Ok(())
}

fn cmd_find(pattern: &str, limit: i64) -> Result<()> {
    let conn = open_db()?;

    // Convert wildcards to SQL LIKE pattern
    let sql_pattern = pattern.replace('*', "%").replace('?', "_");

    let mut stmt = conn.prepare(
        "SELECT name, kind, file_path, line
         FROM symbols
         WHERE name LIKE ?
         ORDER BY name, file_path
         LIMIT ?",
    )?;

    let results: Vec<_> = stmt
        .query_map(params![sql_pattern, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    if results.is_empty() {
        println!("No symbols found matching '{}'", pattern);
        return Ok(());
    }

    println!("=== SYMBOLS MATCHING '{}' ({}) ===", pattern, results.len());
    for (name, kind, file, line) in results {
        let short_path = truncate_path(&file, 35);
        println!("  {} | {} | {}:{}", kind, name, short_path, line);
    }

    Ok(())
}

fn cmd_stats() -> Result<()> {
    let conn = open_db()?;

    let file_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM indexed_files",
        [],
        |row| row.get(0),
    )?;

    let symbol_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM symbols",
        [],
        |row| row.get(0),
    )?;

    let call_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM calls",
        [],
        |row| row.get(0),
    )?;

    let function_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM symbols WHERE kind = 'function'",
        [],
        |row| row.get(0),
    )?;

    let class_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM symbols WHERE kind = 'class'",
        [],
        |row| row.get(0),
    )?;

    // Language breakdown
    let mut stmt = conn.prepare(
        "SELECT language, COUNT(*) FROM indexed_files GROUP BY language ORDER BY COUNT(*) DESC",
    )?;

    let langs: Vec<(String, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    println!("=== CODE GRAPH STATISTICS ===");
    println!("  Files indexed: {}", file_count);
    println!("  Total symbols: {}", symbol_count);
    println!("    Functions: {}", function_count);
    println!("    Classes: {}", class_count);
    println!("  Call edges: {}", call_count);
    println!();
    println!("By language:");
    for (lang, count) in langs {
        println!("  {}: {} files", lang, count);
    }

    Ok(())
}

fn cmd_clear() -> Result<()> {
    let conn = open_db()?;

    conn.execute("DELETE FROM symbols", [])?;
    conn.execute("DELETE FROM calls", [])?;
    conn.execute("DELETE FROM indexed_files", [])?;

    println!("Code graph cleared");
    Ok(())
}

// ============================================================================
// MAIN
// ============================================================================

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Index { path, languages, exclude } => {
            cmd_index(&path, languages.as_deref(), &exclude)?;
        }
        Commands::Callers { name, limit } => {
            cmd_callers(&name, limit)?;
        }
        Commands::Callees { name, limit } => {
            cmd_callees(&name, limit)?;
        }
        Commands::Impact { path, depth } => {
            cmd_impact(&path, depth)?;
        }
        Commands::Find { pattern, limit } => {
            cmd_find(&pattern, limit)?;
        }
        Commands::Stats => {
            cmd_stats()?;
        }
        Commands::Clear => {
            cmd_clear()?;
        }
    }

    Ok(())
}
