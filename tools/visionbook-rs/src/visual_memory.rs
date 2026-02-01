//! Visual Memory Integration
//!
//! High-level API for AI visual memory:
//! - Attach images to Engram notes
//! - Capture screenshots with auto-attachment
//! - Standalone visual memories
//!
//! This module integrates VisionEngram storage with thumbnail generation
//! and provides the main interface for AI agents.

use crate::screenshot::{ScreenshotCapture, VisionProfile};
use crate::thumbnail::{ThumbnailGenerator, ThumbnailConfig, ThumbnailResult};
use crate::veng::{VisionEngram, VisionEntry, VisionEngramStats, flags};
use crate::types::{CaptureTarget, ImageFormat, ScreenshotOptions};

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::fs;

/// Path configuration for visual memory storage
#[derive(Debug, Clone)]
pub struct VisualMemoryPaths {
    /// Base directory for AI-Foundation data
    pub base_dir: PathBuf,
    /// VisionEngram file path
    pub veng_path: PathBuf,
    /// Directory for original images
    pub images_dir: PathBuf,
}

impl VisualMemoryPaths {
    /// Get paths for a specific AI
    pub fn for_ai(ai_id: &str) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let base_dir = home.join(".ai-foundation").join("notebook");
        let veng_path = base_dir.join(format!("{}.veng", ai_id));
        let images_dir = base_dir.join(ai_id).join("images");

        Self {
            base_dir,
            veng_path,
            images_dir,
        }
    }

    /// Get paths from environment
    pub fn from_env() -> Result<Self> {
        let ai_id = std::env::var("AI_ID")
            .context("AI_ID environment variable not set")?;
        Ok(Self::for_ai(&ai_id))
    }
}

/// Visual Memory Manager - main interface for AI visual memories
pub struct VisualMemory {
    pub paths: VisualMemoryPaths,
    store: VisionEngram,
    thumbnail_generator: ThumbnailGenerator,
}

impl VisualMemory {
    /// Open visual memory for the current AI (uses AI_ID env var)
    pub fn open() -> Result<Self> {
        let paths = VisualMemoryPaths::from_env()?;
        Self::open_with_paths(paths)
    }

    /// Open visual memory with specific paths
    pub fn open_with_paths(paths: VisualMemoryPaths) -> Result<Self> {
        // Ensure directories exist
        fs::create_dir_all(&paths.base_dir)
            .context("Failed to create notebook directory")?;
        fs::create_dir_all(&paths.images_dir)
            .context("Failed to create images directory")?;

        let store = VisionEngram::open(&paths.veng_path)
            .context("Failed to open VisionEngram")?;

        let thumbnail_generator = ThumbnailGenerator::new(ThumbnailConfig::default());

        Ok(Self {
            paths,
            store,
            thumbnail_generator,
        })
    }

    /// Attach an image file to an existing Engram note
    pub fn attach_to_note(
        &mut self,
        note_id: u64,
        image_path: impl AsRef<Path>,
        context: Option<&str>,
    ) -> Result<u64> {
        let image_path = image_path.as_ref();

        // Read the image
        let image_data = fs::read(image_path)
            .context(format!("Failed to read image: {}", image_path.display()))?;

        // Generate AI-optimized thumbnail
        let thumbnail = self.thumbnail_generator.from_bytes(&image_data)
            .context("Failed to generate thumbnail")?;

        // Determine format from extension
        let format_str = image_path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png");

        // Create entry
        let mut entry = VisionEntry::new(
            0, // Will be assigned by store
            note_id,
            thumbnail.data.clone(),
            thumbnail.width,
            thumbnail.height,
            thumbnail.original_width,
            thumbnail.original_height,
        );

        if let Some(ctx) = context {
            entry = entry.with_context(ctx);
        }

        // Store the entry
        let visual_id = self.store.store(entry.clone())
            .context("Failed to store visual entry")?;

        // Save original image
        let relative_path = self.store.save_original(visual_id, &image_data, format_str)
            .context("Failed to save original image")?;

        // Update entry with original path (we need to re-store with path)
        // For now, the path is stored but we'd need to update the entry
        // This is a limitation of the current design - we store first, get ID, save original

        tracing::info!(
            "Attached image to note {}: visual ID {} ({:.1}KB thumbnail, {}x{} original)",
            note_id, visual_id, thumbnail.size_kb(),
            thumbnail.original_width, thumbnail.original_height
        );

        Ok(visual_id)
    }

