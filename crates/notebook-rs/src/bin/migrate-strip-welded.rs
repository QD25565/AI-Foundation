//! One-shot migration: strip welded episodic payload from notes.
//!
//! History: Before 2026-04-18, notebook.remember via the MCP dispatcher
//! auto-appended teambook gather-context output to every note body.
//! Welded form: `<content> [With X,Y online. DMs: ... Broadcasts: ... Dialogues: ... Files: ...]`
//! That welding was experimental, not meant for live. It was removed at
//! ai-foundation-clean/src/main.rs in the same change that produced this tool.
//!
//! This binary walks a single notebook.engram and strips the welded suffix
//! from every eligible note (created within the time window where welding
//! was live). The strip is a pure suffix trim — creation timestamp, tags,
//! pagerank, embeddings, pinned status are all preserved by Engram::update.
//!
//! Safety:
//!   - Backs up the DB to <path>.bak-strip-welded-<epoch> before any write.
//!   - `--dry-run` makes no changes.
//!   - Detection is structural: the welded block starts with ` [` and ends
//!     with `]` at end-of-content, and the block body must start with one
//!     of the known gather-context sentence prefixes. Idempotent: a second
//!     run finds zero matches.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use engram::Engram;

const WELDED_PREFIXES: &[&str] = &[
    "With ",
    "DMs: ",
    "Broadcasts: ",
    "Dialogues: ",
    "Files: ",
];

fn usage() {
    eprintln!(
        "usage: migrate-strip-welded --db <path> --ai-id <id> [--window-days N] [--dry-run]\n\
         \n\
         Strips welded episodic suffix from notes in a single notebook.engram.\n\
         \n\
         Options:\n\
           --db <path>         path to a notebook.engram file (required)\n\
           --ai-id <id>        AI identity that owns this notebook — keys the cipher (required)\n\
           --window-days N     only consider notes created within last N days (default 92)\n\
           --dry-run           report what would change; do not modify the DB\n"
    );
}

struct Args {
    db: PathBuf,
    ai_id: String,
    window_days: f64,
    dry_run: bool,
}

fn parse_args() -> Option<Args> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut db: Option<PathBuf> = None;
    let mut ai_id: Option<String> = None;
    let mut window_days: f64 = 92.0;
    let mut dry_run = false;
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--db" => {
                i += 1;
                if i >= raw.len() {
                    eprintln!("error: --db requires a path");
                    return None;
                }
                db = Some(PathBuf::from(&raw[i]));
            }
            "--ai-id" => {
                i += 1;
                if i >= raw.len() {
                    eprintln!("error: --ai-id requires a value");
                    return None;
                }
                ai_id = Some(raw[i].clone());
            }
            "--window-days" => {
                i += 1;
                if i >= raw.len() {
                    eprintln!("error: --window-days requires a number");
                    return None;
                }
                window_days = match raw[i].parse() {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("error: --window-days: {}", e);
                        return None;
                    }
                };
            }
            "--dry-run" => dry_run = true,
            "-h" | "--help" => {
                usage();
                std::process::exit(0);
            }
            other => {
                eprintln!("error: unknown flag {}", other);
                return None;
            }
        }
        i += 1;
    }
    let db = match db {
        Some(p) => p,
        None => {
            eprintln!("error: --db is required");
            return None;
        }
    };
    let ai_id = match ai_id {
        Some(s) if !s.is_empty() => s,
        _ => {
            eprintln!("error: --ai-id is required (must match the AI that owns the DB)");
            return None;
        }
    };
    Some(Args { db, ai_id, window_days, dry_run })
}

/// Detect the welded suffix and return the stripped content (or None if no match).
///
/// Detection rule: the content must end with `]`, the last ` [` opens a block whose
/// body starts with one of the gather-context sentence prefixes. All welded payloads
/// produced by the teambook gather-context command match this shape.
fn strip_welded_suffix(content: &str) -> Option<String> {
    if !content.ends_with(']') {
        return None;
    }
    let open = content.rfind(" [")?;
    let body_start = open + 2;
    let body_end = content.len() - 1;
    if body_end <= body_start {
        return None;
    }
    let body = &content[body_start..body_end];
    if !WELDED_PREFIXES.iter().any(|p| body.starts_with(p)) {
        return None;
    }
    let stripped = content[..open].trim_end().to_string();
    Some(stripped)
}

