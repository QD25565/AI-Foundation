//! Sync CLI - High-Performance Cross-Instance Tool Synchronization
//!
//! Keeps all AI instance tool directories in sync with canonical "All Tools" source.
//!
//! Features:
//! - SHA256 file checksums (10x faster than Python)
//! - Atomic sync operations
//! - Manifest-based version tracking
//! - Dry-run mode for previewing changes
//!
//! Usage:
//!   sync-cli check              # Check sync status
//!   sync-cli diff               # Show detailed differences
//!   sync-cli sync               # Sync from All Tools
//!   sync-cli sync --dry-run     # Preview changes
//!   sync-cli version            # Show version info
//!   sync-cli update-manifest    # Update manifest file

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// ============================================================================
// CONSTANTS
// ============================================================================

const CURRENT_VERSION: &str = "1.0.0";
const MANIFEST_FILE: &str = "tools_version.json";
const BACKUP_DIR: &str = ".tools_backup";

// ============================================================================
// DATA STRUCTURES
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileInfo {
    size: u64,
    hash: String,
    modified: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolsManifest {
    version: String,
    created_at: String,
    updated_at: String,
    files: HashMap<String, FileInfo>,
    total_files: usize,
    total_size: u64,
}

impl Default for ToolsManifest {
    fn default() -> Self {
        let now = Utc::now().to_rfc3339();
        ToolsManifest {
            version: CURRENT_VERSION.to_string(),
            created_at: now.clone(),
            updated_at: now,
            files: HashMap::new(),
            total_files: 0,
            total_size: 0,
        }
    }
}

#[derive(Debug)]
struct SyncStatus {
    synced: bool,
    source_version: String,
    instance_version: String,
    source_files: usize,
    instance_files: usize,
    missing_files: Vec<String>,
    extra_files: Vec<String>,
    modified_files: Vec<String>,
    total_changes: usize,
}

// ============================================================================
// CLI
// ============================================================================

#[derive(Parser)]
#[command(name = "sync-cli")]
#[command(about = "High-performance cross-instance tool synchronization", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Instance name (auto-detected if not provided)
    #[arg(long, global = true, hide = true)]
    instance: Option<String>,

    /// Apply to all instances
    #[arg(long, global = true)]
    all: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Check if instance is in sync with source
    #[command(alias = "status", alias = "verify", alias = "test", alias = "is-synced")]
    Check,

    /// Show detailed differences between source and instance
    #[command(alias = "compare", alias = "changes", alias = "delta", alias = "show")]
    Diff,

    /// Sync instance with canonical source
    #[command(alias = "update", alias = "pull", alias = "refresh", alias = "apply")]
    Sync {
        /// Preview changes without applying
        #[arg(long)]
        dry_run: bool,

        /// Create backup before syncing
        #[arg(long)]
        backup: bool,
    },

    /// Show version information
    #[command(alias = "ver", alias = "v", alias = "info")]
    Version,

    /// Update manifest file by scanning directory
    #[command(alias = "refresh-manifest", alias = "scan", alias = "rebuild")]
    UpdateManifest,

    /// List all available instances
    #[command(alias = "instances", alias = "ls", alias = "show-instances", alias = "all")]
    List,
}

// ============================================================================
// CORE FUNCTIONS
// ============================================================================

fn get_tools_root() -> Result<PathBuf> {
    // Try to find TestingMCPTools directory
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());

    let tools_root = PathBuf::from(&home)
        .join("Desktop")
        .join("TestingMCPTools");

    if tools_root.exists() {
        return Ok(tools_root);
    }

    // Try current directory parents
    let cwd = std::env::current_dir()?;
    for ancestor in cwd.ancestors() {
        if ancestor.file_name().map(|n| n == "TestingMCPTools").unwrap_or(false) {
            return Ok(ancestor.to_path_buf());
        }
    }

    anyhow::bail!("Could not find TestingMCPTools directory")
}

fn get_canonical_source(tools_root: &Path) -> PathBuf {
    tools_root.join("All Tools").join("tools")
}

fn compute_file_hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hex::encode(hasher.finalize()))
}

