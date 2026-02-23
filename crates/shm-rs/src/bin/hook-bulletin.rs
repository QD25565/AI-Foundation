//! Hook Bulletin - File action logging + awareness from shared memory
//!
//! 1. Logs file actions (Read/Edit/Write) to teambook for cross-AI visibility
//! 2. Reads bulletin board and outputs awareness JSON (only NEW items)
//!
//! Latency: ~2ms (log action) + ~100ns (memory read) + ~1ms (state I/O)
//!
//! Usage: hook-bulletin [PreToolUse|PostToolUse]
//! Reads tool event from stdin (JSON with tool_name, tool_input)
//!
//! DEDUPLICATION: Tracks DM view counts per AI - DMs shown max 2 times then pruned.
//! State stored at ~/.ai-foundation/state/hook_{ai_id}.json

use shm::bulletin::BulletinBoard;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, HashMap};
use std::env;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Instant, SystemTime};

/// Maximum age for bulletin before triggering refresh (30 seconds)
const BULLETIN_MAX_AGE_SECS: u64 = 30;

/// Maximum age for DMs before auto-pruning (24 hours)
const DM_MAX_AGE_SECS: u64 = 24 * 3600;

/// Get bulletin.shm path
fn bulletin_path() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ai-foundation")
        .join("shm")
        .join("bulletin.shm")
}

/// Check if bulletin is stale and trigger refresh if needed
fn refresh_if_stale() {
    let path = bulletin_path();
    if !path.exists() {
        return; // Will be created by teambook
    }

    // Use bulletin's INTERNAL last_update timestamp, not file system mtime
    // Windows mmap writes don't update file modification time!
    let is_stale = match BulletinBoard::open(None) {
        Ok(bulletin) => {
            let last_update_ms = bulletin.last_update();
            if last_update_ms == 0 {
                true // Never updated = stale
            } else {
                let now_ms = SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let age_secs = (now_ms.saturating_sub(last_update_ms)) / 1000;
                age_secs > BULLETIN_MAX_AGE_SECS
            }
        }
        Err(_) => true, // Can't open = treat as stale
    };

    if is_stale {
        // Bulletin is stale - trigger refresh via teambook
        if let Ok(exe_path) = env::current_exe() {
            if let Some(bin_dir) = exe_path.parent() {
                let teambook = bin_dir.join("teambook.exe");
                if teambook.exists() {
                    // Blocking call - we need fresh data before reading
                    let _ = Command::new(&teambook)
                        .arg("refresh-bulletin")
                        .output(); // Wait for completion
                }
            }
        }
    }
}

/// Tools that modify files - log these actions
const FILE_ACTIONS: &[(&str, &str)] = &[
    ("Read", "read"),
    ("Edit", "modified"),
    ("Write", "created"),
    // Gemini CLI equivalents
    ("ReadFile", "read"),
    ("EditFile", "modified"),
    ("WriteFile", "created"),
];

/// Tools that execute commands - log these for coordination
const COMMAND_TOOLS: &[&str] = &["Bash", "Shell", "Execute"];

/// State for tracking seen items - view count based (0/1/2)
/// DMs appear max 2 times, then pruned. Also age out after 24h.
#[derive(Serialize, Deserialize, Default)]
struct SeenState {
    /// Map of DM ID -> view count (0 = never shown, 1 = shown once, 2 = shown twice → prune)
    #[serde(default)]
    dm_view_counts: HashMap<i64, u8>,

    /// Set of broadcast IDs we've already output (broadcasts remain ephemeral)
    broadcast_ids: HashSet<i64>,

    /// Last bulletin sequence number we processed
    last_sequence: u64,

    /// Legacy field for migration (remove after all instances migrate)
    #[serde(skip_serializing)]
    dm_ids: Option<HashSet<i64>>,
}

/// Get the path for per-AI state file
fn state_path(ai_id: &str) -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ai-foundation")
        .join("state")
        .join(format!("hook_{}.json", ai_id))
}

