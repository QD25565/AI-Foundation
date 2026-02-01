//! Terminal UI components for Forge
//!
//! Beautiful ratatui-based interface with AI-Foundation branding.

pub mod colors;
pub mod widgets;
pub mod app;

pub use colors::{BrandColors, Gradient, StatusType, LOGO, TAGLINE};
pub use widgets::*;
pub use app::App;
