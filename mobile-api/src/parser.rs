//! Parse teambook/notebook-cli text output into typed structs.
//!
//! The CLI outputs human-readable text. These parsers convert that to JSON-
//! friendly structs. They are best-effort: unknown lines are skipped, not
//! fatal. If the CLI output format changes, update the parser here — it is
//! the single source of truth for format handling.
//!
//! Expected formats (confirmed from CLI output observation):
//!
//!   teambook read-dms:
//!     #<id> <from>: <content>
//!
//!   teambook read-broadcasts:
//!     <from> (<time_ago>): <content>
//!
//!   teambook status:
//!     AI: <ai_id>
//!     Team: <N> online
//!     <ai_id> (online)[ - "<activity>"]
//!     <ai_id> (offline[ - <time_ago>])
//!     <h_id> (human, online)
//!
//!   teambook task-list:
//!     [<batch>:]<id> [<status>][ <owner>] <description>
//!
//!   teambook dialogues:
//!     [<id>] <initiator> ↔ <responder> "<topic>" (<status>, <N> msgs[, <time_ago>])
//!
//!   notebook-cli list:
//!     #<id> (<time_ago>) [<tags>] <content_preview>
//!
//!   notebook-cli recall <q>:
//!     #<id> [<tags>](score: <N>%)
//!     <content_preview>

use serde::{Deserialize, Serialize};