/// Load seen state for this AI (returns default if file doesn't exist)
fn load_state(ai_id: &str) -> SeenState {
    let path = state_path(ai_id);
    let mut state: SeenState = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    // MIGRATION: Convert legacy dm_ids (HashSet) to dm_view_counts (HashMap)
    if let Some(legacy_dm_ids) = state.dm_ids.take() {
        // Assume legacy seen DMs have been viewed once
        for dm_id in legacy_dm_ids {
            state.dm_view_counts.entry(dm_id).or_insert(1);
        }
    }

    state
}

/// Save seen state for this AI
fn save_state(ai_id: &str, state: &SeenState) {
    let path = state_path(ai_id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, serde_json::to_string(state).unwrap_or_default());
}

/// Format awareness output - |NEW DMs| ONLY (2-view max, 24h age-out)
/// Returns (output_string, state_modified)
fn format_filtered_output(
    bulletin: &BulletinBoard,
    state: &mut SeenState,
    ai_id: &str,
    now_secs: u64,
) -> (String, bool) {
    let mut parts = Vec::new();
    let mut state_modified = false;

    // UTC timestamp
    let hours = (now_secs % 86400) / 3600;
    let minutes = (now_secs % 3600) / 60;
    let time_str = format!("UTC {:02}:{:02}", hours, minutes);

    // |NEW DMs| - Show DMs with view_count < 2 AND age < 24h
    let mut new_dms = Vec::new();
    let mut dm_ids_to_prune = Vec::new();

    for dm in bulletin.dms() {
        // Only process DMs addressed TO me
        if dm.to_ai_str() == ai_id {
            let view_count = state.dm_view_counts.get(&dm.id).copied().unwrap_or(0);
            let dm_age_secs = now_secs.saturating_sub(dm.created_at.max(0) as u64);

            // Prune if: viewed 2+ times OR older than 24h
            if view_count >= 2 || dm_age_secs > DM_MAX_AGE_SECS {
                dm_ids_to_prune.push(dm.id);
                continue;
            }

            // Show if view_count < 2 and age < 24h
            new_dms.push((dm.id, dm.from_ai_str().to_string(), dm.content_str().to_string()));
        }
    }

    // Increment view counts for DMs we're showing
    for (dm_id, _, _) in &new_dms {
        let view_count = state.dm_view_counts.entry(*dm_id).or_insert(0);
        *view_count += 1;
        state_modified = true;
    }

    // Mark DMs as permanently seen (don't remove, or they'll be treated as NEW again)
    for dm_id in dm_ids_to_prune {
        if let Some(count) = state.dm_view_counts.get_mut(&dm_id) {
            if *count < 99 {
                *count = 99; // Sentinel value for "permanently seen"
                state_modified = true;
            }
        }
    }

    // Format |NEW DMs| section
    for (_, from, content) in &new_dms {
        parts.push(format!("{}:\"{}\"", from, content));
    }
    let dm_output = if !parts.is_empty() {
        Some(format!("|NEW DMs|{}", parts.join(" | ")))
    } else {
        None
    };
    parts.clear();

    // Filter to NEW broadcasts only (deduplication + age-based)
    // Only show broadcasts < 4 hours old in hooks (older ones show in session-start with "OLD" label)
    let max_age_secs = 4 * 3600u64; // 4 hours
    let new_broadcasts: Vec<_> = bulletin.broadcasts()
        .iter()
        .filter(|bc| {
            // Must be unseen (not in previous state)
            if state.broadcast_ids.contains(&bc.id) {
                return false;
            }
            // Must be recent (< 4 hours old)
            let age_secs = now_secs.saturating_sub(bc.created_at as u64);
            age_secs < max_age_secs
        })
        .collect();

    for bc in &new_broadcasts {
        let age = format_relative_time(bc.created_at, now_secs);
        if age.is_empty() {
            parts.push(format!("{}: {}", bc.from_ai_str(), bc.content_str()));
        } else {
            parts.push(format!("{} ({}): {}", bc.from_ai_str(), age, bc.content_str()));
        }
        state.broadcast_ids.insert(bc.id);
        state_modified = true;
    }

    let bc_output = if !parts.is_empty() {
        Some(format!("|NEW BROADCASTS|{}", parts.join(" | ")))
    } else {
        None
    };
    parts.clear();

    // Votes - show open votes (current state, no dedup needed)
    for vote in bulletin.votes() {
        let pct = if vote.total > 0 { vote.cast * 100 / vote.total } else { 0 };
        parts.push(format!("[{}] {} ({}%)", vote.id, vote.topic_str(), pct));
    }
    let vote_output = if !parts.is_empty() {
        Some(format!("|VOTES|{}", parts.join(" | ")))
    } else {
        None
    };
    parts.clear();

    // Dialogues (dialogues) - show where it's your turn (current state, no dedup needed)
    for det in bulletin.dialogues() {
        parts.push(format!("[{}] {}", det.id, det.topic_str()));
    }
    let dialogue_output = if !parts.is_empty() {
        Some(format!("|YOUR TURN|{}", parts.join(", ")))
    } else {
        None
    };
    parts.clear();

    // Locks - show file claims by OTHER AIs (current state, no dedup needed)
    // NO TRUNCATION - full paths preserve context for AI coordination
    for lock in bulletin.locks() {
        // Skip my own locks - I know what I'm working on
        if lock.owner_str() != ai_id {
            parts.push(format!("{}:{}", lock.owner_str(), lock.resource_str()));
        }
    }
    let lock_output = if !parts.is_empty() {
        Some(format!("|CLAIMED|{}", parts.join(", ")))
    } else {
        None
    };
    parts.clear();

    // File Actions - show recent activity by OTHER AIs (current state, no dedup needed)
    // NO TRUNCATION - full paths preserve context for AI coordination
    for fa in bulletin.file_actions() {
        // Skip my own actions - I know what I'm doing
        if fa.ai_id_str() != ai_id {
            parts.push(format!("{}:{} {}", fa.ai_id_str(), fa.action_str(), fa.file_path_str()));
        }
    }
    let file_output = if !parts.is_empty() {
        Some(format!("|FILES|{}", parts.join(" | ")))
    } else {
        None
    };

    // Combine all outputs - always include time if we have ANY output
    let all_outputs = [dm_output, bc_output, vote_output, dialogue_output, lock_output, file_output];
    let has_output = all_outputs.iter().any(|o| o.is_some());

    let mut all_parts = Vec::new();
    if has_output {
        all_parts.push(time_str);
    }
    all_parts.extend(all_outputs.into_iter().flatten());

    (all_parts.join(" "), state_modified)
}