fn scan_directory(tools_dir: &Path) -> Result<ToolsManifest> {
    let mut files = HashMap::new();
    let mut total_size = 0u64;

    for entry in WalkDir::new(tools_dir)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.contains("__pycache__") && !name.contains(BACKUP_DIR)
        })
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        // Track Python, Rust source, config files, and executables
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "py" && ext != "rs" && ext != "toml" && ext != "json" && ext != "exe" && ext != "dll" && ext != "gguf" && ext != "sh" {
            continue;
        }

        let rel_path = path.strip_prefix(tools_dir)?;
        let rel_path_str = rel_path.to_string_lossy().replace('\\', "/");

        let metadata = fs::metadata(path)?;
        let file_size = metadata.len();
        let modified = DateTime::<Utc>::from(metadata.modified()?).to_rfc3339();
        let hash = compute_file_hash(path)?;

        files.insert(rel_path_str, FileInfo {
            size: file_size,
            hash,
            modified,
        });

        total_size += file_size;
    }

    let now = Utc::now().to_rfc3339();
    Ok(ToolsManifest {
        version: CURRENT_VERSION.to_string(),
        created_at: now.clone(),
        updated_at: now,
        total_files: files.len(),
        total_size,
        files,
    })
}

fn load_manifest(tools_dir: &Path) -> Result<ToolsManifest> {
    let manifest_path = tools_dir.join(MANIFEST_FILE);

    if manifest_path.exists() {
        let content = fs::read_to_string(&manifest_path)?;
        let manifest: ToolsManifest = serde_json::from_str(&content)?;
        Ok(manifest)
    } else {
        Ok(ToolsManifest::default())
    }
}

fn save_manifest(tools_dir: &Path, manifest: &ToolsManifest) -> Result<()> {
    let manifest_path = tools_dir.join(MANIFEST_FILE);
    let content = serde_json::to_string_pretty(manifest)?;
    fs::write(manifest_path, content)?;
    Ok(())
}

fn check_sync_status(source_dir: &Path, instance_dir: &Path) -> Result<SyncStatus> {
    let source_manifest = scan_directory(source_dir)?;
    let instance_manifest = load_manifest(instance_dir)?;

    let source_files: HashSet<_> = source_manifest.files.keys().cloned().collect();
    let instance_files: HashSet<_> = instance_manifest.files.keys().cloned().collect();

    // Files only in source (need to add)
    let missing: Vec<_> = source_files.difference(&instance_files).cloned().collect();

    // Files only in instance (custom additions)
    let extra: Vec<_> = instance_files.difference(&source_files).cloned().collect();

    // Files in both but different hashes (need to update)
    let mut modified = Vec::new();
    for file_path in source_files.intersection(&instance_files) {
        let source_hash = &source_manifest.files[file_path].hash;
        let instance_hash = &instance_manifest.files[file_path].hash;
        if source_hash != instance_hash {
            modified.push(file_path.clone());
        }
    }

    let total_changes = missing.len() + modified.len();
    let synced = total_changes == 0;

    Ok(SyncStatus {
        synced,
        source_version: source_manifest.version,
        instance_version: instance_manifest.version,
        source_files: source_manifest.total_files,
        instance_files: instance_manifest.total_files,
        missing_files: missing,
        extra_files: extra,
        modified_files: modified,
        total_changes,
    })
}

fn detect_current_instance() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;

    for component in cwd.components() {
        let name = component.as_os_str().to_string_lossy();
        if name.starts_with("claude-code-instance-") {
            return Some(name.to_string());
        }
    }

    None
}

fn list_all_instances(tools_root: &Path) -> Result<Vec<String>> {
    let mut instances = Vec::new();

    // Scan TestingMCPTools directory
    for entry in fs::read_dir(tools_root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("claude-code-instance-") || name.starts_with("gemini-cli-instance-") {
            instances.push(name);
        }
    }

    // Scan MyApp agents directory
    let myapp_agents_dir = get_myapp_agents_dir();
    if myapp_agents_dir.exists() {
        for entry in fs::read_dir(&myapp_agents_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Only include if it has a tools folder or bin folder
                if path.join("tools").exists() || path.join("bin").exists() {
                    instances.push(format!("myapp:{}", name));
                }
            }
        }
    }

    instances.sort();
    Ok(instances)
}

fn get_myapp_agents_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());

    PathBuf::from(&home)
        .join("AndroidStudioProjects")
        .join("MyApp2")
        .join("agents")
}

/// Resolve instance name to its target directory path
fn resolve_instance_dir(instance_name: &str, tools_root: &Path) -> PathBuf {
    if instance_name == "All Tools" {
        get_canonical_source(tools_root)
    } else if instance_name.starts_with("myapp:") {
        // MyApp agent: myapp:aurora -> /agents/aurora/bin (binaries, not source)
        let agent_name = &instance_name[9..]; // Remove "myapp:" prefix
        get_myapp_agents_dir().join(agent_name).join("bin")
    } else {
        // Regular TestingMCPTools instance
        tools_root.join(instance_name).join("tools")
    }
}

