//! Embedding Generation for Engram
//!
//! Uses EmbeddingGemma via llama.cpp for 512-dimensional embeddings.
//! MRL (Matryoshka Representation Learning) truncation from 768d to 512d.
//!
//! Embeddings are FOUNDATIONAL to exceptional recall (keyword + semantic + graph).
//! This is not optional - every AI gets embeddings.
//!
//! # Example
//!
//! ```rust,ignore
//! use engram::embedding::{EmbeddingGenerator, EmbeddingConfig};
//!
//! let config = EmbeddingConfig::default().with_model("embeddinggemma-300M-Q8_0.gguf");
//! let mut generator = EmbeddingGenerator::load(config).unwrap();
//! let embedding = generator.embed("Fix OAuth token refresh bug").unwrap();
//! assert_eq!(embedding.len(), 512);
//! ```

use crate::{Result, EngramError, DEFAULT_DIMENSIONS};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaModel, AddBos};

/// Global backend (llama.cpp requires single initialization)
static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();

fn get_backend() -> &'static LlamaBackend {
    BACKEND.get_or_init(|| {
        let mut backend = LlamaBackend::init().expect("Failed to initialize llama.cpp backend");
        backend.void_logs(); // Suppress verbose logs
        backend
    })
}

/// Configuration for embedding generation
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    /// Path to GGUF model file
    pub model_path: PathBuf,
    /// Output dimensions (512 for MRL truncation, 768 for full)
    pub dimensions: usize,
    /// GPU layers to offload (-1 = auto, 0 = CPU only)
    pub gpu_layers: i32,
    /// Context size for the model
    pub context_size: u32,
    /// Normalize embeddings to unit vectors
    pub normalize: bool,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            model_path: PathBuf::from("embeddinggemma-300M-Q8_0.gguf"),
            dimensions: DEFAULT_DIMENSIONS as usize, // 512
            gpu_layers: 0, // CPU only for broad compatibility
            context_size: 8192, // Support long notes (up to ~30k chars)
            normalize: true,
        }
    }
}

impl EmbeddingConfig {
    /// Create config with specific model path
    pub fn with_model<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.model_path = path.as_ref().to_path_buf();
        self
    }

    /// Set GPU layers (-1 = auto, 0 = CPU, n = specific layers)
    pub fn with_gpu_layers(mut self, layers: i32) -> Self {
        self.gpu_layers = layers;
        self
    }

    /// Set output dimensions (512 or 768)
    pub fn with_dimensions(mut self, dims: usize) -> Self {
        self.dimensions = dims;
        self
    }
}

/// Statistics from embedding generation
#[derive(Debug, Clone, Default)]
pub struct EmbeddingStats {
    /// Total embeddings generated
    pub embeddings_generated: u64,
    /// Total tokens processed
    pub tokens_processed: u64,
    /// Average time per embedding (microseconds)
    pub avg_time_us: u64,
}

/// Embedding generator using EmbeddingGemma via llama.cpp
pub struct EmbeddingGenerator {
    model: LlamaModel,
    config: EmbeddingConfig,
    stats: EmbeddingStats,
}

