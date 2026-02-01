//! Anthropic Claude API provider

use std::pin::Pin;
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::types::*;
use super::provider::LlmProvider;

/// Anthropic Claude API provider
pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model,
        }
    }

    fn build_request_body(
        &self,
        messages: &[ChatMessage],
        params: &GenerationParams,
        stream: bool,
    ) -> serde_json::Value {
        // Extract system message
        let system = messages.iter()
            .find(|m| m.role == MessageRole::System)
            .map(|m| m.content.clone());

        // Convert messages (excluding system)
        let messages_json: Vec<serde_json::Value> = messages.iter()
            .filter(|m| m.role != MessageRole::System)
            .map(|msg| {
                let role = match msg.role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "user", // Tool results go as user messages
                    MessageRole::System => "user", // Should be filtered out
                };

                if msg.role == MessageRole::Tool {
                    // Tool result format
                    json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": msg.tool_call_id,
                            "content": msg.content,
                        }]
                    })
                } else {
                    json!({
                        "role": role,
                        "content": msg.content,
                    })
                }
            }).collect();

        let mut body = json!({
            "model": self.model,
            "messages": messages_json,
            "max_tokens": params.max_tokens,
            "stream": stream,
        });

        if let Some(sys) = system {
            body["system"] = json!(sys);
        }

        if params.temperature != 1.0 {
            body["temperature"] = json!(params.temperature);
        }

        if !params.stop_sequences.is_empty() {
            body["stop_sequences"] = json!(params.stop_sequences);
        }

        if !params.tools.is_empty() {
            let tools: Vec<serde_json::Value> = params.tools.iter().map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters,
                })
            }).collect();
            body["tools"] = json!(tools);
        }

        body
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }

    async fn generate(
        &self,
        messages: &[ChatMessage],
        params: &GenerationParams,
    ) -> Result<ChatResponse> {
        let body = self.build_request_body(messages, params, false);

        let response = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to send request to Anthropic API")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error ({}): {}", status, error_text);
        }

        let response_body: AnthropicResponse = response.json().await
            .context("Failed to parse Anthropic response")?;

        let mut content = String::new();
        let mut tool_calls = vec![];

        for block in &response_body.content {
            match block {
                ContentBlock::Text { text } => {
                    content.push_str(text);
                }
                ContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: serde_json::to_string(input).unwrap_or_default(),
                    });
                }
            }
        }

        let finish_reason = match response_body.stop_reason.as_deref() {
            Some("end_turn") => FinishReason::Stop,
            Some("max_tokens") => FinishReason::Length,
            Some("tool_use") => FinishReason::ToolUse,
            Some("stop_sequence") => FinishReason::Stop,
            _ => FinishReason::Stop,
        };

        Ok(ChatResponse {
            content,
            tool_calls,
            finish_reason,
            usage: UsageStats {
                prompt_tokens: response_body.usage.input_tokens,
                completion_tokens: response_body.usage.output_tokens,
                total_tokens: response_body.usage.input_tokens + response_body.usage.output_tokens,
            },
        })
    }

    async fn generate_stream(
        &self,
        messages: &[ChatMessage],
        params: &GenerationParams,
    ) -> Result<Pin<Box<dyn Stream<Item = StreamChunk> + Send>>> {
        let body = self.build_request_body(messages, params, true);

        let response = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to send streaming request to Anthropic API")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Ok(Box::pin(futures::stream::once(async move {
                StreamChunk::Error(format!("Anthropic API error ({}): {}", status, error_text))
            })));
        }

        let stream = response.bytes_stream();

        let mapped = stream
            .map(|result| {
                match result {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        parse_anthropic_sse(&text)
                    }
                    Err(e) => vec![StreamChunk::Error(e.to_string())],
                }
            })
            .flat_map(futures::stream::iter);

        Ok(Box::pin(mapped))
    }

    fn count_tokens(&self, text: &str) -> usize {
        // Rough approximation for Claude
        // Claude tokenizes more efficiently than GPT
        text.len() / 4
    }
}

/// Parse SSE chunks from Anthropic streaming response
fn parse_anthropic_sse(text: &str) -> Vec<StreamChunk> {
    let mut chunks = vec![];

    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(event) = serde_json::from_str::<AnthropicStreamEvent>(data) {
                match event {
                    AnthropicStreamEvent::ContentBlockDelta { delta, .. } => {
                        if let Delta::TextDelta { text } = delta {
                            chunks.push(StreamChunk::Text(text));
                        } else if let Delta::InputJsonDelta { partial_json } = delta {
                            // Tool arguments streaming
                            chunks.push(StreamChunk::ToolCallDelta {
                                id: String::new(), // ID comes from content_block_start
                                arguments_delta: partial_json,
                            });
                        }
                    }
                    AnthropicStreamEvent::ContentBlockStart { content_block, .. } => {
                        if let ContentBlock::ToolUse { id, name, .. } = content_block {
                            chunks.push(StreamChunk::ToolCallStart { id, name });
                        }
                    }
                    AnthropicStreamEvent::ContentBlockStop { index } => {
                        // Tool call completed
                        chunks.push(StreamChunk::ToolCallEnd {
                            id: index.to_string(),
                        });
                    }
                    AnthropicStreamEvent::MessageDelta { delta, usage } => {
                        if let Some(reason) = delta.stop_reason {
                            let fr = match reason.as_str() {
                                "end_turn" => FinishReason::Stop,
                                "max_tokens" => FinishReason::Length,
                                "tool_use" => FinishReason::ToolUse,
                                _ => FinishReason::Stop,
                            };
                            chunks.push(StreamChunk::Done {
                                finish_reason: fr,
                                usage: usage.map(|u| UsageStats {
                                    prompt_tokens: 0,
                                    completion_tokens: u.output_tokens,
                                    total_tokens: u.output_tokens,
                                }),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    chunks
}

// Response types for Anthropic API

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: usize,
    output_tokens: usize,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicStreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: serde_json::Value },

    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },

    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        index: usize,
        delta: Delta,
    },

    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: usize },

    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: MessageDelta,
        usage: Option<StreamUsage>,
    },

    #[serde(rename = "message_stop")]
    MessageStop,

    #[serde(rename = "ping")]
    Ping,

    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Delta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },

    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

#[derive(Debug, Deserialize)]
struct MessageDelta {
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamUsage {
    output_tokens: usize,
}