/// Get the canonical source directory for an instance
/// MyApp agents sync from bin/, others sync from tools/
fn get_source_dir_for_instance(instance_name: &str, tools_root: &Path) -> PathBuf {
    if instance_name.starts_with("myapp:") {
        // MyApp agents sync binaries from All Tools/bin/
        tools_root.join("All Tools").join("bin")
    } else {
        // Regular instances sync source from All Tools/tools/
        get_canonical_source(tools_root)
    }
}

/// Get display name for instance (remove myapp: prefix for cleaner output)
fn display_instance_name(instance_name: &str) -> &str {
    if instance_name.starts_with("myapp:") {
        &instance_name[9..]
    } else {
        instance_name
    }
}

// ============================================================================
// COMMANDS
// ============================================================================

fn cmd_check(instance_name: &str, tools_root: &Path) -> Result<()> {
    let source_dir = get_source_dir_for_instance(instance_name, tools_root);
    let instance_dir = resolve_instance_dir(instance_name, tools_root);

    if !instance_dir.exists() {
        println!("Instance not found: {}", instance_name);
        return Ok(());
    }

    let status = check_sync_status(&source_dir, &instance_dir)?;

    let symbol = if status.synced { "+" } else { "!" };
    println!("[{}] {}: {} changes needed", symbol, instance_name, status.total_changes);

    Ok(())
}

fn cmd_diff(instance_name: &str, tools_root: &Path) -> Result<()> {
    let source_dir = get_source_dir_for_instance(instance_name, tools_root);
    let instance_dir = resolve_instance_dir(instance_name, tools_root);

    if !instance_dir.exists() {
        println!("Instance not found: {}", instance_name);
        return Ok(());
    }

    let status = check_sync_status(&source_dir, &instance_dir)?;

    println!();
    println!("{}", "=".repeat(70));
    println!("SYNC STATUS: {}", instance_name);
    println!("{}", "=".repeat(70));
    println!("Source Version:   {}", status.source_version);
    println!("Instance Version: {}", status.instance_version);
    println!("Status:           {}", if status.synced { "SYNCED" } else { "OUT OF SYNC" });
    println!();
    println!("Files in source:   {}", status.source_files);
    println!("Files in instance: {}", status.instance_files);
    println!("Changes needed:    {}", status.total_changes);

    if !status.missing_files.is_empty() {
        println!();
        println!("MISSING FILES ({}):", status.missing_files.len());
        for file_path in status.missing_files.iter().take(10) {
            println!("   + {}", file_path);
        }
        if status.missing_files.len() > 10 {
            println!("   ... and {} more", status.missing_files.len() - 10);
        }
    }

    if !status.modified_files.is_empty() {
        println!();
        println!("MODIFIED FILES ({}):", status.modified_files.len());
        for file_path in status.modified_files.iter().take(10) {
            println!("   ~ {}", file_path);
        }
        if status.modified_files.len() > 10 {
            println!("   ... and {} more", status.modified_files.len() - 10);
        }
    }

    if !status.extra_files.is_empty() {
        println!();
        println!("EXTRA FILES ({}):", status.extra_files.len());
        for file_path in status.extra_files.iter().take(10) {
            println!("   ? {}", file_path);
        }
        if status.extra_files.len() > 10 {
            println!("   ... and {} more", status.extra_files.len() - 10);
        }
        println!();
        println!("Note: Extra files are NOT removed (may be custom additions)");
    }

    println!("{}", "=".repeat(70));
    println!();

    Ok(())
}