impl EmbeddingGenerator {
    /// Load embedding model from GGUF file
    pub fn load(config: EmbeddingConfig) -> Result<Self> {
        let backend = get_backend();

        // Check model file exists
        if !config.model_path.exists() {
            return Err(EngramError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Model file not found: {:?}", config.model_path),
            )));
        }

        // Load model with GPU settings
        let mut model_params = LlamaModelParams::default();
        if config.gpu_layers >= 0 {
            model_params = model_params.with_n_gpu_layers(config.gpu_layers as u32);
        }

        let model = LlamaModel::load_from_file(backend, &config.model_path, &model_params)
            .map_err(|e| EngramError::EmbeddingError(format!("Failed to load model: {}", e)))?;

        Ok(Self {
            model,
            config,
            stats: EmbeddingStats::default(),
        })
    }

    /// Find model file in common locations
    pub fn find_model(model_name: &str) -> Option<PathBuf> {
        let exe_path = std::env::current_exe().ok();
        let search_paths = [
            // Same directory as executable
            exe_path.as_ref().and_then(|p| p.parent()).map(|p| p.join(model_name)),
            exe_path.as_ref().and_then(|p| p.parent()).map(|p| p.join(format!("{}.gguf", model_name))),
            // bin/ subdirectory
            exe_path.as_ref().and_then(|p| p.parent()).map(|p| p.join("bin").join(model_name)),
            // models/ subdirectory
            exe_path.as_ref().and_then(|p| p.parent()).map(|p| p.join("models").join(model_name)),
            // Current directory
            Some(PathBuf::from(model_name)),
            Some(PathBuf::from(format!("{}.gguf", model_name))),
            // Home directory locations
            dirs::home_dir().map(|h| h.join(".ai-foundation").join("models").join(model_name)),
            dirs::data_dir().map(|d| d.join("ai-foundation").join("models").join(model_name)),
        ];

        for path_opt in search_paths.iter() {
            if let Some(path) = path_opt {
                if path.exists() {
                    return Some(path.clone());
                }
            }
        }
        None
    }

    /// Generate embedding for a single text
    pub fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        let start = std::time::Instant::now();
        let backend = get_backend();

        // Create embedding context
        // n_ubatch must be >= n_tokens for encoder models
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(Some(std::num::NonZeroU32::new(self.config.context_size).unwrap()))
            .with_n_ubatch(self.config.context_size)
            .with_n_batch(self.config.context_size)
            .with_embeddings(true);

        let mut ctx = self.model.new_context(backend, ctx_params)
            .map_err(|e| EngramError::EmbeddingError(format!("Failed to create context: {}", e)))?;

        // Tokenize text
        let tokens = self.model.str_to_token(text, AddBos::Always)
            .map_err(|e| EngramError::EmbeddingError(format!("Failed to tokenize: {}", e)))?;

        // Create batch and process
        let mut batch = llama_cpp_2::llama_batch::LlamaBatch::new(self.config.context_size as usize, 1);
        for (i, token) in tokens.iter().enumerate() {
            batch.add(*token, i as i32, &[0], i == tokens.len() - 1)
                .map_err(|e| EngramError::EmbeddingError(format!("Failed to add token: {}", e)))?;
        }

        ctx.decode(&mut batch)
            .map_err(|e| EngramError::EmbeddingError(format!("Failed to decode: {}", e)))?;

        // Get embeddings from context
        let embeddings = ctx.embeddings_seq_ith(0)
            .map_err(|e| EngramError::EmbeddingError(format!("Failed to get embeddings: {}", e)))?;

        // Apply MRL truncation if needed (768 -> 512)
        let truncated = self.truncate_mrl(embeddings);

        // Normalize if configured
        let result = if self.config.normalize {
            self.normalize(&truncated)
        } else {
            truncated
        };

        // Update stats
        self.stats.embeddings_generated += 1;
        self.stats.tokens_processed += tokens.len() as u64;
        let elapsed_us = start.elapsed().as_micros() as u64;
        self.stats.avg_time_us = (self.stats.avg_time_us * (self.stats.embeddings_generated - 1) + elapsed_us)
            / self.stats.embeddings_generated;

        Ok(result)
    }

    /// Generate embeddings for multiple texts (more efficient for batch operations)
    pub fn embed_batch(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        // For now, process sequentially. Could optimize with proper batching later.
        texts.iter()
            .map(|text| self.embed(text))
            .collect()
    }

    /// Apply MRL truncation (keep first N dimensions)
    fn truncate_mrl(&self, embedding: &[f32]) -> Vec<f32> {
        if embedding.len() <= self.config.dimensions {
            embedding.to_vec()
        } else {
            embedding[..self.config.dimensions].to_vec()
        }
    }

    /// Normalize vector to unit length
    fn normalize(&self, v: &[f32]) -> Vec<f32> {
        let magnitude: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if magnitude > 0.0 {
            v.iter().map(|x| x / magnitude).collect()
        } else {
            v.to_vec()
        }
    }

    /// Get generation statistics
    pub fn stats(&self) -> &EmbeddingStats {
        &self.stats
    }

    /// Get configured dimensions
    pub fn dimensions(&self) -> usize {
        self.config.dimensions
    }
}

/// Result of backfill operation
#[derive(Debug, Clone, Default)]
pub struct BackfillStats {
    /// Notes processed
    pub processed: u64,
    /// Notes skipped (already had embeddings)
    pub skipped: u64,
    /// Notes newly embedded
    pub embedded: u64,
    /// Errors encountered
    pub errors: u64,
    /// Total time in milliseconds
    pub duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = EmbeddingConfig::default();
        assert_eq!(config.dimensions, 512);
        assert_eq!(config.gpu_layers, 0);
        assert!(config.normalize);
    }

    #[test]
    fn test_config_builder() {
        let config = EmbeddingConfig::default()
            .with_dimensions(768)
            .with_gpu_layers(-1)
            .with_model("custom.gguf");

        assert_eq!(config.dimensions, 768);
        assert_eq!(config.gpu_layers, -1);
        assert_eq!(config.model_path, PathBuf::from("custom.gguf"));
    }
}
