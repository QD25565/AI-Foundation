//\! AI Foundation MCP Server - Thin CLI Wrapper Architecture
//\! All tools call CLI executables via subprocess.

use anyhow::Result;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};

mod cli_wrapper;

// Schemas
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RememberInput {
    pub content: String,
    pub tags: Option<String>,
    pub priority: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RecallInput {
    pub query: String,
    pub limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteIdInput { pub id: i64 }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LimitInput { pub limit: Option<i64> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BroadcastInput { pub content: String, pub channel: Option<String> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DmInput { pub to_ai: String, pub content: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ContentInput { pub content: String }

// Server
#[derive(Clone)]
pub struct AiFoundationServer {
    tool_router: ToolRouter<Self>,
}

impl AiFoundationServer {
    pub fn new() -> Self {
        Self { tool_router: Self::tool_router() }
    }
}

#[tool_router]
impl AiFoundationServer {
    #[tool(description = "Save note to memory")]
    async fn notebook_remember(&self, Parameters(input): Parameters<RememberInput>) -> String {
        let mut args = vec\!["remember", &input.content];
        let tags_owned: String;
        if let Some(ref t) = input.tags {
            tags_owned = t.clone();
            args.push("--tags");
            args.push(&tags_owned);
        }
        cli_wrapper::notebook(&args).await
    }

    #[tool(description = "Search notes")]
    async fn notebook_recall(&self, Parameters(input): Parameters<RecallInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::notebook(&["recall", &input.query, "--limit", &limit]).await
    }

    #[tool(description = "Notebook stats")]
    async fn notebook_stats(&self) -> String {
        cli_wrapper::notebook(&["stats"]).await
    }

    #[tool(description = "List notes")]
    async fn notebook_list(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::notebook(&["list", "--limit", &limit]).await
    }

    #[tool(description = "Get note by ID")]
    async fn notebook_get(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::notebook(&["get", &id]).await
    }

    #[tool(description = "Pin note")]
    async fn notebook_pin(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::notebook(&["pin", &id]).await
    }

    #[tool(description = "Unpin note")]
    async fn notebook_unpin(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::notebook(&["unpin", &id]).await
    }

    #[tool(description = "Delete note")]
    async fn notebook_delete(&self, Parameters(input): Parameters<NoteIdInput>) -> String {
        let id = input.id.to_string();
        cli_wrapper::notebook(&["delete", &id]).await
    }

    #[tool(description = "Broadcast message")]
    async fn teambook_broadcast(&self, Parameters(input): Parameters<BroadcastInput>) -> String {
        let channel = input.channel.unwrap_or_else(|| "general".to_string());
        cli_wrapper::teambook(&["broadcast", &input.content, "--channel", &channel]).await
    }

    #[tool(description = "Send DM")]
    async fn teambook_dm(&self, Parameters(input): Parameters<DmInput>) -> String {
        cli_wrapper::teambook(&["dm", &input.to_ai, &input.content]).await
    }

    #[tool(description = "Get DMs")]
    async fn teambook_direct_messages(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::teambook(&["direct-messages", &limit]).await
    }

    #[tool(description = "Get broadcasts")]
    async fn teambook_messages(&self, Parameters(input): Parameters<LimitInput>) -> String {
        let limit = input.limit.unwrap_or(10).to_string();
        cli_wrapper::teambook(&["messages", &limit]).await
    }

    #[tool(description = "Teambook status")]
    async fn teambook_status(&self) -> String {
        cli_wrapper::teambook(&["status"]).await
    }

    #[tool(description = "Who is here")]
    async fn teambook_who_is_here(&self) -> String {
        cli_wrapper::teambook(&["who"]).await
    }

    #[tool(description = "Echo test")]
    async fn util_echo(&self, Parameters(input): Parameters<ContentInput>) -> String {
        input.content
    }

    #[tool(description = "Get UTC time")]
    async fn util_time(&self) -> String {
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string()
    }

    #[tool(description = "Generate UUID")]
    async fn util_uuid(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

#[tool_handler]
impl ServerHandler for AiFoundationServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            name: "ai-f".into(),
            version: "30.0.0".into(),
            instructions: Some("AI Foundation - CLI Wrapper".into()),
            ..Default::default()
        }
    }
    fn get_capabilities(&self) -> ServerCapabilities {
        ServerCapabilities::builder().enable_tools().build()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let server = AiFoundationServer::new();
    server.serve(stdio()).await?.waiting().await?;
    Ok(())
}
