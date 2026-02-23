//! Local LLM provider using llama.cpp
#![allow(dead_code)]
//!
//! Runs GGUF models directly via llama-cpp-2 crate.
//! No external dependencies - fully self-contained.

#[cfg(feature = "local-llm")]
use std::pin::Pin;
#[cfg(feature = "local-llm")]
use std::path::PathBuf;
#[cfg(feature = "local-llm")]
use std::sync::{Arc, OnceLock};

#[cfg(feature = "local-llm")]
use async_trait::async_trait;
#[cfg(feature = "local-llm")]
use futures::Stream;
#[cfg(feature = "local-llm")]
use anyhow::{Context, Result, bail};
#[cfg(feature = "local-llm")]
use tokio::sync::mpsc;

#[cfg(feature = "local-llm")]
use llama_cpp_2::context::params::LlamaContextParams;
#[cfg(feature = "local-llm")]
use llama_cpp_2::llama_backend::LlamaBackend;
#[cfg(feature = "local-llm")]
use llama_cpp_2::llama_batch::LlamaBatch;
#[cfg(feature = "local-llm")]
use llama_cpp_2::model::params::LlamaModelParams;
#[cfg(feature = "local-llm")]
use llama_cpp_2::model::{LlamaModel, AddBos, Special};
#[cfg(feature = "local-llm")]
use llama_cpp_2::sampling::LlamaSampler;
#[cfg(feature = "local-llm")]
use llama_cpp_2::token::LlamaToken;

#[cfg(feature = "local-llm")]
use super::types::*;
#[cfg(feature = "local-llm")]
use super::provider::LlmProvider;

#[cfg(feature = "local-llm")]
static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();

#[cfg(feature = "local-llm")]
fn get_backend() -> &'static LlamaBackend {
    BACKEND.get_or_init(|| {
        let mut backend = LlamaBackend::init().expect("Failed to initialize llama.cpp backend");
        backend.void_logs(); // Suppress verbose logs
        backend
    })
}

/// Local LLM provider using llama.cpp
#[cfg(feature = "local-llm")]
pub struct LocalProvider {
    model_path: PathBuf,
    model_name: String,
    context_size: u32,
    gpu_layers: i32,
}

#[cfg(feature = "local-llm")]
impl LocalProvider {
    pub fn new(model_path: PathBuf, context_size: u32, gpu_layers: i32) -> Self {
        let model_name = model_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("local-model")
            .to_string();

        Self {
            model_path,
            model_name,
            context_size,
            gpu_layers,
        }
    }

    /// Find a GGUF model file in common locations
    pub fn find_model(model_name: &str) -> Result<PathBuf> {
        let exe_path = std::env::current_exe().ok();
        let search_paths = [
            // Same directory as executable
            exe_path.as_ref().and_then(|p| p.parent()).map(|p| p.join(model_name)),
            exe_path.as_ref().and_then(|p| p.parent()).map(|p| p.join(format!("{}.gguf", model_name))),
            // models/ subdirectory
            exe_path.as_ref().and_then(|p| p.parent()).map(|p| p.join("models").join(model_name)),
            exe_path.as_ref().and_then(|p| p.parent()).map(|p| p.join("models").join(format!("{}.gguf", model_name))),
            // Current directory
            Some(PathBuf::from(model_name)),
            Some(PathBuf::from(format!("{}.gguf", model_name))),
            // ~/.forge/models/
            dirs::home_dir().map(|h| h.join(".forge").join("models").join(model_name)),
            dirs::home_dir().map(|h| h.join(".forge").join("models").join(format!("{}.gguf", model_name))),
        ];

        for path_opt in search_paths.iter() {
            if let Some(path) = path_opt {
                if path.exists() {
                    return Ok(path.clone());
                }
            }
        }

        bail!(
            "Model '{}' not found. Place GGUF file in:\n\
            - Same directory as forge.exe\n\
            - ./models/\n\
            - ~/.forge/models/",
            model_name
        )
    }

    /// Format messages into a prompt string for the model
    fn format_prompt(&self, messages: &[ChatMessage]) -> String {
        // Use ChatML format (widely compatible with instruction-tuned models)
        let mut prompt = String::new();

        for msg in messages {
            let role = match msg.role {
                MessageRole::System => "system",
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::Tool => "tool",
            };
            prompt.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", role, msg.content));
        }

        // Add assistant start to prompt continuation
        prompt.push_str("<|im_start|>assistant\n");
        prompt
    }
}