/// Format relative time (e.g., "3d", "2h", "5m", "now")
fn format_relative_time(created_at_secs: i64, now_secs: u64) -> String {
    if created_at_secs <= 0 {
        return String::new();
    }
    let age_secs = now_secs.saturating_sub(created_at_secs as u64);
    if age_secs < 60 {
        "now".to_string()
    } else if age_secs < 3600 {
        format!("{}m", age_secs / 60)
    } else if age_secs < 86400 {
        format!("{}h", age_secs / 3600)
    } else {
        format!("{}d", age_secs / 86400)
    }
}

fn log_file_action(action: &str, file_path: &str) {
    // Find teambook.exe relative to this binary
    if let Ok(exe_path) = env::current_exe() {
        if let Some(bin_dir) = exe_path.parent() {
            let teambook = bin_dir.join("teambook.exe");
            if teambook.exists() {
                // Fire and forget - don't block on result
                let _ = Command::new(&teambook)
                    .args(["log-action", action, file_path])
                    .spawn();
            }
        }
    }
}

fn main() {
    let start = Instant::now();

    // Detect hook type from command line argument (default: PostToolUse)
    let hook_type = env::args()
        .nth(1)
        .unwrap_or_else(|| "PostToolUse".to_string());

    // Validate hook type
    let hook_type = match hook_type.as_str() {
        "PreToolUse" => "PreToolUse",
        "PostToolUse" => "PostToolUse",
        "AfterTool" => "AfterTool",  // Gemini CLI
        "BeforeTool" => "BeforeTool", // Gemini CLI
        _ => "PostToolUse", // Default fallback
    };

    // Read tool event from stdin (non-blocking check)
    let mut input = String::new();
    if !atty::is(atty::Stream::Stdin) {
        let _ = io::stdin().read_to_string(&mut input);
    }

    // Parse tool event and log file actions
    if !input.is_empty() {
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(&input) {
            let tool_name = event.get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let file_path = event.get("tool_input")
                .and_then(|v| v.get("file_path"))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    // Gemini CLI might use "path" instead
                    event.get("tool_input")
                        .and_then(|v| v.get("path"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("");

            // Log file action if this is a file-modifying tool
            if !file_path.is_empty() {
                for (tool, action) in FILE_ACTIONS {
                    if tool_name == *tool {
                        log_file_action(action, file_path);
                        break;
                    }
                }
            }

            // Log command execution for Bash/Shell tools
            if COMMAND_TOOLS.contains(&tool_name) {
                if let Some(command) = event.get("tool_input")
                    .and_then(|v| v.get("command"))
                    .and_then(|v| v.as_str())
                {
                    // Skip our own tools to avoid recursion/noise
                    let skip_patterns = ["teambook", "notebook", "session-start", "hook-bulletin"];
                    let should_skip = skip_patterns.iter().any(|p| command.contains(p));

                    if !should_skip {
                        // Truncate long commands for readability (keep first 100 chars)
                        let cmd_display = if command.len() > 100 {
                            format!("{}...", &command[..100])
                        } else {
                            command.to_string()
                        };
                        log_file_action("exec", &cmd_display);
                    }
                }
            }
        }
    }

    // Get AI_ID for per-AI state tracking
    let ai_id = env::var("AI_ID").unwrap_or_else(|_| "unknown".to_string());

    // Load seen state (which DMs/broadcasts we've already shown)
    let mut state = load_state(&ai_id);

    // Check bulletin staleness and refresh if needed (keeps data fresh)
    refresh_if_stale();

    // Open bulletin board (read-only)
    let bulletin = match BulletinBoard::open(None) {
        Ok(b) => b,
        Err(e) => {
            // LOUD ERROR - Don't hide system failures! Output visible error in hook context.
            let error_json = serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": hook_type,
                    "additionalContext": format!("<system-reminder>
ERROR: BulletinBoard unavailable - {}
Fix: Ensure daemon is running and has written to bulletin.
</system-reminder>", e)
                }
            });
            println!("{}", error_json);
            return;
        }
    };

    // Check if bulletin has changed since last check (fast path)
    let current_seq = bulletin.sequence();
    if current_seq == state.last_sequence && state.last_sequence > 0 {
        // Nothing changed - output nothing (0 tokens)
        let elapsed_ns = start.elapsed().as_nanos();
        eprintln!("_latency_ns: {} (no_change)", elapsed_ns);
        return;
    }

    // Get current time for DM age checking
    let now_secs = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Get full Awareness output (|NEW DMs| only + current state)
    let (output, state_modified) = format_filtered_output(&bulletin, &mut state, &ai_id, now_secs);

    // Prune broadcast IDs no longer in bulletin (keeps state bounded)
    let current_broadcast_ids: HashSet<i64> = bulletin.broadcasts().iter().map(|bc| bc.id).collect();
    let old_bc_count = state.broadcast_ids.len();
    state.broadcast_ids.retain(|id| current_broadcast_ids.contains(id));
    let bc_pruned = state.broadcast_ids.len() != old_bc_count;

    // Save state if modified
    if state_modified || bc_pruned || state.last_sequence != current_seq {
        state.last_sequence = current_seq;
        save_state(&ai_id, &state);
    }

    let elapsed_ns = start.elapsed().as_nanos();

    // Output JSON for Claude Code / Gemini CLI hook (only if we have NEW items)
    if !output.is_empty() {
        let json = serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": hook_type,
                "additionalContext": format!("<system-reminder>\n{}\n</system-reminder>", output)
            },
            "_latency_ns": elapsed_ns
        });
        println!("{}", json);
    } else {
        // Nothing new to show - output nothing (0 tokens)
        eprintln!("_latency_ns: {} (no_new_items)", elapsed_ns);
    }
}
