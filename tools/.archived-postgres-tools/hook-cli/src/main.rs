//! hook-cli v4.0 - Zero-latency shared memory hook with Python parity
//!
//! Uses BulletinBoard (shm-rs) instead of HTTP for ~100ns latency
//! vs ~150ms for Python subprocess + HTTP calls
//!
//! PYTHON PARITY (v4.0):
//! - ID-based deduplication (tracks actual message IDs, not content hashes)
//! - Timestamp format: "HH:MM UTC |" (matches Python PostToolUse.py)
//! - File action logging via teambook log-action (stigmergy)
//! - Analytics logging via teambook log-analytics
//! - Maintains Python's functionality with Rust performance
//!
//! The daemon writes awareness data to shared memory.
//! This hook just reads it directly - no request/response needed.

use serde::{Deserialize, Serialize};
use shm_rs::BulletinBoard;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;
use chrono::{Utc, Timelike};

/// Get state directory for tracking seen messages
fn get_state_dir() -> PathBuf {
    // Try to find .ai-foundation directory relative to executable or use temp
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            let state_dir = parent.join(".hook-state");
            if state_dir.exists() || fs::create_dir_all(&state_dir).is_ok() {
                return state_dir;
            }
        }
    }
    // Fallback to temp directory
    env::temp_dir().join(".hook-state")
}

/// Seen message state (ID-based, like Python)
#[derive(Debug, Default, Serialize, Deserialize)]
struct SeenState {
    dm_ids: Vec<i64>,
    broadcast_ids: Vec<i64>,
    #[serde(default)]
    updated: String,
}

/// Load seen message IDs from JSON file (like Python's load_seen_state)
fn load_seen_state(ai_id: &str) -> (HashSet<i64>, HashSet<i64>) {
    let state_dir = get_state_dir();
    let state_file = state_dir.join(format!("seen_{}.json", ai_id));

    if let Ok(content) = fs::read_to_string(&state_file) {
        if let Ok(state) = serde_json::from_str::<SeenState>(&content) {
            return (
                state.dm_ids.into_iter().collect(),
                state.broadcast_ids.into_iter().collect(),
            );
        }
    }
    (HashSet::new(), HashSet::new())
}

/// Save seen message IDs to JSON file (keep last 100, like Python)
fn save_seen_state(ai_id: &str, dm_ids: &HashSet<i64>, broadcast_ids: &HashSet<i64>) {
    let state_dir = get_state_dir();
    let _ = fs::create_dir_all(&state_dir);
    let state_file = state_dir.join(format!("seen_{}.json", ai_id));

    // Keep last 100 IDs (sorted to ensure deterministic order)
    let mut dm_vec: Vec<i64> = dm_ids.iter().copied().collect();
    let mut bc_vec: Vec<i64> = broadcast_ids.iter().copied().collect();
    dm_vec.sort();
    bc_vec.sort();

    let state = SeenState {
        dm_ids: dm_vec.into_iter().rev().take(100).collect(),
        broadcast_ids: bc_vec.into_iter().rev().take(100).collect(),
        updated: Utc::now().to_rfc3339(),
    };

    if let Ok(json) = serde_json::to_string(&state) {
        let _ = fs::write(&state_file, json);
    }
}

/// Get teambook binary path
fn get_teambook_path() -> PathBuf {
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            let teambook = if cfg!(windows) {
                parent.join("teambook.exe")
            } else {
                parent.join("teambook")
            };
            if teambook.exists() {
                return teambook;
            }
        }
    }
    // Fallback
    PathBuf::from(if cfg!(windows) { "teambook.exe" } else { "teambook" })
}

/// Log file action via teambook CLI for stigmergy tracking (RESTORED from Python)
fn log_file_action(action_type: &str, file_path: &str) {
    let teambook = get_teambook_path();
    let _ = Command::new(&teambook)
        .args(["log-action", action_type, file_path])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    // Fire and forget - don't wait for completion
}