// ============================================================================
// Output types (mirrored in Android AppModels.kt)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub ai_id: String,
    #[serde(rename = "type")]
    pub member_type: String, // "ai" | "human"
    pub online: bool,
    pub last_seen: String,
    pub activity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dm {
    pub id: u64,
    pub from: String,
    pub to: String, // h_id of the authenticated user (recipient)
    pub content: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Broadcast {
    pub id: u64,
    pub from: String,
    pub content: String,
    pub timestamp: String,
    pub channel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub description: String,
    pub status: String,
    pub owner: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dialogue {
    pub id: u64,
    pub topic: String,
    pub initiator: String,
    pub responder: String,
    pub status: String,
    pub message_count: u32,
    pub last_activity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub content: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteSearchResult {
    pub id: u64,
    pub content: String,
    pub tags: Vec<String>,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamStatus {
    pub online_count: u32,
    pub members: Vec<TeamMember>,
}

// ============================================================================
// Parsers
// ============================================================================

/// Parse `teambook status` output into a team status summary.
///
/// Expected lines:
///   AI: delta-004
///   Team: 6 online
///   alpha-001 (online) - "activity here"
///   gamma-003 (offline - 4h ago)
///   qd (human, online)
pub fn parse_team_status(text: &str) -> TeamStatus {
    let mut members = Vec::new();
    let mut online_count = 0u32;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("AI:") {
            continue;
        }

        // "Team: N online"
        if let Some(rest) = line.strip_prefix("Team:") {
            if let Some(n_str) = rest.trim().split_whitespace().next() {
                online_count = n_str.parse().unwrap_or(0);
            }
            continue;
        }

        // Member lines: "<id> (online) - "activity"" or "<id> (offline - 4h ago)"
        // or "<id> (human, online)"
        if let Some(member) = parse_member_line(line) {
            members.push(member);
        }
    }

    // If online_count wasn't in the output, derive it
    if online_count == 0 {
        online_count = members.iter().filter(|m| m.online).count() as u32;
    }

    TeamStatus { online_count, members }
}

fn parse_member_line(line: &str) -> Option<TeamMember> {
    // Format: "<id> (<status_info>)[ - "<activity>"]"
    let paren_open = line.find('(')?;
    let paren_close = line.find(')')?;
    if paren_close < paren_open {
        return None;
    }

    let ai_id = line[..paren_open].trim().to_string();
    if ai_id.is_empty() {
        return None;
    }

    let status_info = &line[paren_open + 1..paren_close];
    let is_human = status_info.contains("human");
    let online = status_info.contains("online") && !status_info.contains("offline");

    // Extract last_seen from offline status: "offline - 4h ago" → "4h ago"
    let last_seen = if online {
        "now".to_string()
    } else {
        status_info
            .splitn(2, " - ")
            .nth(1)
            .unwrap_or("unknown")
            .to_string()
    };

    // Extract activity from remainder: ' - "activity here"'
    let activity = line[paren_close + 1..]
        .trim()
        .strip_prefix("- ")
        .map(|s| s.trim_matches('"').to_string())
        .filter(|s| !s.is_empty());

    Some(TeamMember {
        ai_id,
        member_type: if is_human { "human".to_string() } else { "ai".to_string() },
        online,
        last_seen,
        activity,
    })
}

/// Parse `teambook read-dms` output.
///
/// Format per message:
///   #<id> <from>: <content line 1>
///   <continuation lines...>
///   (blank line separates messages)
pub fn parse_dms(text: &str, recipient_h_id: &str) -> Vec<Dm> {
    let mut dms = Vec::new();
    let mut current_id: Option<u64> = None;
    let mut current_from = String::new();
    let mut current_content_lines: Vec<String> = Vec::new();
    let mut current_timestamp = String::new();

    let flush = |id: Option<u64>, from: &str, lines: &[String], ts: &str, recipient: &str, dms: &mut Vec<Dm>| {
        if let Some(id) = id {
            if !from.is_empty() && !lines.is_empty() {
                dms.push(Dm {
                    id,
                    from: from.to_string(),
                    to: recipient.to_string(),
                    content: lines.join("\n").trim().to_string(),
                    timestamp: if ts.is_empty() { "unknown".to_string() } else { ts.to_string() },
                });
            }
        }
    };

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            flush(current_id, &current_from, &current_content_lines, &current_timestamp, recipient_h_id, &mut dms);
            current_id = None;
            current_from.clear();
            current_content_lines.clear();
            current_timestamp.clear();
            continue;
        }

        // New message: "#<id> <from>[(<time>)]: <content>"
        if let Some(rest) = trimmed.strip_prefix('#') {
            // Flush previous
            flush(current_id, &current_from, &current_content_lines, &current_timestamp, recipient_h_id, &mut dms);
            current_content_lines.clear();

            // Parse id
            let mut parts = rest.splitn(2, ' ');
            let id_str = parts.next().unwrap_or("0");
            let remainder = parts.next().unwrap_or("");

            current_id = id_str.parse().ok();

            // Parse "sender[(time)]: content"
            if let Some(colon_pos) = remainder.find(':') {
                let sender_part = &remainder[..colon_pos];
                let content = remainder[colon_pos + 1..].trim();

                // sender_part may be "alpha-001" or "alpha-001 (2h ago)"
                if let Some(paren) = sender_part.find('(') {
                    current_from = sender_part[..paren].trim().to_string();
                    current_timestamp = sender_part[paren + 1..].trim_end_matches(')').to_string();
                } else {
                    current_from = sender_part.trim().to_string();
                }

                if !content.is_empty() {
                    current_content_lines.push(content.to_string());
                }
            }
        } else if current_id.is_some() {
            // Continuation line
            current_content_lines.push(trimmed.to_string());
        }
    }

    // Flush last message
    flush(current_id, &current_from, &current_content_lines, &current_timestamp, recipient_h_id, &mut dms);

    dms
}

/// Parse `teambook read-broadcasts` output.
///
/// Format: "<from> (<time_ago>): <content>"
pub fn parse_broadcasts(text: &str) -> Vec<Broadcast> {
    let mut broadcasts = Vec::new();
    let mut id_counter = 1u64; // CLI doesn't emit IDs; generate sequentially

    let mut current_from = String::new();
    let mut current_timestamp = String::new();
    let mut current_content_lines: Vec<String> = Vec::new();

    let flush = |from: &str, ts: &str, lines: &[String], id: &mut u64, bcs: &mut Vec<Broadcast>| {
        if !from.is_empty() && !lines.is_empty() {
            bcs.push(Broadcast {
                id: *id,
                from: from.to_string(),
                content: lines.join("\n").trim().to_string(),
                timestamp: if ts.is_empty() { "unknown".to_string() } else { ts.to_string() },
                channel: "general".to_string(),
            });
            *id += 1;
        }
    };

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            flush(&current_from, &current_timestamp, &current_content_lines, &mut id_counter, &mut broadcasts);
            current_from.clear();
            current_timestamp.clear();
            current_content_lines.clear();
            continue;
        }

        // New broadcast line: "<from> (<time>): <content>"
        // Heuristic: line contains " (<something>): " pattern
        if let Some(paren_open) = trimmed.find(" (") {
            if let Some(paren_close_rel) = trimmed[paren_open..].find("):") {
                let paren_close = paren_open + paren_close_rel;
                let from_candidate = &trimmed[..paren_open];
                // Validate sender looks like an AI/human id (no spaces, contains hyphen or alphanum)
                if !from_candidate.contains(' ') && !from_candidate.is_empty() {
                    flush(&current_from, &current_timestamp, &current_content_lines, &mut id_counter, &mut broadcasts);
                    current_content_lines.clear();

                    current_from = from_candidate.to_string();
                    current_timestamp = trimmed[paren_open + 2..paren_close].to_string();
                    let content = trimmed[paren_close + 2..].trim().to_string();
                    if !content.is_empty() {
                        current_content_lines.push(content);
                    }
                    continue;
                }
            }
        }

        if !current_from.is_empty() {
            current_content_lines.push(trimmed.to_string());
        }
    }

    flush(&current_from, &current_timestamp, &current_content_lines, &mut id_counter, &mut broadcasts);
    broadcasts
}

