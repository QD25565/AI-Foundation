// Screenshot capture functionality using xcap

use crate::types::{CaptureTarget, ImageFormat, ScreenshotOptions, VisionResult};
use anyhow::{Context, bail};
use std::path::Path;
use xcap::Window;

/// AI vision profile settings for auto-optimization
#[derive(Debug, Clone)]
pub struct VisionProfile {
    pub optimal_resolution: (u32, u32),  // width, height
    pub thumbnail_resolution: (u32, u32),
    pub preferred_format: ImageFormat,
    pub quality: u8,
}

impl Default for VisionProfile {
    fn default() -> Self {
        Self {
            optimal_resolution: (960, 540),
            thumbnail_resolution: (640, 360),
            preferred_format: ImageFormat::Png,
            quality: 85,
        }
    }
}

impl VisionProfile {
    /// Load profile from AI_CAPABILITIES_PROFILE in notebook vault
    pub fn load_from_vault() -> Option<Self> {
        let ai_id = std::env::var("AI_ID").ok()?;

        let output = std::process::Command::new("python")
            .args(&["-m", "tools.notebook", "vault_retrieve", "--key", "AI_CAPABILITIES_PROFILE"])
            .env("AI_ID", &ai_id)
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json_str = stdout.split('|').nth(1)?.trim();
        let profile: serde_json::Value = serde_json::from_str(json_str).ok()?;

        let vision = profile.get("vision")?;

        // Parse resolution strings like "960x540"
        let optimal = vision.get("optimal_resolution")?.as_str()?;
        let (opt_w, opt_h) = Self::parse_resolution(optimal)?;

        let thumb = vision.get("thumbnail_resolution")?.as_str()?;
        let (thumb_w, thumb_h) = Self::parse_resolution(thumb)?;

        let format_str = vision.get("preferred_format")?.as_str().unwrap_or("png");
        let format = match format_str.to_lowercase().as_str() {
            "jpeg" | "jpg" => ImageFormat::Jpeg,
            "webp" => ImageFormat::WebP,
            _ => ImageFormat::Png,
        };

        Some(Self {
            optimal_resolution: (opt_w, opt_h),
            thumbnail_resolution: (thumb_w, thumb_h),
            preferred_format: format,
            quality: 85,
        })
    }

    fn parse_resolution(s: &str) -> Option<(u32, u32)> {
        let parts: Vec<&str> = s.split('x').collect();
        if parts.len() == 2 {
            let w = parts[0].parse().ok()?;
            let h = parts[1].parse().ok()?;
            Some((w, h))
        } else {
            None
        }
    }
}

pub struct ScreenshotCapture;

impl ScreenshotCapture {
    /// Capture a screenshot based on options
    pub async fn capture(options: ScreenshotOptions) -> VisionResult<()> {
        match options.target {
            CaptureTarget::Screen => {
                Self::capture_screen(&options.output_path, options.format).await
            }
            CaptureTarget::Monitor(index) => {
                Self::capture_monitor(index, &options.output_path, options.format).await
            }
            CaptureTarget::Window(ref title) => {
                Self::capture_window(title, &options.output_path, options.format).await
            }
            CaptureTarget::Region { x, y, width, height } => {
                Self::capture_region(x, y, width, height, &options.output_path, options.format).await
            }
            CaptureTarget::Web { .. } => {
                // Web screenshots handled by browser module
                bail!("Web screenshots require browser module - use navigate + screenshot commands")
            }
        }
    }

    /// Capture entire screen (primary monitor)
    pub async fn capture_screen(output_path: &Path, format: ImageFormat) -> VisionResult<()> {
        Self::capture_monitor(0, output_path, format).await
    }

