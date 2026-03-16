//! Skill routing layer — maps A2A messages to CLI invocations.
//!
//! Resolution order:
//!   1. `message.metadata.skillId`       — explicit routing (preferred)
//!   2. `data` part with a `skillId` key — structured invocation
//!   3. First `text` part                — passed as-is to CLI (e.g. "teambook bc hello")
//!
//! ## Alias system
//!
//! Each canonical skill ID has 4-8 hidden aliases that mirror the CLI's own
//! alias system. Only the canonical IDs appear in the Agent Card; all aliases
//! route silently to the same handler.
//!
//! Example: `teambook-bc`, `teambook-announce`, `teambook-shout` all normalise
//! to `teambook-broadcast` before dispatch.
//!
//! ## Text command passthrough
//!
//! Plain-text commands ("teambook bc hello", "notebook search foo") are passed
//! verbatim to the CLI binary. Because the CLIs already handle their own
//! command aliases at the binary level, no text-command normalization is needed.

pub mod notebook;
pub mod teambook;

use std::path::PathBuf;

use crate::rpc::Message;

// ─── Skill invocation ─────────────────────────────────────────────────────────

/// A fully-resolved CLI invocation produced by `route()`.
///
/// The dispatch layer passes this to `cli::run_to_completion` or
/// `cli::run_streaming` without further interpretation.
#[derive(Debug)]
pub struct SkillInvocation {
    /// Binary name (e.g. `"teambook"`, `"notebook-cli"`).
    pub exe: String,
    /// Arguments forwarded verbatim to the binary.
    pub args: Vec<String>,
}

// ─── Public entry point ───────────────────────────────────────────────────────

/// Route an incoming A2A message to a CLI invocation.
///
/// Returns `Err` with a human-readable message if no skill matches.
/// The error is surfaced to the caller as a `-32602 INVALID_PARAMS` JSON-RPC error.
pub fn route(message: &Message, _bin_dir: &PathBuf) -> Result<SkillInvocation, String> {
    // 1. Explicit skillId in message metadata.
    if let Some(skill_id) = message.skill_id() {
        return route_by_id(skill_id, message);
    }

    // 2. Data part with embedded skillId.
    if let Some(data) = message.first_data() {
        if let Some(skill_id) = data.get("skillId").and_then(|v| v.as_str()) {
            return route_by_id(skill_id, message);
        }
    }

    // 3. Plain-text passthrough: forward directly to the CLI binary.
    //    The CLI handles its own aliases at the binary level — no normalisation needed here.
    if let Some(text) = message.first_text() {
        return route_text_command(text);
    }

    Err(
        "No routable content: set metadata.skillId, include a data part with skillId, \
         or send a text command (e.g. 'teambook broadcast hello' or 'notebook recall rust patterns')"
            .to_string(),
    )
}

// ─── Private routing ──────────────────────────────────────────────────────────

/// Normalise a skill ID alias to its canonical form, then dispatch.
fn route_by_id(skill_id: &str, message: &Message) -> Result<SkillInvocation, String> {
    let canonical = normalize_skill_id(skill_id);
    if canonical.starts_with("teambook-") {
        return teambook::route(canonical, message);
    }
    if canonical.starts_with("notebook-") {
        return notebook::route(canonical, message);
    }
    Err(format!(
        "Unknown skill: '{}'. \
         Check /.well-known/agent.json for the full skill catalog.",
        skill_id
    ))
}

