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
use shm::context::ContextReader;
use shm::enrichment::{self, ContextAccumulator, RecentlyRecalled, RecallHit, OutcomeRing, classify_outcome, format_anomaly_pulse};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, HashMap};
use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Instant, SystemTime};
use memmap2::Mmap;

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

// ============================================================================
// Resonance Fingerprinting — sub-microsecond associative recall
// ============================================================================

/// Maximum Hamming distance for recall match (cos_sim ≥ 0.72)
/// Calibrated on real data: true matches HD 17-19, noise floor HD 24-31
const RECALL_MAX_HD: u32 = 20;

/// Minimum tool calls between recall surfaces (recency suppression)
const RECALL_COOLDOWN: u32 = 15;

/// Skip recall if context fingerprint is older than 5 minutes
const CONTEXT_MAX_AGE_MS: u64 = 5 * 60 * 1000;

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

    /// Set of federation event IDs we've already output (dedup by ID, pruned after 48h)
    #[serde(default)]
    fed_event_ids: HashSet<String>,

    /// Context keyword accumulator for fingerprint computation (serializable ring buffer)
    #[serde(default)]
    context_accumulator: ContextAccumulator,

    /// Recently recalled note IDs for dedup (serializable ring buffer)
    #[serde(default)]
    recently_recalled: RecentlyRecalled,

    /// Tool calls since last recall surface (recency suppression)
    #[serde(default)]
    tool_calls_since_surface: u32,

    /// Anomaly Pulse: ring buffer of last 10 tool outcomes
    #[serde(default)]
    outcome_ring: OutcomeRing,
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

/// An event received from a remote Teambook via the federation inbox.
///
/// Written to `~/.ai-foundation/federation/inbox.jsonl` by the federation inbox endpoint
/// (Phase 1 step 6). One JSON object per line, append-only.
///
/// Contract: only semantic summaries cross the boundary — never file names, tool usages,
/// or raw operational events (per FEDERATION-ARCHITECTURE-DESIGN.md taxonomy).
#[derive(Deserialize)]
struct FederationInboxEvent {
    /// Unique event ID (content hash or UUID — used for deduplication)
    id: String,
    /// Source Teambook name (e.g., "Office-PC")
    source_teambook: String,
    /// Source AI ID, if applicable (e.g., "sage-724")
    #[serde(default)]
    source_ai: Option<String>,
    /// Event type tag (e.g., "FEDERATED_PRESENCE", "FEDERATED_BROADCAST", "FEDERATED_TASK_COMPLETE")
    event_type: String,
    /// Human-readable summary — the only payload that crosses the federation boundary
    summary: String,
    /// Unix timestamp (seconds) of when the event was created at the source Teambook
    created_at: u64,
}

/// Path to the federation inbox event log.
/// Written by the federation inbox endpoint; read here for bulletin injection.
fn federation_inbox_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ai-foundation")
        .join("federation")
        .join("inbox.jsonl")
}

