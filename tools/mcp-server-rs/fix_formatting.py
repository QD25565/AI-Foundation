import re

with open('src/main.rs', 'r') as f:
    content = f.read()

# Fix notebook_recall to show score breakdown like CLI
old_recall = '''    #[tool(description = "Search notes with hybrid search")]
    async fn notebook_recall(&self, Parameters(input): Parameters<RecallInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.recall(Some(&input.query), input.limit.unwrap_or(10), false) {
            Ok(results) => {
                if results.is_empty() { return format!("No notes for '{}'", input.query); }
                let mut out = format!("{} notes:\\n", results.len());
                for r in results { out.push_str(&format!("#{}: {}\\n", r.note.id, &r.note.content[..80.min(r.note.content.len())])); }
                out
            }
            Err(e) => format!("Error: {}", e),
        }
    }'''

new_recall = '''    #[tool(description = "Search notes with hybrid search (vector + keyword + graph)")]
    async fn notebook_recall(&self, Parameters(input): Parameters<RecallInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.recall(Some(&input.query), input.limit.unwrap_or(10), false) {
            Ok(results) => {
                if results.is_empty() { return format!("No notes matching '{}'", input.query); }
                let mut out = format!("Smart recall: {} result(s)\\n\\n", results.len());
                for r in results {
                    // Score breakdown like CLI
                    let mut score_parts = Vec::new();
                    if r.semantic_score > 0.0 { score_parts.push(format!("vector:{:.0}%", r.semantic_score * 100.0)); }
                    if r.keyword_score > 0.0 { score_parts.push(format!("keyword:{:.0}%", r.keyword_score * 100.0)); }
                    if r.graph_score > 0.0 { score_parts.push(format!("graph:{:.0}%", r.graph_score * 100.0)); }
                    let score_str = if score_parts.is_empty() { "direct".to_string() } else { score_parts.join(" + ") };

                    let tags = if r.note.tags.is_empty() { String::new() } else { format!("[{}]", r.note.tags.join(", ")) };
                    let preview = &r.note.content[..120.min(r.note.content.len())];
                    let pinned = if r.note.pinned { " [PINNED]" } else { "" };

                    out.push_str(&format!("Note #{} | Score: {:.1}% ({}){}\\nTags: {}\\n{}...\\n\\n",
                        r.note.id, r.final_score * 100.0, score_str, pinned, tags, preview));
                }
                out.trim_end().to_string()
            }
            Err(e) => format!("Error: {}", e),
        }
    }'''

content = content.replace(old_recall, new_recall)
print("Fixed recall" if old_recall in content or new_recall in content else "Recall pattern not found - may already be fixed")

# Fix notebook_stats to show all fields like CLI
old_stats = '''    #[tool(description = "Notebook statistics")]
    async fn notebook_stats(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_stats() { Ok(s) => format!("Notes:{}|Pinned:{}|Vault:{}", s.note_count, s.pinned_count, s.vault_entries), Err(e) => format!("Error: {}", e) }
    }'''

new_stats = '''    #[tool(description = "Notebook statistics")]
    async fn notebook_stats(&self) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_stats() {
            Ok(s) => format!(
                "Notebook Statistics (AI: {}):\\n  Total notes: {}\\n  Pinned notes: {}\\n  Embeddings: {}\\n  Graph edges: {}\\n  Vault entries: {}\\n  Total content size: {} bytes",
                ai_id, s.note_count, s.pinned_count, s.embedding_count, s.edge_count, s.vault_entries, s.total_content_size
            ),
            Err(e) => format!("Error: {}", e)
        }
    }'''

content = content.replace(old_stats, new_stats)
print("Fixed stats")

# Fix notebook_list to show relative time and tags like CLI
old_list = '''    #[tool(description = "List recent notes")]
    async fn notebook_list(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.list_notes(input.limit.unwrap_or(10)) {
            Ok(notes) => {
                if notes.is_empty() { return "No notes".to_string(); }
                let mut out = format!("{} notes:\\n", notes.len());
                for n in notes { out.push_str(&format!("#{}: {}\\n", n.id, &n.content[..60.min(n.content.len())])); }
                out
            }
            Err(e) => format!("Error: {}", e),
        }
    }'''

