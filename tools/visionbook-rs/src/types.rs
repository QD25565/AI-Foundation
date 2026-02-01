// Core types for visionbook

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Result type alias for visionbook operations
pub type VisionResult<T> = anyhow::Result<T>;

/// Screenshot format options
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
    WebP,
}

impl Default for ImageFormat {
    fn default() -> Self {
        ImageFormat::Png
    }
}

/// Target for screenshot capture
#[derive(Debug, Clone)]
pub enum CaptureTarget {
    /// Capture entire screen (primary monitor)
    Screen,
    /// Capture specific monitor by index (0-based)
    Monitor(usize),
    /// Capture specific window by title (fuzzy match)
    Window(String),
    /// Capture specific region (x, y, width, height)
    Region { x: i32, y: i32, width: u32, height: u32 },
    /// Capture web page or element
    Web { url: String, selector: Option<String> },
}

/// Screenshot output options
#[derive(Debug, Clone)]
pub struct ScreenshotOptions {
    pub target: CaptureTarget,
    pub output_path: PathBuf,
    pub format: ImageFormat,
    pub quality: Option<u8>,
}

/// Browser navigation options
#[derive(Debug, Clone)]
pub struct NavigationOptions {
    pub url: String,
    pub wait_until: WaitCondition,
    pub timeout_ms: u64,
}

impl Default for NavigationOptions {
    fn default() -> Self {
        Self {
            url: String::new(),
            wait_until: WaitCondition::Load,
            timeout_ms: 30000,
        }
    }
}

/// Wait conditions for browser operations
#[derive(Debug, Clone, Copy)]
pub enum WaitCondition {
    /// Wait for initial HTML load
    Load,
    /// Wait for DOMContentLoaded event
    DomContentLoaded,
    /// Wait for all resources (images, scripts, etc.)
    NetworkIdle,
}

/// Element selector types
#[derive(Debug, Clone)]
pub enum ElementSelector {
    Css(String),
    XPath(String),
    Text(String),
}

impl ElementSelector {
    pub fn as_str(&self) -> &str {
        match self {
            ElementSelector::Css(s) => s,
            ElementSelector::XPath(s) => s,
            ElementSelector::Text(s) => s,
        }
    }
}

/// PDF generation options
#[derive(Debug, Clone)]
pub struct PdfOptions {
    pub url: String,
    pub output_path: PathBuf,
    pub landscape: bool,
    pub print_background: bool,
    pub scale: f64,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            url: String::new(),
            output_path: PathBuf::new(),
            landscape: false,
            print_background: true,
            scale: 1.0,
        }
    }
}

/// Video recording options
#[derive(Debug, Clone)]
pub struct VideoOptions {
    pub target: CaptureTarget,
    pub output_path: PathBuf,
    pub duration_secs: u64,
    pub fps: u32,
}

impl Default for VideoOptions {
    fn default() -> Self {
        Self {
            target: CaptureTarget::Screen,
            output_path: PathBuf::new(),
            duration_secs: 10,
            fps: 30,
        }
    }
}

/// Mobile device presets
#[derive(Debug, Clone)]
pub enum MobileDevice {
    IPhone12,
    IPhone13Pro,
    IPhone14ProMax,
    IPhoneSE,
    GalaxyS21,
    GalaxyS22Ultra,
    Pixel5,
    Pixel7Pro,
    IPadPro,
    IPadMini,
    Custom {
        width: u32,
        height: u32,
        device_scale_factor: f64,
        user_agent: String,
    },
}

impl MobileDevice {
    pub fn viewport(&self) -> (u32, u32, f64) {
        match self {
            MobileDevice::IPhone12 => (390, 844, 3.0),
            MobileDevice::IPhone13Pro => (390, 844, 3.0),
            MobileDevice::IPhone14ProMax => (430, 932, 3.0),
            MobileDevice::IPhoneSE => (375, 667, 2.0),
            MobileDevice::GalaxyS21 => (360, 800, 3.0),
            MobileDevice::GalaxyS22Ultra => (412, 915, 3.5),
            MobileDevice::Pixel5 => (393, 851, 2.75),
            MobileDevice::Pixel7Pro => (412, 892, 3.5),
            MobileDevice::IPadPro => (1024, 1366, 2.0),
            MobileDevice::IPadMini => (768, 1024, 2.0),
            MobileDevice::Custom { width, height, device_scale_factor, .. } => {
                (*width, *height, *device_scale_factor)
            }
        }
    }
}

/// Network interception rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterceptRule {
    pub url_pattern: String,
    pub action: InterceptAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InterceptAction {
    Block,
    Mock { response_body: String, status_code: u16 },
    ModifyHeaders { headers: Vec<(String, String)> },
}
