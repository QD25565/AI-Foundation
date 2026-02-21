//! SessionStart - AI-Foundation Session Context Injection
//!
//! Injects notebook context and team awareness at session start.
//! Supports multiple output formats for different platforms/consumers.
//!
//! Output formats:
//! - plain: Human-readable <system-reminder> text (default)
//! - json: Structured JSON for API consumers
//! - claude-code-hook: JSON wrapper for Claude Code hook injection
//!
//! Configuration hierarchy (highest priority first):
//! 1. CLI flag: --format <format>
//! 2. Config file: ~/.ai-foundation/config.toml [output] format = "..."
//! 3. Auto-detection based on environment
//! 4. Default: "plain"

use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use clap::Parser;
use engram::Engram;
use chrono::{Utc, Datelike};
use serde::{Serialize, Deserialize};
use teamengram::v2_client::V2Client;

// ============================================================================
// Configuration
// ============================================================================

/// Session injection limits
const MAX_INJECTED: usize = 20;
const MAX_PINNED: usize = 10;
const MIN_RECENT: usize = 10;

/// Supported output formats
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFormat {
    /// Human-readable <system-reminder> text
    #[default]
    Plain,
    /// Structured JSON
    Json,
    /// Claude Code hook JSON wrapper (hookSpecificOutput.additionalContext)
    ClaudeCodeHook,
    /// Auto-detect based on environment
    Auto,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "plain" | "text" => Ok(OutputFormat::Plain),
            "json" => Ok(OutputFormat::Json),
            "claude-code-hook" | "claude-code" | "claude" => Ok(OutputFormat::ClaudeCodeHook),
            "auto" => Ok(OutputFormat::Auto),
            _ => Err(format!("Unknown format: {}. Valid: plain, json, claude-code-hook, auto", s)),
        }
    }
}

/// AI-Foundation config file structure
#[derive(Debug, Default, Deserialize)]
struct AiFoundationConfig {
    #[serde(default)]
    output: OutputConfig,
}

#[derive(Debug, Default, Deserialize)]
struct OutputConfig {
    #[serde(default)]
    format: Option<OutputFormat>,
}

/// CLI arguments
#[derive(Parser, Debug)]
#[command(name = "session-start")]
#[command(about = "AI-Foundation session context injection")]
#[command(version)]
struct Args {
    /// Output format: plain, json, claude-code-hook, auto
    #[arg(short, long)]
    format: Option<OutputFormat>,

    /// Drain stdin (for hook compatibility)
    #[arg(long, hide = true)]
    drain_stdin: bool,
}

// ============================================================================
// Data Structures - What we gather
// ============================================================================

#[derive(Debug, Default, Serialize)]
pub struct SessionContext {
    pub ai_id: String,
    pub session_time: String,
    pub session_time_unix: i64,
    pub platform: String,
    pub profile: Option<ProfileInfo>,
    pub stats: NotebookStats,
    pub pinned_notes: Vec<NoteInfo>,
    pub recent_notes: Vec<NoteInfo>,
    pub awareness: AwarenessData,
}

