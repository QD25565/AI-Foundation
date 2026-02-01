//! AI-Foundation gradient color system
//!
//! Beautiful per-character gradient rendering from Battleship Grey to Asparagus Green.
//! Ported from the Python installer's GradientColors class.

use ratatui::style::Color;

/// Core brand colors for AI-Foundation
#[derive(Debug, Clone, Copy)]
pub struct BrandColors;

impl BrandColors {
    /// Battleship Grey - Starting color for gradients
    pub const BATTLESHIP_GREY: (u8, u8, u8) = (135, 135, 135);

    /// Asparagus Green - Ending color for gradients
    pub const ASPARAGUS_GREEN: (u8, u8, u8) = (130, 164, 115);

    /// Warning Yellow
    pub const WARNING_YELLOW: (u8, u8, u8) = (255, 193, 7);

    /// Error Red
    pub const ERROR_RED: (u8, u8, u8) = (220, 53, 69);

    /// Info Cyan
    pub const INFO_CYAN: (u8, u8, u8) = (0, 188, 212);

    /// Success Green (same as Asparagus)
    pub const SUCCESS: (u8, u8, u8) = Self::ASPARAGUS_GREEN;

    /// Dim text
    pub const DIM: (u8, u8, u8) = (100, 100, 100);

    /// Convert RGB tuple to ratatui Color
    pub fn to_color(rgb: (u8, u8, u8)) -> Color {
        Color::Rgb(rgb.0, rgb.1, rgb.2)
    }

    /// Get the primary gradient start color as ratatui Color
    pub fn grey() -> Color {
        Self::to_color(Self::BATTLESHIP_GREY)
    }

    /// Get the primary gradient end color as ratatui Color
    pub fn green() -> Color {
        Self::to_color(Self::ASPARAGUS_GREEN)
    }
}

/// Gradient generator for smooth color transitions
#[derive(Debug, Clone)]
pub struct Gradient {
    start: (u8, u8, u8),
    end: (u8, u8, u8),
}

impl Default for Gradient {
    fn default() -> Self {
        Self::brand()
    }
}

impl Gradient {
    /// Create a new gradient between two colors
    pub fn new(start: (u8, u8, u8), end: (u8, u8, u8)) -> Self {
        Self { start, end }
    }

    /// Create the default AI-Foundation brand gradient (Grey → Green)
    pub fn brand() -> Self {
        Self::new(BrandColors::BATTLESHIP_GREY, BrandColors::ASPARAGUS_GREEN)
    }

    /// Create a warning gradient (Yellow → Orange)
    pub fn warning() -> Self {
        Self::new(BrandColors::WARNING_YELLOW, (255, 150, 0))
    }

    /// Create an error gradient (Red → Dark Red)
    pub fn error() -> Self {
        Self::new(BrandColors::ERROR_RED, (180, 30, 50))
    }

    /// Create an info gradient (Cyan → Blue)
    pub fn info() -> Self {
        Self::new(BrandColors::INFO_CYAN, (30, 136, 229))
    }

    /// Linear interpolation between two colors
    /// t should be in range [0.0, 1.0]
    pub fn lerp(&self, t: f32) -> (u8, u8, u8) {
        let t = t.clamp(0.0, 1.0);
        (
            (self.start.0 as f32 + (self.end.0 as f32 - self.start.0 as f32) * t) as u8,
            (self.start.1 as f32 + (self.end.1 as f32 - self.start.1 as f32) * t) as u8,
            (self.start.2 as f32 + (self.end.2 as f32 - self.start.2 as f32) * t) as u8,
        )
    }

    /// Get color at position t as ratatui Color
    pub fn at(&self, t: f32) -> Color {
        let rgb = self.lerp(t);
        Color::Rgb(rgb.0, rgb.1, rgb.2)
    }

