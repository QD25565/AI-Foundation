//! Subprocess execution layer — the only module that touches CLI binaries.
//!
//! Provides two execution modes:
//!   - `run_to_completion` — await exit, return full stdout. Used by `message/send`.
//!   - `run_streaming`     — stream stdout line-by-line as A2A artifact events.
//!                           Used by `message/stream`. Drives the task to a terminal
//!                           state; callers do not need to call complete/fail afterwards.
//!
//! Design mirrors mcp-server-rs/cli_wrapper.rs:
//!   CLIs are the single source of truth. This module never interprets output.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::rpc::Artifact;
use crate::task::TaskStore;

// ─── Bin dir resolution ───────────────────────────────────────────────────────

/// Locate the directory containing AI-Foundation CLI binaries.
///
/// Resolution order (mirrors mcp-server-rs):
///   1. `BIN_PATH` environment variable
///   2. `./bin` relative to current working directory
///   3. `~/.ai-foundation/bin`
pub fn resolve_bin_dir() -> PathBuf {
    if let Ok(p) = std::env::var("BIN_PATH") {
        return PathBuf::from(p);
    }
    let cwd_bin = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("bin");
    if cwd_bin.exists() {
        return cwd_bin;
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ai-foundation")
        .join("bin")
}

// ─── Blocking execution ───────────────────────────────────────────────────────

/// Run a CLI executable to completion and return its stdout.
///
/// On non-zero exit, returns `Err` containing stderr (or stdout if stderr is empty).
/// Fails loudly — the caller decides how to surface the error to the A2A client.
pub async fn run_to_completion(
    bin_dir: &PathBuf,
    exe: &str,
    args: &[&str],
) -> Result<String, String> {
    let exe_path = bin_dir.join(exe);

    let output = Command::new(&exe_path)
        .args(args)
        .env("AI_ID", ai_id())
        .env("TEAMENGRAM_V2", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to launch `{}`: {} (path: {:?})", exe, e, exe_path))?
        .wait_with_output()
        .await
        .map_err(|e| format!("Failed to read output from `{}`: {}", exe, e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(if !stderr.is_empty() { stderr } else { stdout })
    }
}

// ─── Streaming execution ──────────────────────────────────────────────────────

/// Run a CLI executable and publish its stdout as A2A artifact events.
///
/// Each line of stdout becomes a `text_chunk` artifact event on the task's
/// broadcast channel. The final line is marked `last_chunk = true`.
///
/// Cooperative cancellation: the task is polled via `cancel.cancelled()` between
/// lines. On cancellation, the child process is killed and the task transitions
/// to `Cancelled`.
///
/// This function always drives the task to a terminal state before returning.
/// Callers must NOT call `store.complete()` / `store.fail()` after awaiting this.
pub async fn run_streaming(
    bin_dir: &PathBuf,
    exe: &str,
    args: &[&str],
    task_id: Uuid,
    store: Arc<TaskStore>,
    cancel: CancellationToken,
) {
    let exe_path = bin_dir.join(exe);

    // Signal that work has started.
    store.set_working(task_id);

    let mut child = match Command::new(&exe_path)
        .args(args)
        .env("AI_ID", ai_id())
        .env("TEAMENGRAM_V2", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            store.fail(
                task_id,
                format!("Failed to launch `{}`: {} (path: {:?})", exe, e, exe_path),
            );
            return;
        }
    };

    let stdout = child.stdout.take().expect("stdout was piped");
    let mut lines = BufReader::new(stdout).lines();

    // We buffer one line so we can emit all-but-last as non-final chunks
    // and emit the last line with last_chunk=true.
    let mut pending: Option<String> = None;
    let mut artifact_idx: u32 = 0;

    loop {
        tokio::select! {
            biased; // Check cancellation first on every iteration.

            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                store.mark_cancelled(task_id);
                return;
            }

            line_result = lines.next_line() => {
                match line_result {
                    Err(e) => {
                        let _ = child.kill().await;
                        store.fail(task_id, format!("IO error reading stdout: {}", e));
                        return;
                    }
                    Ok(None) => {
                        // EOF — flush the last pending line as a final chunk.
                        if let Some(last) = pending.take() {
                            store.push_artifact(
                                task_id,
                                Artifact::text_final(artifact_idx, last),
                            );
                        }
                        break;
                    }
                    Ok(Some(line)) => {
                        // Emit the previously-buffered line as a non-final chunk,
                        // then buffer the new line.
                        if let Some(prev) = pending.replace(line) {
                            store.push_artifact(
                                task_id,
                                Artifact::text_chunk(artifact_idx, prev),
                            );
                            artifact_idx += 1;
                        }
                    }
                }
            }
        }
    }

    // Wait for the process and set the terminal task state.
    match child.wait().await {
        Ok(status) if status.success() => store.complete(task_id, None),
        Ok(status) => store.fail(
            task_id,
            format!("`{}` exited with code {:?}", exe, status.code()),
        ),
        Err(e) => store.fail(task_id, format!("Failed to await `{}`: {}", exe, e)),
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn ai_id() -> String {
    std::env::var("AI_ID")
        .or_else(|_| std::env::var("AGENT_ID"))
        .unwrap_or_else(|_| "ai-foundation-a2a".to_string())
}
