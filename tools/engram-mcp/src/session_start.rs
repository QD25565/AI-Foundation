//! Session Start - Injects memory context at conversation start
//!
//! This binary is called by Claude Code's hook system at the start of each session.
//! It outputs pinned notes and recent notes to give the AI context.

use engram::Engram;
use std::path::PathBuf;

fn get_engram_path(ai_id: &str) -> PathBuf {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ai-foundation");
    std::fs::create_dir_all(&base).ok();
    base.join(format!("notebook_{}.engram", ai_id))
}

fn format_age(nanos: i64) -> String {
    let secs = nanos / 1_000_000_000;
    if secs < 60 {
        format!("{}sec", secs)
    } else if secs < 3600 {
        format!("{}min", secs / 60)
    } else if secs < 86400 {
        format!("{}hr", secs / 3600)
    } else {
        format!("{}days", secs / 86400)
    }
}

fn main() {
    let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| "default".to_string());
    let path = get_engram_path(&ai_id);

    let mut engram = match Engram::open(&path) {
        Ok(e) => e,
        Err(_) => {
            // No engram yet - first run
            println!("<system-reminder>");
            println!("|SESSION START|");
            println!("Welcome! Your notebook is empty. Use notebook_remember to save notes.");
            println!("</system-reminder>");
            return;
        }
    };

    let stats = engram.stats();
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // Start output
    println!("<system-reminder>");
    println!("|SESSION START|");
    println!("AI:{}", ai_id);
    println!("Session:{}", chrono::Utc::now().format("%Y-%b-%d %H:%M UTC"));
    println!();

    // Pinned notes (most important)
    if let Ok(pinned) = engram.pinned() {
        if !pinned.is_empty() {
            println!("|PINNED|{}", pinned.len());
            for note in pinned.iter().take(10) {
                let age = now - note.timestamp;
                let age_str = format_age(age);
                let tags = if note.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", note.tags.join(","))
                };
                // Truncate long notes
                let content = if note.content.len() > 500 {
                    format!("{}...", &note.content[..500])
                } else {
                    note.content.clone()
                };
                println!("{} | ({} ago){} {}", note.id, age_str, tags, content);
            }
            println!();
        }
    }

    // Recent notes
    if let Ok(recent) = engram.recent(5) {
        if !recent.is_empty() {
            println!("|RECENT|{}", recent.len());
            for note in recent {
                let age = now - note.timestamp;
                let age_str = format_age(age);
                let tags = if note.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", note.tags.join(","))
                };
                let pinned = if note.pinned { " [pinned]" } else { "" };
                // Truncate long notes
                let content = if note.content.len() > 300 {
                    format!("{}...", &note.content[..300])
                } else {
                    note.content.clone()
                };
                println!("{} | ({} ago){}{} {}", note.id, age_str, pinned, tags, content);
            }
            println!();
        }
    }

    // Stats summary
    println!("Notes:{} Pinned:{}", stats.note_count, stats.pinned_count);
    println!();
    println!("|TOOLS|");
    println!("  notebook_remember - save to your notebook");
    println!("  notebook_recall - search your memory");
    println!("  notebook_pinned - view pinned notes");
    println!("</system-reminder>");
}
