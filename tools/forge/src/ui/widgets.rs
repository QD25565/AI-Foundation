//! Custom widgets for Forge UI
//!
//! Gradient-styled widgets that match AI-Foundation branding.
#![allow(dead_code)]

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Widget},
};

use super::colors::{Gradient, BrandColors, StatusType, LOGO, separator};

/// A gradient-styled text widget
pub struct GradientText<'a> {
    content: &'a str,
    gradient: Gradient,
}

impl<'a> GradientText<'a> {
    pub fn new(content: &'a str) -> Self {
        Self {
            content,
            gradient: Gradient::brand(),
        }
    }

    pub fn gradient(mut self, gradient: Gradient) -> Self {
        self.gradient = gradient;
        self
    }

    /// Convert to a Line with gradient colors
    pub fn to_line(&self) -> Line<'a> {
        let colors = self.gradient.for_text(self.content);
        let spans: Vec<Span> = self.content
            .chars()
            .zip(colors.into_iter())
            .map(|(c, color)| {
                Span::styled(c.to_string(), Style::default().fg(color))
            })
            .collect();
        Line::from(spans)
    }
}

impl Widget for GradientText<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let line = self.to_line();
        let paragraph = Paragraph::new(line);
        paragraph.render(area, buf);
    }
}

/// The Forge logo widget with gradient colors
pub struct Logo {
    animated: bool,
    frame: usize,
}

impl Default for Logo {
    fn default() -> Self {
        Self::new()
    }
}

impl Logo {
    pub fn new() -> Self {
        Self {
            animated: false,
            frame: 0,
        }
    }

    pub fn animated(mut self, frame: usize) -> Self {
        self.animated = true;
        self.frame = frame;
        self
    }

    /// Get the logo as gradient-colored lines
    pub fn to_text(&self) -> Text<'static> {
        let gradient = Gradient::brand();
        let colors_per_line = gradient.for_ascii_art(LOGO);

        let lines: Vec<Line> = LOGO.iter().enumerate().map(|(line_idx, line)| {
            let colors = &colors_per_line[line_idx];

            // For animation, only show characters up to a certain point
            let visible_chars = if self.animated {
                let chars_per_frame = 3;
                let total_chars: usize = LOGO.iter().take(line_idx).map(|l| l.len()).sum();
                let current_pos = self.frame * chars_per_frame;
                if current_pos > total_chars {
                    (current_pos - total_chars).min(line.len())
                } else {
                    0
                }
            } else {
                line.len()
            };

            let spans: Vec<Span> = line.chars().enumerate().map(|(i, c)| {
                if i < visible_chars {
                    let color = colors.get(i).copied().unwrap_or(BrandColors::grey());
                    Span::styled(c.to_string(), Style::default().fg(color))
                } else {
                    Span::raw(" ")
                }
            }).collect();

            Line::from(spans)
        }).collect();

        Text::from(lines)
    }
}

impl Widget for Logo {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let text = self.to_text();
        let paragraph = Paragraph::new(text).alignment(Alignment::Center);
        paragraph.render(area, buf);
    }
}

/// A gradient separator line
pub struct Separator {
    width: usize,
}

impl Separator {
    pub fn new(width: usize) -> Self {
        Self { width }
    }

    pub fn to_line(&self) -> Line<'static> {
        let chars_colors = separator(self.width);
        let spans: Vec<Span> = chars_colors
            .into_iter()
            .map(|(c, color)| Span::styled(c.to_string(), Style::default().fg(color)))
            .collect();
        Line::from(spans)
    }
}

impl Widget for Separator {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let line = self.to_line();
        let paragraph = Paragraph::new(line);
        paragraph.render(area, buf);
    }
}

/// A status message with icon
pub struct StatusMessage<'a> {
    message: &'a str,
    status: StatusType,
}

impl<'a> StatusMessage<'a> {
    pub fn new(message: &'a str, status: StatusType) -> Self {
        Self { message, status }
    }

    pub fn success(message: &'a str) -> Self {
        Self::new(message, StatusType::Success)
    }

    pub fn error(message: &'a str) -> Self {
        Self::new(message, StatusType::Error)
    }

    pub fn warning(message: &'a str) -> Self {
        Self::new(message, StatusType::Warning)
    }

    pub fn info(message: &'a str) -> Self {
        Self::new(message, StatusType::Info)
    }

    pub fn pending(message: &'a str) -> Self {
        Self::new(message, StatusType::Pending)
    }

    pub fn to_line(&self) -> Line<'a> {
        let icon_span = Span::styled(
            format!("{} ", self.status.icon()),
            Style::default().fg(self.status.color()),
        );

        let gradient = Gradient::brand();
        let colors = gradient.for_text(self.message);
        let message_spans: Vec<Span> = self.message
            .chars()
            .zip(colors.into_iter())
            .map(|(c, color)| Span::styled(c.to_string(), Style::default().fg(color)))
            .collect();

        let mut spans = vec![icon_span];
        spans.extend(message_spans);
        Line::from(spans)
    }
}

impl Widget for StatusMessage<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let line = self.to_line();
        let paragraph = Paragraph::new(line);
        paragraph.render(area, buf);
    }
}

/// A gradient-bordered block
pub struct GradientBlock<'a> {
    title: Option<&'a str>,
    borders: Borders,
}

impl<'a> GradientBlock<'a> {
    pub fn new() -> Self {
        Self {
            title: None,
            borders: Borders::ALL,
        }
    }

    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    pub fn borders(mut self, borders: Borders) -> Self {
        self.borders = borders;
        self
    }

    pub fn to_block(&self) -> Block<'a> {
        let mut block = Block::default()
            .borders(self.borders)
            .border_style(Style::default().fg(BrandColors::grey()));

