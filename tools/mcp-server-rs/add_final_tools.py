import re

with open('src/main.rs', 'r') as f:
    content = f.read()

# New schemas for final tools
new_schemas = '''
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TagSearchInput {
    #[schemars(description = "Tag to search for")]
    pub tag: String,
    #[schemars(description = "Max results")]
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DateRangeInput {
    #[schemars(description = "Start date (YYYY-MM-DD)")]
    pub start: String,
    #[schemars(description = "End date (YYYY-MM-DD)")]
    pub end: String,
    #[schemars(description = "Max results")]
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EntityIdInput {
    #[schemars(description = "Entity ID")]
    pub entity_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EntityMergeInput {
    #[schemars(description = "Source entity ID (will be deleted)")]
    pub source_id: String,
    #[schemars(description = "Target entity ID (will be kept)")]
    pub target_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GraphPathInput {
    #[schemars(description = "Start note ID")]
    pub from_id: i64,
    #[schemars(description = "End note ID")]
    pub to_id: i64,
    #[schemars(description = "Max path length")]
    pub max_depth: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GraphTraverseInput {
    #[schemars(description = "Starting note ID")]
    pub start_id: i64,
    #[schemars(description = "Max depth")]
    pub depth: Option<i64>,
    #[schemars(description = "Relation type filter")]
    pub relation: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StrategyIdInput {
    #[schemars(description = "Strategy ID")]
    pub strategy_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QuickNoteInput {
    #[schemars(description = "Quick note content")]
    pub content: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TemplateInput {
    #[schemars(description = "Template name")]
    pub name: String,
    #[schemars(description = "Template content")]
    pub content: String,
    #[schemars(description = "Tags")]
    pub tags: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ApplyTemplateInput {
    #[schemars(description = "Template name")]
    pub name: String,
    #[schemars(description = "Variables to substitute (JSON)")]
    pub vars: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TagMergeInput {
    #[schemars(description = "Old tag name")]
    pub old_tag: String,
    #[schemars(description = "New tag name")]
    pub new_tag: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SimilarityInput {
    #[schemars(description = "Note ID to find similar notes for")]
    pub note_id: i64,
    #[schemars(description = "Max results")]
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnnotateInput {
    #[schemars(description = "Note ID")]
    pub note_id: i64,
    #[schemars(description = "Annotation key")]
    pub key: String,
    #[schemars(description = "Annotation value")]
    pub value: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ImportJsonInput {
    #[schemars(description = "JSON content to import")]
    pub json: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SummaryInput {
    #[schemars(description = "Time range: day, week, month")]
    pub range: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConversationInput {
    #[schemars(description = "Conversation ID")]
    pub conversation_id: String,
    #[schemars(description = "Message content")]
    pub message: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ContextExtractInput {
    #[schemars(description = "Text to extract context from")]
    pub text: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchUpdateInput {
    #[schemars(description = "Note IDs (comma-separated)")]
    pub ids: String,
    #[schemars(description = "Field to update")]
    pub field: String,
    #[schemars(description = "New value")]
    pub value: String,
}
'''

