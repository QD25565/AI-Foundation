//! Health checking for AI instances

use crate::config::Config;
use anyhow::Result;
use colored::*;
use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub struct HealthReport {
    pub instance_name: String,
    pub daemon_status: ServiceStatus,
    pub mcp_status: ServiceStatus,
    pub notebook_status: ServiceStatus,
    pub teambook_status: ServiceStatus,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum ServiceStatus {
    Ok,
    Warning,
    Error,
    Unknown,
}

impl ServiceStatus {
    pub fn as_str(&self) -> ColoredString {
        match self {
            ServiceStatus::Ok => "OK".green(),
            ServiceStatus::Warning => "WARN".yellow(),
            ServiceStatus::Error => "ERR".red(),
            ServiceStatus::Unknown => "-".dimmed(),
        }
    }
}

pub fn check_health(config: &Config) -> Result<()> {
    let mut reports = Vec::new();

    println!("{:<25} {:>8} {:>8} {:>10} {:>10}",
        "INSTANCE".bold(),
        "DAEMON".bold(),
        "MCP".bold(),
        "NOTEBOOK".bold(),
        "TEAMBOOK".bold()
    );
    println!("{}", "─".repeat(70));

    for instance in &config.instances {
        if !instance.enabled {
            continue;
        }

        let report = check_instance_health(instance)?;

        println!("{:<25} {:>8} {:>8} {:>10} {:>10}",
            instance.name.cyan(),
            report.daemon_status.as_str(),
            report.mcp_status.as_str(),
            report.notebook_status.as_str(),
            report.teambook_status.as_str()
        );

        reports.push(report);
    }

    println!("{}", "─".repeat(70));

    // Print issues
    let all_issues: Vec<_> = reports.iter()
        .filter(|r| !r.issues.is_empty())
        .collect();

    if !all_issues.is_empty() {
        println!("\n{}", "Issues Found:".red().bold());
        for report in all_issues {
            for issue in &report.issues {
                println!("  {} {}: {}", "•".red(), report.instance_name.cyan(), issue);
            }
        }
    } else {
        println!("\n{}", "All instances healthy!".green());
    }

    Ok(())
}

fn check_instance_health(instance: &crate::config::Instance) -> Result<HealthReport> {
    let mut issues = Vec::new();
    let bin_path = Path::new(&instance.path).join("bin");

    // Check daemon status
    let daemon_status = if is_daemon_running() {
        ServiceStatus::Ok
    } else {
        issues.push("Daemon not running".to_string());
        ServiceStatus::Warning
    };

    // Check MCP server (if process list shows it)
    let mcp_status = if is_mcp_running() {
        ServiceStatus::Ok
    } else {
        ServiceStatus::Unknown // MCP is started by Claude Code, not always running
    };

    // Check notebook-cli
    let notebook_status = check_cli_tool(&bin_path.join("notebook-cli.exe"), &["stats"]);
    if matches!(notebook_status, ServiceStatus::Error) {
        issues.push("notebook-cli not responding".to_string());
    }

    // Check teambook
    let teambook_status = check_cli_tool(&bin_path.join("teambook.exe"), &["status"]);
    if matches!(teambook_status, ServiceStatus::Error) {
        issues.push("teambook not responding".to_string());
    }

    Ok(HealthReport {
        instance_name: instance.name.clone(),
        daemon_status,
        mcp_status,
        notebook_status,
        teambook_status,
        issues,
    })
}

fn is_daemon_running() -> bool {
    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq teamengram-daemon.exe"])
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                stdout.contains("teamengram-daemon.exe")
            }
            Err(_) => false,
        }
    }

    #[cfg(not(windows))]
    {
        let output = Command::new("pgrep")
            .arg("-x")
            .arg("teamengram-daemon")
            .output();

        match output {
            Ok(out) => out.status.success(),
            Err(_) => false,
        }
    }
}

fn is_mcp_running() -> bool {
    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq ai-foundation-mcp.exe"])
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                stdout.contains("ai-foundation-mcp.exe")
            }
            Err(_) => false,
        }
    }

    #[cfg(not(windows))]
    {
        false
    }
}

fn check_cli_tool(tool_path: &Path, args: &[&str]) -> ServiceStatus {
    if !tool_path.exists() {
        return ServiceStatus::Error;
    }

    let output = Command::new(tool_path)
        .args(args)
        .output();

    match output {
        Ok(out) => {
            if out.status.success() {
                ServiceStatus::Ok
            } else {
                ServiceStatus::Warning
            }
        }
        Err(_) => ServiceStatus::Error,
    }
}