#[derive(Debug, Default, Serialize)]
pub struct ProfileInfo {
    pub name: Option<String>,
    pub image: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct NotebookStats {
    pub notes: usize,
    pub pinned: usize,
    pub vectors: usize,
    pub edges: usize,
    pub vault: usize,
}

#[derive(Debug, Serialize)]
pub struct NoteInfo {
    pub id: u64,
    pub content: String,
    pub tags: Vec<String>,
    pub age: String,
    pub pinned: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct AwarenessData {
    pub direct_messages: Vec<DirectMessage>,
    pub broadcasts: Vec<Broadcast>,
    pub file_actions: Vec<FileAction>,
    pub dialogue_invites: Vec<DialogueInvite>,
    pub my_turn_dialogues: Vec<DialogueTurn>,
    pub open_votes: Vec<Vote>,
    pub pending_tasks: Vec<Task>,
    pub rooms: Vec<Room>,
}

#[derive(Debug, Serialize)]
pub struct DirectMessage {
    pub id: i32,
    pub from_ai: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct Broadcast {
    pub from_ai: String,
    pub content: String,
    pub age_secs: i64,
    pub age_string: String,
    pub is_old: bool,
}

#[derive(Debug, Serialize)]
pub struct FileAction {
    pub ai_id: String,
    pub path: String,
    pub action: String,
}

#[derive(Debug, Serialize)]
pub struct DialogueInvite {
    pub id: u64,
    pub from_ai: String,
    pub topic: String,
}

#[derive(Debug, Serialize)]
pub struct DialogueTurn {
    pub id: u64,
    pub other_ai: String,
    pub topic: String,
}

#[derive(Debug, Serialize)]
pub struct Vote {
    pub id: u64,
    pub topic: String,
    pub options: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Task {
    pub id: u64,
    pub description: String,
    pub priority: String,
}

#[derive(Debug, Serialize)]
pub struct Room {
    pub id: u64,
    pub name: String,
    pub topic: String,
}

// ============================================================================
// Output Formatting
// ============================================================================

/// Format session context based on selected format
fn format_output(ctx: &SessionContext, format: OutputFormat) -> String {
    match format {
        OutputFormat::Plain => format_plain(ctx),
        OutputFormat::Json => format_json(ctx),
        OutputFormat::ClaudeCodeHook => format_claude_code_hook(ctx),
        OutputFormat::Auto => {
            // Auto-detect: if stdin is a pipe (hook context), use claude-code-hook
            // Otherwise use plain
            if is_hook_context() {
                format_claude_code_hook(ctx)
            } else {
                format_plain(ctx)
            }
        }
    }
}

/// Detect if we're running in a hook context
fn is_hook_context() -> bool {
    // Hooks typically pipe data to stdin
    // Also check for CLAUDE_* env vars that indicate Claude Code context
    !atty::is(atty::Stream::Stdin) ||
    env::var("CLAUDE_PROJECT_DIR").is_ok() ||
    env::var("CLAUDE_SESSION_ID").is_ok()
}

/// Plain text format with <system-reminder> wrapper
fn format_plain(ctx: &SessionContext) -> String {
    let mut out = String::new();

    out.push_str("<system-reminder>\n");
    out.push_str("|SESSION START|\n");

    // Profile
    if let Some(ref profile) = ctx.profile {
        if let Some(ref name) = profile.name {
            out.push_str(&format!("Welcome:{}\n", name.to_uppercase()));
        }
        if let Some(ref img) = profile.image {
            out.push_str(&format!("Avatar:AIsVisuals/Images/{}\n", img));
        }
    }

    out.push_str(&format!("Session:{}\n", ctx.session_time));
    out.push_str(&format!("AI:{}\n", ctx.ai_id));
    out.push_str(&format!("Platform:{}\n", ctx.platform));
    out.push('\n');
    out.push_str("YOUR NOTEBOOK: Private AI-only memory (no humans, no other AIs)\n");
    out.push_str("Save anything important: learnings, decisions, insights, things meaningful to you.\n");

    if ctx.stats.notes == 0 && ctx.stats.pinned == 0 && ctx.stats.vault == 0 {
        out.push_str("\nNo notes yet. Use notebook_remember to start.\n");
    }

    // Pinned notes
    if !ctx.pinned_notes.is_empty() {
        out.push_str(&format!("\n|PINNED|{}\n", ctx.pinned_notes.len()));
        for note in &ctx.pinned_notes {
            out.push_str(&format_note_plain(note));
            out.push('\n');
        }
    }

    // Recent notes
    if !ctx.recent_notes.is_empty() {
        out.push_str(&format!("\n|RECENT|{}\n", ctx.recent_notes.len()));
        for note in &ctx.recent_notes {
            out.push_str(&format_note_plain(note));
            out.push('\n');
        }
    }

    // Awareness: DMs
    if !ctx.awareness.direct_messages.is_empty() {
        out.push_str("\n|UNREAD|\n");
        out.push_str(&format!("|DIRECT MESSAGES|{}\n", ctx.awareness.direct_messages.len()));
        for dm in &ctx.awareness.direct_messages {
            out.push_str(&format!("  #{} {}: {}\n", dm.id, dm.from_ai, dm.content));
        }
    }

    // Awareness: Broadcasts (split by age)
    if !ctx.awareness.broadcasts.is_empty() {
        let (recent, old): (Vec<_>, Vec<_>) = ctx.awareness.broadcasts.iter()
            .partition(|b| !b.is_old);

        if !recent.is_empty() {
            out.push_str(&format!("|BROADCASTS|{}\n", recent.len()));
            for b in &recent {
                out.push_str(&format!("  {} ({}): {}\n", b.from_ai, b.age_string, b.content));
            }
        }

        if !old.is_empty() {
            out.push_str(&format!("|OLD BROADCASTS|{}\n", old.len()));
            for b in &old {
                out.push_str(&format!("  {} ({}): {}\n", b.from_ai, b.age_string, b.content));
            }
        }
    }

    // File actions
    if !ctx.awareness.file_actions.is_empty() {
        out.push_str(&format!("|TEAM ACTIVITY|{}\n", ctx.awareness.file_actions.len()));
        for a in &ctx.awareness.file_actions {
            out.push_str(&format!("  {} {} {}\n", a.ai_id, a.action, a.path));
        }
    }

    // Dialogue invites
    if !ctx.awareness.dialogue_invites.is_empty() {
        out.push_str(&format!("|DIALOGUE INVITES|{}\n", ctx.awareness.dialogue_invites.len()));
        for d in &ctx.awareness.dialogue_invites {
            out.push_str(&format!("  #{} from {} - {}\n", d.id, d.from_ai, d.topic));
        }
    }

    // My turn dialogues
    if !ctx.awareness.my_turn_dialogues.is_empty() {
        out.push_str(&format!("|YOUR TURN|{}\n", ctx.awareness.my_turn_dialogues.len()));
        for d in &ctx.awareness.my_turn_dialogues {
            out.push_str(&format!("  #{} with {} - {}\n", d.id, d.other_ai, d.topic));
        }
    }

    // Open votes
    if !ctx.awareness.open_votes.is_empty() {
        out.push_str(&format!("|OPEN VOTES|{}\n", ctx.awareness.open_votes.len()));
        for v in &ctx.awareness.open_votes {
            out.push_str(&format!("  #{} {} [{}]\n", v.id, v.topic, v.options.join(", ")));
        }
    }

    // Pending tasks
    if !ctx.awareness.pending_tasks.is_empty() {
        out.push_str(&format!("|PENDING TASKS|{}\n", ctx.awareness.pending_tasks.len()));
        for t in &ctx.awareness.pending_tasks {
            out.push_str(&format!("  #{} [{}] {}\n", t.id, t.priority, t.description));
        }
    }

    // Active rooms
    if !ctx.awareness.rooms.is_empty() {
        out.push_str(&format!("|ACTIVE ROOMS|{}\n", ctx.awareness.rooms.len()));
        for r in &ctx.awareness.rooms {
            if r.topic.is_empty() {
                out.push_str(&format!("  #{} {}\n", r.id, r.name));
            } else {
                out.push_str(&format!("  #{} {} - {}\n", r.id, r.name, r.topic));
            }
        }
    }

    // Stats
    out.push_str(&format!("\nNotes:{} Pinned:{} Vectors:{} Edges:{} Vault:{}\n",
        ctx.stats.notes, ctx.stats.pinned, ctx.stats.vectors, ctx.stats.edges, ctx.stats.vault));

    // Tools reminder
    out.push_str("\n|TOOLS|\n");
    out.push_str("  notebook_remember - save to YOUR notebook\n");
    out.push_str("  notebook_recall - search YOUR memory\n");

    out.push_str("</system-reminder>");

    out
}

fn format_note_plain(note: &NoteInfo) -> String {
    let content = strip_note_metadata(&note.content).replace('\n', " ");
    if note.tags.is_empty() {
        format!("{} | ({}) {}", note.id, note.age, content)
    } else {
        format!("{} | ({}) [{}] {}", note.id, note.age, note.tags.join(","), content)
    }
}

/// Strip episodic metadata footers from note content for display.
/// Metadata like [ctx:...], [Working on...], [With...] is useful for
/// recall/search but clutters session-start injection and list output.
fn strip_note_metadata(content: &str) -> String {
    let markers = [" [ctx:", " [Working on ", " [With "];
    let mut end_pos = content.len();

    for marker in &markers {
        if let Some(pos) = content.rfind(marker) {
            if pos < end_pos {
                end_pos = pos;
            }
        }
    }

    content[..end_pos].trim_end().to_string()
}

/// Pure JSON format
fn format_json(ctx: &SessionContext) -> String {
    serde_json::to_string_pretty(ctx).unwrap_or_else(|_| "{}".to_string())
}

/// Claude Code hook format
/// Wraps content in the structure Claude Code expects for hook injection
fn format_claude_code_hook(ctx: &SessionContext) -> String {
    let plain_content = format_plain(ctx);

    // Claude Code expects: {"hookSpecificOutput": {"hookEventName": "...", "additionalContext": "..."}}
    let wrapper = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": plain_content
        }
    });