# New tools to add
new_tools = '''
    // ============== NOTE SEARCH VARIANTS ==============

    #[tool(description = "Search notes by tag")]
    async fn notebook_search_by_tag(&self, Parameters(input): Parameters<TagSearchInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.search_by_tag(&input.tag, input.limit.unwrap_or(20)) {
            Ok(notes) => {
                if notes.is_empty() { return format!("No notes with tag '{}'", input.tag); }
                let mut out = format!("{} notes with tag '{}':\\n", notes.len(), input.tag);
                for n in notes { out.push_str(&format!("#{}: {}...\\n", n.id, &n.content[..60.min(n.content.len())])); }
                out.trim_end().to_string()
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Search notes by date range")]
    async fn notebook_search_by_date(&self, Parameters(input): Parameters<DateRangeInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.search_by_date_range(&input.start, &input.end, input.limit.unwrap_or(20)) {
            Ok(notes) => {
                if notes.is_empty() { return format!("No notes between {} and {}", input.start, input.end); }
                let mut out = format!("{} notes:\\n", notes.len());
                for n in notes { out.push_str(&format!("#{} [{}]: {}...\\n", n.id, n.created.format("%Y-%m-%d"), &n.content[..50.min(n.content.len())])); }
                out.trim_end().to_string()
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Find similar notes using embeddings")]
    async fn notebook_find_similar(&self, Parameters(input): Parameters<SimilarityInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.find_similar_notes(input.note_id, input.limit.unwrap_or(5)) {
            Ok(notes) => {
                if notes.is_empty() { return "No similar notes found".to_string(); }
                let mut out = format!("Similar to #{} ({} results):\\n", input.note_id, notes.len());
                for (n, score) in notes { out.push_str(&format!("#{} ({:.0}%): {}...\\n", n.id, score * 100.0, &n.content[..60.min(n.content.len())])); }
                out.trim_end().to_string()
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== ENTITY EXTENDED ==============

    #[tool(description = "Delete an entity")]
    async fn entity_delete(&self, Parameters(input): Parameters<EntityIdInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.delete_entity(&input.entity_id) {
            Ok(true) => format!("Deleted entity {}", input.entity_id),
            Ok(false) => "Entity not found".to_string(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Merge two entities")]
    async fn entity_merge(&self, Parameters(input): Parameters<EntityMergeInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.merge_entities(&input.source_id, &input.target_id) {
            Ok(_) => format!("Merged {} into {}", input.source_id, input.target_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get entity by ID")]
    async fn entity_get(&self, Parameters(input): Parameters<EntityIdInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_entity(&input.entity_id) {
            Ok(Some(e)) => format!("Entity: {}\\nType: {}\\nConfidence: {:.0}%\\nAliases: {}\\nProperties: {}",
                e.name, e.entity_type, e.confidence * 100.0,
                e.aliases.join(", "), e.properties),
            Ok(None) => "Entity not found".to_string(),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== GRAPH EXTENDED ==============

    #[tool(description = "Find path between two notes")]
    async fn graph_find_path(&self, Parameters(input): Parameters<GraphPathInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.find_path(input.from_id, input.to_id, input.max_depth.unwrap_or(5) as usize) {
            Ok(Some(path)) => format!("Path: {}", path.iter().map(|id| format!("#{}", id)).collect::<Vec<_>>().join(" -> ")),
            Ok(None) => "No path found".to_string(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Traverse graph from a starting note")]
    async fn graph_traverse(&self, Parameters(input): Parameters<GraphTraverseInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.traverse_graph(input.start_id, input.depth.unwrap_or(2) as usize, input.relation.as_deref()) {
            Ok(nodes) => {
                if nodes.is_empty() { return "No connected nodes".to_string(); }
                format!("{} connected nodes: {}", nodes.len(), nodes.iter().map(|(id, rel, d)| format!("#{} ({}, depth {})", id, rel, d)).collect::<Vec<_>>().join(", "))
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get graph clusters")]
    async fn graph_clusters(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_clusters(input.limit.unwrap_or(10) as usize) {
            Ok(clusters) => {
                if clusters.is_empty() { return "No clusters found".to_string(); }
                let mut out = format!("{} clusters:\\n", clusters.len());
                for (i, cluster) in clusters.iter().enumerate() {
                    out.push_str(&format!("Cluster {}: {} notes\\n", i + 1, cluster.len()));
                }
                out.trim_end().to_string()
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== PLAYBOOK EXTENDED ==============

    #[tool(description = "Get strategy by ID")]
    async fn playbook_get_strategy(&self, Parameters(input): Parameters<StrategyIdInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_strategy(&input.strategy_id) {
            Ok(Some((title, context, approach, score, uses))) =>
                format!("Strategy: {}\\nContext: {}\\nApproach: {}\\nScore: {:.1}%\\nUses: {}", title, context, approach, score * 100.0, uses),
            Ok(None) => "Strategy not found".to_string(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Delete a strategy")]
    async fn playbook_delete_strategy(&self, Parameters(input): Parameters<StrategyIdInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.delete_strategy(&input.strategy_id) {
            Ok(true) => format!("Deleted strategy {}", input.strategy_id),
            Ok(false) => "Strategy not found".to_string(),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== TAGS MANAGEMENT ==============

    #[tool(description = "List all tags with counts")]
    async fn notebook_list_tags(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.list_tags() {
            Ok(tags) => {
                if tags.is_empty() { return "No tags".to_string(); }
                format!("{} tags: {}", tags.len(), tags.iter().map(|(t, c)| format!("{}({})", t, c)).collect::<Vec<_>>().join(", "))
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Rename a tag across all notes")]
    async fn notebook_rename_tag(&self, Parameters(input): Parameters<TagMergeInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.rename_tag(&input.old_tag, &input.new_tag) {
            Ok(count) => format!("Renamed '{}' to '{}' in {} notes", input.old_tag, input.new_tag, count),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Merge two tags")]
    async fn notebook_merge_tags(&self, Parameters(input): Parameters<TagMergeInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.merge_tags(&input.old_tag, &input.new_tag) {
            Ok(count) => format!("Merged '{}' into '{}' ({} notes affected)", input.old_tag, input.new_tag, count),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== TEMPLATES ==============

    #[tool(description = "Save a note template")]
    async fn notebook_save_template(&self, Parameters(input): Parameters<TemplateInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.save_template(&input.name, &input.content, &parse_tags(input.tags)) {
            Ok(_) => format!("Template '{}' saved", input.name),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Apply a template to create a note")]
    async fn notebook_apply_template(&self, Parameters(input): Parameters<ApplyTemplateInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        let vars: std::collections::HashMap<String, String> = input.vars
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        match notebook.apply_template(&input.name, &vars) {
            Ok(id) => format!("Created note #{} from template '{}'", id, input.name),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "List available templates")]
    async fn notebook_list_templates(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.list_templates() {
            Ok(templates) => {
                if templates.is_empty() { return "No templates".to_string(); }
                format!("{} templates: {}", templates.len(), templates.join(", "))
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== QUICK NOTES (SCRATCHPAD) ==============

    #[tool(description = "Add a quick note to scratchpad")]
    async fn notebook_quick_note(&self, Parameters(input): Parameters<QuickNoteInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.add_quick_note(&input.content) {
            Ok(id) => format!("Quick note added (ID: {})", id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get scratchpad contents")]
    async fn notebook_get_scratchpad(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_scratchpad() {
            Ok(notes) => {
                if notes.is_empty() { return "Scratchpad empty".to_string(); }
                let mut out = format!("Scratchpad ({} notes):\\n", notes.len());
                for n in notes { out.push_str(&format!("- {}\\n", n)); }
                out.trim_end().to_string()
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Clear scratchpad")]
    async fn notebook_clear_scratchpad(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.clear_scratchpad() {
            Ok(count) => format!("Cleared {} notes from scratchpad", count),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== IMPORT/EXPORT ==============

    #[tool(description = "Import notes from JSON")]
    async fn notebook_import_json(&self, Parameters(input): Parameters<ImportJsonInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.import_from_json(&input.json) {
            Ok(count) => format!("Imported {} notes", count),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Export notes to markdown")]
    async fn notebook_export_markdown(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.export_to_markdown(input.limit.unwrap_or(100)) {
            Ok(md) => md,
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== ANALYTICS ==============

    #[tool(description = "Get activity summary")]
    async fn notebook_summary(&self, Parameters(input): Parameters<SummaryInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        let range = input.range.as_deref().unwrap_or("week");
        match notebook.get_activity_summary(range) {
            Ok(summary) => summary,
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get tag trends")]
    async fn notebook_tag_trends(&self, Parameters(input): Parameters<SummaryInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        let range = input.range.as_deref().unwrap_or("week");
        match notebook.get_tag_trends(range) {
            Ok(trends) => trends,
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== ANNOTATIONS ==============

    #[tool(description = "Add annotation to a note")]
    async fn notebook_annotate(&self, Parameters(input): Parameters<AnnotateInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.add_annotation(input.note_id, &input.key, &input.value) {
            Ok(_) => format!("Annotation added to note #{}", input.note_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get annotations for a note")]
    async fn notebook_get_annotations(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_annotations(input.id) {
            Ok(annots) => {
                if annots.is_empty() { return format!("No annotations on note #{}", input.id); }
                annots.iter().map(|(k, v)| format!("{}: {}", k, v)).collect::<Vec<_>>().join("\\n")
            },
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== BATCH EXTENDED ==============

    #[tool(description = "Update multiple notes")]
    async fn notebook_batch_update(&self, Parameters(input): Parameters<BatchUpdateInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        let ids: Vec<i64> = input.ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        match notebook.batch_update(&ids, &input.field, &input.value) {
            Ok(count) => format!("Updated {} notes", count),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== CONTEXT EXTRACTION ==============

    #[tool(description = "Extract entities and concepts from text")]
    async fn notebook_extract_context(&self, Parameters(input): Parameters<ContextExtractInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.extract_context(&input.text) {
            Ok(ctx) => ctx,
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== SESSION EXTENDED ==============

    #[tool(description = "Get current session info")]
    async fn notebook_session_info(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_session_info() {
            Ok(info) => info,
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "End current session")]
    async fn notebook_end_session(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.end_session() {
            Ok(_) => "Session ended".to_string(),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ============== CONVERSATION TRACKING ==============

    #[tool(description = "Start a conversation thread")]
    async fn notebook_start_conversation(&self, Parameters(input): Parameters<ConversationInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.start_conversation(&input.conversation_id) {
            Ok(_) => format!("Conversation '{}' started", input.conversation_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Add message to conversation")]
    async fn notebook_add_to_conversation(&self, Parameters(input): Parameters<ConversationInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        let msg = input.message.as_deref().unwrap_or("");
        match notebook.add_to_conversation(&input.conversation_id, msg) {
            Ok(_) => format!("Message added to '{}'", input.conversation_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get conversation history")]
    async fn notebook_get_conversation(&self, Parameters(input): Parameters<ConversationInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_conversation(&input.conversation_id) {
            Ok(messages) => {
                if messages.is_empty() { return format!("Conversation '{}' is empty", input.conversation_id); }
                messages.join("\\n---\\n")
            },
            Err(e) => format!("Error: {}", e),
        }
    }
'''

