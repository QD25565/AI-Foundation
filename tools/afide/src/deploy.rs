//! Deployment functionality

use crate::config::Config;
use crate::registry::{self, BinaryInfo};
use anyhow::{Result, Context};
use colored::*;
use std::collections::HashMap;
use std::path::Path;
use std::fs;
use std::process::Command;

#[derive(Debug)]
pub struct DeployResult {
    pub instance: String,
    pub file: String,
    pub status: DeployStatus,
}

#[derive(Debug)]
pub enum DeployStatus {
    Deployed,
    Skipped(String),
    Failed(String),
}

pub fn deploy(
    config: &Config,
    files: &[String],
    all: bool,
    targets: Option<&[String]>,
    force: bool,
    dry_run: bool,
) -> Result<()> {
    let source_bins = registry::get_source_binaries(config)?;

    // Determine which files to deploy
    let files_to_deploy: Vec<&String> = if all || files.is_empty() {
        source_bins.keys().collect()
    } else {
        files.iter().collect()
    };

    if files_to_deploy.is_empty() {
        println!("{}", "No files to deploy".yellow());
        return Ok(());
    }

    println!("Deploying {} file(s)...\n", files_to_deploy.len().to_string().cyan());

    let mut total_deployed = 0;
    let mut total_skipped = 0;
    let mut total_failed = 0;

    for instance in &config.instances {
        if !instance.enabled {
            continue;
        }

        // Filter by target instances if specified
        if let Some(target_list) = targets {
            if !target_list.iter().any(|t| t == &instance.name) {
                continue;
            }
        }

        println!("--- {} ---", instance.name.magenta());

        let bin_path = Path::new(&instance.path).join("bin");
        let target_bins = registry::get_binaries_in_dir(&bin_path)?;

        for file_name in &files_to_deploy {
            let source_info = match source_bins.get(*file_name) {
                Some(info) => info,
                None => {
                    println!("  {} {} - Source not found", "FAIL".red(), file_name);
                    total_failed += 1;
                    continue;
                }
            };

            let result = deploy_file(
                source_info,
                &bin_path,
                target_bins.get(*file_name),
                force,
                dry_run,
            );

            match result {
                DeployStatus::Deployed => {
                    println!("  {} {}", "OK".green(), file_name);
                    total_deployed += 1;
                }
                DeployStatus::Skipped(reason) => {
                    println!("  {} {} - {}", "SKIP".yellow(), file_name, reason);
                    total_skipped += 1;
                }
                DeployStatus::Failed(err) => {
                    println!("  {} {} - {}", "FAIL".red(), file_name, err);
                    total_failed += 1;
                }
            }
        }
        println!();
    }

    // Summary
    println!("{}", "═".repeat(50));
    println!("Deployed: {}  Skipped: {}  Failed: {}",
        total_deployed.to_string().green(),
        total_skipped.to_string().yellow(),
        if total_failed > 0 { total_failed.to_string().red() } else { total_failed.to_string().green() }
    );

    Ok(())
}

fn deploy_file(
    source: &BinaryInfo,
    target_dir: &Path,
    target_existing: Option<&BinaryInfo>,
    force: bool,
    dry_run: bool,
) -> DeployStatus {
    // Check if target directory exists
    if !target_dir.exists() {
        if dry_run {
            return DeployStatus::Deployed; // Would create
        }
        if let Err(e) = fs::create_dir_all(target_dir) {
            return DeployStatus::Failed(format!("Cannot create directory: {}", e));
        }
    }

    // Check if already up to date
    if let Some(existing) = target_existing {
        if source.hash == existing.hash {
            return DeployStatus::Skipped("Already up to date".to_string());
        }
    }

    // Check if process is running (Windows)
    let process_name = source.name.trim_end_matches(".exe");
    if is_process_running(process_name) {
        if force {
            if !dry_run {
                if let Err(e) = kill_process(process_name) {
                    return DeployStatus::Failed(format!("Cannot stop process: {}", e));
                }
            }
        } else {
            return DeployStatus::Skipped("Process running (use --force)".to_string());
        }
    }

    if dry_run {
        return DeployStatus::Deployed; // Would deploy
    }

    // Copy the file
    let target_path = target_dir.join(&source.name);
    match fs::copy(&source.path, &target_path) {
        Ok(_) => DeployStatus::Deployed,
        Err(e) => DeployStatus::Failed(e.to_string()),
    }
}

fn is_process_running(name: &str) -> bool {
    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {}.exe", name)])
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                stdout.contains(&format!("{}.exe", name))
            }
            Err(_) => false,
        }
    }

    #[cfg(not(windows))]
    {
        let output = Command::new("pgrep")
            .arg("-x")
            .arg(name)
            .output();

        match output {
            Ok(out) => out.status.success(),
            Err(_) => false,
        }
    }
}

fn kill_process(name: &str) -> Result<()> {
    #[cfg(windows)]
    {
        Command::new("taskkill")
            .args(["/F", "/IM", &format!("{}.exe", name)])
            .output()
            .context("Failed to kill process")?;

        // Wait a bit for process to terminate
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    #[cfg(not(windows))]
    {
        Command::new("pkill")
            .arg("-9")
            .arg(name)
            .output()
            .context("Failed to kill process")?;

        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    Ok(())
}

pub fn sync_configs(config: &Config, dry_run: bool) -> Result<()> {
    let source_path = Path::new(&config.source.path).parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid source path"))?;

    // Config files to sync
    let config_files = [
        ".mcp.json",
        "settings.json",
        "CLAUDE.md",
    ];

    println!("Syncing configuration files...\n");

    for instance in &config.instances {
        if !instance.enabled {
            continue;
        }

        println!("--- {} ---", instance.name.magenta());
        let instance_path = Path::new(&instance.path);

        for config_file in &config_files {
            let source_file = source_path.join(config_file);
            let target_file = instance_path.join(config_file);

            if !source_file.exists() {
                continue;
            }

            // For CLAUDE.md, we might want special handling per instance
            if *config_file == "CLAUDE.md" && target_file.exists() {
                println!("  {} {} - Instance-specific (skipped)", "SKIP".yellow(), config_file);
                continue;
            }

            if dry_run {
                println!("  {} {} - Would sync", "DRY".cyan(), config_file);
                continue;
            }

            match fs::copy(&source_file, &target_file) {
                Ok(_) => println!("  {} {}", "OK".green(), config_file),
                Err(e) => println!("  {} {} - {}", "FAIL".red(), config_file, e),
            }
        }
        println!();
    }

    Ok(())
}
