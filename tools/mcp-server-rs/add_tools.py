import re

# Read the file
with open('src/main.rs', 'r') as f:
    content = f.read()

# New input schemas to add after existing ones (after PathInput struct)
new_schemas = '''
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EntityCreateInput {
    #[schemars(description = "Entity name")]
    pub name: String,
    #[schemars(description = "Type: person, project, concept, etc")]
    pub entity_type: String,
    #[schemars(description = "JSON properties")]
    pub properties: Option<String>,
    #[schemars(description = "Alternative names")]
    pub aliases: Option<Vec<String>>,
    #[schemars(description = "Confidence 0-1")]
    pub confidence: Option<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EntityFindInput {
    #[schemars(description = "Entity name to find")]
    pub name: String,
    #[schemars(description = "Filter by type")]
    pub entity_type: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EntityListInput {
    #[schemars(description = "Filter by type")]
    pub entity_type: Option<String>,
    #[schemars(description = "Max results")]
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EntityUpdateInput {
    #[schemars(description = "Entity ID")]
    pub entity_id: String,
    #[schemars(description = "Field to update")]
    pub field: String,
    #[schemars(description = "New value")]
    pub value: String,
    #[schemars(description = "Why updating")]
    pub rationale: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EntityRelateInput {
    #[schemars(description = "From entity ID")]
    pub from_id: String,
    #[schemars(description = "To entity ID")]
    pub to_id: String,
    #[schemars(description = "Relationship type")]
    pub relation: String,
    #[schemars(description = "Strength 0-1")]
    pub strength: Option<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EntityRelatedInput {
    #[schemars(description = "Entity ID")]
    pub entity_id: String,
    #[schemars(description = "Filter by relation type")]
    pub relation: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EntityHistoryInput {
    #[schemars(description = "Entity ID")]
    pub entity_id: String,
    #[schemars(description = "Max results")]
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GraphLinkInput {
    #[schemars(description = "From note ID")]
    pub from_id: i64,
    #[schemars(description = "To note ID")]
    pub to_id: i64,
    #[schemars(description = "Relation type")]
    pub relation: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GraphUnlinkInput {
    #[schemars(description = "From note ID")]
    pub from_id: i64,
    #[schemars(description = "To note ID")]
    pub to_id: i64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PlaybookStrategyInput {
    #[schemars(description = "Strategy title")]
    pub title: String,
    #[schemars(description = "Context/situation")]
    pub context: String,
    #[schemars(description = "The approach")]
    pub approach: String,
    #[schemars(description = "Tags")]
    pub tags: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PlaybookInsightInput {
    #[schemars(description = "The discovery")]
    pub discovery: String,
    #[schemars(description = "Confidence 0-1")]
    pub confidence: Option<f64>,
    #[schemars(description = "Tags")]
    pub tags: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PlaybookPatternInput {
    #[schemars(description = "Situation")]
    pub situation: String,
    #[schemars(description = "Pattern")]
    pub pattern: String,
    #[schemars(description = "Strength 0-1")]
    pub strength: Option<f64>,
    #[schemars(description = "Tags")]
    pub tags: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PlaybookFeedbackInput {
    #[schemars(description = "Strategy ID")]
    pub strategy_id: String,
    #[schemars(description = "Was it helpful?")]
    pub helpful: bool,
}
'''