        if let Some(title) = self.title {
            // Create gradient title
            let gradient = Gradient::brand();
            let colors = gradient.for_text(title);
            let spans: Vec<Span> = title
                .chars()
                .zip(colors.into_iter())
                .map(|(c, color)| Span::styled(c.to_string(), Style::default().fg(color).bold()))
                .collect();
            block = block.title(Line::from(spans));
        }

        block
    }
}

impl Default for GradientBlock<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Chat message display widget
pub struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
    is_user: bool,
}

impl<'a> ChatMessage<'a> {
    pub fn user(content: &'a str) -> Self {
        Self {
            role: "You",
            content,
            is_user: true,
        }
    }

    pub fn assistant(content: &'a str) -> Self {
        Self {
            role: "Forge",
            content,
            is_user: false,
        }
    }

    pub fn system(content: &'a str) -> Self {
        Self {
            role: "System",
            content,
            is_user: false,
        }
    }

    pub fn to_text(&self) -> Text<'a> {
        let role_color = if self.is_user {
            BrandColors::to_color(BrandColors::INFO_CYAN)
        } else {
            BrandColors::green()
        };

        let role_line = Line::from(vec![
            Span::styled(
                format!("{}:", self.role),
                Style::default().fg(role_color).bold(),
            ),
        ]);

        let content_lines: Vec<Line> = self.content
            .lines()
            .map(|line| Line::from(Span::raw(line.to_string())))
            .collect();

        let mut lines = vec![role_line];
        lines.extend(content_lines);
        lines.push(Line::from("")); // Empty line after message

        Text::from(lines)
    }
}

/// Input field with gradient styling
pub struct InputField<'a> {
    content: &'a str,
    cursor_pos: usize,
    placeholder: &'a str,
}

impl<'a> InputField<'a> {
    pub fn new(content: &'a str, cursor_pos: usize) -> Self {
        Self {
            content,
            cursor_pos,
            placeholder: "Type a message...",
        }
    }

    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = placeholder;
        self
    }

    pub fn to_paragraph(&self) -> Paragraph<'a> {
        let display_text = if self.content.is_empty() {
            Span::styled(
                self.placeholder.to_string(),
                Style::default().fg(BrandColors::to_color(BrandColors::DIM)),
            )
        } else {
            Span::raw(self.content.to_string())
        };

        Paragraph::new(Line::from(display_text))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BrandColors::grey()))
                    .title(Span::styled(
                        " Message ",
                        Style::default().fg(BrandColors::green()).bold(),
                    )),
            )
    }
}

/// Tool approval dialog
pub struct ToolApproval<'a> {
    tool_name: &'a str,
    description: &'a str,
    args: Vec<(&'a str, &'a str)>,
}

impl<'a> ToolApproval<'a> {
    pub fn new(tool_name: &'a str, description: &'a str) -> Self {
        Self {
            tool_name,
            description,
            args: vec![],
        }
    }

    pub fn arg(mut self, name: &'a str, value: &'a str) -> Self {
        self.args.push((name, value));
        self
    }

    pub fn to_text(&self) -> Text<'a> {
        let mut lines = vec![
            Line::from(vec![
                Span::styled("Tool: ", Style::default().fg(BrandColors::grey())),
                Span::styled(self.tool_name, Style::default().fg(BrandColors::green()).bold()),
            ]),
            Line::from(Span::styled(
                self.description,
                Style::default().fg(BrandColors::to_color(BrandColors::DIM)),
            )),
            Line::from(""),
        ];

        if !self.args.is_empty() {
            lines.push(Line::from(Span::styled(
                "Arguments:",
                Style::default().fg(BrandColors::grey()),
            )));

            for (name, value) in &self.args {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("{}: ", name), Style::default().fg(BrandColors::grey())),
                    Span::raw(value.to_string()),
                ]));
            }

            lines.push(Line::from(""));
        }

        lines.push(Line::from(vec![
            Span::styled("[Y]", Style::default().fg(BrandColors::green()).bold()),
            Span::raw(" Allow  "),
            Span::styled("[N]", Style::default().fg(BrandColors::to_color(BrandColors::ERROR_RED)).bold()),
            Span::raw(" Deny  "),
            Span::styled("[A]", Style::default().fg(BrandColors::to_color(BrandColors::INFO_CYAN)).bold()),
            Span::raw(" Always allow"),
        ]));

        Text::from(lines)
    }
}

/// Loading spinner with gradient
pub struct Spinner {
    frame: usize,
}

impl Spinner {
    const FRAMES: &'static [&'static str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    pub fn new(frame: usize) -> Self {
        Self { frame }
    }

    pub fn to_span(&self) -> Span<'static> {
        let idx = self.frame % Self::FRAMES.len();
        let gradient = Gradient::brand();
        let t = (self.frame % 20) as f32 / 20.0;
        let color = gradient.at(t);

        Span::styled(
            Self::FRAMES[idx].to_string(),
            Style::default().fg(color),
        )
    }
}

/// Token streaming display
pub struct TokenStream<'a> {
    tokens: &'a [String],
    show_cursor: bool,
}

impl<'a> TokenStream<'a> {
    pub fn new(tokens: &'a [String]) -> Self {
        Self {
            tokens,
            show_cursor: true,
        }
    }

    pub fn cursor(mut self, show: bool) -> Self {
        self.show_cursor = show;
        self
    }

    pub fn to_text(&self) -> Text<'static> {
        let content: String = self.tokens.join("");
        let mut text = content.clone();

        if self.show_cursor {
            text.push('▌');
        }

        Text::raw(text)
    }
}