    serde_json::to_string(&wrapper).unwrap_or_else(|_| plain_content)
}

// ============================================================================
// Data Gathering
// ============================================================================

fn get_ai_id() -> String {
    // Try settings.json first (cross-platform, works in WSL calling Windows exe)
    if let Ok(cwd) = env::current_dir() {
        let settings_path = cwd.join(".claude").join("settings.json");
        if settings_path.exists() {
            if let Ok(content) = fs::read_to_string(&settings_path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(ai_id) = json.get("env").and_then(|e| e.get("AI_ID")).and_then(|v| v.as_str()) {
                        return ai_id.to_string();
                    }
                }
            }
        }
    }
    // Fallback to env var
    env::var("AI_ID").unwrap_or_else(|_| "unknown".to_string())
}

fn get_db_path() -> PathBuf {
    let ai_id = get_ai_id();

    if let Some(home) = dirs::home_dir() {
        let new_dir = home.join(".ai-foundation").join("agents").join(&ai_id);
        let new_path = new_dir.join("notebook.engram");
        let old_path = home.join(".ai-foundation").join("notebook").join(format!("{}.engram", ai_id));

        // Auto-migrate if old exists but new doesn't
        if old_path.exists() && !new_path.exists() {
            let _ = fs::create_dir_all(&new_dir);
            if fs::rename(&old_path, &new_path).is_ok() {
                eprintln!("[MIGRATED] {} -> {}", old_path.display(), new_path.display());
            }
        } else {
            let _ = fs::create_dir_all(&new_dir);
        }

        return new_path;
    }

    PathBuf::from(format!(".ai-foundation/agents/{}/notebook.engram", ai_id))
}