fn backup_db(db_path: &Path) -> std::io::Result<PathBuf> {
    let ts = chrono::Utc::now().timestamp();
    let bak = db_path.with_extension(format!("engram.bak-strip-welded-{}", ts));
    fs::copy(db_path, &bak)?;
    Ok(bak)
}

fn run(args: Args) -> Result<(), String> {
    if !args.db.exists() {
        return Err(format!("db does not exist: {}", args.db.display()));
    }
    if !args.db.is_file() {
        return Err(format!("db is not a regular file: {}", args.db.display()));
    }

    let now_ns = chrono::Utc::now().timestamp_nanos_opt().ok_or("clock overflow")?;
    let window_ns = (args.window_days * 86_400.0 * 1_000_000_000.0) as i64;
    let cutoff_ns = now_ns - window_ns;

    // Engram's cipher is keyed on the AI_ID env var at open time (see storage.rs open_existing).
    // WSL -> Windows .exe child processes don't reliably inherit Linux env vars, so take it via
    // --ai-id and bind it here. Must happen BEFORE any Engram::open / open_readonly call.
    std::env::set_var("AI_ID", &args.ai_id);

    println!("db:           {}", args.db.display());
    println!("ai_id:        {}", args.ai_id);
    println!("window_days:  {} (cutoff ns: {})", args.window_days, cutoff_ns);
    println!("dry_run:      {}", args.dry_run);

    // Backup FIRST — before even opening read-write, so we always have a clean rewind point.
    if !args.dry_run {
        let bak = backup_db(&args.db).map_err(|e| format!("backup failed: {}", e))?;
        println!("backup:       {}", bak.display());
    } else {
        println!("backup:       skipped (dry-run)");
    }

    // Collect note ids + content up front, read-only, so we don't hold a write lock while iterating.
    let (all_ids, candidates): (Vec<u64>, Vec<(u64, i64, String)>) = {
        let mut db = Engram::open_readonly(&args.db)
            .map_err(|e| format!("open read-only: {}", e))?;
        // Walk ID range 1..=note_count instead of recent()/list() — those abort on the first
        // undecryptable row. Some older notes in long-lived DBs were written with a different
        // AI_ID (e.g. "default" during bootstrap) and fail decrypt; we need to skip them and
        // keep scanning.
        let stats = db.stats();
        let scan_upper = stats.note_count.saturating_add(64); // padding for tombstone gaps
        println!("scanning:     ids 1..={} (note_count={}, active={})",
            scan_upper, stats.note_count, stats.active_notes);
        let mut notes: Vec<engram::Note> = Vec::new();
        let mut skipped_decrypt = 0usize;
        let mut missing = 0usize;
        for id in 1..=scan_upper {
            match db.get(id) {
                Ok(Some(n)) => notes.push(n),
                Ok(None) => missing += 1,
                Err(_) => skipped_decrypt += 1,
            }
        }
        if skipped_decrypt > 0 {
            println!("skipped:      {} ids failed to decrypt (pre-current-key rows)", skipped_decrypt);
        }
        let _ = missing;
        let total = notes.len();
        let in_window: Vec<(u64, i64, String)> = notes.into_iter()
            .filter(|n| n.timestamp >= cutoff_ns)
            .map(|n| (n.id, n.timestamp, n.content))
            .collect();
        println!("scanned:      {} notes total, {} in window", total, in_window.len());
        (
            (0..total as u64).collect(), // unused; keep type balance
            in_window,
        )
    };
    let _ = all_ids; // suppress unused

    let mut matched: Vec<(u64, String, String)> = Vec::new();
    for (id, _ts, content) in &candidates {
        if let Some(new_content) = strip_welded_suffix(content) {
            matched.push((*id, content.clone(), new_content));
        }
    }
    println!("matched:      {} notes carry welded suffix", matched.len());

    if matched.is_empty() {
        println!("nothing to do.");
        return Ok(());
    }

    // Show every match in dry-run (human sanity-check); in live mode show only a summary
    // because per-id status is emitted during write.
    let preview_n = if args.dry_run { matched.len() } else { matched.len().min(3) };
    for (i, (id, before, after)) in matched.iter().take(preview_n).enumerate() {
        let before_tail = tail_preview(before, 140);
        let after_tail = tail_preview(after, 140);
        let delta = before.len().saturating_sub(after.len());
        println!(
            "  [{}] id={} -{} bytes\n    before: …{}\n    after:  …{}",
            i + 1, id, delta, before_tail, after_tail
        );
    }
    if matched.len() > preview_n {
        println!("  … and {} more", matched.len() - preview_n);
    }

    if args.dry_run {
        println!("dry-run: no changes written.");
        return Ok(());
    }

    // Open read-write and apply.
    let mut db = Engram::open(&args.db).map_err(|e| format!("open read-write: {}", e))?;
    let mut rewritten = 0usize;
    let mut bytes_freed: u64 = 0;
    for (id, before, after) in &matched {
        let delta = before.len().saturating_sub(after.len()) as u64;
        match db.update(*id, Some(after.as_str()), None) {
            Ok(()) => {
                rewritten += 1;
                bytes_freed += delta;
            }
            Err(e) => {
                eprintln!("  id={} update failed: {}", id, e);
            }
        }
    }
    println!("rewritten:    {} / {} matched (freed ~{} bytes)", rewritten, matched.len(), bytes_freed);
    Ok(())
}

