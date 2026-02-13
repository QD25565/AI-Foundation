//! CLI Wrapper - Thin subprocess layer for MCP tools
//!
//! Instead of complex daemon connections with struct parsing that can break,
//! we simply call the battle-tested CLI executables and return their output.
//!
//! Benefits:
//! - If CLI works, MCP works (single source of truth)
//! - No parsing mismatches between daemon/client/MCP
//! - Simpler maintenance - fix CLI once, MCP inherits fix
//! - 15-50ms latency is negligible (Claude API calls are 500-2000ms)

use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

/// Get the bin directory path
/// Priority: BIN_PATH env var > working directory ./bin > home/.ai-foundation/bin
fn get_bin_dir() -> PathBuf {
    // Check BIN_PATH environment variable first
    if let Ok(bin_path) = std::env::var("BIN_PATH") {
        return PathBuf::from(bin_path);
    }

    // Check for ./bin relative to current working directory
    let cwd_bin = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("bin");
    if cwd_bin.exists() {
        return cwd_bin;
    }

    // Fall back to ~/.ai-foundation/bin
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ai-foundation")
        .join("bin")
}

/// Get executable name with platform-appropriate extension
fn exe_name(base: &str) -> String {
    #[cfg(windows)]
    {
        format!("{}.exe", base)
    }
    #[cfg(not(windows))]
    {
        base.to_string()
    }
}

/// Run a teambook CLI command and return output
///
/// # Arguments
/// * `args` - Command arguments (e.g., ["rooms", "5"] for "teambook rooms 5")
///
/// # Returns
/// * CLI stdout on success
/// * Formatted error message on failure
pub async fn teambook(args: &[&str]) -> String {
    run_cli(&exe_name("teambook"), args).await
}

/// Run a teambook CLI command with V1 backend (for project/feature operations)
///
/// V2 event sourcing doesn't have project/feature support yet,
/// so we force V1 mode for these operations.
pub async fn teambook_v1(args: &[&str]) -> String {
    // Prepend --v2 false to force V1 mode
    let mut v1_args = vec!["--v2", "false"];
    v1_args.extend(args.iter().copied());
    run_cli(&exe_name("teambook"), &v1_args).await
}

/// Run a notebook CLI command and return output
///
/// # Arguments
/// * `args` - Command arguments (e.g., ["stats"] for "notebook-cli.exe stats")
///
/// # Returns
/// * CLI stdout on success
/// * Formatted error message on failure
pub async fn notebook(args: &[&str]) -> String {
    run_cli(&exe_name("notebook-cli"), args).await
}

/// Run a visionbook CLI command and return output
///
/// VisionEngram visual memory: attach images to notes, AI-optimized thumbnails.
///
/// # Arguments
/// * `args` - Command arguments (e.g., ["attach", "1005", "image.png"])
///
/// # Returns
/// * CLI stdout on success
/// * Formatted error message on failure
pub async fn visionbook(args: &[&str]) -> String {
    run_cli(&exe_name("visionbook"), args).await
}

/// Run a CLI command and return its output
///
/// # Arguments
/// * `exe` - Executable name (e.g., "teambook.exe")
/// * `args` - Command arguments
///
/// # Returns
/// * CLI stdout on success (trimmed)
/// * Formatted error message on failure
async fn run_cli(exe: &str, args: &[&str]) -> String {
    let bin_dir = get_bin_dir();
    let exe_path = bin_dir.join(exe);

    // Get AI_ID for the CLI
    let ai_id = std::env::var("AI_ID")
        .or_else(|_| std::env::var("AGENT_ID"))
        .unwrap_or_else(|_| "unknown".to_string());

    // V2 event sourcing is the default - it gives us event-driven wake
    // V2 is now the default - gives us event-driven wake
    let v2_disabled = std::env::var("TEAMENGRAM_V2")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("false"))
        .unwrap_or(false); // V2 default ON

    let mut cmd = Command::new(&exe_path);
    cmd.args(args).env("AI_ID", &ai_id);

    // V2 is default - always pass unless explicitly disabled
    if !v2_disabled {
        cmd.env("TEAMENGRAM_V2", "1");
    }

    let result = cmd
        .stdin(Stdio::null()) // No stdin = non-interactive
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let child = match result {
        Ok(child) => child,
        Err(e) => {
            return format!("Error: Failed to run {}: {}\nPath: {:?}", exe, e, exe_path);
        }
    };

    match child.wait_with_output().await {
        Ok(output) => {
            if output.status.success() {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                if !stderr.is_empty() {
                    format!("Error: {}", stderr.trim())
                } else if !stdout.is_empty() {
                    stdout.trim().to_string()
                } else {
                    format!("Error: {} exited with code {:?}", exe, output.status.code())
                }
            }
        }
        Err(e) => {
            format!("Error: Failed to get output from {}: {}", exe, e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_teambook_status() {
        let result = teambook(&["status"]).await;
        // Should return AI ID or error about connection
        assert!(!result.is_empty());
    }

    #[tokio::test]
    async fn test_notebook_stats() {
        let result = notebook(&["stats"]).await;
        // Should return stats or error
        assert!(!result.is_empty());
    }
}

/// Run a firebase CLI command and return output
///
/// Firebase API access: Crashlytics, Firestore, Auth
///
/// # Arguments
/// * `args` - Command arguments (e.g., ["crashlytics", "list"])
///
/// # Returns
/// * CLI stdout on success
/// * Formatted error message on failure
pub async fn firebase(args: &[&str]) -> String {
    run_cli(&exe_name("firebase"), args).await
}