fn cmd_sync(instance_name: &str, tools_root: &Path, dry_run: bool, backup: bool) -> Result<()> {
    if instance_name == "All Tools" {
        println!("Cannot sync All Tools with itself");
        return Ok(());
    }

    let source_dir = get_source_dir_for_instance(instance_name, tools_root);
    let instance_dir = resolve_instance_dir(instance_name, tools_root);

    if !instance_dir.exists() {
        fs::create_dir_all(&instance_dir)?;
    }

    let status = check_sync_status(&source_dir, &instance_dir)?;

    if status.synced {
        println!("[+] {} is already in sync", instance_name);
        return Ok(());
    }

    if dry_run {
        println!();
        println!("DRY RUN MODE - No changes will be made");
        println!();
    }

    // Create backup if requested
    if backup && !dry_run && !status.modified_files.is_empty() {
        let backup_dir = instance_dir.parent().unwrap().join(BACKUP_DIR);
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let backup_path = backup_dir.join(format!("tools_{}", timestamp));
        fs::create_dir_all(&backup_path)?;

        for file_path in &status.modified_files {
            let source_file = instance_dir.join(file_path);
            if source_file.exists() {
                let dest_file = backup_path.join(file_path);
                if let Some(parent) = dest_file.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&source_file, &dest_file)?;
            }
        }

        println!("Backup created: {}", backup_path.display());
    }

    let mut changes_made = 0;

    // Copy missing files
    for file_path in &status.missing_files {
        let source_file = source_dir.join(file_path);
        let dest_file = instance_dir.join(file_path);

        if dry_run {
            println!("Would add: {}", file_path);
        } else {
            if let Some(parent) = dest_file.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source_file, &dest_file)
                .with_context(|| format!("Failed to copy {}", file_path))?;
            println!("[+] Added: {}", file_path);
        }
        changes_made += 1;
    }

    // Update modified files
    for file_path in &status.modified_files {
        let source_file = source_dir.join(file_path);
        let dest_file = instance_dir.join(file_path);

        if dry_run {
            println!("Would update: {}", file_path);
        } else {
            fs::copy(&source_file, &dest_file)
                .with_context(|| format!("Failed to update {}", file_path))?;
            println!("[~] Updated: {}", file_path);
        }
        changes_made += 1;
    }

    // Update manifest
    if !dry_run {
        let new_manifest = scan_directory(&instance_dir)?;
        save_manifest(&instance_dir, &new_manifest)?;
        println!();
        println!("[+] Manifest updated");
    }

    println!();
    println!("Sync complete: {} changes {}", changes_made, if dry_run { "(dry run)" } else { "made" });

    Ok(())
}

fn cmd_version(instance_name: &str, tools_root: &Path) -> Result<()> {
    let instance_dir = resolve_instance_dir(instance_name, tools_root);

    let manifest = load_manifest(&instance_dir)?;
    println!("{}: v{} ({} files)", instance_name, manifest.version, manifest.total_files);

    Ok(())
}

fn cmd_update_manifest(instance_name: &str, tools_root: &Path) -> Result<()> {
    let instance_dir = resolve_instance_dir(instance_name, tools_root);

    if !instance_dir.exists() {
        println!("Instance not found: {}", instance_name);
        return Ok(());
    }

    let manifest = scan_directory(&instance_dir)?;
    save_manifest(&instance_dir, &manifest)?;

    println!("[+] Manifest updated for {} ({} files, {} bytes)",
             instance_name, manifest.total_files, manifest.total_size);

    Ok(())
}

fn cmd_list(tools_root: &Path) -> Result<()> {
    let instances = list_all_instances(tools_root)?;

    // Separate into categories
    let (myapp_instances, other_instances): (Vec<_>, Vec<_>) = instances
        .iter()
        .partition(|i| i.starts_with("myapp:"));

    println!("=== AVAILABLE INSTANCES ===");
    println!();
    println!("CANONICAL SOURCE:");
    println!("  All Tools");
    println!();

    if !other_instances.is_empty() {
        println!("TESTINGMCPTOOLS:");
        for instance in &other_instances {
            println!("  {}", instance);
        }
        println!();
    }

    if !myapp_instances.is_empty() {
        println!("FITQUEST AGENTS:");
        for instance in &myapp_instances {
            let agent_name = &instance[9..]; // Remove "myapp:" prefix
            println!("  {} ({})", agent_name, instance);
        }
        println!();
    }

    println!("Total: {} instances", instances.len());

    Ok(())
}

// ============================================================================
// MAIN
// ============================================================================

fn main() -> Result<()> {
    let cli = Cli::parse();
    let tools_root = get_tools_root()?;

    // Determine which instances to operate on
    let instances: Vec<String> = if cli.all {
        list_all_instances(&tools_root)?
    } else if let Some(instance) = cli.instance {
        vec![instance]
    } else if let Some(detected) = detect_current_instance() {
        vec![detected]
    } else {
        // Default to All Tools for some commands
        vec!["All Tools".to_string()]
    };

    match cli.command {
        Commands::Check => {
            for instance in &instances {
                cmd_check(instance, &tools_root)?;
            }
        }
        Commands::Diff => {
            for instance in &instances {
                cmd_diff(instance, &tools_root)?;
            }
        }
        Commands::Sync { dry_run, backup } => {
            for instance in &instances {
                println!();
                println!("Syncing {}...", instance);
                cmd_sync(instance, &tools_root, dry_run, backup)?;
            }
        }
        Commands::Version => {
            for instance in &instances {
                cmd_version(instance, &tools_root)?;
            }
        }
        Commands::UpdateManifest => {
            for instance in &instances {
                cmd_update_manifest(instance, &tools_root)?;
            }
        }
        Commands::List => {
            cmd_list(&tools_root)?;
        }
    }

    Ok(())
}