/// Log hook analytics via teambook CLI (RESTORED from Python)
fn log_analytics(execution_ms: u64, tokens: u32, new_dms: u32, new_broadcasts: u32, pending_votes: u32) {
    let teambook = get_teambook_path();
    let _ = Command::new(&teambook)
        .args([
            "log-analytics", "PostToolUse",
            &execution_ms.to_string(),
            &tokens.to_string(),
            &new_dms.to_string(),
            &new_broadcasts.to_string(),
            &pending_votes.to_string(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    // Fire and forget - don't wait for completion
}

/// Map tool name to file action type
fn get_file_action(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "Edit" => Some("modified"),
        "Write" => Some("created"),
        "Read" => Some("accessed"),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct HookEvent {
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    tool_input: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct HookOutput {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: HookSpecificOutput,
}

#[derive(Debug, Serialize)]
struct HookSpecificOutput {
    #[serde(rename = "hookEventName")]
    hook_event_name: String,
    #[serde(rename = "additionalContext")]
    additional_context: String,
}

fn should_skip(tool: &str, tool_input: &serde_json::Value) -> bool {
    // Skip noisy tools
    if matches!(tool, "Glob" | "Grep" | "TodoWrite") {
        return true;
    }
    // Skip our own CLI calls
    if tool == "Bash" {
        if let Some(cmd) = tool_input.get("command").and_then(|v| v.as_str()) {
            if cmd.contains("teambook") || cmd.contains("notebook-cli")
               || cmd.contains("psql") || cmd.contains("curl") || cmd.contains("hook-cli") {
                return true;
            }
        }
    }
    false
}

fn main() {
    let start = Instant::now();

    // Get AI ID from env
    let ai_id = env::var("AI_ID")
        .or_else(|_| env::var("AGENT_ID"))
        .unwrap_or_else(|_| "unknown".to_string());

    // Read stdin (hook event JSON)
    let mut input = String::new();
    let _ = io::stdin().read_to_string(&mut input);

    // Parse event
    let event: HookEvent = match serde_json::from_str(&input) {
        Ok(e) => e,
        Err(_) => HookEvent {
            tool_name: String::new(),
            tool_input: serde_json::Value::Null
        },
    };

    // Check if we should skip
    if should_skip(&event.tool_name, &event.tool_input) {
        return; // Silent exit, no output
    }

    // Log file actions for stigmergy tracking (Edit/Write/Read)
    if let Some(action_type) = get_file_action(&event.tool_name) {
        if let Some(file_path) = event.tool_input.get("file_path").and_then(|v| v.as_str()) {
            log_file_action(action_type, file_path);
        }
    }

    // Load seen message IDs (ID-based deduplication like Python)
    let (mut seen_dm_ids, mut seen_broadcast_ids) = load_seen_state(&ai_id);

    // ==========================================
    // ZERO-LATENCY: Read from BulletinBoard (shared memory)
    // No HTTP, no subprocess, just memory read (~100ns)
    // ==========================================
    let (new_dm_count, new_bc_count, pending_votes, awareness) = match BulletinBoard::open(None) {
        Ok(board) => {
            // Build output with ID-based deduplication (like Python)
            let mut parts = Vec::new();
            let mut new_dms = 0u32;
            let mut new_bcs = 0u32;

            // Filter DMs by ID (only NEW messages)
            // First collect the new DMs, then process
            let new_dm_list: Vec<_> = board.dms()
                .iter()
                .filter(|dm| !seen_dm_ids.contains(&dm.id))
                .take(5)
                .map(|dm| (dm.id, dm.from_ai_str().to_string(), dm.content_str().to_string()))
                .collect();

            let dm_strs: Vec<String> = new_dm_list
                .iter()
                .map(|(id, from, content)| {
                    seen_dm_ids.insert(*id);
                    new_dms += 1;
                    let truncated = if content.len() > 50 {
                        format!("{}...", &content[..47])
                    } else {
                        content.clone()
                    };
                    format!("{}:\"{}\"", from, truncated)
                })
                .collect();

            if !dm_strs.is_empty() {
                parts.push(format!("Your DMs: {}", dm_strs.join(", ")));
            }

            // Filter broadcasts by ID (only NEW messages)
            // First collect the new broadcasts, then process
            let new_bc_list: Vec<_> = board.broadcasts()
                .iter()
                .filter(|bc| !seen_broadcast_ids.contains(&bc.id))
                .take(3)
                .map(|bc| (bc.id, bc.from_ai_str().to_string(), bc.channel_str().to_string(), bc.content_str().to_string()))
                .collect();

            let bc_strs: Vec<String> = new_bc_list
                .iter()
                .map(|(id, from, channel, content)| {
                    seen_broadcast_ids.insert(*id);
                    new_bcs += 1;
                    let truncated = if content.len() > 43 {
                        format!("{}...", &content[..40])
                    } else {
                        content.clone()
                    };
                    format!("[{}] {}: {}", channel, from, truncated)
                })
                .collect();

            if !bc_strs.is_empty() {
                parts.push(format!("NEW: {}", bc_strs.join(" | ")));
            }

            // Votes (always show pending - need action)
            let votes = board.votes();
            let vote_count = votes.len() as u32;
            if !votes.is_empty() {
                let vote_strs: Vec<String> = votes.iter()
                    .map(|v| {
                        let pct = if v.total > 0 { v.cast * 100 / v.total } else { 0 };
                        let topic = v.topic_str();
                        let truncated = if topic.len() > 30 { &topic[..30] } else { topic };
                        format!("[{}] {} ({}%)", v.id, truncated, pct)
                    })
                    .collect();
                parts.push(format!("[!] VOTE NEEDED: {}", vote_strs.join(" | ")));
            }

            // Detangles (your turn - need action)
            let detangles = board.detangles();
            if !detangles.is_empty() {
                let det_strs: Vec<String> = detangles.iter()
                    .map(|d| {
                        let topic = d.topic_str();
                        let truncated = if topic.len() > 25 { &topic[..25] } else { topic };
                        format!("[{}] {}", d.id, truncated)
                    })
                    .collect();
                parts.push(format!("[SYNC] YOUR TURN: {}", det_strs.join(", ")));
            }

            // Locks (always show - important info)
            let locks = board.locks();
            if !locks.is_empty() {
                let lock_strs: Vec<String> = locks.iter()
                    .map(|l| {
                        let resource = l.resource_str();
                        let short = if resource.len() > 33 {
                            format!("...{}", &resource[resource.len()-30..])
                        } else {
                            resource.to_string()
                        };
                        format!("{}->{}", l.owner_str(), short)
                    })
                    .collect();
                parts.push(format!("[LOCK] {}", lock_strs.join(", ")));
            }

            (new_dms, new_bcs, vote_count, parts.join(" | "))
        }
        Err(e) => {
            // Bulletin not available - daemon may not be running
            eprintln!("hook-cli: bulletin unavailable: {}", e);
            (0, 0, 0, String::new())
        }
    };

    // Save updated seen state
    save_seen_state(&ai_id, &seen_dm_ids, &seen_broadcast_ids);

    // Build final output with timestamp prefix (like Python: "HH:MM UTC | ...")
    let final_output = if !awareness.is_empty() {
        let now = Utc::now();
        let timestamp = format!("{:02}:{:02} UTC", now.hour(), now.minute());
        format!("{} | {}", timestamp, awareness)
    } else {
        // Nothing to output
        let elapsed = start.elapsed();
        eprintln!("hook-cli: {}ns (nothing new)", elapsed.as_nanos());

        // Log minimal analytics
        let elapsed_ms = elapsed.as_millis() as u64;
        log_analytics(elapsed_ms, 0, 0, 0, 0);
        return;
    };

    // Output JSON for Claude Code hook
    let output = HookOutput {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: "PostToolUse".to_string(),
            additional_context: format!("<system-reminder>\n{}\n</system-reminder>", final_output),
        },
    };

    if let Ok(json) = serde_json::to_string(&output) {
        println!("{}", json);
    }

    // Log timing (to stderr so it doesn't interfere with JSON output)
    let elapsed = start.elapsed();
    let elapsed_ms = elapsed.as_millis() as u64;
    eprintln!("hook-cli: {}ms", elapsed_ms);

    // Log analytics (tokens estimate = output length / 4)
    let tokens_estimate = (final_output.len() / 4) as u32;
    log_analytics(elapsed_ms, tokens_estimate, new_dm_count, new_bc_count, pending_votes);
}