    /// Generate a vector of colors for text of given length
    /// Only counts visible characters (skips whitespace for gradient calculation)
    pub fn for_text(&self, text: &str) -> Vec<Color> {
        let visible_count = text.chars().filter(|c| !c.is_whitespace()).count();
        if visible_count == 0 {
            return vec![self.at(0.0); text.len()];
        }

        let mut colors = Vec::with_capacity(text.len());
        let mut visible_idx = 0;

        for c in text.chars() {
            if c.is_whitespace() {
                colors.push(Color::Reset);
            } else {
                let t = visible_idx as f32 / (visible_count - 1).max(1) as f32;
                colors.push(self.at(t));
                visible_idx += 1;
            }
        }

        colors
    }

    /// Generate colors for multi-line ASCII art
    /// Gradient flows horizontally across the entire width
    pub fn for_ascii_art(&self, lines: &[&str]) -> Vec<Vec<Color>> {
        let max_width = lines.iter().map(|l| l.len()).max().unwrap_or(0);
        if max_width == 0 {
            return vec![];
        }

        lines.iter().map(|line| {
            line.chars().enumerate().map(|(i, _)| {
                let t = i as f32 / (max_width - 1).max(1) as f32;
                self.at(t)
            }).collect()
        }).collect()
    }
}

/// Status icon and color combinations
#[derive(Debug, Clone, Copy)]
pub enum StatusType {
    Success,
    Error,
    Warning,
    Info,
    Pending,
}

impl StatusType {
    /// Get the icon for this status type
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Success => "✓",
            Self::Error => "✗",
            Self::Warning => "⚠",
            Self::Info => "→",
            Self::Pending => "○",
        }
    }

    /// Get the color for this status type
    pub fn color(&self) -> Color {
        match self {
            Self::Success => BrandColors::to_color(BrandColors::ASPARAGUS_GREEN),
            Self::Error => BrandColors::to_color(BrandColors::ERROR_RED),
            Self::Warning => BrandColors::to_color(BrandColors::WARNING_YELLOW),
            Self::Info => BrandColors::to_color(BrandColors::BATTLESHIP_GREY),
            Self::Pending => BrandColors::to_color(BrandColors::BATTLESHIP_GREY),
        }
    }
}

/// Helper to create styled separator lines
pub fn separator(width: usize) -> Vec<(char, Color)> {
    let gradient = Gradient::brand();
    (0..width).map(|i| {
        let t = i as f32 / (width - 1).max(1) as f32;
        ('═', gradient.at(t))
    }).collect()
}

/// The AI-Foundation logo as ASCII art
pub const LOGO: &[&str] = &[
    "  ███████╗ ██████╗ ██████╗  ██████╗ ███████╗",
    "  ██╔════╝██╔═══██╗██╔══██╗██╔════╝ ██╔════╝",
    "  █████╗  ██║   ██║██████╔╝██║  ███╗█████╗  ",
    "  ██╔══╝  ██║   ██║██╔══██╗██║   ██║██╔══╝  ",
    "  ██║     ╚██████╔╝██║  ██║╚██████╔╝███████╗",
    "  ╚═╝      ╚═════╝ ╚═╝  ╚═╝ ╚═════╝ ╚══════╝",
];

/// Tagline for the CLI
pub const TAGLINE: &str = "Empowering AIs everywhere, always";

/// Smaller logo for constrained spaces
pub const LOGO_SMALL: &[&str] = &[
    "╔═╗╔═╗╦═╗╔═╗╔═╗",
    "╠╣ ║ ║╠╦╝║ ╦║╣ ",
    "╚  ╚═╝╩╚═╚═╝╚═╝",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gradient_lerp() {
        let g = Gradient::brand();

        // At t=0, should be start color
        assert_eq!(g.lerp(0.0), BrandColors::BATTLESHIP_GREY);

        // At t=1, should be end color
        assert_eq!(g.lerp(1.0), BrandColors::ASPARAGUS_GREEN);

        // At t=0.5, should be midpoint
        let mid = g.lerp(0.5);
        assert!(mid.0 > 130 && mid.0 < 135);
    }

    #[test]
    fn test_gradient_for_text() {
        let g = Gradient::brand();
        let colors = g.for_text("Hello");
        assert_eq!(colors.len(), 5);
    }
}
