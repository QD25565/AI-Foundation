//! Subprocess runner for teambook and notebook-cli binaries.
//!
//! Respects BIN_PATH env var, falling back to ~/.ai-foundation/bin/.

use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::process::Command;

/// Resolve the path to a named binary in the AI-Foundation bin directory.
pub fn bin_path(name: &str) -> PathBuf {
    let dir = if let Ok(path) = std::env::var("BIN_PATH") {
        PathBuf::from(path)
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".ai-foundation")
            .join("bin")
    };

    let ext = if cfg!(target_os = "windows") || cfg!(target_os = "linux") && is_wsl() {
        ".exe"
    } else {
        ""
    };

    dir.join(format!("{}{}", name, ext))
}

fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/version")
        .map(|v| v.to_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

/// Run a teambook subcommand and return stdout as a string.
/// e.g. `teambook_run(&["read-dms", "--limit", "20"])`
pub async fn teambook_run(args: &[&str]) -> Result<String> {
    let bin = bin_path("teambook");
    let output = Command::new(&bin)
        .args(args)
        .env("TEAMENGRAM_V2", "1")
        .output()
        .await
        .with_context(|| format!("Failed to run teambook with args {:?}", args))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("teambook {:?} failed: {}", args, stderr)
    }
}

/// Run a notebook-cli subcommand and return stdout.
/// e.g. `notebook_run(&["list", "--limit", "20"])`
pub async fn notebook_run(args: &[&str]) -> Result<String> {
    let bin = bin_path("notebook-cli");
    // Notebook-cli needs AI_ID from environment (already set in process env)
    let output = Command::new(&bin)
        .args(args)
        .env("TEAMENGRAM_V2", "1")
        .output()
        .await
        .with_context(|| format!("Failed to run notebook-cli with args {:?}", args))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("notebook-cli {:?} failed: {}", args, stderr)
    }
}