/// Normalise any known alias to its canonical skill ID.
///
/// Canonical IDs are the ones exposed in the Agent Card.
/// Aliases are accepted silently — they never appear in the catalog.
/// Unknown IDs pass through unchanged (will fail in the module's match).
///
/// Source of truth: teamengram-rs and notebook-rs CLI source code.
#[rustfmt::skip]
fn normalize_skill_id(id: &str) -> &str {
    match id {
        // ── teambook: broadcast ────────────────────────────────────────────────
        | "teambook-bc"
        | "teambook-announce"
        | "teambook-shout"
        | "teambook-yell"
        | "teambook-all"
            => "teambook-broadcast",

        // ── teambook: direct message ───────────────────────────────────────────
        | "teambook-send"
        | "teambook-msg"
        | "teambook-message"
        | "teambook-pm"
        | "teambook-whisper"
            => "teambook-dm",

        // ── teambook: read DMs ─────────────────────────────────────────────────
        | "teambook-read-dms"
        | "teambook-dms"
        | "teambook-inbox"
        | "teambook-mail"
        | "teambook-received"
            => "teambook-direct-messages",

        // ── teambook: read broadcasts ──────────────────────────────────────────
        | "teambook-messages"
        | "teambook-msgs"
        | "teambook-broadcasts"
        | "teambook-feed"
        | "teambook-listen"
            => "teambook-read-broadcasts",

        // ── teambook: status ───────────────────────────────────────────────────
        | "teambook-get-status"
        | "teambook-who"
        | "teambook-online"
        | "teambook-team"
        | "teambook-here"
            => "teambook-status",

        // ── teambook: standby ──────────────────────────────────────────────────
        | "teambook-wait"
        | "teambook-sleep"
        | "teambook-idle"
        | "teambook-await"
            => "teambook-standby",

        // ── teambook: dialogue start ───────────────────────────────────────────
        | "teambook-dialogue"
        | "teambook-chat"
        | "teambook-converse"
        | "teambook-new-dialogue"
        | "teambook-start-dialogue"
        | "teambook-talk"
        | "teambook-begin-dialogue"
        | "teambook-create-dialogue"
            => "teambook-dialogue-start",

        // ── teambook: dialogue respond ─────────────────────────────────────────
        | "teambook-reply"
        | "teambook-respond"
        | "teambook-answer"
        | "teambook-dialogue-reply"
        | "teambook-respond-dialogue"
        | "teambook-dialogue-answer"
        | "teambook-chat-reply"
        | "teambook-continue-dialogue"
            => "teambook-dialogue-respond",

        // ── teambook: dialogue list ────────────────────────────────────────────
        | "teambook-list-dialogues"
        | "teambook-chats"
        | "teambook-my-dialogues"
        | "teambook-conversations"
        | "teambook-invites"
        | "teambook-dialogue-invites"
        | "teambook-pending-chats"
        | "teambook-incoming"
        | "teambook-dialogue-read"
        | "teambook-read-dialogue"
        | "teambook-my-turn"
            => "teambook-dialogues",

        // ── teambook: dialogue end ─────────────────────────────────────────────
        | "teambook-end-dialogue"
        | "teambook-close-dialogue"
        | "teambook-finish-dialogue"
        | "teambook-done-dialogue"
        | "teambook-close-chat"
        | "teambook-dialogue-close"
        | "teambook-end-chat"
        | "teambook-finish-chat"
            => "teambook-dialogue-end",

        // ── teambook: task create ──────────────────────────────────────────────
        | "teambook-task"
        | "teambook-add-task"
        | "teambook-task-add"
        | "teambook-new-task"
        | "teambook-create-task"
        | "teambook-add"
        | "teambook-batch"
        | "teambook-batch-create"
        | "teambook-new-batch"
        | "teambook-create-batch"
            => "teambook-task-create",

        // ── teambook: task update ──────────────────────────────────────────────
        | "teambook-task-complete"
        | "teambook-complete"
        | "teambook-done"
        | "teambook-finish"
        | "teambook-resolve"
        | "teambook-close-task"
        | "teambook-task-start"
        | "teambook-begin-task"
        | "teambook-work-on"
        | "teambook-start"
        | "teambook-task-block"
        | "teambook-pause-task"
        | "teambook-block"
        | "teambook-task-claim"
        | "teambook-claim-task"
        | "teambook-take"
        | "teambook-grab"
        | "teambook-claim"
        | "teambook-task-done"
        | "teambook-batch-done"
        | "teambook-close-batch"
        | "teambook-finish-batch"
        | "teambook-batch-close"
            => "teambook-task-update",

        // ── teambook: task get ─────────────────────────────────────────────────
        | "teambook-get-task"
        | "teambook-show-task"
        | "teambook-task-details"
        | "teambook-view-task"
        | "teambook-inspect-task"
        | "teambook-batch-get"
        | "teambook-show-batch"
        | "teambook-batch-details"
        | "teambook-view-batch"
            => "teambook-task-get",

        // ── teambook: task list ────────────────────────────────────────────────
        | "teambook-tasks"
        | "teambook-list-tasks"
        | "teambook-queue"
        | "teambook-pending-tasks"
        | "teambook-all-tasks"
        | "teambook-batches"
        | "teambook-list-batches"
        | "teambook-my-batches"
        | "teambook-open-batches"
        | "teambook-task-stats"
        | "teambook-queue-stats"
        | "teambook-task-summary"
            => "teambook-task-list",

        // ── teambook: file claims ──────────────────────────────────────────────
        | "teambook-claims"
        | "teambook-file-claims"
        | "teambook-all-claims"
        | "teambook-who-claimed"
        | "teambook-claimed-files"
            => "teambook-list-claims",

        | "teambook-check-claim"
        | "teambook-check-file"
        | "teambook-file-status"
        | "teambook-is-claimed"
        | "teambook-who-owns"
        | "teambook-file-owner"
            => "teambook-who-has",

        | "teambook-lock-file"
        | "teambook-reserve"
        | "teambook-claim-edit"
        | "teambook-own"
            => "teambook-claim-file",

        | "teambook-release"
        | "teambook-unclaim"
        | "teambook-free-file"
        | "teambook-unlock-file"
        | "teambook-give-back"
            => "teambook-release-file",

        // ── teambook: awareness / whoami ───────────────────────────────────────
        | "teambook-aware"
        | "teambook-notifications"
        | "teambook-alerts"
        | "teambook-check-all"
            => "teambook-awareness",

        | "teambook-whoami"
        | "teambook-id"
        | "teambook-identity"
        | "teambook-me"
        | "teambook-self"
            => "teambook-whoami",

        // ── notebook: remember ─────────────────────────────────────────────────
        | "notebook-save"
        | "notebook-note"
        | "notebook-mem"
            => "notebook-remember",

        // ── notebook: recall ───────────────────────────────────────────────────
        | "notebook-search"
        | "notebook-find"
        | "notebook-query"
        | "notebook-lookup"
            => "notebook-recall",

        // ── notebook: list ─────────────────────────────────────────────────────
        | "notebook-ls"
        | "notebook-recent"
        | "notebook-show"
        | "notebook-all"
            => "notebook-list",

        // ── notebook: pinned ───────────────────────────────────────────────────
        | "notebook-starred"
        | "notebook-favorites"
        | "notebook-important"
            => "notebook-pinned",

        // ── notebook: pin/unpin ────────────────────────────────────────────────
        | "notebook-star"
        | "notebook-mark"
        | "notebook-favorite"
            => "notebook-pin",

        | "notebook-unstar"
        | "notebook-unmark"
        | "notebook-unfavorite"
            => "notebook-unpin",

        // ── notebook: get ──────────────────────────────────────────────────────
        | "notebook-read"
        | "notebook-view"
        | "notebook-fetch"
            => "notebook-get",

        // ── notebook: update ───────────────────────────────────────────────────
        | "notebook-edit"
        | "notebook-modify"
        | "notebook-change"
            => "notebook-update",

        // ── notebook: delete ───────────────────────────────────────────────────
        | "notebook-rm"
        | "notebook-remove"
        | "notebook-trash"
        | "notebook-forget"
            => "notebook-delete",

        // ── notebook: related ──────────────────────────────────────────────────
        | "notebook-related-to"
        | "notebook-connections"
        | "notebook-edges"
            => "notebook-related",

        // ── notebook: stats ────────────────────────────────────────────────────
        | "notebook-stat"
        | "notebook-info"
        | "notebook-status"
        | "notebook-summary"
            => "notebook-stats",

        | "notebook-graph"
        | "notebook-kg-stats"
        | "notebook-knowledge-graph"
            => "notebook-graph-stats",

        // Pass through — either already canonical or will fail in the module match.
        other => other,
    }
}