#[cfg(feature = "local-llm")]
#[async_trait]
impl LlmProvider for LocalProvider {
    fn name(&self) -> &str {
        &self.model_name
    }

    async fn is_available(&self) -> bool {
        self.model_path.exists()
    }

    async fn generate(
        &self,
        messages: &[ChatMessage],
        params: &GenerationParams,
    ) -> Result<ChatResponse> {
        let backend = get_backend();

        // Load model
        let mut model_params = LlamaModelParams::default();
        if self.gpu_layers >= 0 {
            model_params = model_params.with_n_gpu_layers(self.gpu_layers as u32);
        }

        let model = LlamaModel::load_from_file(backend, &self.model_path, &model_params)
            .context("Failed to load GGUF model")?;

        // Create context
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(Some(std::num::NonZeroU32::new(self.context_size).unwrap()));
        let mut ctx = model.new_context(backend, ctx_params)
            .context("Failed to create context")?;

        // Tokenize prompt
        // Note: OLMo-3 and many ChatML models don't use BOS token
        let prompt = self.format_prompt(messages);
        let tokens = model.str_to_token(&prompt, AddBos::Never)
            .context("Failed to tokenize prompt")?;

        // Create batch and decode prompt
        let mut batch = LlamaBatch::new(self.context_size as usize, 1);
        for (i, token) in tokens.iter().enumerate() {
            batch.add(*token, i as i32, &[0], i == tokens.len() - 1)?;
        }
        ctx.decode(&mut batch).context("Failed to decode prompt")?;

        // Sample tokens - chain multiple samplers together
        // IMPORTANT: Must end with a selection sampler (dist or greedy)
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::top_k(40),
            LlamaSampler::top_p(0.9, 1),
            LlamaSampler::min_p(0.1, 1),
            LlamaSampler::temp(params.temperature),
            LlamaSampler::dist(0), // Final selection - 0 = random seed
        ]);

        let mut output_tokens: Vec<LlamaToken> = vec![];
        let mut n_cur = tokens.len();
        let max_tokens = params.max_tokens.min(self.context_size as usize - tokens.len());

        // Stop sequences
        let stop_sequences: Vec<&str> = params.stop_sequences.iter().map(|s| s.as_str()).collect();
        let eos_token = "<|im_end|>";

        while output_tokens.len() < max_tokens {
            // Sample next token
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            output_tokens.push(token);

            // Check for EOS
            if model.is_eog_token(token) {
                break;
            }

            // Decode token to text for stop sequence check
            let token_str = model.token_to_str(token, Special::Tokenize)
                .unwrap_or_default();

            // Check stop sequences
            let current_output: String = output_tokens.iter()
                .filter_map(|t| model.token_to_str(*t, Special::Tokenize).ok())
                .collect();

            if current_output.contains(eos_token) {
                break;
            }
            for stop in &stop_sequences {
                if current_output.contains(stop) {
                    break;
                }
            }

            // Add token to batch for next iteration
            batch.clear();
            batch.add(token, n_cur as i32, &[0], true)?;
            n_cur += 1;

            ctx.decode(&mut batch).context("Failed to decode token")?;
        }

        // Convert tokens to string
        let content: String = output_tokens.iter()
            .filter_map(|t| model.token_to_str(*t, Special::Tokenize).ok())
            .collect();

        // Clean up stop sequences from output
        let mut cleaned = content;
        if let Some(pos) = cleaned.find(eos_token) {
            cleaned = cleaned[..pos].to_string();
        }
        for stop in &stop_sequences {
            if let Some(pos) = cleaned.find(stop) {
                cleaned = cleaned[..pos].to_string();
            }
        }

        Ok(ChatResponse {
            content: cleaned.trim().to_string(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: UsageStats {
                prompt_tokens: tokens.len(),
                completion_tokens: output_tokens.len(),
                total_tokens: tokens.len() + output_tokens.len(),
            },
        })
    }

    async fn generate_stream(
        &self,
        messages: &[ChatMessage],
        params: &GenerationParams,
    ) -> Result<Pin<Box<dyn Stream<Item = StreamChunk> + Send>>> {
        // For streaming, we run generation in a separate task and send chunks
        let model_path = self.model_path.clone();
        let context_size = self.context_size;
        let gpu_layers = self.gpu_layers;
        let messages = messages.to_vec();
        let params = params.clone();
        let prompt = self.format_prompt(&messages);

        let (tx, rx) = mpsc::channel::<StreamChunk>(100);

        // Spawn blocking task for inference
        tokio::task::spawn_blocking(move || {
            let backend = get_backend();

            // Load model
            let mut model_params = LlamaModelParams::default();
            if gpu_layers >= 0 {
                model_params = model_params.with_n_gpu_layers(gpu_layers as u32);
            }

            let model = match LlamaModel::load_from_file(backend, &model_path, &model_params) {
                Ok(m) => m,
                Err(e) => {
                    let _ = tx.blocking_send(StreamChunk::Error(format!("Failed to load model: {}", e)));
                    return;
                }
            };

            // Create context
            let ctx_params = LlamaContextParams::default()
                .with_n_ctx(Some(std::num::NonZeroU32::new(context_size).unwrap()));
            let mut ctx = match model.new_context(backend, ctx_params) {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.blocking_send(StreamChunk::Error(format!("Failed to create context: {}", e)));
                    return;
                }
            };

            // Tokenize prompt (no BOS for ChatML models like OLMo-3)
            let tokens = match model.str_to_token(&prompt, AddBos::Never) {
                Ok(t) => t,
                Err(e) => {
                    let _ = tx.blocking_send(StreamChunk::Error(format!("Failed to tokenize: {}", e)));
                    return;
                }
            };

            // Create batch and decode prompt
            let mut batch = LlamaBatch::new(context_size as usize, 1);
            for (i, token) in tokens.iter().enumerate() {
                if batch.add(*token, i as i32, &[0], i == tokens.len() - 1).is_err() {
                    let _ = tx.blocking_send(StreamChunk::Error("Failed to add token to batch".to_string()));
                    return;
                }
            }
            if ctx.decode(&mut batch).is_err() {
                let _ = tx.blocking_send(StreamChunk::Error("Failed to decode prompt".to_string()));
                return;
            }

            // Sample tokens - chain must end with selection sampler (dist or greedy)
            let mut sampler = LlamaSampler::chain_simple([
                LlamaSampler::top_k(40),
                LlamaSampler::top_p(0.9, 1),
                LlamaSampler::min_p(0.1, 1),
                LlamaSampler::temp(params.temperature),
                LlamaSampler::dist(0), // Final selection - 0 = random seed
            ]);

            let mut n_cur = tokens.len();
            let max_tokens = params.max_tokens.min(context_size as usize - tokens.len());
            let mut generated = 0;
            let eos_token = "<|im_end|>";
            let mut accumulated = String::new();

            while generated < max_tokens {
                // Sample next token
                let token = sampler.sample(&ctx, batch.n_tokens() - 1);
                generated += 1;

                // Check for EOS
                if model.is_eog_token(token) {
                    break;
                }

                // Decode token to text
                if let Ok(token_str) = model.token_to_str(token, Special::Tokenize) {
                    accumulated.push_str(&token_str);

                    // Check for stop sequence
                    if accumulated.contains(eos_token) {
                        // Send up to stop sequence
                        if let Some(pos) = accumulated.find(eos_token) {
                            let final_chunk = &accumulated[..pos];
                            if !final_chunk.is_empty() {
                                let _ = tx.blocking_send(StreamChunk::Text(final_chunk.to_string()));
                            }
                        }
                        break;
                    }

                    // Send token
                    let _ = tx.blocking_send(StreamChunk::Text(token_str));
                }

                // Add token to batch for next iteration
                batch.clear();
                if batch.add(token, n_cur as i32, &[0], true).is_err() {
                    break;
                }
                n_cur += 1;

                if ctx.decode(&mut batch).is_err() {
                    let _ = tx.blocking_send(StreamChunk::Error("Decode failed".to_string()));
                    break;
                }
            }

            // Send done
            let _ = tx.blocking_send(StreamChunk::Done {
                finish_reason: FinishReason::Stop,
                usage: Some(UsageStats {
                    prompt_tokens: tokens.len(),
                    completion_tokens: generated,
                    total_tokens: tokens.len() + generated,
                }),
            });
        });

        // Convert channel to stream
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    fn count_tokens(&self, text: &str) -> usize {
        // Rough estimate - actual count would require loading model
        text.len() / 4
    }
}

// Stub implementation when feature is disabled
#[cfg(not(feature = "local-llm"))]
pub struct LocalProvider;

#[cfg(not(feature = "local-llm"))]
impl LocalProvider {
    pub fn new(_model_path: std::path::PathBuf, _context_size: u32, _gpu_layers: i32) -> Self {
        Self
    }

    pub fn find_model(_model_name: &str) -> anyhow::Result<std::path::PathBuf> {
        anyhow::bail!("Local LLM support not compiled. Build with --features local-llm")
    }
}