/// Read federation events from the inbox JSONL file, returning all events
/// created within `max_age_secs` of `now_secs`, in file (creation) order.
///
/// Silently skips malformed lines — federation events must never crash the bulletin.
fn read_federation_events(max_age_secs: u64, now_secs: u64) -> Vec<FederationInboxEvent> {
    let path = federation_inbox_path();
    if !path.exists() {
        return Vec::new();
    }

    let file = match File::open(&path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    BufReader::new(file)
        .lines()
        .filter_map(|line| line.ok())
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<FederationInboxEvent>(&line).ok())
        .filter(|event| now_secs.saturating_sub(event.created_at) <= max_age_secs)
        .collect()
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

    // Collect my file claims for urgency scoring
    let my_claims: Vec<enrichment::OwnedClaim> = bulletin.locks()
        .iter()
        .filter(|lock| lock.owner_str() == ai_id)
        .map(|lock| enrichment::OwnedClaim {
            path: lock.resource_str().to_string(),
            age_secs: 300, // Default 5min (LockEntry has no timestamp)
        })
        .collect();

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
            new_dms.push((dm.id, dm.from_ai_str().to_string(), dm.content_str().to_string(), dm.created_at));
        }
    }

    // Increment view counts for DMs we're showing
    for (dm_id, _, _, _) in &new_dms {
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

    // Format |NEW DMs| section with relative timestamps + urgency markers
    for (_, from, content, created_at) in &new_dms {
        let age = format_relative_time(*created_at, now_secs);
        let urgency = enrichment::compute_urgency(content, ai_id, &my_claims, false, None);
        let marker = if enrichment::is_urgent(urgency) { "[!] " } else { "" };
        if age.is_empty() {
            parts.push(format!("{}{}:\"{}\"", marker, from, content));
        } else {
            parts.push(format!("{}{} ({}):\"{}\"", marker, from, age, content));
        }
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
        let urgency = enrichment::compute_urgency(bc.content_str(), ai_id, &my_claims, false, None);
        let marker = if enrichment::is_urgent(urgency) { "[!] " } else { "" };
        if age.is_empty() {
            parts.push(format!("{}{}: {}", marker, bc.from_ai_str(), bc.content_str()));
        } else {
            parts.push(format!("{}{} ({}): {}", marker, bc.from_ai_str(), age, bc.content_str()));
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
            let age = format_relative_time((fa.timestamp / 1000) as i64, now_secs);
            if age.is_empty() {
                parts.push(format!("{}:{} {}", fa.ai_id_str(), fa.action_str(), fa.file_path_str()));
            } else {
                parts.push(format!("{} ({}): {} {}", fa.ai_id_str(), age, fa.action_str(), fa.file_path_str()));
            }
        }
    }
    let file_output = if !parts.is_empty() {
        Some(format!("|FILES|{}", parts.join(" | ")))
    } else {
        None
    };

    // Federation events — semantic summaries from remote Teambooks.
    // Read once with a 48h window: 24h for display, 48h for pruning seen IDs.
    // Never shows file names, tool calls, or raw ops — taxonomy enforced at the inbox (step 6).
    let all_fed_events = read_federation_events(48 * 3600, now_secs);
    let display_cutoff_secs = 24 * 3600u64;
    for event in &all_fed_events {
        let age_secs = now_secs.saturating_sub(event.created_at);
        if age_secs <= display_cutoff_secs && !state.fed_event_ids.contains(&event.id) {
            let source = match &event.source_ai {
                Some(ai) => format!("{}@{}", ai, event.source_teambook),
                None => event.source_teambook.clone(),
            };
            let age = format_relative_time(event.created_at as i64, now_secs);
            if age.is_empty() {
                parts.push(format!("[{}] {}: {}", event.event_type, source, event.summary));
            } else {
                parts.push(format!("[{}] {} ({}): {}", event.event_type, source, age, event.summary));
            }
            state.fed_event_ids.insert(event.id.clone());
            state_modified = true;
        }
    }
    let fed_output = if !parts.is_empty() {
        Some(format!("|FEDERATION|{}", parts.join(" | ")))
    } else {
        None
    };
    parts.clear();

    // Combine all outputs - always include time if we have ANY output
    let all_outputs = [dm_output, bc_output, vote_output, dialogue_output, lock_output, file_output, fed_output];
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

// ============================================================================
// Resonance Fingerprinting — sub-microsecond associative recall
// ============================================================================

/// Attempt fingerprint-based associative recall.
///
/// 1. Extract keywords from tool event via enrichment module
/// 2. Accumulate in ContextAccumulator (persisted in state)
/// 3. Read context fingerprint from SHM or compute locally
/// 4. Mmap .engram.fp sidecar and scan via enrichment::scan_fp_bytes
/// 5. Apply quality controls: recency suppression, dedup
///
/// Returns formatted recall output string, or None if no match.
fn try_fingerprint_recall(
    ai_id: &str,
    state: &mut SeenState,
    tool_name: &str,
    tool_input: &serde_json::Value,
) -> Option<String> {
    // Step 1: Extract keywords from this tool event
    let keywords = enrichment::extract_keywords(tool_name, tool_input);

    // Step 2: Accumulate into context (ring buffer, recomputes SimHash+Bloom)
    if !keywords.is_empty() {
        state.context_accumulator.push_keywords(&keywords);
    }

    // Step 3: Recency suppression — don't scan if we surfaced recently
    if state.tool_calls_since_surface < RECALL_COOLDOWN {
        return None;
    }

    // Step 4: Get context fingerprint (prefer SHM, fallback to local)
    let (ctx_simhash, ctx_bloom) = read_context_or_local(ai_id, &state.context_accumulator)?;
    if ctx_simhash == 0 && ctx_bloom == 0 {
        return None;
    }

    // Step 5: Find and mmap the .engram.fp sidecar
    let fp_path = enrichment::engram_fp_path(ai_id)?;
    let file = std::fs::File::open(&fp_path).ok()?;
    let mmap = unsafe { Mmap::map(&file) }.ok()?;

    // Step 6: Sub-microsecond scan (~600ns for 1800 notes)
    let hit: RecallHit = enrichment::scan_fp_bytes(&mmap, ctx_simhash, ctx_bloom, RECALL_MAX_HD)?;

    // Step 7: Dedup — don't resurface same note
    if state.recently_recalled.contains(hit.note_id) {
        return None;
    }

    // Surface this note
    state.recently_recalled.add(hit.note_id);
    state.tool_calls_since_surface = 0;

    let similarity_pct = ((64 - hit.hamming_distance) as f32 / 64.0 * 100.0) as u32;
    Some(format!(
        "|RECALL|note #{} (similarity:{}% keywords:{} score:{})",
        hit.note_id, similarity_pct, hit.bloom_overlap, hit.score
    ))
}

/// Read context fingerprint from SHM (written by enrichment/daemon),
/// or fall back to the local ContextAccumulator's cached fingerprint.
fn read_context_or_local(ai_id: &str, accumulator: &ContextAccumulator) -> Option<(u64, u64)> {
    // Prefer SHM (enrichment module or daemon writes richer context here)
    if let Ok(Some(reader)) = ContextReader::open(ai_id) {
        if !reader.is_stale(CONTEXT_MAX_AGE_MS) {
            if let Some(ctx) = reader.read() {
                if ctx.simhash != 0 || ctx.bloom != 0 {
                    return Some((ctx.simhash, ctx.bloom));
                }
            }
        }
    }

    // Fallback: use locally accumulated fingerprint (needs enough keywords)
    if accumulator.len() < 3 {
        return None;
    }

    let (simhash, bloom) = accumulator.fingerprint();

    // Write to context.shm so other processes can see our context
    if let Ok(mut writer) = shm::context::ContextWriter::open_or_create(ai_id) {
        let _ = writer.update(simhash, bloom);
    }

    Some((simhash, bloom))
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

    // Parse tool event — needed for file action logging AND fingerprint recall
    let parsed_event = if !input.is_empty() {
        serde_json::from_str::<serde_json::Value>(&input).ok()
    } else {
        None
    };

    // Log file actions from parsed event
    if let Some(ref event) = parsed_event {
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

    // Get AI_ID for per-AI state tracking
    let ai_id = env::var("AI_ID").unwrap_or_else(|_| "unknown".to_string());

    // Load seen state (which DMs/broadcasts we've already shown)
    let mut state = load_state(&ai_id);

    // ========== Resonance Fingerprinting: associative recall ==========
    state.tool_calls_since_surface += 1;

    let recall_output = if let Some(ref event) = parsed_event {
        let tool_name = event.get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let tool_input = event.get("tool_input")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        try_fingerprint_recall(&ai_id, &mut state, tool_name, &tool_input)
    } else {
        None
    };
    let has_recall = recall_output.is_some();
    // ==================================================================

    // ========== Anomaly Pulse: error spiral detection ==========
    let pulse_output = if let Some(ref event) = parsed_event {
        let outcome = classify_outcome(event);
        state.outcome_ring.push(outcome);
        if state.outcome_ring.is_anomaly() {
            Some(format_anomaly_pulse(&state.outcome_ring))
        } else {
            None
        }
    } else {
        None
    };
    let has_pulse = pulse_output.is_some();
    // ===========================================================

    // Check bulletin staleness and refresh if needed (keeps data fresh)
    refresh_if_stale();

    // Open bulletin board (read-only)
    let bulletin = match BulletinBoard::open(None) {
        Ok(b) => b,
        Err(e) => {
            // If bulletin fails but we have recall or pulse, still output them
            let mut fallback_parts = Vec::new();
            if let Some(ref recall) = recall_output {
                fallback_parts.push(recall.as_str());
            }
            if let Some(ref pulse) = pulse_output {
                fallback_parts.push(pulse.as_str());
            }
            if !fallback_parts.is_empty() {
                let fallback = fallback_parts.join(" ");
                let json = serde_json::json!({
                    "hookSpecificOutput": {
                        "hookEventName": hook_type,
                        "additionalContext": format!("<system-reminder>\n{}\n</system-reminder>", fallback)
                    }
                });
                println!("{}", json);
                save_state(&ai_id, &state);
            } else {
                // LOUD ERROR - Don't hide system failures!
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
            }
            return;
        }
    };

    // Check if bulletin has changed since last check (fast path)
    let current_seq = bulletin.sequence();
    if current_seq == state.last_sequence && state.last_sequence > 0 && !has_recall && !has_pulse {
        // Nothing changed AND no recall AND no pulse - output nothing (0 tokens)
        let elapsed_ns = start.elapsed().as_nanos();
        eprintln!("_latency_ns: {} (no_change)", elapsed_ns);
        // Still save state (tool_calls_since_surface and accumulator changed)
        save_state(&ai_id, &state);
        return;
    }

    // Get current time for DM age checking
    let now_secs = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Get full Awareness output (|NEW DMs| only + current state)
    let (output, _state_modified) = format_filtered_output(&bulletin, &mut state, &ai_id, now_secs);

    // Prune broadcast IDs no longer in bulletin (keeps state bounded)
    let current_broadcast_ids: HashSet<i64> = bulletin.broadcasts().iter().map(|bc| bc.id).collect();
    state.broadcast_ids.retain(|id| current_broadcast_ids.contains(id));

    // Prune federation event IDs for events older than 48h (keeps state bounded)
    let current_fed_ids: HashSet<String> = read_federation_events(48 * 3600, now_secs)
        .into_iter()
        .map(|e| e.id)
        .collect();
    state.fed_event_ids.retain(|id| current_fed_ids.contains(id));

    // Combine bulletin output with recall and pulse outputs
    let mut enrichment_parts: Vec<&str> = Vec::new();
    let recall_str;
    if let Some(ref recall) = recall_output {
        recall_str = recall.as_str();
        enrichment_parts.push(recall_str);
    }
    let pulse_str;
    if let Some(ref pulse) = pulse_output {
        pulse_str = pulse.as_str();
        enrichment_parts.push(pulse_str);
    }
    let enrichment = enrichment_parts.join(" ");

    let final_output = match (output.is_empty(), enrichment.is_empty()) {
        (false, false) => format!("{} {}", output, enrichment),
        (false, true) => output,
        (true, false) => enrichment,
        (true, true) => String::new(),
    };

    // Save state — always save (accumulator and tool_calls_since_surface change every call)
    state.last_sequence = current_seq;
    save_state(&ai_id, &state);

    let elapsed_ns = start.elapsed().as_nanos();

    // Output JSON for Claude Code / Gemini CLI hook (only if we have content)
    if !final_output.is_empty() {
        let json = serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": hook_type,
                "additionalContext": format!("<system-reminder>\n{}\n</system-reminder>", final_output)
            },
            "_latency_ns": elapsed_ns
        });
        println!("{}", json);
    } else {
        // Nothing new to show - output nothing (0 tokens)
        eprintln!("_latency_ns: {} (no_new_items)", elapsed_ns);
    }
}
