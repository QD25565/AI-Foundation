// Visionbook: High-performance visual tools for AI agents
// Part of AI-Foundation
//
// Provides:
// - Screenshot capture (screens, windows, regions, web pages)
// - Browser automation (navigate, click, type, scrape)
// - PDF generation from web pages
// - Video recording
// - Network interception
// - Mobile device emulation
// - VisionEngram: Visual memory storage linked to Engram notes
// - AI-optimized thumbnail generation

pub mod screenshot;
pub mod browser;
pub mod network;
pub mod emulation;
pub mod pdf;
pub mod video;
pub mod types;
pub mod thumbnail;
pub mod veng;
pub mod visual_memory;

// Re-export main types
pub use screenshot::{ScreenshotCapture, VisionProfile};
pub use browser::BrowserSession;
pub use network::{NetworkInterceptor, RequestInterceptor, ResponseMock};
pub use emulation::{DeviceEmulator, parse_device, list_devices};
pub use pdf::{PdfGenerator, PdfOptionsBuilder};
pub use video::{VideoRecorder, VideoOptionsBuilder};
pub use types::*;

// VisionEngram exports
pub use thumbnail::{ThumbnailGenerator, ThumbnailConfig, ThumbnailResult, generate_ai_thumbnail};
pub use veng::{VisionEngram, VisionEntry, VisionEngramStats, ThumbnailFormat};
pub use visual_memory::{VisualMemory, VisualMemoryPaths};

use anyhow::Result;

/// Initialize the visionbook library
pub fn init() -> Result<()> {
    // Initialize tracing/logging if not already initialized
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .try_init()
        .ok(); // Ignore error if already initialized

    Ok(())
}

/// Get library version
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init() {
        assert!(init().is_ok());
    }

    #[test]
    fn test_version() {
        assert_eq!(version(), "1.0.0");
    }
}