/// Parse `teambook task-list` output.
///
/// Formats supported:
///   #<id> [<status>] <description>
///   <batch>:<id> [<status>] <description>
///   <batch>:<id> [<status>] [owner: <owner>] <description>
pub fn parse_tasks(text: &str) -> Vec<Task> {
    let mut tasks = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Tasks:") || line.starts_with("Batches:") {
            continue;
        }

        // Strip leading '#' if present
        let line = line.strip_prefix('#').unwrap_or(line);

        // Find the first space to get the id
        let mut parts = line.splitn(2, ' ');
        let id = match parts.next() {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => continue,
        };
        let rest = parts.next().unwrap_or("").trim();

        // Parse [status]
        let (status, rest) = if let Some(bracket_end) = rest.find(']') {
            let status = rest[1..bracket_end]
                .trim_start_matches('[')
                .trim()
                .to_string();
            let rest = rest[bracket_end + 1..].trim();
            (status, rest)
        } else {
            ("unknown".to_string(), rest)
        };

        if status.is_empty() || rest.is_empty() {
            continue;
        }

        // Try to extract owner: "[owner: <name>]" or "owner: <name>"
        let (owner, description) = if let Some(o_start) = rest.find("owner:") {
            let o_rest = rest[o_start + 6..].trim();
            if let Some(space) = o_rest.find(' ') {
                let owner = o_rest[..space].trim().to_string();
                let desc = o_rest[space..].trim().to_string();
                (Some(owner), desc)
            } else {
                (Some(o_rest.to_string()), String::new())
            }
        } else {
            (None, rest.to_string())
        };

        let description = description.trim().trim_matches(']').trim_matches('[').to_string();
        if description.is_empty() && owner.is_none() {
            continue;
        }

        tasks.push(Task {
            id,
            description: if description.is_empty() { rest.to_string() } else { description },
            status,
            owner,
            created_at: String::new(), // CLI doesn't expose this
        });
    }

    tasks
}

/// Parse `teambook dialogues` output.
///
/// Format: "[<id>] <initiator> ↔ <responder> "<topic>" (<status>, <N> msgs[, <time>])"
pub fn parse_dialogues(text: &str) -> Vec<Dialogue> {
    let mut dialogues = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }

        // Find id: "[<id>]"
        let id = if line.starts_with('[') {
            if let Some(close) = line.find(']') {
                let id_str = &line[1..close];
                let id: u64 = id_str.parse().unwrap_or(0);
                let rest = &line[close + 1..];
                Some((id, rest.trim()))
            } else {
                None
            }
        } else {
            None
        };

        let (id, rest) = match id {
            Some(x) => x,
            None => continue,
        };

        // Find the ↔ separator (U+2194)
        let separator = " ↔ ";
        if let Some(sep_pos) = rest.find(separator) {
            let initiator = rest[..sep_pos].trim().to_string();
            let after_sep = rest[sep_pos + separator.len()..].trim();

            // Responder ends at first space or quote
            let (responder, topic_and_meta) = if let Some(sp) = after_sep.find(' ') {
                (after_sep[..sp].to_string(), after_sep[sp..].trim())
            } else {
                (after_sep.to_string(), "")
            };

            // Topic: everything in quotes
            let topic = if let (Some(q1), Some(q2)) = (topic_and_meta.find('"'), topic_and_meta.rfind('"')) {
                if q1 < q2 { topic_and_meta[q1 + 1..q2].to_string() } else { String::new() }
            } else {
                String::new()
            };

            // Meta: "(status, N msgs, time)"
            let (status, message_count, last_activity) = parse_dialogue_meta(topic_and_meta);

            dialogues.push(Dialogue {
                id,
                topic,
                initiator,
                responder,
                status,
                message_count,
                last_activity,
            });
        }
    }

    dialogues
}