    /// Attach image from raw bytes
    pub fn attach_bytes_to_note(
        &mut self,
        note_id: u64,
        image_data: &[u8],
        format: &str,
        context: Option<&str>,
    ) -> Result<u64> {
        // Generate AI-optimized thumbnail
        let thumbnail = self.thumbnail_generator.from_bytes(image_data)
            .context("Failed to generate thumbnail")?;

        // Create entry
        let mut entry = VisionEntry::new(
            0,
            note_id,
            thumbnail.data.clone(),
            thumbnail.width,
            thumbnail.height,
            thumbnail.original_width,
            thumbnail.original_height,
        );

        if let Some(ctx) = context {
            entry = entry.with_context(ctx);
        }

        // Store the entry
        let visual_id = self.store.store(entry.clone())?;

        // Save original image
        self.store.save_original(visual_id, image_data, format)?;

        Ok(visual_id)
    }

    /// Capture screenshot and attach to note
    pub async fn capture_to_note(
        &mut self,
        note_id: u64,
        target: CaptureTarget,
        context: Option<&str>,
    ) -> Result<u64> {
        // Create temp file for screenshot
        let temp_path = std::env::temp_dir().join(format!("veng_capture_{}.png", note_id));

        let options = ScreenshotOptions {
            target,
            output_path: temp_path.clone(),
            format: ImageFormat::Png,
            quality: None,
        };

        // Capture screenshot
        ScreenshotCapture::capture(options).await
            .context("Failed to capture screenshot")?;

        // Attach the screenshot
        let visual_id = self.attach_to_note(note_id, &temp_path, context)?;

        // Mark as screenshot
        if let Some(mut entry) = self.store.get(visual_id)? {
            entry.flags |= flags::IS_SCREENSHOT;
            // Would need update method - for now the flag is set on creation
        }

        // Clean up temp file
        let _ = fs::remove_file(&temp_path);

        Ok(visual_id)
    }

    /// Store a standalone visual memory (not attached to a note)
    pub fn remember_visual(
        &mut self,
        image_path: impl AsRef<Path>,
        context: &str,
    ) -> Result<u64> {
        self.attach_to_note(0, image_path, Some(context))
    }

    /// Get visual memories for a note
    pub fn get_visuals_for_note(&self, note_id: u64) -> Result<Vec<VisionEntry>> {
        self.store.get_by_note(note_id)
    }

    /// Get a specific visual by ID
    pub fn get_visual(&self, id: u64) -> Result<Option<VisionEntry>> {
        self.store.get(id)
    }

    /// List recent visual memories
    pub fn list_recent(&self, limit: usize) -> Result<Vec<VisionEntry>> {
        self.store.list_recent(limit)
    }

    /// Delete a visual memory
    pub fn delete_visual(&mut self, id: u64) -> Result<()> {
        self.store.delete(id)
    }

    /// Get statistics
    pub fn stats(&self) -> VisionEngramStats {
        self.store.stats()
    }

    /// Get the full path to an original image
    pub fn get_original_path(&self, relative_path: &str) -> PathBuf {
        self.store.get_original_path(relative_path)
    }
}

/// Quick functions for common operations
pub mod quick {
    use super::*;

    /// Attach an image to a note (uses AI_ID from env)
    pub fn attach(note_id: u64, image_path: &str, context: Option<&str>) -> Result<u64> {
        let mut vm = VisualMemory::open()?;
        vm.attach_to_note(note_id, image_path, context)
    }

    /// Get visuals for a note
    pub fn get_for_note(note_id: u64) -> Result<Vec<VisionEntry>> {
        let vm = VisualMemory::open()?;
        vm.get_visuals_for_note(note_id)
    }

    /// List recent visuals
    pub fn recent(limit: usize) -> Result<Vec<VisionEntry>> {
        let vm = VisualMemory::open()?;
        vm.list_recent(limit)
    }

    /// Get stats
    pub fn stats() -> Result<VisionEngramStats> {
        let vm = VisualMemory::open()?;
        Ok(vm.stats())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_paths_for_ai() {
        let paths = VisualMemoryPaths::for_ai("test-ai");
        assert!(paths.veng_path.to_string_lossy().contains("test-ai.veng"));
        assert!(paths.images_dir.to_string_lossy().contains("images"));
    }
}
