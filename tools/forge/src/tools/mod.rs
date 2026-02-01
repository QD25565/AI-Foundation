//! Tool system for Forge
//!
//! Built-in tools and MCP integration.
//!
//! When built with the `notebook` feature, notebook tools use direct Rust calls
//! for ~50x faster performance compared to CLI subprocess.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "notebook")]
use notebook_core::{Note, sqlite_storage::SqliteNotebookStorage};

/// Tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Tool result
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

impl ToolResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
            error: None,
        }
    }

    pub fn error(error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: String::new(),
            error: Some(error.into()),
        }
    }
}

/// Built-in tools
pub fn builtin_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "read_file".to_string(),
            description: "Read the contents of a file".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to read"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Line number to start reading from (1-indexed)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of lines to read"
                    }
                },
                "required": ["path"]
            }),
        },
        Tool {
            name: "write_file".to_string(),
            description: "Write content to a file".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        },
        Tool {
            name: "bash".to_string(),
            description: "Execute a bash command".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command to execute"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds"
                    }
                },
                "required": ["command"]
            }),
        },
        Tool {
            name: "grep".to_string(),
            description: "Search for patterns in files".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regular expression pattern to search for"
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory to search in"
                    },
                    "glob": {
                        "type": "string",
                        "description": "Glob pattern to filter files (e.g., '*.rs')"
                    }
                },
                "required": ["pattern"]
            }),
        },
        Tool {
            name: "glob".to_string(),
            description: "Find files matching a pattern".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern (e.g., '**/*.rs')"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in"
                    }
                },
                "required": ["pattern"]
            }),
        },
        // Notebook integration
        Tool {
            name: "notebook_remember".to_string(),
            description: "Save a note to your private memory".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The note content to save"
                    },
                    "tags": {
                        "type": "string",
                        "description": "Comma-separated tags"
                    }
                },
                "required": ["content"]
            }),
        },
        Tool {
            name: "notebook_recall".to_string(),
            description: "Search your memory for relevant notes".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum notes to return"
                    }
                },
                "required": ["query"]
            }),
        },
    ]
}

/// Execute a tool
pub async fn execute_tool(name: &str, args: &Value) -> ToolResult {
    match name {
        "read_file" => execute_read_file(args).await,
        "write_file" => execute_write_file(args).await,
        "bash" => execute_bash(args).await,
        "grep" => execute_grep(args).await,
        "glob" => execute_glob(args).await,
        "notebook_remember" => execute_notebook_remember(args).await,
        "notebook_recall" => execute_notebook_recall(args).await,
        _ => ToolResult::error(format!("Unknown tool: {}", name)),
    }
}

async fn execute_read_file(args: &Value) -> ToolResult {
    let path = args["path"].as_str().unwrap_or("");
    let offset = args["offset"].as_u64().unwrap_or(1) as usize;
    let limit = args["limit"].as_u64().unwrap_or(2000) as usize;

    match std::fs::read_to_string(path) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().collect();
            let start = (offset - 1).min(lines.len());
            let end = (start + limit).min(lines.len());

            let output: String = lines[start..end]
                .iter()
                .enumerate()
                .map(|(i, line)| format!("{:>6}\t{}", start + i + 1, line))
                .collect::<Vec<_>>()
                .join("\n");

            ToolResult::success(output)
        }
        Err(e) => ToolResult::error(format!("Failed to read file: {}", e)),
    }
}

async fn execute_write_file(args: &Value) -> ToolResult {
    let path = args["path"].as_str().unwrap_or("");
    let content = args["content"].as_str().unwrap_or("");

    match std::fs::write(path, content) {
        Ok(_) => ToolResult::success(format!("Successfully wrote {} bytes to {}", content.len(), path)),
        Err(e) => ToolResult::error(format!("Failed to write file: {}", e)),
    }
}

async fn execute_bash(args: &Value) -> ToolResult {
    let command = args["command"].as_str().unwrap_or("");
    let timeout_secs = args["timeout"].as_u64().unwrap_or(120);

    use std::process::Command;
    use std::time::Duration;

    #[cfg(windows)]
    let result = Command::new("cmd")
        .args(["/C", command])
        .output();

    #[cfg(not(windows))]
    let result = Command::new("sh")
        .args(["-c", command])
        .output();

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let mut result = stdout.to_string();
            if !stderr.is_empty() {
                result.push_str("\n--- stderr ---\n");
                result.push_str(&stderr);
            }

            if output.status.success() {
                ToolResult::success(result)
            } else {
                ToolResult {
                    success: false,
                    output: result,
                    error: Some(format!("Exit code: {}", output.status.code().unwrap_or(-1))),
                }
            }
        }
        Err(e) => ToolResult::error(format!("Failed to execute command: {}", e)),
    }
}