new_list = '''    #[tool(description = "List recent notes")]
    async fn notebook_list(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.list_notes(input.limit.unwrap_or(10)) {
            Ok(notes) => {
                if notes.is_empty() { return "No notes".to_string(); }
                let mut out = format!("Recent notes ({}):\\n\\n", notes.len());
                let now = chrono::Utc::now();
                for n in notes {
                    let age = now.signed_duration_since(n.created);
                    let age_str = if age.num_days() > 0 { format!("{}d ago", age.num_days()) }
                        else if age.num_hours() > 0 { format!("{}h ago", age.num_hours()) }
                        else { format!("{}m ago", age.num_minutes()) };
                    let tags = if n.tags.is_empty() { String::new() } else { format!("[{}]", n.tags.join(", ")) };
                    let pinned = if n.pinned { " [PINNED]" } else { "" };
                    let preview = &n.content[..80.min(n.content.len())];
                    out.push_str(&format!("#{} {} {}{}: {}...\\n", n.id, age_str, tags, pinned, preview));
                }
                out.trim_end().to_string()
            }
            Err(e) => format!("Error: {}", e),
        }
    }'''

content = content.replace(old_list, new_list)
print("Fixed list")

# Fix notebook_get to show full details
old_get = '''    #[tool(description = "Get note by ID")]
    async fn notebook_get(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_note(input.id) {
            Ok(Some(n)) => format!("#{}\\nTags: {}\\n\\n{}", n.id, n.tags.join(","), n.content),
            Ok(None) => format!("Note #{} not found", input.id),
            Err(e) => format!("Error: {}", e),
        }
    }'''

new_get = '''    #[tool(description = "Get note by ID")]
    async fn notebook_get(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_note(input.id) {
            Ok(Some(n)) => {
                let pinned = if n.pinned { " [PINNED]" } else { "" };
                let tags = if n.tags.is_empty() { "none".to_string() } else { n.tags.join(", ") };
                format!("Note #{}{}\\nCreated: {}\\nTags: [{}]\\nPriority: {:?}\\n\\n{}",
                    n.id, pinned, n.created.format("%Y-%m-%d %H:%M:%S UTC"), tags, n.priority, n.content)
            },
            Ok(None) => format!("Note #{} not found", input.id),
            Err(e) => format!("Error: {}", e),
        }
    }'''

content = content.replace(old_get, new_get)
print("Fixed get")

# Fix notebook_remember to show more detail
old_remember = '''    #[tool(description = "Save a note to your private memory")]
    async fn notebook_remember(&self, Parameters(input): Parameters<RememberInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        let mut note = Note::new(input.content, parse_tags(input.tags));
        note.priority = input.priority.map(|p| parse_priority(&p)).unwrap_or(NotePriority::Normal);
        match notebook.remember(&note) {
            Ok(id) => format!("Note saved|ID: {}|Tags: {}", id, note.tags.join(", ")),
            Err(e) => format!("Error: {}", e),
        }
    }'''

new_remember = '''    #[tool(description = "Save a note to your private memory (auto-generates embeddings and links)")]
    async fn notebook_remember(&self, Parameters(input): Parameters<RememberInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        let tags = parse_tags(input.tags.clone());
        let mut note = Note::new(input.content.clone(), tags.clone());
        note.priority = input.priority.map(|p| parse_priority(&p)).unwrap_or(NotePriority::Normal);
        match notebook.remember(&note) {
            Ok(id) => {
                let tags_str = if tags.is_empty() { "none".to_string() } else { tags.join(", ") };
                let preview = &input.content[..60.min(input.content.len())];
                format!("Note saved: ID {}\\nTags: {}\\nPreview: {}...", id, tags_str, preview)
            },
            Err(e) => format!("Error: {}", e),
        }
    }'''

content = content.replace(old_remember, new_remember)
print("Fixed remember")

with open('src/main.rs', 'w') as f:
    f.write(content)

print("\\nDone! Formatting updated to match CLI output.")