    /// Capture specific monitor by index (0-based)
    pub async fn capture_monitor(index: usize, output_path: &Path, format: ImageFormat) -> VisionResult<()> {
        let screens = xcap::Monitor::all()
            .context("Failed to enumerate monitors")?;

        let monitor = screens.into_iter()
            .nth(index)
            .context(format!("Monitor {} not found. Use 'visionbook list-monitors' to see available monitors.", index))?;

        let image = monitor.capture_image()
            .context("Failed to capture monitor")?;

        Self::save_image(image, output_path, format)
            .context("Failed to save screenshot")?;

        tracing::info!("Monitor {} screenshot saved to {}", index, output_path.display());
        Ok(())
    }

    /// Capture specific window by title (fuzzy match)
    pub async fn capture_window(title: &str, output_path: &Path, format: ImageFormat) -> VisionResult<()> {
        let windows = xcap::Window::all()
            .context("Failed to enumerate windows")?;

        // Fuzzy match on window title
        let window = windows.into_iter()
            .find(|w| {
                w.title()
                    .ok()
                    .map(|t| t.to_lowercase().contains(&title.to_lowercase()))
                    .unwrap_or(false)
            })
            .context(format!("No window found matching title: '{}'", title))?;

        let image = window.capture_image()
            .context("Failed to capture window")?;

        Self::save_image(image, output_path, format)
            .context("Failed to save screenshot")?;

        let window_title = window.title().unwrap_or_else(|_| "Unknown".to_string());
        tracing::info!("Window '{}' screenshot saved to {}", window_title, output_path.display());
        Ok(())
    }

    /// Capture specific region of screen
    pub async fn capture_region(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        output_path: &Path,
        format: ImageFormat,
    ) -> VisionResult<()> {
        let screens = xcap::Monitor::all()
            .context("Failed to enumerate monitors")?;

        let primary = screens.into_iter()
            .next()
            .context("No monitors found")?;

        let full_image = primary.capture_image()
            .context("Failed to capture screen")?;

        // Crop to region
        let cropped = image::imageops::crop_imm(
            &full_image,
            x as u32,
            y as u32,
            width,
            height,
        ).to_image();

        Self::save_image(cropped, output_path, format)
            .context("Failed to save screenshot")?;

        tracing::info!("Region ({},{} {}x{}) screenshot saved to {}",
            x, y, width, height, output_path.display());
        Ok(())
    }

    /// List all available windows
    pub fn list_windows() -> VisionResult<Vec<String>> {
        let windows = xcap::Window::all()
            .context("Failed to enumerate windows")?;

        Ok(windows.into_iter()
            .filter_map(|w| w.title().ok())
            .collect())
    }

    /// List all available monitors
    pub fn list_monitors() -> VisionResult<Vec<String>> {
        let monitors = xcap::Monitor::all()
            .context("Failed to enumerate monitors")?;

        Ok(monitors.into_iter()
            .enumerate()
            .filter_map(|(i, m)| {
                Some(format!("Monitor {} - {}x{}", i, m.width().ok()?, m.height().ok()?))
            })
            .collect())
    }

    /// Capture with auto-optimization using AI vision profile
    pub async fn capture_auto_optimized(options: ScreenshotOptions, thumbnail: bool) -> VisionResult<()> {
        // Load AI vision profile or use defaults
        let profile = VisionProfile::load_from_vault()
            .unwrap_or_else(|| {
                tracing::info!("No AI vision profile found, using defaults");
                VisionProfile::default()
            });

        let target_resolution = if thumbnail {
            profile.thumbnail_resolution
        } else {
            profile.optimal_resolution
        };

        tracing::info!("Auto-optimizing to {}x{} {:?}",
            target_resolution.0, target_resolution.1, profile.preferred_format);

        // Capture the raw image first
        let raw_image = match &options.target {
            CaptureTarget::Screen => Self::capture_monitor_raw(0).await?,
            CaptureTarget::Monitor(index) => Self::capture_monitor_raw(*index).await?,
            CaptureTarget::Window(title) => Self::capture_window_raw(title).await?,
            CaptureTarget::Region { x, y, width, height } => {
                Self::capture_region_raw(*x, *y, *width, *height).await?
            }
            CaptureTarget::Web { .. } => {
                bail!("Web screenshots require browser module")
            }
        };

        // Resize to optimal resolution
        let resized = Self::resize_image(raw_image, target_resolution.0, target_resolution.1);

        // Save with preferred format
        let format = options.format; // Could override with profile.preferred_format
        Self::save_image(resized, &options.output_path, format)?;

        tracing::info!("Auto-optimized screenshot saved to {}", options.output_path.display());
        Ok(())
    }

