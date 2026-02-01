//! Main application state and rendering
//!
//! Manages the Forge UI state and renders the interface.

use std::collections::VecDeque;

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Widget, Wrap},
    Frame,
};

use super::colors::{BrandColors, Gradient, LOGO, TAGLINE};
use super::widgets::{GradientBlock, GradientText, Logo, Separator, Spinner, StatusMessage};

/// Message role in conversation
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
}

/// A message in the conversation
#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub tool_name: Option<String>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_name: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_name: None,
        }
    }

    pub fn tool(name: impl Into<String>, result: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: result.into(),
            tool_name: Some(name.into()),
        }
    }
}

/// Tool awaiting approval
#[derive(Debug, Clone)]
pub struct PendingTool {
    pub name: String,
    pub description: String,
    pub args: Vec<(String, String)>,
}

/// Current input mode
#[derive(Debug, Clone, PartialEq)]
pub enum InputMode {
    Normal,
    Editing,
    ToolApproval,
}

/// Slash command definition
#[derive(Debug, Clone)]
pub struct SlashCommand {
    pub name: &'static str,
    pub description: &'static str,
    pub has_arg: bool,
}

/// Available slash commands
pub const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand { name: "/models", description: "List available models", has_arg: false },
    SlashCommand { name: "/model", description: "Switch to a model", has_arg: true },
    SlashCommand { name: "/tools", description: "List available tools", has_arg: false },
    SlashCommand { name: "/help", description: "Show help", has_arg: false },
    SlashCommand { name: "/clear", description: "Clear conversation", has_arg: false },
    SlashCommand { name: "/quit", description: "Exit Forge", has_arg: false },
];

/// Application state
pub struct App {
    /// Current input mode
    pub input_mode: InputMode,

    /// User input buffer
    pub input: String,

    /// Cursor position in input
    pub cursor_pos: usize,

    /// Conversation history
    pub messages: Vec<Message>,

    /// Scroll position for messages
    pub scroll: usize,

    /// Is the assistant currently generating?
    pub is_generating: bool,

    /// Current streaming tokens
    pub streaming_tokens: Vec<String>,

    /// Pending tool approval
    pub pending_tool: Option<PendingTool>,

    /// Animation frame counter
    pub frame: usize,

    /// Show welcome screen
    pub show_welcome: bool,

    /// Model name
    pub model_name: String,

    /// AI ID (from config)
    pub ai_id: String,

    /// Status messages (temporary notifications)
    pub status_messages: VecDeque<(String, std::time::Instant)>,

    /// Should quit
    pub should_quit: bool,

    /// Autocomplete state
    pub autocomplete_index: usize,
    pub show_autocomplete: bool,