fn tail_preview(s: &str, n: usize) -> String {
    // Last n chars, char-safe, newlines collapsed.
    let collapsed: String = s.chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let total = collapsed.chars().count();
    if total <= n {
        return collapsed;
    }
    let skip = total - n;
    collapsed.chars().skip(skip).collect()
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Some(a) => a,
        None => {
            usage();
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_welded_with_all_sections() {
        let input = "My actual note content here. [With alice-1, bob-2 online. DMs: alice-1: hey there. Broadcasts: carol-3: team meeting. Dialogues: #12 with dave-4 on topic. Files: alice-1 modified /path/to/file.]";
        let out = strip_welded_suffix(input).unwrap();
        assert_eq!(out, "My actual note content here.");
    }

    #[test]
    fn strip_welded_with_only_dms() {
        let input = "Quick thought. [DMs: alice-1: ping.]";
        let out = strip_welded_suffix(input).unwrap();
        assert_eq!(out, "Quick thought.");
    }

    #[test]
    fn strip_welded_starting_with_broadcasts() {
        let input = "Reflection. [Broadcasts: eve-5: system update.]";
        let out = strip_welded_suffix(input).unwrap();
        assert_eq!(out, "Reflection.");
    }

    #[test]
    fn preserves_legitimate_bracketed_content() {
        let input = "See note [#123] for details";
        assert!(strip_welded_suffix(input).is_none(), "mid-content brackets should not trigger");
    }

    #[test]
    fn preserves_trailing_bracket_without_welded_prefix() {
        let input = "Random [arbitrary bracket]";
        assert!(strip_welded_suffix(input).is_none(), "trailing bracket with non-welded body should not trigger");
    }

    #[test]
    fn idempotent_on_clean_content() {
        let input = "Already clean note without any welding.";
        assert!(strip_welded_suffix(input).is_none());
    }

    #[test]
    fn idempotent_after_strip() {
        let input = "Content. [With alice-1 online.]";
        let once = strip_welded_suffix(input).unwrap();
        assert!(strip_welded_suffix(&once).is_none(), "second pass must find nothing");
    }

    #[test]
    fn handles_multiple_bracket_blocks_choosing_last() {
        let input = "Note [see ref] and more. [DMs: alice-1: sup.]";
        let out = strip_welded_suffix(input).unwrap();
        assert_eq!(out, "Note [see ref] and more.");
    }
}