    /// Capture monitor by index and return raw image
    async fn capture_monitor_raw(index: usize) -> VisionResult<image::RgbaImage> {
        let screens = xcap::Monitor::all()
            .context("Failed to enumerate monitors")?;

        let monitor = screens.into_iter()
            .nth(index)
            .context(format!("Monitor {} not found", index))?;

        monitor.capture_image()
            .context("Failed to capture monitor")
    }

    /// Capture window and return raw image
    async fn capture_window_raw(title: &str) -> VisionResult<image::RgbaImage> {
        let windows = xcap::Window::all()
            .context("Failed to enumerate windows")?;

        let window = windows.into_iter()
            .find(|w| {
                w.title()
                    .ok()
                    .map(|t| t.to_lowercase().contains(&title.to_lowercase()))
                    .unwrap_or(false)
            })
            .context(format!("No window found matching title: '{}'", title))?;

        window.capture_image()
            .context("Failed to capture window")
    }

    /// Capture region and return raw image
    async fn capture_region_raw(x: i32, y: i32, width: u32, height: u32) -> VisionResult<image::RgbaImage> {
        let screens = xcap::Monitor::all()
            .context("Failed to enumerate monitors")?;

        let primary = screens.into_iter()
            .next()
            .context("No monitors found")?;

        let full_image = primary.capture_image()
            .context("Failed to capture screen")?;

        Ok(image::imageops::crop_imm(
            &full_image,
            x as u32,
            y as u32,
            width,
            height,
        ).to_image())
    }

    /// Resize image maintaining aspect ratio
    fn resize_image(image: image::RgbaImage, max_width: u32, max_height: u32) -> image::RgbaImage {
        let (orig_width, orig_height) = (image.width(), image.height());

        // Calculate scale to fit within target while maintaining aspect ratio
        let scale_w = max_width as f32 / orig_width as f32;
        let scale_h = max_height as f32 / orig_height as f32;
        let scale = scale_w.min(scale_h).min(1.0); // Never upscale

        let new_width = (orig_width as f32 * scale) as u32;
        let new_height = (orig_height as f32 * scale) as u32;

        if new_width == orig_width && new_height == orig_height {
            return image; // No resize needed
        }

        image::imageops::resize(
            &image,
            new_width,
            new_height,
            image::imageops::FilterType::Lanczos3,
        )
    }

    /// Save image to file with specified format
    fn save_image(
        image: image::RgbaImage,
        output_path: &Path,
        format: ImageFormat,
    ) -> VisionResult<()> {
        match format {
            ImageFormat::Png => {
                image.save_with_format(output_path, image::ImageFormat::Png)
                    .context("Failed to save PNG")?;
            }
            ImageFormat::Jpeg => {
                // Convert RGBA to RGB for JPEG
                let rgb_image = image::DynamicImage::ImageRgba8(image).to_rgb8();
                rgb_image.save_with_format(output_path, image::ImageFormat::Jpeg)
                    .context("Failed to save JPEG")?;
            }
            ImageFormat::WebP => {
                image.save_with_format(output_path, image::ImageFormat::WebP)
                    .context("Failed to save WebP")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_list_monitors() {
        let monitors = ScreenshotCapture::list_monitors().unwrap();
        assert!(!monitors.is_empty());
        println!("Monitors: {:?}", monitors);
    }

    #[tokio::test]
    async fn test_list_windows() {
        let windows = ScreenshotCapture::list_windows().unwrap();
        println!("Windows: {:?}", windows);
    }
}
