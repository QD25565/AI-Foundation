//! Provider trait and factory
#![allow(dead_code)]

use std::pin::Pin;
use async_trait::async_trait;
use futures::Stream;
use anyhow::Result;

use super::types::*;
use crate::config::{ProviderConfig, ProviderType, ModelConfig};

/// Trait for LLM providers
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Get provider name
    fn name(&self) -> &str;

    /// Check if provider is available (API key set, model loaded, etc.)
    async fn is_available(&self) -> bool;

    /// Generate a complete response (non-streaming)
    async fn generate(
        &self,
        messages: &[ChatMessage],
        params: &GenerationParams,
    ) -> Result<ChatResponse>;

    /// Generate a streaming response
    async fn generate_stream(
        &self,
        messages: &[ChatMessage],
        params: &GenerationParams,
    ) -> Result<Pin<Box<dyn Stream<Item = StreamChunk> + Send>>>;

    /// Count tokens in text (approximate for some providers)
    fn count_tokens(&self, text: &str) -> usize {
        // Default: rough estimate of 4 chars per token
        text.len() / 4
    }
}

/// Create a provider from config
pub async fn create_provider(
    provider_config: &ProviderConfig,
    model_config: &ModelConfig,
) -> Result<Box<dyn LlmProvider>> {
    match provider_config.provider_type {
        ProviderType::OpenAI => {
            let api_key = provider_config.api_key_env
                .as_ref()
                .and_then(|env| std::env::var(env).ok())
                .ok_or_else(|| anyhow::anyhow!("API key not found for OpenAI provider"))?;

            let api_base = provider_config.api_base
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

            Ok(Box::new(super::openai::OpenAIProvider::new(
                api_key,
                api_base,
                model_config.name.clone(),
            )))
        }

        ProviderType::Anthropic => {
            let api_key = provider_config.api_key_env
                .as_ref()
                .and_then(|env| std::env::var(env).ok())
                .ok_or_else(|| anyhow::anyhow!("API key not found for Anthropic provider"))?;

            Ok(Box::new(super::anthropic::AnthropicProvider::new(
                api_key,
                model_config.name.clone(),
            )))
        }

        ProviderType::Local => {
            #[cfg(feature = "local-llm")]
            {
                let model_path = provider_config.model_path
                    .as_ref()
                    .map(|p| std::path::PathBuf::from(p))
                    .or_else(|| super::local::LocalProvider::find_model(&model_config.name).ok())
                    .ok_or_else(|| anyhow::anyhow!(
                        "Model path not specified and '{}' not found.\n\
                        Set model_path in config or place GGUF file in ./models/",
                        model_config.name
                    ))?;

                let gpu_layers = provider_config.gpu_layers.unwrap_or(-1); // -1 = auto
                let context_size = model_config.context_size as u32;

                Ok(Box::new(super::local::LocalProvider::new(
                    model_path,
                    context_size,
                    gpu_layers,
                )))
            }

            #[cfg(not(feature = "local-llm"))]
            {
                anyhow::bail!(
                    "Local LLM support not compiled.\n\
                    Build with: cargo build --release --features local-llm --target x86_64-pc-windows-gnu"
                )
            }
        }

        ProviderType::Google => {
            // TODO: Implement Google/Gemini provider
            anyhow::bail!("Google provider not yet implemented")
        }
    }
}

/// A mock provider for testing
#[derive(Debug)]
pub struct MockProvider {
    name: String,
    responses: Vec<String>,
    current: std::sync::atomic::AtomicUsize,
}

impl MockProvider {
    pub fn new(responses: Vec<String>) -> Self {
        Self {
            name: "mock".to_string(),
            responses,
            current: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn is_available(&self) -> bool {
        true
    }

    async fn generate(
        &self,
        _messages: &[ChatMessage],
        _params: &GenerationParams,
    ) -> Result<ChatResponse> {
        let idx = self.current.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let response = self.responses.get(idx % self.responses.len())
            .cloned()
            .unwrap_or_else(|| "Mock response".to_string());

        Ok(ChatResponse {
            content: response,
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: UsageStats::default(),
        })
    }

    async fn generate_stream(
        &self,
        messages: &[ChatMessage],
        params: &GenerationParams,
    ) -> Result<Pin<Box<dyn Stream<Item = StreamChunk> + Send>>> {
        // For mock, just return complete response as single chunk
        let response = self.generate(messages, params).await?;

        let chunks = vec![
            StreamChunk::Text(response.content),
            StreamChunk::Done {
                finish_reason: FinishReason::Stop,
                usage: Some(response.usage),
            },
        ];

        Ok(Box::pin(futures::stream::iter(chunks)))
    }
}
