//! Engram MCP Server - Standalone AI Memory
//!
//! A simple MCP server that gives an AI persistent memory.
//! No team coordination, no complexity - just remember and recall.

use anyhow::Result;
use engram::Engram;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::Deserialize;
use std::sync::Mutex;

// ============== Input Schemas ==============

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RememberInput {
    /// Note content to save
    pub content: String,
    /// Comma-separated tags (optional)
    pub tags: Option<String>,
    /// Priority level (optional)
    pub priority: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecallInput {
    /// Search query
    pub query: String,
    /// Max results (default 10)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NoteIdInput {
    /// Note ID
    pub id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LimitInput {
    /// Max results (default 10)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateNoteInput {
    /// Note ID to update
    pub id: i64,
    /// New content (optional)
    pub content: Option<String>,
    /// New tags, comma-separated (optional)
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddTagsInput {
    /// Note ID
    pub note_id: i64,
    /// Comma-separated tags to add
    pub tags: String,
}

// ============== Server ==============

pub struct EngramServer {
    tool_router: ToolRouter<Self>,
    engram: Mutex<Engram>,
    ai_id: String,
}

impl Clone for EngramServer {
    fn clone(&self) -> Self {
        // Re-open engram for the clone (required by rmcp)
        let ai_id = self.ai_id.clone();
        let path = get_engram_path(&ai_id);
        let engram = Engram::open(&path).expect("Failed to open engram");
        Self {
            tool_router: Self::tool_router(),
            engram: Mutex::new(engram),
            ai_id,
        }
    }
}

fn get_engram_path(ai_id: &str) -> std::path::PathBuf {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".ai-foundation");
    std::fs::create_dir_all(&base).ok();
    base.join(format!("notebook_{}.engram", ai_id))
}

fn format_note(note: &engram::Note) -> String {
    let age = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) - note.timestamp;
    let age_str = format_age(age);
    let pinned = if note.pinned { " [pinned]" } else { "" };
    let tags = if note.tags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", note.tags.join(","))
    };
    format!(
        "{} | ({} ago){}{} {}",
        note.id, age_str, pinned, tags, note.content
    )
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

impl EngramServer {
    pub fn new() -> Result<Self> {
        let ai_id = std::env::var("AI_ID").unwrap_or_else(|_| "default".to_string());
        let path = get_engram_path(&ai_id);

        eprintln!("[engram-mcp] AI_ID: {}", ai_id);
        eprintln!("[engram-mcp] Storage: {}", path.display());

        let engram = Engram::open(&path)?;

        Ok(Self {
            tool_router: Self::tool_router(),
            engram: Mutex::new(engram),
            ai_id,
        })
    }
}

#[tool_router]
impl EngramServer {
    #[tool(description = "Save a note to your private memory")]
    async fn notebook_remember(&self, Parameters(input): Parameters<RememberInput>) -> String {
        let mut engram = self.engram.lock().unwrap();

        let tags: Vec<&str> = input.tags
            .as_ref()
            .map(|t| t.split(',').map(|s| s.trim()).collect())
            .unwrap_or_default();

        match engram.remember(&input.content, &tags) {
            Ok(id) => format!("Saved note #{}", id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Search notes")]
    async fn notebook_recall(&self, Parameters(input): Parameters<RecallInput>) -> String {
        let mut engram = self.engram.lock().unwrap();
        let limit = input.limit.unwrap_or(10) as usize;

        match engram.recall_by_keyword(&input.query, limit) {
            Ok(results) => {
                if results.is_empty() {
                    "No notes found".to_string()
                } else {
                    let mut out = format!("|RESULTS|{}\n", results.len());
                    for r in results {
                        out.push_str(&format_note(&r.note));
                        out.push('\n');
                    }
                    out
                }
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Notebook statistics")]
    async fn notebook_stats(&self) -> String {
        let mut engram = self.engram.lock().unwrap();
        let stats = engram.stats();
        format!(
            "Notes:{} Pinned:{} Vectors:{} Edges:{}\nAI_ID:{}\nPath:{}",
            stats.note_count,
            stats.pinned_count,
            stats.vector_count,
            stats.edge_count,
            self.ai_id,
            get_engram_path(&self.ai_id).display()
        )
    }

    #[tool(description = "List recent notes")]
    async fn notebook_list(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let mut engram = self.engram.lock().unwrap();
        let limit = input.limit.unwrap_or(10) as usize;

        match engram.recent(limit) {
            Ok(notes) => {
                if notes.is_empty() {
                    "No notes yet".to_string()
                } else {
                    let mut out = format!("|RECENT|{}\n", notes.len());
                    for note in notes {
                        out.push_str(&format_note(&note));
                        out.push('\n');
                    }
                    out
                }
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get note by ID")]
    async fn notebook_get(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let mut engram = self.engram.lock().unwrap();

        match engram.get(input.id as u64) {
            Ok(Some(note)) => format_note(&note),
            Ok(None) => format!("Note #{} not found", input.id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Pin a note")]
    async fn notebook_pin(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let mut engram = self.engram.lock().unwrap();

        match engram.pin(input.id as u64) {
            Ok(_) => format!("Pinned note #{}", input.id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Unpin a note")]
    async fn notebook_unpin(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let mut engram = self.engram.lock().unwrap();

        match engram.unpin(input.id as u64) {
            Ok(_) => format!("Unpinned note #{}", input.id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get pinned notes")]
    async fn notebook_pinned(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let mut engram = self.engram.lock().unwrap();

        match engram.pinned() {
            Ok(notes) => {
                let limit = input.limit.unwrap_or(50) as usize;
                let notes: Vec<_> = notes.into_iter().take(limit).collect();
                if notes.is_empty() {
                    "No pinned notes".to_string()
                } else {
                    let mut out = format!("|PINNED|{}\n", notes.len());
                    for note in notes {
                        out.push_str(&format_note(&note));
                        out.push('\n');
                    }
                    out
                }
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Delete a note")]
    async fn notebook_delete(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let mut engram = self.engram.lock().unwrap();

        match engram.forget(input.id as u64) {
            Ok(_) => format!("Deleted note #{}", input.id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Update a note")]
    async fn notebook_update(&self, Parameters(input): Parameters<UpdateNoteInput>) -> String {
        let mut engram = self.engram.lock().unwrap();

        // Get existing note
        let existing = match engram.get(input.id as u64) {
            Ok(Some(n)) => n,
            Ok(None) => return format!("Note #{} not found", input.id),
            Err(e) => return format!("Error: {}", e),
        };

        let content = input.content.unwrap_or(existing.content);
        let tags: Vec<String> = input.tags
            .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or(existing.tags);

        // Delete old and create new
        if let Err(e) = engram.forget(input.id as u64) {
            return format!("Error deleting old note: {}", e);
        }

        let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
        match engram.remember(&content, &tag_refs) {
            Ok(new_id) => format!("Updated note #{} -> #{}", input.id, new_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Add tags to a note")]
    async fn notebook_add_tags(&self, Parameters(input): Parameters<AddTagsInput>) -> String {
        let mut engram = self.engram.lock().unwrap();

        // Get existing note
        let existing = match engram.get(input.note_id as u64) {
            Ok(Some(n)) => n,
            Ok(None) => return format!("Note #{} not found", input.note_id),
            Err(e) => return format!("Error: {}", e),
        };

        // Merge tags
        let new_tags: Vec<&str> = input.tags.split(',').map(|s| s.trim()).collect();
        let mut all_tags = existing.tags.clone();
        for tag in new_tags {
            if !all_tags.iter().any(|t| t == tag) {
                all_tags.push(tag.to_string());
            }
        }

        // Delete old and create new
        if let Err(e) = engram.forget(input.note_id as u64) {
            return format!("Error: {}", e);
        }

        let tag_refs: Vec<&str> = all_tags.iter().map(|s| s.as_str()).collect();
        match engram.remember(&existing.content, &tag_refs) {
            Ok(new_id) => format!("Added tags to note #{} -> #{}", input.note_id, new_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Show related notes")]
    async fn notebook_related(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let engram = self.engram.lock().unwrap();

        let related = engram.get_related(input.id as u64);
        if related.is_empty() {
            format!("No notes related to #{}", input.id)
        } else {
            let mut out = format!("|RELATED|{}\n", related.len());
            for (id, weight, edge_type) in related {
                out.push_str(&format!("  #{} ({:.2}, {:?})\n", id, weight, edge_type));
            }
            out
        }
    }
}

#[tool_handler]
impl ServerHandler for EngramServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("AI Memory - Use notebook_remember to save notes, notebook_recall to search, notebook_pinned to see important notes.".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let server = EngramServer::new()?;
    eprintln!("[engram-mcp] Server started");
    server.serve(stdio()).await?.waiting().await?;
    Ok(())
}