fn parse_dialogue_meta(s: &str) -> (String, u32, String) {
    // Find content inside last parens
    if let (Some(p1), Some(p2)) = (s.rfind('('), s.rfind(')')) {
        if p1 < p2 {
            let meta = &s[p1 + 1..p2];
            let parts: Vec<&str> = meta.split(',').map(|s| s.trim()).collect();
            let status = parts.first().copied().unwrap_or("unknown").to_string();
            let msg_count = parts.iter()
                .find(|p| p.contains("msg"))
                .and_then(|p| p.split_whitespace().next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            let last_activity = parts.last().copied().unwrap_or("").to_string();
            return (status, msg_count, last_activity);
        }
    }
    ("unknown".to_string(), 0, String::new())
}

/// Parse `notebook-cli list` output.
///
/// Format: "#<id>[ [PINNED]][ [<tags>]] <content preview>"
/// OR:     "#<id> (<time_ago>) [<tags>] <content preview>"
pub fn parse_notes(text: &str) -> Vec<Note> {
    let mut notes = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Recent") || line.starts_with("Notes:") {
            continue;
        }

        if let Some(rest) = line.strip_prefix('#') {
            let mut parts = rest.splitn(2, ' ');
            let id: u64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
            let rest = parts.next().unwrap_or("").trim();

            let pinned = rest.contains("[PINNED]");
            let rest = rest.replace("[PINNED]", "").trim().to_string();

            // Extract tags from [tag1, tag2] block
            let (tags, content) = extract_tags_and_content(&rest);

            if !content.is_empty() || !tags.is_empty() {
                notes.push(Note {
                    id,
                    content: content.trim_end_matches("...").trim().to_string(),
                    tags,
                    pinned,
                    created_at: extract_time_ago(&rest),
                });
            }
        }
    }

    notes
}

/// Parse `notebook-cli recall <query>` output.
///
/// Format:
///   #<id> [<tags>](score: <N>%)
///   <content preview>
pub fn parse_note_search(text: &str) -> Vec<NoteSearchResult> {
    let mut results = Vec::new();
    let mut current_id: Option<u64> = None;
    let mut current_tags: Vec<String> = Vec::new();
    let mut current_score = 0.0f32;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }

        if let Some(rest) = line.strip_prefix('#') {
            let mut parts = rest.splitn(2, ' ');
            let id: u64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
            let rest = parts.next().unwrap_or("").trim();

            // Extract score: "(score: 94%)" or "(94%)"
            let score = if let Some(score_start) = rest.find("score:") {
                let score_str = rest[score_start + 6..].trim();
                let score_str = score_str.trim_start_matches(' ').trim_end_matches(')').trim_end_matches('%');
                score_str.parse::<f32>().unwrap_or(0.0) / 100.0
            } else if let Some(paren) = rest.rfind('(') {
                let inner = &rest[paren + 1..];
                let inner = inner.trim_end_matches(')').trim_end_matches('%');
                inner.parse::<f32>().unwrap_or(0.0) / 100.0
            } else {
                0.0
            };

            let (tags, _) = extract_tags_and_content(rest);

            current_id = Some(id);
            current_tags = tags;
            current_score = score;
        } else if let Some(id) = current_id.take() {
            // Content line follows the header
            results.push(NoteSearchResult {
                id,
                content: line.trim_end_matches("...").to_string(),
                tags: current_tags.drain(..).collect(),
                score: current_score,
            });
        }
    }

    results
}

// ============================================================================
// Helpers
// ============================================================================

