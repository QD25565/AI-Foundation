//! Common types for LLM interactions
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Message role in conversation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,

    /// Tool call ID (for tool responses)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,

    /// Tool name (for tool responses)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            name: Some(name.into()),
        }
    }
}

/// A tool call requested by the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String, // JSON string
}

/// Tool definition for the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value, // JSON Schema
}

/// Generation parameters
#[derive(Debug, Clone)]
pub struct GenerationParams {
    pub temperature: f32,
    pub max_tokens: usize,
    pub stop_sequences: Vec<String>,
    pub tools: Vec<ToolDefinition>,
}

impl Default for GenerationParams {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            max_tokens: 4096,
            stop_sequences: vec![],
            tools: vec![],
        }
    }
}

/// A chunk of streaming response
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Text content
    Text(String),

    /// Tool call (may be partial during streaming)
    ToolCallStart {
        id: String,
        name: String,
    },

    /// Tool call arguments (streamed)
    ToolCallDelta {
        id: String,
        arguments_delta: String,
    },

    /// Tool call completed
    ToolCallEnd {
        id: String,
    },

    /// Generation finished
    Done {
        finish_reason: FinishReason,
        usage: Option<UsageStats>,
    },

    /// Error occurred
    Error(String),
}

/// Reason generation finished
#[derive(Debug, Clone, PartialEq)]
pub enum FinishReason {
    Stop,
    Length,
    ToolUse,
    ContentFilter,
    Error,
}

/// Token usage statistics
#[derive(Debug, Clone, Default)]
pub struct UsageStats {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

/// Complete response (non-streaming)
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: FinishReason,
    pub usage: UsageStats,
}

impl Default for ChatResponse {
    fn default() -> Self {
        Self {
            content: String::new(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: UsageStats::default(),
        }
    }
}