# Insert schemas after the last existing schema
schema_marker = "pub struct CodeGraphSymbolInput {"
if schema_marker in content:
    # Find the end of this struct
    idx = content.find(schema_marker)
    # Find next closing brace after this
    end_idx = content.find("}", idx) + 1
    content = content[:end_idx] + new_schemas + content[end_idx:]
    print("Inserted schemas after CodeGraphSymbolInput")
else:
    # Try alternate location
    alt_marker = "pub struct PlaybookListInput {"
    if alt_marker in content:
        idx = content.find(alt_marker)
        end_idx = content.find("}", idx) + 1
        content = content[:end_idx] + new_schemas + content[end_idx:]
        print("Inserted schemas after PlaybookListInput")
    else:
        print("WARNING: Could not find schema insertion point")

# Insert tools before closing impl brace (before #[tool_handler])
tool_marker = "}\\n\\n#[tool_handler]"
if tool_marker in content:
    content = content.replace(tool_marker, new_tools + "\\n}\\n\\n#[tool_handler]")
    print("Inserted tools")
else:
    # Try simpler pattern
    lines = content.split('\\n')
    for i, line in enumerate(lines):
        if '#[tool_handler]' in line and i > 0:
            # Insert before this line
            lines.insert(i, new_tools)
            content = '\\n'.join(lines)
            print("Inserted tools before #[tool_handler]")
            break
    else:
        print("WARNING: Could not find tool insertion point")

with open('src/main.rs', 'w') as f:
    f.write(content)

print("Done!")
