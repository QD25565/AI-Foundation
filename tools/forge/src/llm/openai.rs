//! OpenAI-compatible API provider
//!
//! Works with OpenAI, Azure, local servers (llama.cpp server, ollama), Groq, Together, etc.

use std::pin::Pin;
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::types::*;
use super::provider::LlmProvider;

/// OpenAI-compatible API provider
pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    api_base: String,
    model: String,
}

impl OpenAIProvider {
    pub fn new(api_key: String, api_base: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            api_base,
            model,
        }
    }

    fn build_request_body(
        &self,
        messages: &[ChatMessage],
        params: &GenerationParams,
        stream: bool,
    ) -> serde_json::Value {
        let messages_json: Vec<serde_json::Value> = messages.iter().map(|msg| {
            let mut obj = json!({
                "role": match msg.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                },
                "content": msg.content,
            });

            if let Some(ref id) = msg.tool_call_id {
                obj["tool_call_id"] = json!(id);
            }
            if let Some(ref name) = msg.name {
                obj["name"] = json!(name);
            }

            obj
        }).collect();

        let mut body = json!({
            "model": self.model,
            "messages": messages_json,
            "temperature": params.temperature,
            "max_tokens": params.max_tokens,
            "stream": stream,
        });

        if !params.stop_sequences.is_empty() {
            body["stop"] = json!(params.stop_sequences);
        }

        if !params.tools.is_empty() {
            let tools: Vec<serde_json::Value> = params.tools.iter().map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            }).collect();
            body["tools"] = json!(tools);
        }

        body
    }
}

#[async_trait]
impl LlmProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
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
            .post(format!("{}/chat/completions", self.api_base))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to send request to OpenAI API")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error ({}): {}", status, error_text);
        }

        let response_body: OpenAIResponse = response.json().await
            .context("Failed to parse OpenAI response")?;

        let choice = response_body.choices.first()
            .ok_or_else(|| anyhow::anyhow!("No choices in response"))?;

        let mut tool_calls = vec![];
        if let Some(ref calls) = choice.message.tool_calls {
            for call in calls {
                tool_calls.push(ToolCall {
                    id: call.id.clone(),
                    name: call.function.name.clone(),
                    arguments: call.function.arguments.clone(),
                });
            }
        }

        let finish_reason = match choice.finish_reason.as_deref() {
            Some("stop") => FinishReason::Stop,
            Some("length") => FinishReason::Length,
            Some("tool_calls") => FinishReason::ToolUse,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => FinishReason::Stop,
        };

        Ok(ChatResponse {
            content: choice.message.content.clone().unwrap_or_default(),
            tool_calls,
            finish_reason,
            usage: UsageStats {
                prompt_tokens: response_body.usage.prompt_tokens,
                completion_tokens: response_body.usage.completion_tokens,
                total_tokens: response_body.usage.total_tokens,
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
            .post(format!("{}/chat/completions", self.api_base))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to send streaming request to OpenAI API")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Ok(Box::pin(futures::stream::once(async move {
                StreamChunk::Error(format!("OpenAI API error ({}): {}", status, error_text))
            })));
        }

        let stream = response.bytes_stream();

        let mapped = stream
            .map(|result| {
                match result {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        parse_sse_chunks(&text)
                    }
                    Err(e) => vec![StreamChunk::Error(e.to_string())],
                }
            })
            .flat_map(futures::stream::iter);

        Ok(Box::pin(mapped))
    }

    fn count_tokens(&self, text: &str) -> usize {
        // Rough approximation: ~4 chars per token for English
        // TODO: Use tiktoken for accurate counting
        text.len() / 4
    }
}

/// Parse SSE chunks from OpenAI streaming response
fn parse_sse_chunks(text: &str) -> Vec<StreamChunk> {
    let mut chunks = vec![];

    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if data == "[DONE]" {
                chunks.push(StreamChunk::Done {
                    finish_reason: FinishReason::Stop,
                    usage: None,
                });
                continue;
            }

            if let Ok(event) = serde_json::from_str::<OpenAIStreamEvent>(data) {
                if let Some(choice) = event.choices.first() {
                    // Text content
                    if let Some(ref content) = choice.delta.content {
                        if !content.is_empty() {
                            chunks.push(StreamChunk::Text(content.clone()));
                        }
                    }

                    // Tool calls
                    if let Some(ref tool_calls) = choice.delta.tool_calls {
                        for tc in tool_calls {
                            if let Some(ref func) = tc.function {
                                // New tool call
                                if func.name.is_some() {
                                    chunks.push(StreamChunk::ToolCallStart {
                                        id: tc.id.clone().unwrap_or_default(),
                                        name: func.name.clone().unwrap_or_default(),
                                    });
                                }

                                // Arguments delta
                                if let Some(ref args) = func.arguments {
                                    if !args.is_empty() {
                                        chunks.push(StreamChunk::ToolCallDelta {
                                            id: tc.id.clone().unwrap_or_default(),
                                            arguments_delta: args.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    }

                    // Finish reason
                    if let Some(ref reason) = choice.finish_reason {
                        let fr = match reason.as_str() {
                            "stop" => FinishReason::Stop,
                            "length" => FinishReason::Length,
                            "tool_calls" => FinishReason::ToolUse,
                            "content_filter" => FinishReason::ContentFilter,
                            _ => FinishReason::Stop,
                        };
                        chunks.push(StreamChunk::Done {
                            finish_reason: fr,
                            usage: None,
                        });
                    }
                }
            }
        }
    }

    chunks
}

// Response types for OpenAI API

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
    usage: OpenAIUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAIToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAIToolCall {
    id: String,
    function: OpenAIFunction,
}

#[derive(Debug, Deserialize)]
struct OpenAIFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: usize,
    completion_tokens: usize,
    total_tokens: usize,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamEvent {
    choices: Vec<OpenAIStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamChoice {
    delta: OpenAIStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAIStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamToolCall {
    id: Option<String>,
    function: Option<OpenAIStreamFunction>,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamFunction {
    name: Option<String>,
    arguments: Option<String>,
}
