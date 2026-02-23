//! LLM abstraction layer
//!
//! Model-agnostic interface for both local and API-based models.

pub mod types;
pub mod provider;
pub mod openai;
pub mod anthropic;
pub mod local;

pub use types::*;
pub use provider::*;
#[cfg(feature = "local-llm")]
pub use local::LocalProvider;