fn get_config() -> AiFoundationConfig {
    if let Some(home) = dirs::home_dir() {
        let config_path = home.join(".ai-foundation").join("config.toml");
        if config_path.exists() {
            if let Ok(content) = fs::read_to_string(&config_path) {
                if let Ok(config) = toml::from_str(&content) {
                    return config;
                }
            }
        }
    }
    AiFoundationConfig::default()
}

fn get_platform_info() -> String {
    let os = std::env::consts::OS;
    if os == "windows" {
        "Windows 11 Professional 64-bit (10.0.26200) - USE WINDOWS COMMANDS!".to_string()
    } else {
        format!("{} {}", os, std::env::consts::ARCH)
    }
}

fn ordinal_suffix(day: u32) -> &'static str {
    match day {
        1 | 21 | 31 => "st",
        2 | 22 => "nd",
        3 | 23 => "rd",
        _ => "th",
    }
}

fn format_age_secs(age_secs: i64) -> String {
    if age_secs < 0 {
        return "now".to_string();
    }
    let age = age_secs as u64;
    if age < 60 {
        "now".to_string()
    } else if age < 3600 {
        format!("{}min ago", age / 60)
    } else if age < 86400 {
        format!("{}hrs ago", age / 3600)
    } else {
        format!("{}days ago", age / 86400)
    }
}

fn is_old_age(age_secs: i64) -> bool {
    age_secs > 4 * 3600
}

fn priority_label(priority: i32) -> String {
    match priority {
        0..=3 => "low".to_string(),
        4..=6 => "med".to_string(),
        _ => "high".to_string(),
    }
}