# New tools to add
new_tools = '''
    // ============== ENTITY TOOLS ==============

    #[tool(description = "Create an entity (person, concept, project)")]
    async fn entity_create(&self, Parameters(input): Parameters<EntityCreateInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.create_entity(&input.name, &input.entity_type, &input.properties.unwrap_or_default(), input.aliases.unwrap_or_default(), input.confidence.unwrap_or(1.0)) {
            Ok(id) => format!("Entity created|ID: {}", id), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Find an entity by name")]
    async fn entity_find(&self, Parameters(input): Parameters<EntityFindInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.find_entity(&input.name, input.entity_type.as_deref()) {
            Ok(Some(e)) => format!("{}|{}|{}", e.id, e.name, e.entity_type), Ok(None) => "Not found".into(), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "List entities")]
    async fn entity_list(&self, Parameters(input): Parameters<EntityListInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.list_entities(input.entity_type.as_deref(), input.limit.unwrap_or(20), "updated") {
            Ok(list) => { if list.is_empty() { "No entities".into() } else { list.iter().map(|e| format!("{}:{}", e.name, e.entity_type)).collect::<Vec<_>>().join("|") } }, Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Update an entity field")]
    async fn entity_update(&self, Parameters(input): Parameters<EntityUpdateInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.update_entity(&input.entity_id, &input.field, &input.value, &input.rationale.unwrap_or_default(), 1.0) {
            Ok(Some(id)) => format!("Updated: {}", id), Ok(None) => "Not found".into(), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Create relationship between entities")]
    async fn entity_relate(&self, Parameters(input): Parameters<EntityRelateInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.update_relationship(&input.from_id, &input.to_id, &input.relation, input.strength.unwrap_or(1.0), vec![]) {
            Ok((id, _)) => format!("Related: {}", id), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Get related entities")]
    async fn entity_related(&self, Parameters(input): Parameters<EntityRelatedInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_related_entities(&input.entity_id, input.relation.as_deref(), 0.0) {
            Ok(list) => { if list.is_empty() { "None".into() } else { list.iter().map(|(e, r, s)| format!("{}:{}:{:.1}", e.name, r, s)).collect::<Vec<_>>().join("|") } }, Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Get entity history")]
    async fn entity_history(&self, Parameters(input): Parameters<EntityHistoryInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_entity_updates(Some(&input.entity_id), input.limit.unwrap_or(10)) {
            Ok(list) => { if list.is_empty() { "No history".into() } else { format!("{} updates", list.len()) } }, Err(e) => format!("Error: {}", e)
        }
    }

    // ============== GRAPH TOOLS ==============

    #[tool(description = "Link two notes")]
    async fn graph_link(&self, Parameters(input): Parameters<GraphLinkInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.link_notes(input.from_id, input.to_id, &input.relation.unwrap_or_else(|| "related".into()), 1.0) {
            Ok(_) => format!("Linked {} -> {}", input.from_id, input.to_id), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Unlink two notes")]
    async fn graph_unlink(&self, Parameters(input): Parameters<GraphUnlinkInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.unlink_notes(input.from_id, input.to_id) {
            Ok(true) => "Unlinked".into(), Ok(false) => "Not linked".into(), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Get linked notes")]
    async fn graph_get_linked(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_linked_notes(input.id, 1) {
            Ok(list) => { if list.is_empty() { "None".into() } else { list.iter().map(|(n, r, w)| format!("#{}:{}:{:.1}", n.id, r, w)).collect::<Vec<_>>().join("|") } }, Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Show graph edges")]
    async fn graph_show(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_all_edges(input.limit.unwrap_or(50)) {
            Ok(edges) => format!("{} edges", edges.len()), Err(e) => format!("Error: {}", e)
        }
    }

    // ============== PLAYBOOK TOOLS ==============

    #[tool(description = "Add a strategy to playbook")]
    async fn playbook_strategy(&self, Parameters(input): Parameters<PlaybookStrategyInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.add_strategy(&input.title, &input.context, &input.approach, &parse_tags(input.tags)) {
            Ok(id) => format!("Strategy: {}", id), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Add an insight to playbook")]
    async fn playbook_insight(&self, Parameters(input): Parameters<PlaybookInsightInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.add_insight(&input.discovery, &[], input.confidence.unwrap_or(0.8), &parse_tags(input.tags)) {
            Ok(id) => format!("Insight: {}", id), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Add a pattern to playbook")]
    async fn playbook_pattern(&self, Parameters(input): Parameters<PlaybookPatternInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.add_pattern(&input.situation, &input.pattern, &[], input.strength.unwrap_or(0.8), &parse_tags(input.tags)) {
            Ok(id) => format!("Pattern: {}", id), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Give feedback on strategy")]
    async fn playbook_feedback(&self, Parameters(input): Parameters<PlaybookFeedbackInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.strategy_feedback(&input.strategy_id, input.helpful) {
            Ok(true) => "Recorded".into(), Ok(false) => "Not found".into(), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "List top strategies")]
    async fn playbook_top(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.list_strategies(input.limit.unwrap_or(10)) {
            Ok(list) => { if list.is_empty() { "None".into() } else { list.iter().map(|(id, t, _, s, _)| format!("{}:{:.1}:{}", id, s, t)).collect::<Vec<_>>().join("|") } }, Err(e) => format!("Error: {}", e)
        }
    }

    // ============== MAINTENANCE TOOLS ==============

    #[tool(description = "Check notebook schema")]
    async fn notebook_check_schema(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.check_schema() {
            Ok(s) => format!("Tables:{}|Missing:{}", s.tables_found, s.missing_tables.len()), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Migrate notebook schema")]
    async fn notebook_migrate(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.force_migrate() {
            Ok(r) => format!("Tables:{}|Indices:{}", r.tables_created, r.indices_created), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Repair notebook")]
    async fn notebook_repair(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.repair() {
            Ok(r) => format!("Integrity:{}", r.integrity_ok), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Backfill embeddings")]
    async fn notebook_backfill(&self) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.backfill_all() {
            Ok(s) => format!("E:{}|T:{}|S:{}", s.embeddings_added, s.temporal_links, s.semantic_links), Err(e) => format!("Error: {}", e)
        }
    }

    #[tool(description = "Get pinned notes")]
    async fn notebook_pinned(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let state = self.state.read().await;
        let notebook = state.notebook.lock().unwrap();
        match notebook.get_pinned_notes(input.limit.unwrap_or(20)) {
            Ok(list) => { if list.is_empty() { "None".into() } else { format!("{} pinned", list.len()) } }, Err(e) => format!("Error: {}", e)
        }
    }
'''

# Insert schemas after PathInput
schema_marker = "pub struct PathInput {\n    #[schemars(description = \"File path\")]\n    pub path: String,\n}"
if schema_marker in content:
    content = content.replace(schema_marker, schema_marker + new_schemas)
    print("Inserted schemas after PathInput")
else:
    print("ERROR: Schema marker not found")

# Insert tools before closing impl brace
tool_marker = "}\n\n#[tool_handler]"
if tool_marker in content:
    content = content.replace(tool_marker, new_tools + "\n}\n\n#[tool_handler]")
    print("Inserted tools before #[tool_handler]")
else:
    print("ERROR: Tool marker not found")

with open('src/main.rs', 'w') as f:
    f.write(content)

print("Done!")
