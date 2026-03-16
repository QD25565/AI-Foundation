//! Notebook skill handlers.
//!
//! Maps canonical `notebook-*` skill IDs to `notebook-cli` invocations.
//! Aliases are resolved upstream in `skills::normalize_skill_id()` before
//! reaching this module — only canonical IDs arrive here.
//!
//! Source of truth for subcommand names and args: notebook-rs CLI source.
//! Output format: pipe-delimited (`field1|field2|field3`).

use crate::rpc::Message;
use crate::skills::SkillInvocation;

/// Route a canonical `notebook-*` skill ID to a `notebook-cli` invocation.
pub fn route(skill_id: &str, message: &Message) -> Result<SkillInvocation, String> {
    let data = message.first_data();

    let args: Vec<String> = match skill_id {
        // ── Write ─────────────────────────────────────────────────────────────

        "notebook-remember" => {
            let content = require_str(data, "content", message.first_text(),
                "notebook-remember requires 'content'")?;
            let mut args = vec!["remember".to_string(), content];
            if let Some(tags) = opt_str(data, "tags") {
                args.push("--tags".to_string());
                args.push(tags);
            }
            if let Some(priority) = opt_str(data, "priority") {
                args.push("--priority".to_string());
                args.push(priority);
            }
            if opt_bool(data, "pin") {
                args.push("--pin".to_string());
            }
            args
        }

        "notebook-update" => {
            let id = require_str(data, "id", None,
                "notebook-update requires 'id'")?;
            let mut args = vec!["update".to_string(), id];
            if let Some(content) = opt_str(data, "content") {
                args.push("--content".to_string());
                args.push(content);
            }
            if let Some(tags) = opt_str(data, "tags") {
                args.push("--tags".to_string());
                args.push(tags);
            }
            args
        }

        "notebook-add-tags" => {
            let id = require_str(data, "id", None,
                "notebook-add-tags requires 'id'")?;
            let tags = require_str(data, "tags", message.first_text(),
                "notebook-add-tags requires 'tags'")?;
            vec!["add-tags".to_string(), id, tags]
        }

        // ── Read ──────────────────────────────────────────────────────────────

        "notebook-recall" => {
            let query = require_str(data, "query", message.first_text(),
                "notebook-recall requires 'query'")?;
            let mut args = vec!["recall".to_string(), query];
            if let Some(limit) = opt_str(data, "limit") {
                args.push("--limit".to_string());
                args.push(limit);
            }
            args
        }

        "notebook-list" => {
            let limit = opt_u64(data, "limit", 20);
            let mut args = vec!["list".to_string(), limit];
            if opt_bool(data, "pinned_only") {
                args.push("--pinned-only".to_string());
            }
            if let Some(tag) = opt_str(data, "tag") {
                args.push("--tag".to_string());
                args.push(tag);
            }
            args
        }

        "notebook-tags" => {
            let limit = opt_u64(data, "limit", 50);
            vec!["tags".to_string(), limit]
        }

        "notebook-work" => {
            let content = require_str(data, "content", message.first_text(),
                "notebook-work requires 'content'")?;
            let mut args = vec!["work".to_string(), content];
            if let Some(ttl) = opt_str(data, "ttl_hours") {
                args.push("--ttl".to_string());
                args.push(ttl);
            }
            if let Some(tags) = opt_str(data, "tags") {
                args.push("--tags".to_string());
                args.push(tags);
            }
            args
        }

        "notebook-pinned" => {
            let limit = opt_u64(data, "limit", 50);
            vec!["pinned".to_string(), limit]
        }

        "notebook-get" => {
            let id = require_str(data, "id", message.first_text(),
                "notebook-get requires 'id'")?;
            vec!["get".to_string(), id]
        }

        "notebook-related" => {
            let id = require_str(data, "id", message.first_text(),
                "notebook-related requires 'id'")?;
            let mut args = vec!["related".to_string(), id];
            if let Some(edge_type) = opt_str(data, "edge_type") {
                args.push("--edge-type".to_string());
                args.push(edge_type);
            }
            args
        }

        "notebook-stats" => vec!["stats".to_string()],

        "notebook-graph-stats" => vec!["graph-stats".to_string()],

        // ── Pin / unpin ───────────────────────────────────────────────────────

        "notebook-pin" => {
            let id = require_str(data, "id", message.first_text(),
                "notebook-pin requires 'id'")?;
            vec!["pin".to_string(), id]
        }

        "notebook-unpin" => {
            let id = require_str(data, "id", message.first_text(),
                "notebook-unpin requires 'id'")?;
            vec!["unpin".to_string(), id]
        }

        // ── Delete ────────────────────────────────────────────────────────────

        "notebook-delete" => {
            let id = require_str(data, "id", message.first_text(),
                "notebook-delete requires 'id'")?;
            vec!["delete".to_string(), id]
        }

        // ── Graph traversal ───────────────────────────────────────────────────

        "notebook-link" => {
            let from = require_str(data, "from_id", None,
                "notebook-link requires 'from_id'")?;
            let to = require_str(data, "to_id", None,
                "notebook-link requires 'to_id'")?;
            let mut args = vec!["link".to_string(), from, to];
            if let Some(rel) = opt_str(data, "relationship") {
                args.push("--relationship".to_string());
                args.push(rel);
            }
            if let Some(w) = opt_str(data, "weight") {
                args.push("--weight".to_string());
                args.push(w);
            }
            args
        }

        "notebook-traverse" => {
            let id = require_str(data, "id", message.first_text(),
                "notebook-traverse requires 'id'")?;
            let mut args = vec!["traverse".to_string(), id];
            if let Some(depth) = opt_str(data, "depth") {
                args.push("--depth".to_string());
                args.push(depth);
            }
            if let Some(edge_type) = opt_str(data, "edge_type") {
                args.push("--edge-type".to_string());
                args.push(edge_type);
            }
            args
        }

        "notebook-health-check" => {
            let mut args = vec!["health-check".to_string()];
            if opt_bool(data, "fix") {
                args.push("--fix".to_string());
            }
            args
        }

        // ── Vault ─────────────────────────────────────────────────────────────

        "notebook-vault-set" => {
            let key = require_str(data, "key", None,
                "notebook-vault-set requires 'key'")?;
            let value = require_str(data, "value", message.first_text(),
                "notebook-vault-set requires 'value'")?;
            vec!["vault".to_string(), "set".to_string(), key, value]
        }

        "notebook-vault-get" => {
            let key = require_str(data, "key", message.first_text(),
                "notebook-vault-get requires 'key'")?;
            vec!["vault".to_string(), "get".to_string(), key]
        }

        "notebook-vault-list" => vec!["vault".to_string(), "list".to_string()],

        other => return Err(format!(
            "Unknown notebook skill: '{}'. \
             Check /.well-known/agent.json for the skill catalog.",
            other
        )),
    };

    Ok(SkillInvocation { exe: "notebook-cli".to_string(), args })
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

fn opt_bool(data: Option<&serde_json::Value>, key: &str) -> bool {
    data.and_then(|d| d.get(key))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}