    /// Text selection (start, end) - None means no selection
    pub selection: Option<(usize, usize)>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            input_mode: InputMode::Normal,
            input: String::new(),
            cursor_pos: 0,
            messages: vec![],
            scroll: 0,
            is_generating: false,
            streaming_tokens: vec![],
            pending_tool: None,
            frame: 0,
            show_welcome: true,
            autocomplete_index: 0,
            show_autocomplete: false,
            selection: None,
            model_name: "Not connected".to_string(),
            ai_id: "forge".to_string(),
            status_messages: VecDeque::new(),
            should_quit: false,
        }
    }

    /// Advance animation frame
    pub fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);

        // Clean old status messages (older than 5 seconds)
        let now = std::time::Instant::now();
        while let Some((_, time)) = self.status_messages.front() {
            if now.duration_since(*time).as_secs() > 5 {
                self.status_messages.pop_front();
            } else {
                break;
            }
        }
    }

    /// Add a status message
    pub fn add_status(&mut self, message: impl Into<String>) {
        self.status_messages.push_back((message.into(), std::time::Instant::now()));
    }

    /// Handle character input
    pub fn on_char(&mut self, c: char) {
        if self.input_mode == InputMode::Editing {
            // Clear selection when typing
            self.selection = None;
            self.input.insert(self.cursor_pos, c);
            self.cursor_pos += 1;
            // Update autocomplete state
            self.update_autocomplete();
        }
    }

    /// Handle backspace
    pub fn on_backspace(&mut self) {
        if self.input_mode == InputMode::Editing && self.cursor_pos > 0 {
            self.selection = None;
            self.cursor_pos -= 1;
            self.input.remove(self.cursor_pos);
            self.update_autocomplete();
        }
    }

    /// Handle delete
    pub fn on_delete(&mut self) {
        if self.input_mode == InputMode::Editing && self.cursor_pos < self.input.len() {
            self.selection = None;
            self.input.remove(self.cursor_pos);
            self.update_autocomplete();
        }
    }

    /// Move cursor left
    pub fn cursor_left(&mut self) {
        self.selection = None;
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        self.selection = None;
        if self.cursor_pos < self.input.len() {
            self.cursor_pos += 1;
        }
    }

    /// Move cursor left with selection (Shift+Left)
    pub fn cursor_left_select(&mut self) {
        if self.cursor_pos > 0 {
            let new_pos = self.cursor_pos - 1;
            match self.selection {
                Some((start, _)) => self.selection = Some((start, new_pos)),
                None => self.selection = Some((self.cursor_pos, new_pos)),
            }
            self.cursor_pos = new_pos;
        }
    }

    /// Move cursor right with selection (Shift+Right)
    pub fn cursor_right_select(&mut self) {
        if self.cursor_pos < self.input.len() {
            let new_pos = self.cursor_pos + 1;
            match self.selection {
                Some((start, _)) => self.selection = Some((start, new_pos)),
                None => self.selection = Some((self.cursor_pos, new_pos)),
            }
            self.cursor_pos = new_pos;
        }
    }

    /// Get selected text
    pub fn get_selected_text(&self) -> Option<String> {
        self.selection.map(|(start, end)| {
            let (s, e) = if start <= end { (start, end) } else { (end, start) };
            self.input[s..e].to_string()
        })
    }

    /// Update autocomplete state based on current input
    fn update_autocomplete(&mut self) {
        if self.input.starts_with('/') {
            self.show_autocomplete = true;
            // Reset index if filtered list changes
            let filtered = self.get_filtered_commands();
            if self.autocomplete_index >= filtered.len() {
                self.autocomplete_index = 0;
            }
        } else {
            self.show_autocomplete = false;
            self.autocomplete_index = 0;
        }
    }

    /// Get commands filtered by current input
    pub fn get_filtered_commands(&self) -> Vec<&'static SlashCommand> {
        if !self.input.starts_with('/') {
            return vec![];
        }
        SLASH_COMMANDS
            .iter()
            .filter(|cmd| cmd.name.starts_with(&self.input) || self.input == "/")
            .collect()
    }

    /// Cycle to next autocomplete option
    pub fn autocomplete_next(&mut self) {
        let filtered = self.get_filtered_commands();
        if !filtered.is_empty() {
            self.autocomplete_index = (self.autocomplete_index + 1) % filtered.len();
        }
    }

    /// Cycle to previous autocomplete option
    pub fn autocomplete_prev(&mut self) {
        let filtered = self.get_filtered_commands();
        if !filtered.is_empty() {
            self.autocomplete_index = if self.autocomplete_index == 0 {
                filtered.len() - 1
            } else {
                self.autocomplete_index - 1
            };
        }
    }

    /// Complete the current autocomplete selection (Tab)
    pub fn autocomplete_complete(&mut self) {
        let filtered = self.get_filtered_commands();
        if let Some(cmd) = filtered.get(self.autocomplete_index) {
            self.input = if cmd.has_arg {
                format!("{} ", cmd.name)
            } else {
                cmd.name.to_string()
            };
            self.cursor_pos = self.input.len();
            self.show_autocomplete = false;
        }
    }

    /// Check if autocomplete should be shown
    pub fn should_show_autocomplete(&self) -> bool {
        self.show_autocomplete && self.input.starts_with('/') && !self.get_filtered_commands().is_empty()
    }

    /// Submit current input
    pub fn submit(&mut self) -> Option<String> {
        if self.input.is_empty() {
            return None;
        }

        let message = self.input.clone();
        self.messages.push(Message::user(&message));
        self.input.clear();
        self.cursor_pos = 0;
        self.show_welcome = false;

        Some(message)
    }

    /// Add assistant message (use for non-streaming responses)
    pub fn add_assistant_message(&mut self, content: impl Into<String>) {
        self.messages.push(Message::assistant(content));
        self.streaming_tokens.clear();
        self.is_generating = false;
    }

    /// Add streaming token
    pub fn add_token(&mut self, token: impl Into<String>) {
        self.streaming_tokens.push(token.into());
    }

    /// Finalize streaming - convert accumulated tokens to a message
    /// Returns the final content for conversation history
    pub fn finalize_streaming(&mut self) -> String {
        let content: String = self.streaming_tokens.join("");

        // Process content - hide <think> blocks
        let display_content = Self::process_think_blocks(&content);

        self.messages.push(Message::assistant(display_content));
        self.streaming_tokens.clear();
        self.is_generating = false;

        content // Return raw content for conversation history
    }

    /// Process think blocks - collapse them for display (for finalized messages)
    fn process_think_blocks(content: &str) -> String {
        // Find <think>...</think> and replace with collapsed indicator
        let mut result = String::new();
        let mut remaining = content;

        while let Some(start) = remaining.find("<think>") {
            // Add content before <think>
            result.push_str(&remaining[..start]);

            // Find closing </think>
            if let Some(end) = remaining.find("</think>") {
                let think_content = &remaining[start + 7..end];
                let lines = think_content.lines().count();
                result.push_str(&format!("💭 [Reasoning: {} lines - expand with 't']\n", lines));
                remaining = &remaining[end + 8..];
            } else {
                // No closing tag, show as-is
                remaining = &remaining[start..];
                break;
            }
        }

        result.push_str(remaining);
        result.trim().to_string()
    }

    /// Process think blocks during streaming (handles incomplete blocks)
    fn process_streaming_think(content: &str) -> String {
        // During streaming, we might have:
        // 1. Complete <think>...</think> blocks - collapse them
        // 2. Incomplete <think>... without closing - show "thinking..." indicator
        // 3. No think block - show content as-is

        let mut result = String::new();
        let mut remaining = content;
        let mut in_think = false;
        let mut think_line_count = 0;

        while !remaining.is_empty() {
            if let Some(start) = remaining.find("<think>") {
                // Add content before <think>
                result.push_str(&remaining[..start]);
                remaining = &remaining[start + 7..];
                in_think = true;
                think_line_count = 0;

                // Look for closing tag
                if let Some(end) = remaining.find("</think>") {
                    let think_content = &remaining[..end];
                    think_line_count = think_content.lines().count();
                    result.push_str(&format!("💭 [Reasoning: {} lines]\n", think_line_count));
                    remaining = &remaining[end + 8..];
                    in_think = false;
                } else {
                    // Still thinking - count lines so far
                    think_line_count = remaining.lines().count();
                    result.push_str(&format!("💭 [Reasoning: {}+ lines...]\n", think_line_count));
                    break;
                }
            } else if in_think {
                // Should not reach here, but handle gracefully
                break;
            } else {
                // No more think blocks, add remaining content
                result.push_str(remaining);
                break;
            }
        }

        result
    }

    /// Get current token count for display
    pub fn streaming_token_count(&self) -> usize {
        self.streaming_tokens.len()
    }

    /// Scroll messages up (towards older messages)
    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    /// Scroll messages down (towards newer messages)
    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    /// Scroll up by a page (10 lines)
    pub fn scroll_page_up(&mut self) {
        self.scroll = self.scroll.saturating_add(10);
    }

    /// Scroll down by a page (10 lines)
    pub fn scroll_page_down(&mut self) {
        self.scroll = self.scroll.saturating_sub(10);
    }

    /// Scroll to the top (oldest messages)
    pub fn scroll_to_top(&mut self) {
        self.scroll = usize::MAX / 2; // Large value, will be clamped during render
    }

    /// Scroll to the bottom (newest messages)
    pub fn scroll_to_bottom(&mut self) {
        self.scroll = 0;
    }

    /// Request tool approval
    pub fn request_tool_approval(&mut self, tool: PendingTool) {
        self.pending_tool = Some(tool);
        self.input_mode = InputMode::ToolApproval;
    }

    /// Approve pending tool
    pub fn approve_tool(&mut self) -> Option<PendingTool> {
        self.input_mode = InputMode::Editing;
        self.pending_tool.take()
    }

    /// Deny pending tool
    pub fn deny_tool(&mut self) {
        self.input_mode = InputMode::Editing;
        self.pending_tool = None;
    }

    /// Render the UI
    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();

        // Main layout
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),  // Title bar
                Constraint::Min(10),    // Chat area
                Constraint::Length(3),  // Input area
                Constraint::Length(1),  // Status bar
            ])
            .split(area);

        self.render_title_bar(frame, chunks[0]);
        self.render_chat_area(frame, chunks[1]);
        self.render_input_area(frame, chunks[2]);
        self.render_status_bar(frame, chunks[3]);

        // Render tool approval popup if needed
        if let Some(ref tool) = self.pending_tool {
            self.render_tool_approval(frame, tool);
        }
    }

    fn render_title_bar(&self, frame: &mut Frame, area: Rect) {
        let gradient = Gradient::brand();

        // Create gradient title
        let title = " FORGE ";
        let colors = gradient.for_text(title);
        let title_spans: Vec<Span> = title
            .chars()
            .zip(colors.into_iter())
            .map(|(c, color)| Span::styled(c.to_string(), Style::default().fg(color).bold()))
            .collect();

        // Only show model name (not ai_id which is for AI agents)
        let model_info = format!(" {} ", self.model_name);

        // Build the title bar
        let mut spans = vec![
            Span::styled("═", Style::default().fg(BrandColors::grey())),
        ];
        spans.extend(title_spans);

        // Fill with gradient separator (account for both edge chars)
        let used = 1 + title.len() + model_info.len() + 1; // start ═ + title + model_info + end ═
        let remaining = (area.width as usize).saturating_sub(used);
        for i in 0..remaining {
            let t = i as f32 / remaining.max(1) as f32;
            spans.push(Span::styled("═", Style::default().fg(gradient.at(t))));
        }

        spans.push(Span::styled(&model_info, Style::default().fg(BrandColors::grey())));
        spans.push(Span::styled("═", Style::default().fg(BrandColors::green())));

        let title_line = Line::from(spans);
        frame.render_widget(Paragraph::new(title_line), area);
    }

    fn render_chat_area(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::LEFT | Borders::RIGHT)
            .border_style(Style::default().fg(BrandColors::grey()));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.show_welcome {
            self.render_welcome(frame, inner);
        } else {
            self.render_messages(frame, inner);
        }
    }

    fn render_welcome(&self, frame: &mut Frame, area: Rect) {
        let logo = Logo::new(); // No animation - instant display
        let logo_text = logo.to_text();
        let logo_height = logo_text.lines.len() as u16;

        // Center the welcome content
        let content_height = logo_height + 6;
        let vertical_padding = (area.height.saturating_sub(content_height)) / 2;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(vertical_padding),
                Constraint::Length(logo_height),
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(area);

        // Logo
        frame.render_widget(
            Paragraph::new(logo_text).alignment(Alignment::Center),
            chunks[1],
        );

        // Tagline with gradient
        let gradient = Gradient::brand();
        let colors = gradient.for_text(TAGLINE);
        let tagline_spans: Vec<Span> = TAGLINE
            .chars()
            .zip(colors.into_iter())
            .map(|(c, color)| Span::styled(c.to_string(), Style::default().fg(color)))
            .collect();
        frame.render_widget(
            Paragraph::new(Line::from(tagline_spans)).alignment(Alignment::Center),
            chunks[3],
        );

        // Instructions
        let help = "Press Enter to start chatting │ Ctrl+C to quit";
        frame.render_widget(
            Paragraph::new(help)
                .alignment(Alignment::Center)
                .style(Style::default().fg(BrandColors::to_color(BrandColors::DIM))),
            chunks[5],
        );
    }

    fn render_messages(&self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = vec![];

        for msg in &self.messages {
            let (role_text, role_color) = match msg.role {
                Role::User => ("You", BrandColors::INFO_CYAN),
                Role::Assistant => ("Forge", BrandColors::ASPARAGUS_GREEN),
                Role::System => ("System", BrandColors::WARNING_YELLOW),
                Role::Tool => {
                    let name = msg.tool_name.as_deref().unwrap_or("tool");
                    (name, BrandColors::BATTLESHIP_GREY)
                }
            };

            // Role line
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}:", role_text),
                    Style::default().fg(BrandColors::to_color(role_color)).bold(),
                ),
            ]));

            // Content lines
            for line in msg.content.lines() {
                lines.push(Line::from(line.to_string()));
            }

            lines.push(Line::from("")); // Spacing
        }

        // Add streaming tokens if generating
        if self.is_generating && !self.streaming_tokens.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Forge:", Style::default().fg(BrandColors::green()).bold()),
            ]));

            let content: String = self.streaming_tokens.join("");

            // Hide <think> blocks during streaming too
            let display_content = Self::process_streaming_think(&content);
            for line in display_content.lines() {
                lines.push(Line::from(line.to_string()));
            }

            // Blinking cursor
            if self.frame % 10 < 5 {
                lines.push(Line::from("▌"));
            }
        } else if self.is_generating {
            // Show spinner while waiting
            let spinner = Spinner::new(self.frame);
            lines.push(Line::from(vec![
                spinner.to_span(),
                Span::raw(" Thinking..."),
            ]));
        }

        let text = Text::from(lines);
        let paragraph = Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((self.scroll as u16, 0));

        frame.render_widget(paragraph, area);
    }

    fn render_input_area(&self, frame: &mut Frame, area: Rect) {
        let gradient = Gradient::brand();

        // Create gradient border title
        let title = " Message ";
        let colors = gradient.for_text(title);
        let title_spans: Vec<Span> = title
            .chars()
            .zip(colors.into_iter())
            .map(|(c, color)| Span::styled(c.to_string(), Style::default().fg(color).bold()))
            .collect();

        // Build input text with selection highlighting
        let display_line = if self.input.is_empty() && self.input_mode != InputMode::Editing {
            Line::from(Span::styled(
                "Press Enter to type a message...",
                Style::default().fg(BrandColors::to_color(BrandColors::DIM)),
            ))
        } else if let Some((sel_start, sel_end)) = self.selection {
            // Render with selection highlight
            let (start, end) = if sel_start <= sel_end { (sel_start, sel_end) } else { (sel_end, sel_start) };
            let mut spans = vec![];
            if start > 0 {
                spans.push(Span::raw(&self.input[..start]));
            }
            spans.push(Span::styled(
                &self.input[start..end],
                Style::default().bg(BrandColors::to_color(BrandColors::BATTLESHIP_GREY)).fg(ratatui::style::Color::Black),
            ));
            if end < self.input.len() {
                spans.push(Span::raw(&self.input[end..]));
            }
            Line::from(spans)
        } else {
            Line::from(Span::raw(&self.input))
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if self.input_mode == InputMode::Editing {
                BrandColors::green()
            } else {
                BrandColors::grey()
            }))
            .title(Line::from(title_spans));

        let paragraph = Paragraph::new(display_line).block(block);
        frame.render_widget(paragraph, area);

        // Show cursor when editing
        if self.input_mode == InputMode::Editing {
            frame.set_cursor_position((
                area.x + 1 + self.cursor_pos as u16,
                area.y + 1,
            ));
        }

        // Render autocomplete popup
        if self.should_show_autocomplete() {
            self.render_autocomplete(frame, area);
        }
    }

    fn render_autocomplete(&self, frame: &mut Frame, input_area: Rect) {
        let filtered = self.get_filtered_commands();
        if filtered.is_empty() {
            return;
        }

        // Position popup above the input area
        let popup_height = (filtered.len() as u16 + 2).min(8);
        let popup_width = 40.min(input_area.width);
        let popup_y = input_area.y.saturating_sub(popup_height);
        let popup_area = Rect::new(input_area.x, popup_y, popup_width, popup_height);

        // Clear background
        frame.render_widget(Clear, popup_area);

        // Build command list
        let items: Vec<Line> = filtered
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let is_selected = i == self.autocomplete_index;
                let style = if is_selected {
                    Style::default().bg(BrandColors::green()).fg(ratatui::style::Color::Black)
                } else {
                    Style::default().fg(BrandColors::grey())
                };
                Line::from(vec![
                    Span::styled(format!(" {} ", cmd.name), style.bold()),
                    Span::styled(
                        cmd.description,
                        if is_selected {
                            style
                        } else {
                            Style::default().fg(BrandColors::to_color(BrandColors::DIM))
                        },
                    ),
                ])
            })
            .collect();

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(BrandColors::grey()))
            .title(Span::styled(" Commands ", Style::default().fg(BrandColors::green())));

        let paragraph = Paragraph::new(items).block(block);
        frame.render_widget(paragraph, popup_area);
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        #![allow(unused_variables)] // gradient reserved for future use
        // Build status bar content
        let mode_text = match self.input_mode {
            InputMode::Normal => "NORMAL",
            InputMode::Editing => "INSERT",
            InputMode::ToolApproval => "APPROVE",
        };

        let status = if let Some((msg, _)) = self.status_messages.front() {
            msg.clone()
        } else {
            "Ready".to_string()
        };

        // Left side: mode
        let mode_span = Span::styled(
            format!(" {} ", mode_text),
            Style::default().fg(BrandColors::grey()).bold(),
        );

        // Middle: status or token count during generation
        let status_span = if self.is_generating {
            let token_count = self.streaming_token_count();
            Span::styled(
                format!(" {} tokens ", token_count),
                Style::default().fg(BrandColors::green()),
            )
        } else {
            Span::styled(
                format!(" {} ", status),
                Style::default().fg(BrandColors::to_color(BrandColors::DIM)),
            )
        };

        // Right side: help (shorter when generating to show tokens)
        let help = if self.is_generating {
            " Esc: Cancel "
        } else {
            " Ctrl+C: Quit │ Enter: Send │ Esc: Cancel "
        };
        let help_span = Span::styled(help, Style::default().fg(BrandColors::grey()));

        let line = Line::from(vec![mode_span, status_span, help_span]);
        frame.render_widget(Paragraph::new(line), area);
    }

    fn render_tool_approval(&self, frame: &mut Frame, tool: &PendingTool) {
        let area = frame.area();

        // Center popup
        let popup_width = 60.min(area.width - 4);
        let popup_height = 12.min(area.height - 4);
        let popup_x = (area.width - popup_width) / 2;
        let popup_y = (area.height - popup_height) / 2;
        let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

        // Clear background
        frame.render_widget(Clear, popup_area);

        // Build content
        let mut lines = vec![
            Line::from(vec![
                Span::styled("Tool: ", Style::default().fg(BrandColors::grey())),
                Span::styled(&tool.name, Style::default().fg(BrandColors::green()).bold()),
            ]),
            Line::from(Span::styled(
                &tool.description,
                Style::default().fg(BrandColors::to_color(BrandColors::DIM)),
            )),
            Line::from(""),
        ];

        if !tool.args.is_empty() {
            lines.push(Line::from(Span::styled(
                "Arguments:",
                Style::default().fg(BrandColors::grey()),
            )));

            for (name, value) in &tool.args {
                let display_value = if value.len() > 40 {
                    format!("{}...", &value[..40])
                } else {
                    value.clone()
                };
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("{}: ", name), Style::default().fg(BrandColors::grey())),
                    Span::raw(display_value),
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
            Span::raw(" Always"),
        ]));

        let block = Block::default()
            .title(Span::styled(
                " Tool Approval ",
                Style::default().fg(BrandColors::green()).bold(),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(BrandColors::green()));

        let paragraph = Paragraph::new(Text::from(lines))
            .block(block)
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, popup_area);
    }
}
