//! Teambook skill handlers.
//!
//! Maps canonical `teambook-*` skill IDs to `teambook` CLI invocations.
//! Aliases are resolved upstream in `skills::normalize_skill_id()` before
//! reaching this module — only canonical IDs arrive here.
//!
//! Source of truth for subcommand names and args: teamengram-rs CLI source.
//! Output format: pipe-delimited (`field1|field2|field3`).

use crate::rpc::Message;
use crate::skills::SkillInvocation;

/// Route a canonical `teambook-*` skill ID to a `teambook` CLI invocation.
pub fn route(skill_id: &str, message: &Message) -> Result<SkillInvocation, String> {
    let data = message.first_data();

    let args: Vec<String> = match skill_id {
        // ── Status / identity ─────────────────────────────────────────────────

        "teambook-status" => vec!["status".to_string()],

        "teambook-whoami" => vec!["whoami".to_string()],

        "teambook-awareness" => {
            let limit = opt_u64(data, "limit", 5);
            vec!["awareness".to_string(), limit]
        }

        // ── Messaging ─────────────────────────────────────────────────────────

        "teambook-broadcast" => {
            let content = require_str(data, "content", message.first_text(),
                "teambook-broadcast requires 'content'")?;
            let mut args = vec!["broadcast".to_string(), content];
            if let Some(ch) = opt_str(data, "channel") {
                args.push("--channel".to_string());
                args.push(ch);
            }
            args
        }

        "teambook-dm" => {
            let to_ai = require_str(data, "to_ai", None,
                "teambook-dm requires 'to_ai'")?;
            let content = require_str(data, "content", message.first_text(),
                "teambook-dm requires 'content'")?;
            vec!["dm".to_string(), to_ai, content]
        }

        "teambook-direct-messages" => {
            let limit = opt_u64(data, "limit", 10);
            vec!["direct-messages".to_string(), limit]
        }

        "teambook-read-broadcasts" => {
            let limit = opt_u64(data, "limit", 20);
            let mut args = vec!["messages".to_string(), limit];
            if let Some(ch) = opt_str(data, "channel") {
                args.push("--channel".to_string());
                args.push(ch);
            }
            args
        }

        // ── Standby ───────────────────────────────────────────────────────────

        "teambook-standby" => {
            let mut args = vec!["standby".to_string()];
            if let Some(t) = opt_str(data, "timeout") {
                args.push(t);
            }
            args
        }

        // ── Dialogues ─────────────────────────────────────────────────────────

        "teambook-dialogue-start" => {
            let responder = require_str(data, "responder", None,
                "teambook-dialogue-start requires 'responder'")?;
            let topic = require_str(data, "topic", message.first_text(),
                "teambook-dialogue-start requires 'topic'")?;
            vec!["dialogue-create".to_string(), responder, topic]
        }

        "teambook-dialogue-respond" => {
            let dialogue_id = require_str(data, "dialogue_id", None,
                "teambook-dialogue-respond requires 'dialogue_id'")?;
            let response = require_str(data, "response", message.first_text(),
                "teambook-dialogue-respond requires 'response'")?;
            vec!["dialogue-respond".to_string(), dialogue_id, response]
        }

        "teambook-dialogues" => {
            let limit = opt_u64(data, "limit", 10);
            let mut args = vec!["dialogue-list".to_string(), limit];
            if let Some(filter) = opt_str(data, "filter") {
                args.push("--filter".to_string());
                args.push(filter);
            }
            if let Some(id) = opt_str(data, "id") {
                args.push("--id".to_string());
                args.push(id);
            }
            args
        }

        "teambook-dialogue-end" => {
            let dialogue_id = require_str(data, "dialogue_id", None,
                "teambook-dialogue-end requires 'dialogue_id'")?;
            let mut args = vec!["dialogue-end".to_string(), dialogue_id];
            if let Some(status) = opt_str(data, "status") {
                args.push(status);
            }
            if let Some(summary) = opt_str(data, "summary") {
                args.push("--summary".to_string());
                args.push(summary);
            }
            args
        }

        // ── Tasks ─────────────────────────────────────────────────────────────

        "teambook-task-create" => {
            let description = require_str(data, "description", message.first_text(),
                "teambook-task-create requires 'description'")?;
            let mut args = vec!["task-create".to_string(), description];
            if let Some(tasks) = opt_str(data, "tasks") {
                args.push("--tasks".to_string());
                args.push(tasks);
            }
            args
        }

        "teambook-task-update" => {
            let id = require_str(data, "id", None,
                "teambook-task-update requires 'id'")?;
            let status = require_str(data, "status", None,
                "teambook-task-update requires 'status' (done | claimed | started | blocked | closed)")?;
            let mut args = vec!["task-update".to_string(), id, status];
            if let Some(reason) = opt_str(data, "reason") {
                args.push("--reason".to_string());
                args.push(reason);
            }
            args
        }

        "teambook-task-get" => {
            let id = require_str(data, "id", message.first_text(),
                "teambook-task-get requires 'id'")?;
            vec!["task-get".to_string(), id]
        }

        "teambook-task-list" => {
            let mut args = vec!["task-list".to_string()];
            if let Some(filter) = opt_str(data, "filter") {
                args.push("--filter".to_string());
                args.push(filter);
            }
            args
        }

        // ── File claims ───────────────────────────────────────────────────────

        "teambook-list-claims" => {
            let limit = opt_u64(data, "limit", 20);
            vec!["list-claims".to_string(), limit]
        }

        "teambook-who-has" => {
            let path = require_str(data, "path", message.first_text(),
                "teambook-who-has requires 'path'")?;
            vec!["check-file".to_string(), path]
        }

        "teambook-claim-file" => {
            let path = require_str(data, "path", None,
                "teambook-claim-file requires 'path'")?;
            let working_on = require_str(data, "working_on", message.first_text(),
                "teambook-claim-file requires 'working_on'")?;
            let mut args = vec!["claim-file".to_string(), path, working_on];
            if let Some(dur) = opt_str(data, "duration") {
                args.push("--duration".to_string());
                args.push(dur);
            }
            args
        }

        "teambook-release-file" => {
            let path = require_str(data, "path", message.first_text(),
                "teambook-release-file requires 'path'")?;
            vec!["release-file".to_string(), path]
        }

        other => return Err(format!(
            "Unknown teambook skill: '{}'. \
             Check /.well-known/agent.json for the skill catalog.",
            other
        )),
    };

    Ok(SkillInvocation { exe: "teambook".to_string(), args })
}

// ─── Arg helpers ──────────────────────────────────────────────────────────────

fn require_str<'a>(
    data: Option<&'a serde_json::Value>,
    key: &str,
    text_fallback: Option<&'a str>,
    err: &str,
) -> Result<String, String> {
    if let Some(v) = data.and_then(|d| d.get(key)).and_then(|v| v.as_str()) {
        return Ok(v.to_string());
    }
    text_fallback.map(str::to_owned).ok_or_else(|| err.to_string())
}

fn opt_str(data: Option<&serde_json::Value>, key: &str) -> Option<String> {
    data.and_then(|d| d.get(key)).and_then(|v| v.as_str()).map(str::to_owned)
}

fn opt_u64(data: Option<&serde_json::Value>, key: &str, default: u64) -> String {
    data.and_then(|d| d.get(key))
        .and_then(|v| v.as_u64())
        .unwrap_or(default)
        .to_string()
}
