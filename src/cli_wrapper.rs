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
use tokio::sync::OnceCell;

/// Process-lifetime cache for the resolved AI ID.
///
/// Resolution is expensive on the cold path (one subprocess call to `teambook whoami`),
/// but it only runs once per MCP server process — every tool call after the first
/// reads the cached value.
static AI_ID_CACHE: OnceCell<String> = OnceCell::const_new();

/// Resolve the AI ID for CLI subprocess calls.
///
/// Resolution order:
/// 1. `AI_ID` environment variable
/// 2. `AGENT_ID` environment variable
/// 3. `teambook whoami` — reads the identity from teambook's own store
///    (works regardless of how the MCP server was launched: native Windows,
///    WSL-to-Windows, Linux, etc., as long as teambook is set up).
/// 4. Literal "unknown" as a last-resort sentinel.
///
/// Cached for the lifetime of the MCP server process. First tool call triggers
/// the subprocess fallback (if env is missing); subsequent calls are free.
async fn resolve_ai_id() -> String {
    AI_ID_CACHE
        .get_or_init(|| async {
            if let Ok(id) = std::env::var("AI_ID") {
                let id = id.trim().to_string();
                if !id.is_empty() && id != "unknown" {
                    return id;
                }
            }
            if let Ok(id) = std::env::var("AGENT_ID") {
                let id = id.trim().to_string();
                if !id.is_empty() && id != "unknown" {
                    return id;
                }
            }
            if let Some(id) = whoami_from_teambook().await {
                return id;
            }
            "unknown".to_string()
        })
        .await
        .clone()
}

/// Call `teambook whoami` and parse the `AI:<id>` line from its identity banner.
///
/// Used as a fallback when the AI_ID env var isn't set — teambook has its own
/// identity resolution that works without env plumbing, so we leverage that
/// rather than duplicating the logic here.
async fn whoami_from_teambook() -> Option<String> {
    let bin_dir = get_bin_dir();
    let exe_path = bin_dir.join(exe_name("teambook"));
    let output = Command::new(&exe_path)
        .arg("whoami")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(id) = line.trim().strip_prefix("AI:") {
            let id = id.trim();
            if !id.is_empty() && id != "unknown" {
                return Some(id.to_string());
            }
        }
    }
    None
}

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

/// Run a profile CLI command and return output
pub async fn profile(args: &[&str]) -> String {
    run_cli(&exe_name("profile-cli"), args).await
}

/// Register presence via the V1 daemon RPC path (creates OS mutex).
///
/// V2 outbox writes (TEAMENGRAM_V2=1) update presence events but don't create
/// the OS-level named mutex that is_ai_online() checks. The V1 daemon path
/// does: the daemon acquires a PresenceMutex for each AI that connects and
/// holds it for its lifetime. This one-time call at MCP server startup ensures
/// the daemon knows we're alive.
///
/// Deliberately does NOT set TEAMENGRAM_V2 on the subprocess — we need the V1
/// code path, not the V2 outbox writer.
pub async fn register_presence_v1(ai_id: &str) {
    let bin_dir = get_bin_dir();
    let exe_path = bin_dir.join(exe_name("teambook"));
    let _ = Command::new(&exe_path)
        .args(["update-presence", "active", "MCP server online", "--v2", "false"])
        .env("AI_ID", ai_id)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;
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

    // Get AI_ID for the CLI. Falls back to `teambook whoami` when env is missing
    // (e.g. WSL-to-Windows launches where env vars don't propagate without WSLENV).
    let ai_id = resolve_ai_id().await;

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

// ============== Caller-ID variants (for HTTP API) ==============
//
// These run the same CLIs but with an explicit caller_id instead of
// reading AI_ID from the environment. Used by the HTTP server to
// execute commands on behalf of human users (H_ID like "human-yourname").

/// Run a CLI command with a specific caller ID
///
/// Sets `current_dir` to the system temp directory so the subprocess
/// does NOT pick up `.claude/settings.json` from the HTTP server's CWD.
/// Without this, teambook resolves AI_ID from settings.json (which contains
/// the hosting AI's identity) and ignores the env var we pass.
async fn run_cli_as(exe: &str, args: &[&str], caller_id: &str) -> String {
    let bin_dir = get_bin_dir();
    let exe_path = bin_dir.join(exe);

    let mut cmd = Command::new(&exe_path);
    cmd.args(args)
        .env("AI_ID", caller_id)
        .env("TEAMENGRAM_V2", "1")
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = match cmd.spawn() {
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

/// Run a teambook CLI command as a specific user
pub async fn teambook_as(args: &[&str], caller_id: &str) -> String {
    run_cli_as(&exe_name("teambook"), args, caller_id).await
}

/// Run a notebook CLI command as a specific user
pub async fn notebook_as(args: &[&str], caller_id: &str) -> String {
    run_cli_as(&exe_name("notebook-cli"), args, caller_id).await
}