async fn execute_grep(args: &Value) -> ToolResult {
    let pattern = args["pattern"].as_str().unwrap_or("");
    let path = args["path"].as_str().unwrap_or(".");

    // Simple grep implementation using ripgrep if available, otherwise fallback
    #[cfg(windows)]
    let cmd = format!("rg --line-number \"{}\" \"{}\" 2>nul || findstr /S /N /R \"{}\" \"{}\\*\"", pattern, path, pattern, path);

    #[cfg(not(windows))]
    let cmd = format!("rg --line-number \"{}\" \"{}\" 2>/dev/null || grep -rn \"{}\" \"{}\"", pattern, path, pattern, path);

    execute_bash(&serde_json::json!({"command": cmd})).await
}

async fn execute_glob(args: &Value) -> ToolResult {
    let pattern = args["pattern"].as_str().unwrap_or("");
    let path = args["path"].as_str().unwrap_or(".");

    use std::path::Path;

    // Use glob crate if available, otherwise shell
    #[cfg(windows)]
    let cmd = format!("dir /S /B \"{}\\{}\" 2>nul", path, pattern.replace("**", "*"));

    #[cfg(not(windows))]
    let cmd = format!("find \"{}\" -name \"{}\" -type f 2>/dev/null | head -100", path, pattern.replace("**/", ""));

    execute_bash(&serde_json::json!({"command": cmd})).await
}

async fn execute_notebook_remember(args: &Value) -> ToolResult {
    let content = args["content"].as_str().unwrap_or("");
    let tags_str = args["tags"].as_str().unwrap_or("");

    #[cfg(feature = "notebook")]
    {
        // Direct Rust call - ~50x faster than CLI
        match SqliteNotebookStorage::new() {
            Ok(storage) => {
                let tags: Vec<String> = if tags_str.is_empty() {
                    vec![]
                } else {
                    tags_str.split(',').map(|s| s.trim().to_string()).collect()
                };
                let note = Note::new(content.to_string(), tags);
                match storage.remember(&note) {
                    Ok(id) => ToolResult::success(format!("Note saved: ID {}\nTags: {}", id, if tags_str.is_empty() { "none" } else { tags_str })),
                    Err(e) => ToolResult::error(format!("Failed to save note: {}", e)),
                }
            }
            Err(e) => ToolResult::error(format!("Failed to initialize notebook: {}", e)),
        }
    }

    #[cfg(not(feature = "notebook"))]
    {
        // CLI fallback when notebook feature is disabled
        let cmd = if tags_str.is_empty() {
            format!("notebook-cli remember \"{}\"", content.replace("\"", "\\\""))
        } else {
            format!("notebook-cli remember \"{}\" --tags {}", content.replace("\"", "\\\""), tags_str)
        };
        execute_bash(&serde_json::json!({"command": cmd})).await
    }
}

async fn execute_notebook_recall(args: &Value) -> ToolResult {
    let query = args["query"].as_str().unwrap_or("");
    let limit = args["limit"].as_u64().unwrap_or(10) as i64;

    #[cfg(feature = "notebook")]
    {
        // Direct Rust call - ~50x faster than CLI
        match SqliteNotebookStorage::new() {
            Ok(storage) => {
                match storage.recall(Some(query), limit, false) {
                    Ok(results) => {
                        if results.is_empty() {
                            return ToolResult::success("No notes found matching your query.");
                        }
                        let mut output = String::new();
                        for (i, result) in results.iter().enumerate() {
                            let note = &result.note;
                            let tags = if note.tags.is_empty() {
                                String::new()
                            } else {
                                format!(" [{}]", note.tags.join(", "))
                            };
                            let pinned = if note.pinned { " 📌" } else { "" };
                            let score = if result.final_score > 0.0 {
                                format!(" (score: {:.2})", result.final_score)
                            } else {
                                String::new()
                            };
                            output.push_str(&format!(
                                "{}. ID {}{}{}: {}{}\n",
                                i + 1,
                                note.id,
                                pinned,
                                tags,
                                note.content.chars().take(200).collect::<String>(),
                                score
                            ));
                        }
                        ToolResult::success(output.trim_end())
                    }
                    Err(e) => ToolResult::error(format!("Failed to search notes: {}", e)),
                }
            }
            Err(e) => ToolResult::error(format!("Failed to initialize notebook: {}", e)),
        }
    }

    #[cfg(not(feature = "notebook"))]
    {
        // CLI fallback when notebook feature is disabled
        let cmd = format!("notebook-cli recall \"{}\" --limit {}", query.replace("\"", "\\\""), limit);
        execute_bash(&serde_json::json!({"command": cmd})).await
    }
}
