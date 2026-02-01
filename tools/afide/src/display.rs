//! Display and formatting utilities

use crate::config::Config;
use crate::registry::{self, BinaryInfo, InstanceStatus};
use crate::drift::DriftReport;
use anyhow::Result;
use colored::*;
use std::collections::HashMap;
use std::path::Path;

pub fn print_header(title: &str) {
    println!();
    println!("{}", "═".repeat(50).cyan());
    println!("  {} - {}", "AI-Foundation IDE".cyan().bold(), title.white().bold());
    println!("{}", "═".repeat(50).cyan());
    println!();
}

pub fn print_status_table(
    instances: &[InstanceStatus],
    source_bins: &HashMap<String, BinaryInfo>,
    verbose: bool,
) -> Result<()> {
    println!("Source: {} ({} binaries)\n",
        instances.first()
            .map(|_| "All Tools/bin")
            .unwrap_or("N/A")
            .cyan(),
        source_bins.len().to_string().green()
    );

    println!("{:<25} {:<18} {:<14} {:>8} {:>8}",
        "INSTANCE".bold(),
        "AI_ID".bold(),
        "TYPE".bold(),
        "STATUS".bold(),
        "DRIFT".bold()
    );
    println!("{}", "─".repeat(78));

    let mut total_drift = 0;
    let mut instances_with_drift = 0;

    for status in instances {
        let drift_display = if status.drift_count > 0 {
            instances_with_drift += 1;
            total_drift += status.drift_count;
            format!("{} files", status.drift_count).red()
        } else {
            "0 files".green()
        };

        let status_display = if !status.exists {
            "MISSING".red()
        } else if !status.bin_dir_exists {
            "NO BIN".yellow()
        } else if status.drift_count > 0 {
            "DRIFT".yellow()
        } else {
            "OK".green()
        };

        // Truncate AI_ID if too long
        let ai_id = if status.instance.ai_id.len() > 16 {
            format!("{}...", &status.instance.ai_id[..13])
        } else {
            status.instance.ai_id.clone()
        };

        println!("{:<25} {:<18} {:<14} {:>8} {:>8}",
            status.instance.name.cyan(),
            ai_id,
            status.instance.instance_type,
            status_display,
            drift_display
        );
    }

    println!("{}", "─".repeat(78));
    println!("\nSummary: {}/{} instances have drift ({} total files)",
        if instances_with_drift > 0 {
            instances_with_drift.to_string().red()
        } else {
            instances_with_drift.to_string().green()
        },
        instances.len(),
        total_drift
    );

    if instances_with_drift > 0 {
        println!("\nRun {} to see details, {} to fix.",
            "afide drift".yellow(),
            "afide deploy --all".green()
        );
    }

    Ok(())
}

pub fn print_drift_report(reports: &[DriftReport]) -> Result<()> {
    let reports_with_drift: Vec<_> = reports.iter()
        .filter(|r| r.has_drift())
        .collect();

    if reports_with_drift.is_empty() {
        println!("{}", "All instances are in sync!".green().bold());
        return Ok(());
    }

    for report in &reports_with_drift {
        println!("{}:", report.instance_name.cyan().bold());

        if !report.missing.is_empty() {
            println!("  {} ({}):", "MISSING".red(), report.missing.len());
            for file in &report.missing {
                println!("    {} {}", "•".red(), file);
            }
        }

        if !report.outdated.is_empty() {
            println!("  {} ({}):", "OUTDATED".yellow(), report.outdated.len());
            for file in &report.outdated {
                println!("    {} {} (source: {}, target: {})",
                    "•".yellow(),
                    file.name,
                    file.source_modified.green(),
                    file.target_modified.red()
                );
            }
        }

        if !report.extra.is_empty() {
            println!("  {} ({}):", "EXTRA".dimmed(), report.extra.len());
            for file in &report.extra {
                println!("    {} {}", "•".dimmed(), file.dimmed());
            }
        }

        println!();
    }

    let total_missing: usize = reports.iter().map(|r| r.missing.len()).sum();
    let total_outdated: usize = reports.iter().map(|r| r.outdated.len()).sum();

    println!("{}", "─".repeat(50));
    println!("Total: {} missing, {} outdated across {} instances",
        total_missing.to_string().red(),
        total_outdated.to_string().yellow(),
        reports_with_drift.len()
    );
    println!("\nRun {} to fix all drift.", "afide deploy --all".green());

    Ok(())
}

pub fn print_instance_list(config: &Config) -> Result<()> {
    println!("{:<25} {:<18} {:<14} {:>8}",
        "NAME".bold(),
        "AI_ID".bold(),
        "TYPE".bold(),
        "ENABLED".bold()
    );
    println!("{}", "─".repeat(70));

    for instance in &config.instances {
        let enabled = if instance.enabled {
            "Yes".green()
        } else {
            "No".red()
        };

        let ai_id = if instance.ai_id.len() > 16 {
            format!("{}...", &instance.ai_id[..13])
        } else {
            instance.ai_id.clone()
        };

        println!("{:<25} {:<18} {:<14} {:>8}",
            instance.name.cyan(),
            ai_id,
            instance.instance_type,
            enabled
        );
    }

    println!("\nTotal: {} instances", config.instances.len());
    Ok(())
}

pub fn print_instance_info(config: &Config, name: &str) -> Result<()> {
    let instance = config.instances.iter()
        .find(|i| i.name == name)
        .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", name))?;

    print_header(&format!("Instance: {}", name));

    println!("{}:", "Configuration".bold());
    println!("  Name:     {}", instance.name.cyan());
    println!("  Path:     {}", instance.path);
    println!("  AI ID:    {}", instance.ai_id.green());
    println!("  Type:     {}", instance.instance_type);
    println!("  Enabled:  {}", if instance.enabled { "Yes".green() } else { "No".red() });

    let bin_path = Path::new(&instance.path).join("bin");
    let bins = registry::get_binaries_in_dir(&bin_path)?;

    println!("\n{}:", "Binaries".bold());
    println!("  Directory: {}", bin_path.display());
    println!("  Count:     {} executables", bins.len());

    if !bins.is_empty() {
        println!("\n  {}:", "Files".bold());
        let mut sorted_bins: Vec<_> = bins.values().collect();
        sorted_bins.sort_by(|a, b| a.name.cmp(&b.name));

        for bin in sorted_bins.iter().take(10) {
            println!("    {} ({:.1} KB, {})",
                bin.name.cyan(),
                bin.size as f64 / 1024.0,
                bin.modified.format("%Y-%m-%d %H:%M")
            );
        }

        if bins.len() > 10 {
            println!("    ... and {} more", bins.len() - 10);
        }
    }

    // Check for config files
    let instance_path = Path::new(&instance.path);
    println!("\n{}:", "Config Files".bold());

    for config_file in &[".mcp.json", "settings.json", "CLAUDE.md"] {
        let file_path = instance_path.join(config_file);
        let status = if file_path.exists() {
            "Present".green()
        } else {
            "Missing".yellow()
        };
        println!("  {}: {}", config_file, status);
    }

    Ok(())
}