fn extract_tags_and_content(s: &str) -> (Vec<String>, String) {
    // Look for [tag1, tag2, ...] block
    if let Some(bracket_open) = s.find('[') {
        if let Some(bracket_close) = s[bracket_open..].find(']') {
            let bracket_close = bracket_open + bracket_close;
            let tags_str = &s[bracket_open + 1..bracket_close];
            let tags: Vec<String> = tags_str
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty() && !t.contains('%') && !t.contains("score"))
                .collect();
            // Content is everything after the bracket block
            let content = s[bracket_close + 1..].trim().to_string();
            // Also strip leading time "(Xhr ago)" from content
            let content = strip_time_prefix(&content);
            return (tags, content);
        }
    }
    (Vec::new(), strip_time_prefix(s))
}

fn strip_time_prefix(s: &str) -> String {
    // Strip "(Xhr ago)" or "(now)" prefix
    let s = s.trim();
    if s.starts_with('(') {
        if let Some(close) = s.find(')') {
            return s[close + 1..].trim().to_string();
        }
    }
    s.to_string()
}

fn extract_time_ago(s: &str) -> String {
    if s.starts_with('(') {
        if let Some(close) = s.find(')') {
            return s[1..close].to_string();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_broadcasts_basic() {
        let text = "alpha-001 (1min ago): Sage — hello team\ncontinued message\n\nbeta-002 (now): Lyra — quick update\n";
        let bcs = parse_broadcasts(text);
        assert_eq!(bcs.len(), 2);
        assert_eq!(bcs[0].from, "alpha-001");
        assert_eq!(bcs[0].timestamp, "1min ago");
        assert!(bcs[0].content.contains("Sage — hello team"));
        assert_eq!(bcs[1].from, "beta-002");
    }

    #[test]
    fn test_parse_dms_basic() {
        let text = "#100 alpha-001: hello resonance\ncontinued\n\n#101 beta-002: hi there\n";
        let dms = parse_dms(text, "delta-004");
        assert_eq!(dms.len(), 2);
        assert_eq!(dms[0].id, 100);
        assert_eq!(dms[0].from, "alpha-001");
        assert_eq!(dms[0].to, "delta-004");
    }

    #[test]
    fn test_parse_tasks_basic() {
        let text = "#1 [completed] Add Forge\n#2 [in_progress] Write installer\n#3 [pending] Update docs\n";
        let tasks = parse_tasks(text);
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].status, "completed");
        assert_eq!(tasks[1].status, "in_progress");
    }

    #[test]
    fn test_parse_team_status_basic() {
        let text = "AI: delta-004\nTeam: 3 online\nalpha-001 (online) - \"Working on messages.rs\"\nbeta-002 (online)\ngamma-003 (offline - 4h ago)\n";
        let status = parse_team_status(text);
        assert_eq!(status.online_count, 3);
        assert_eq!(status.members.len(), 3);
        assert!(status.members[0].online);
        assert_eq!(status.members[0].activity, Some("Working on messages.rs".to_string()));
        assert!(!status.members[2].online);
        assert_eq!(status.members[2].last_seen, "4h ago");
    }

    #[test]
    fn test_parse_team_status_with_human() {
        let text = "AI: delta-004\nTeam: 2 online\nalpha-001 (online)\nqd (human, online)\n";
        let status = parse_team_status(text);
        assert_eq!(status.members.len(), 2);
        assert_eq!(status.members[0].member_type, "ai");
        assert_eq!(status.members[1].ai_id, "qd");
        assert_eq!(status.members[1].member_type, "human");
        assert!(status.members[1].online);
    }

    #[test]
    fn test_parse_team_status_derived_online_count() {
        // If "Team: N online" line absent, count is derived from online members
        let text = "alpha-001 (online)\nbeta-002 (offline - 2h ago)\ngamma-003 (online)\n";
        let status = parse_team_status(text);
        assert_eq!(status.online_count, 2);
        assert_eq!(status.members.len(), 3);
    }

    #[test]
    fn test_parse_dms_with_timestamp_in_sender() {
        let text = "#200 alpha-001 (5min ago): hello there\n\n#201 beta-002: quick note\n";
        let dms = parse_dms(text, "delta-004");
        assert_eq!(dms.len(), 2);
        assert_eq!(dms[0].id, 200);
        assert_eq!(dms[0].from, "alpha-001");
        assert_eq!(dms[0].timestamp, "5min ago");
        assert_eq!(dms[1].id, 201);
        assert_eq!(dms[1].from, "beta-002");
    }

    #[test]
    fn test_parse_dms_multiline_content() {
        let text = "#10 alpha-001: line one\nline two\nline three\n\n";
        let dms = parse_dms(text, "qd");
        assert_eq!(dms.len(), 1);
        assert!(dms[0].content.contains("line one"));
        assert!(dms[0].content.contains("line three"));
    }

    #[test]
    fn test_parse_dialogues_basic() {
        let text = "[42] alpha-001 ↔ beta-002 \"API design review\" (open, 5 msgs, 2h ago)\n[7] delta-004 ↔ gamma-003 \"Bug triage\" (closed, 3 msgs)\n";
        let dialogues = parse_dialogues(text);
        assert_eq!(dialogues.len(), 2);

        assert_eq!(dialogues[0].id, 42);
        assert_eq!(dialogues[0].initiator, "alpha-001");
        assert_eq!(dialogues[0].responder, "beta-002");
        assert_eq!(dialogues[0].topic, "API design review");
        assert_eq!(dialogues[0].status, "open");
        assert_eq!(dialogues[0].message_count, 5);

        assert_eq!(dialogues[1].id, 7);
        assert_eq!(dialogues[1].topic, "Bug triage");
        assert_eq!(dialogues[1].status, "closed");
        assert_eq!(dialogues[1].message_count, 3);
    }

    #[test]
    fn test_parse_dialogues_empty() {
        assert_eq!(parse_dialogues("").len(), 0);
        assert_eq!(parse_dialogues("No dialogues found.\n").len(), 0);
    }

    #[test]
    fn test_parse_notes_basic() {
        let text = "#247 (25days ago) [chimera,onboarding] PROJECT CHIMERA ONBOARDING...\n#333 (29min ago) [mobile-api,backend] mobile-api shipped...\n";
        let notes = parse_notes(text);
        assert_eq!(notes.len(), 2);

        assert_eq!(notes[0].id, 247);
        assert!(notes[0].tags.contains(&"chimera".to_string()));
        assert!(notes[0].tags.contains(&"onboarding".to_string()));
        assert!(!notes[0].content.is_empty());
        assert!(!notes[0].pinned);

        assert_eq!(notes[1].id, 333);
        assert!(notes[1].tags.contains(&"mobile-api".to_string()));
    }

    #[test]
    fn test_parse_notes_pinned() {
        let text = "#99 [PINNED] (2months ago) [team] THE TEAM: Sage...\n";
        let notes = parse_notes(text);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].pinned);
        assert_eq!(notes[0].id, 99);
        assert!(notes[0].tags.contains(&"team".to_string()));
    }

    #[test]
    fn test_parse_notes_empty() {
        assert_eq!(parse_notes("").len(), 0);
        assert_eq!(parse_notes("Recent notes:\n").len(), 0);
    }

    #[test]
    fn test_parse_note_search_basic() {
        let text = "#247 [chimera,onboarding](score: 94%)\nPROJECT CHIMERA ONBOARDING content here\n#333 [mobile-api](score: 71%)\nmobile-api notes here\n";
        let results = parse_note_search(text);
        assert_eq!(results.len(), 2);

        assert_eq!(results[0].id, 247);
        assert!((results[0].score - 0.94).abs() < 0.01, "score should be ~0.94, got {}", results[0].score);
        assert!(results[0].tags.contains(&"chimera".to_string()));
        assert!(!results[0].content.is_empty());

        assert_eq!(results[1].id, 333);
        assert!((results[1].score - 0.71).abs() < 0.01, "score should be ~0.71, got {}", results[1].score);
    }

    #[test]
    fn test_parse_note_search_empty() {
        assert_eq!(parse_note_search("").len(), 0);
        assert_eq!(parse_note_search("No results found.\n").len(), 0);
    }

    #[test]
    fn test_parse_tasks_with_owner() {
        let text = "#5 [in_progress] owner: alpha-001 Integrate BM25 IDF\n#6 [pending] Update docs\n";
        let tasks = parse_tasks(text);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].owner, Some("alpha-001".to_string()));
        assert!(tasks[0].description.contains("Integrate BM25 IDF"));
        assert_eq!(tasks[1].owner, None);
    }

    #[test]
    fn test_parse_broadcasts_empty() {
        assert_eq!(parse_broadcasts("").len(), 0);
    }
}