/// Pass a plain-text command through to the appropriate CLI binary.
///
/// Supports: `teambook <subcommand> [args…]` and `notebook <subcommand> [args…]`
/// The CLI binary handles its own command aliases at the binary level.
fn route_text_command(text: &str) -> Result<SkillInvocation, String> {
    let parts: Vec<&str> = text.trim().splitn(3, ' ').collect();
    match parts.as_slice() {
        [cmd, sub, rest] if *cmd == "teambook" => Ok(SkillInvocation {
            exe: "teambook".to_string(),
            args: std::iter::once(*sub)
                .chain(rest.split_whitespace())
                .map(str::to_owned)
                .collect(),
        }),
        [cmd, sub] if *cmd == "teambook" => Ok(SkillInvocation {
            exe: "teambook".to_string(),
            args: vec![sub.to_string()],
        }),
        [cmd, sub, rest] if *cmd == "notebook" => Ok(SkillInvocation {
            exe: "notebook-cli".to_string(),
            args: std::iter::once(*sub)
                .chain(rest.split_whitespace())
                .map(str::to_owned)
                .collect(),
        }),
        [cmd, sub] if *cmd == "notebook" => Ok(SkillInvocation {
            exe: "notebook-cli".to_string(),
            args: vec![sub.to_string()],
        }),
        _ => Err(format!(
            "Unrecognised text command: {:?}. \
             Use 'teambook <subcommand> [args]' or 'notebook <subcommand> [args]'.",
            text.trim()
        )),
    }
}
