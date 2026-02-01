content = r'''//! AI Foundation MCP Server - Thin CLI Wrapper Architecture
//! All tools call CLI executables via subprocess.

use anyhow::Result;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};

mod cli_wrapper;

// ============== Input Schemas ==============

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RememberInput { pub content: String, pub tags: Option<String>, pub priority: Option<String> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RecallInput { pub query: String, pub limit: Option<i64> }

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

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateNoteInput { pub id: i64, pub content: Option<String>, pub tags: Option<String> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddTagsInput { pub note_id: i64, pub tags: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchIdsInput { pub ids: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchTagInput { pub ids: String, pub tags: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TimeRangeInput { pub start: String, pub end: Option<String>, pub limit: Option<i64> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TraverseInput { pub note_id: i64, pub max_depth: Option<i32> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PathFindInput { pub from_id: i64, pub to_id: i64, pub max_depth: Option<i32> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AutoLinkInput { pub note_id: i64, pub top_k: Option<u32>, pub min_similarity: Option<f32>, pub window_minutes: Option<i32> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GraphLinkInput { pub from_id: i64, pub to_id: i64, pub relation: Option<String> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VaultStoreInput { pub key: String, pub value: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VaultGetInput { pub key: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskAddInput { pub description: String, pub priority: Option<i32>, pub tags: Option<String> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskIdInput { pub id: i32 }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskCompleteInput { pub id: i32, pub result: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskBlockInput { pub id: i32, pub reason: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskListInput { pub status: Option<String>, pub limit: Option<i32> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TaskUpdateInput { pub id: i32, pub status: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FindTaskInput { pub query: String, pub limit: Option<i32> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteTaskInput { pub task_id: i32 }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ClaimTaskByIdInput { pub task_id: i32 }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueStartInput { pub responder: String, pub topic: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueIdInput { pub dialogue_id: u64 }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DialogueEndInput { pub dialogue_id: u64, pub status: Option<String> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RoomCreateInput { pub name: String, pub topic: Option<String> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RoomIdInput { pub room_id: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VoteCreateInput { pub topic: String, pub options: String, pub voters: i32 }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VoteCastInput { pub vote_id: i32, pub choice: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VoteIdInput { pub vote_id: i32 }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LockAcquireInput { pub resource: String, pub working_on: String, pub duration: Option<u32> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LockResourceInput { pub resource: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FileClaimInput { pub path: String, pub duration: Option<i32> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PathInput { pub path: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FileActionInput { pub file_path: String, pub action: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StigmergySenseInput { pub location: String, pub pheromone_type: Option<String> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StandbyInput { pub timeout: Option<i64> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PresenceInput { pub status: Option<String>, pub current_task: Option<String> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AiIdInput { pub ai_id: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectCreateInput { pub name: String, pub goal: String }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectIdInput { pub project_id: i32 }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectTaskInput { pub project_id: i32, pub title: String, pub priority: Option<i32> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateFeatureInput { pub project_id: i32, pub name: String, pub overview: String, pub directory: Option<String> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetFeatureInput { pub feature_id: i32 }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListFeaturesInput { pub project_id: i32 }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ChannelInput { pub channel: String, pub limit: Option<i32> }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct IdentityVerifyInput { pub fingerprint: String }

// ============== Server ==============

#[derive(Clone)]
pub struct AiFoundationServer {
    tool_router: ToolRouter<Self>,
}

impl AiFoundationServer {
    pub fn new() -> Self {
        Self { tool_router: Self::tool_router() }
    }
}
'''
with open('main.rs', 'w') as f:
    f.write(content)
print('Part 1 written')