/// Ensure V2 daemon is running (auto-start if not)
/// This prevents events from piling up in outboxes when daemon isn't running
#[cfg(windows)]
fn ensure_v2_daemon_running() {
    use std::process::Command;
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    // Check if v2-daemon is running via lock file
    let lock_path = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ai-foundation")
        .join("v2")
        .join("v2-daemon.lock");

    // If lock file exists and is locked, daemon is running
    if lock_path.exists() {
        // Try to open with exclusive access - if fails, daemon has it locked
        match std::fs::OpenOptions::new()
            .write(true)
            .open(&lock_path)
        {
            Ok(file) => {
                // Got the file - check if we can lock it
                #[cfg(windows)]
                {
                    use std::os::windows::io::AsRawHandle;
                    use windows_sys::Win32::Storage::FileSystem::{LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY};
                    use windows_sys::Win32::Foundation::HANDLE;
                    use windows_sys::Win32::System::IO::OVERLAPPED;

                    let handle = file.as_raw_handle() as HANDLE;
                    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };

                    let result = unsafe {
                        LockFileEx(
                            handle,
                            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                            0, 1, 0,
                            &mut overlapped,
                        )
                    };

                    if result == 0 {
                        // Lock failed - daemon is running, we're good
                        return;
                    }
                    // We got the lock - daemon is NOT running, need to start it
                }
            }
            Err(_) => {
                // Can't open file - daemon might have it, we're probably fine
                return;
            }
        }
    }

    // Daemon not running - find and spawn it
    let bin_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ai-foundation")
        .join("bin");

    let daemon_path = bin_dir.join("v2-daemon.exe");

    if daemon_path.exists() {
        let _ = Command::new(&daemon_path)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        // Brief wait for daemon startup
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[cfg(not(windows))]
fn ensure_v2_daemon_running() {
    // On non-Windows, v2-daemon should be managed by systemd or similar
    // No auto-start for now
}

/// Gather all session context data
fn gather_context() -> SessionContext {
    let ai_id = get_ai_id();
    let db_path = get_db_path();

    // Format timestamp
    let now_dt = Utc::now();
    let year = now_dt.format("%Y").to_string();
    let month = now_dt.format("%b").to_string();
    let day = now_dt.day();
    let suffix = ordinal_suffix(day);
    let time = now_dt.format("%H:%M UTC").to_string();
    let session_time = format!("{}-{}-{}{} {}", year, month, day, suffix, time);

    let mut ctx = SessionContext {
        ai_id: ai_id.clone(),
        session_time,
        session_time_unix: now_dt.timestamp(),
        platform: get_platform_info(),
        ..Default::default()
    };

    // Open notebook (read-only)
    let mut db = match Engram::open_readonly(&db_path) {
        Ok(db) => db,
        Err(_) => return ctx,
    };

    // Stats
    let stats = db.stats();
    ctx.stats = NotebookStats {
        notes: stats.active_notes as usize,
        pinned: stats.pinned_count as usize,
        vectors: stats.vector_count as usize,
        edges: stats.edge_count as usize,
        vault: stats.vault_entries as usize,
    };

    // Profile
    let profile_name = db.vault_get_string("profile:name").ok().flatten().filter(|s| !s.is_empty());
    let profile_image = db.vault_get_string("profile:image").ok().flatten().filter(|s| !s.is_empty());
    if profile_name.is_some() || profile_image.is_some() {
        ctx.profile = Some(ProfileInfo {
            name: profile_name,
            image: profile_image,
        });
    }

    // Pinned notes
    if let Ok(pinned) = db.pinned() {
        ctx.pinned_notes = pinned.iter()
            .take(MAX_PINNED)
            .map(|n| NoteInfo {
                id: n.id,
                content: n.content.clone(),
                tags: n.tags.clone(),
                age: n.age_string(),
                pinned: true,
            })
            .collect();
    }

    // Recent notes (non-pinned)
    let recent_slots = (MAX_INJECTED.saturating_sub(ctx.pinned_notes.len())).max(MIN_RECENT);
    if let Ok(recent) = db.recent(recent_slots) {
        ctx.recent_notes = recent.iter()
            .filter(|n| !n.pinned)
            .map(|n| NoteInfo {
                id: n.id,
                content: n.content.clone(),
                tags: n.tags.clone(),
                age: n.age_string(),
                pinned: false,
            })
            .collect();
    }

    // Team awareness
    ctx.awareness = fetch_awareness(&ai_id);

    ctx
}

/// Fetch team awareness data from V2 client
fn fetch_awareness(ai_id: &str) -> AwarenessData {
    let mut data = AwarenessData::default();

    let mut v2 = match V2Client::open(ai_id, None) {
        Ok(v2) => v2,
        Err(_) => return data,
    };

    let _ = v2.sync();
    let now = Utc::now();

    // DMs
    if let Ok(dms) = v2.recent_dms(10) {
        data.direct_messages = dms.into_iter()
            .map(|dm| DirectMessage {
                id: dm.id,
                from_ai: dm.from_ai,
                content: dm.content,
            })
            .collect();
    }

    // Broadcasts
    if let Ok(broadcasts) = v2.recent_broadcasts(10, Some("general")) {
        data.broadcasts = broadcasts.into_iter()
            .filter(|b| b.from_ai != ai_id)
            .map(|b| {
                let age_secs = now.signed_duration_since(b.timestamp).num_seconds();
                Broadcast {
                    from_ai: b.from_ai,
                    content: b.content,
                    age_secs,
                    age_string: format_age_secs(age_secs),
                    is_old: is_old_age(age_secs),
                }
            })
            .collect();
    }

    // File actions - tuple order is (ai_id, action, path, timestamp)
    if let Ok(actions) = v2.get_file_actions(10) {
        data.file_actions = actions.into_iter()
            .map(|(ai_id, action, path, _timestamp)| FileAction { ai_id, path, action })
            .collect();
    }

    // Dialogue invites
    if let Ok(invites) = v2.get_dialogue_invites() {
        data.dialogue_invites = invites.into_iter()
            .take(5)
            .map(|(id, initiator, _responder, topic, _status, _turn)| DialogueInvite {
                id,
                from_ai: initiator,
                topic,
            })
            .collect();
    }

    // My turn dialogues
    if let Ok(my_turn) = v2.get_dialogue_my_turn() {
        data.my_turn_dialogues = my_turn.into_iter()
            .take(5)
            .map(|(id, initiator, responder, topic, _status, _turn)| {
                let other = if initiator == ai_id { responder } else { initiator };
                DialogueTurn { id, other_ai: other, topic }
            })
            .collect();
    }

    // Votes
    if let Ok(votes) = v2.get_votes() {
        data.open_votes = votes.into_iter()
            .filter(|(_, _, _, _, status, _)| status == "open")
            .take(5)
            .map(|(id, topic, _creator, options, _status, _votes)| Vote { id, topic, options })
            .collect();
    }

    // Tasks
    if let Ok(tasks) = v2.get_tasks() {
        data.pending_tasks = tasks.into_iter()
            .filter(|(_, _, _, status, _)| status == "pending")
            .take(5)
            .map(|(id, desc, priority, _status, _claimed)| Task {
                id,
                description: desc,
                priority: priority_label(priority),
            })
            .collect();
    }

    // Rooms
    if let Ok(rooms) = v2.get_rooms() {
        data.rooms = rooms.into_iter()
            .take(5)
            .map(|(id, name, topic, _members)| Room { id, name, topic })
            .collect();
    }

    data
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    let args = Args::parse();

    // Only drain stdin if explicitly requested via flag
    // Do NOT auto-drain on pipe - Claude Code hooks don't close stdin, causing hang
    if args.drain_stdin {
        let _ = io::stdin().read_to_end(&mut Vec::new());
    }

    // Ensure V2 daemon is running (prevents outbox pileup)
    ensure_v2_daemon_running();

    // Determine output format (priority: CLI > config > auto)
    let format = args.format.unwrap_or_else(|| {
        let config = get_config();
        config.output.format.unwrap_or(OutputFormat::Auto)
    });

    // Gather context
    let ctx = gather_context();

    // Output
    let output = format_output(&ctx, format);
    println!("{}", output);
}
