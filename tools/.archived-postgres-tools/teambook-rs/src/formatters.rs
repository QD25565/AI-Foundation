//! Output Formatting Module - Clear, Self-Evident Output
//!
//! Provides consistent, human-readable formatting for all tool outputs.
//! Handles terminal capability detection and symbol fallbacks.

use std::io::{self, IsTerminal};

/// Terminal capability detection
pub struct TerminalCapabilities;

impl TerminalCapabilities {
    /// Check if stdout is a terminal (TTY)
    pub fn is_tty() -> bool {
        io::stdout().is_terminal()
    }

    /// Check if terminal supports Unicode output
    pub fn supports_unicode() -> bool {
        // On Windows, check for UTF-8 codepage or modern terminal
        #[cfg(windows)]
        {
            // Check LANG or LC_ALL for UTF-8
            if let Ok(lang) = std::env::var("LANG") {
                if lang.to_lowercase().contains("utf-8") || lang.to_lowercase().contains("utf8") {
                    return true;
                }
            }
            // Check if Windows Terminal or modern console
            if std::env::var("WT_SESSION").is_ok() {
                return true;
            }
            // Check for ConEmu/Cmder
            if std::env::var("ConEmuANSI").is_ok() {
                return true;
            }
            // Default: modern Windows usually supports Unicode
            true
        }
        #[cfg(not(windows))]
        {
            // Unix systems typically support Unicode
            if let Ok(lang) = std::env::var("LANG") {
                return lang.to_lowercase().contains("utf-8") || lang.to_lowercase().contains("utf8");
            }
            true
        }
    }

    /// Check if terminal supports emoji
    pub fn supports_emoji() -> bool {
        // Emoji requires UTF-8 and modern terminal
        if !Self::supports_unicode() {
            return false;
        }
        #[cfg(windows)]
        {
            // Windows Terminal supports emoji
            std::env::var("WT_SESSION").is_ok()
        }
        #[cfg(not(windows))]
        {
            true
        }
    }

    /// Check if terminal supports ANSI colors
    pub fn supports_colors() -> bool {
        if !Self::is_tty() {
            return false;
        }
        // Check NO_COLOR environment variable (standard)
        if std::env::var("NO_COLOR").is_ok() {
            return false;
        }
        // Check CLICOLOR=0
        if let Ok(val) = std::env::var("CLICOLOR") {
            if val == "0" {
                return false;
            }
        }
        // Check TERM for dumb terminal
        if let Ok(term) = std::env::var("TERM") {
            if term == "dumb" {
                return false;
            }
        }
        true
    }
}

/// Symbol definitions with Unicode and ASCII fallbacks
pub struct Symbols;

impl Symbols {
    /// Get symbol with appropriate fallback based on terminal capabilities
    pub fn get(name: &str) -> &'static str {
        let use_unicode = TerminalCapabilities::supports_unicode();
        let use_emoji = TerminalCapabilities::supports_emoji();

        match name {
            // Status symbols
            "success" => if use_unicode { "✓" } else { "[OK]" },
            "working" => if use_unicode { "◆" } else { "[W]" },
            "blocked" => if use_unicode { "✗" } else { "[X]" },
            "warning" => if use_emoji { "⚠️" } else if use_unicode { "⚠" } else { "[!]" },
            "error" => if use_emoji { "❌" } else { "[ERROR]" },
            "info" => if use_emoji { "ℹ️" } else { "[INFO]" },

            // Formatting symbols
            "separator" => if use_unicode { "═" } else { "=" },
            "arrow" => if use_unicode { "→" } else { "->" },
            "dot" => if use_unicode { "•" } else { "-" },
            "pipe" => "|",

            // Document symbols
            "note" => if use_emoji { "📝" } else { "[NOTE]" },
            "pin" => if use_emoji { "📌" } else { "[PIN]" },
            "inbox" => if use_emoji { "📧" } else { "[INBOX]" },
            "broadcast" => if use_emoji { "📢" } else { "[BROADCAST]" },
            "task" => if use_emoji { "📋" } else { "[TASK]" },
            "tip" => if use_emoji { "💡" } else { "[TIP]" },
            "package" => if use_emoji { "📦" } else { "[PKG]" },
            "chart" => if use_emoji { "📊" } else { "[CHART]" },
            "wrench" => if use_emoji { "🔧" } else { "[TOOL]" },
            "check" => if use_emoji { "✅" } else { "[DONE]" },
            "bell" => if use_emoji { "🔔" } else { "[BELL]" },
            "sync" => if use_emoji { "🔄" } else { "[SYNC]" },
            "lock" => if use_emoji { "🔒" } else { "[LOCK]" },
            "unlock" => if use_emoji { "🔓" } else { "[UNLOCK]" },
            "ai" => if use_emoji { "🤖" } else { "[AI]" },
            "online" => if use_unicode { "●" } else { "[*]" },
            "offline" => if use_unicode { "○" } else { "[ ]" },
            "dm" => if use_emoji { "💬" } else { "[DM]" },
            "send" => if use_unicode { "→" } else { "->" },
            "receive" => if use_unicode { "←" } else { "<-" },

            // Default fallback - return empty for unknown symbols
            _ => "",
        }
    }

    /// Get separator line
    pub fn separator(width: usize) -> String {
        let sep = Self::get("separator");
        sep.repeat(width)
    }
}

/// Output formatter for consistent messaging
pub struct OutputFormatter;

impl OutputFormatter {
    /// Format a success message
    pub fn success(message: &str) -> String {
        format!("{} {}", Symbols::get("success"), message)
    }

    /// Format an error message
    pub fn error(message: &str) -> String {
        format!("{} {}", Symbols::get("error"), message)
    }

    /// Format an info message
    pub fn info(message: &str) -> String {
        format!("{} {}", Symbols::get("info"), message)
    }

    /// Format a warning message
    pub fn warning(message: &str) -> String {
        format!("{} {}", Symbols::get("warning"), message)
    }

    /// Format a section header
    pub fn header(title: &str) -> String {
        let sep = Symbols::separator(60);
        format!("{}\n{}\n{}", sep, title, sep)
    }

    /// Format a simple header with title only
    pub fn section(title: &str) -> String {
        format!("{}\n{}", title, "=".repeat(60))
    }

    /// Format a count summary (e.g., "Notes: 5 | Pinned: 2")
    pub fn count_summary(counts: &[(&str, i32)]) -> String {
        counts
            .iter()
            .map(|(label, count)| format!("{}: {}", label, count))
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// Format a pipe-delimited row
    pub fn pipe_row(fields: &[&str]) -> String {
        fields.join("|")
    }

    /// Format error with context and suggestions
    pub fn error_with_context(
        error_type: &str,
        message: &str,
        syntax: Option<&str>,
        example: Option<&str>,
        suggestion: Option<&str>,
    ) -> String {
        let mut output = Vec::new();
        output.push(format!("{} ERROR: {}", Symbols::get("error"), error_type));
        output.push(String::new());
        output.push("What Happened:".to_string());
        output.push(format!("  {}", message));
        output.push(String::new());

        if let Some(syn) = syntax {
            output.push("Syntax:".to_string());
            output.push(format!("  {}", syn));
            output.push(String::new());
        }

        if let Some(ex) = example {
            output.push("Example:".to_string());
            output.push(format!("  {}", ex));
            output.push(String::new());
        }

        if let Some(sug) = suggestion {
            output.push("How to Fix:".to_string());
            output.push(format!("  {}", sug));
            output.push(String::new());
        }

        output.push("Need More Help?".to_string());
        output.push("  Run: --help".to_string());

        output.join("\n")
    }

    /// Format a table with headers and rows
    pub fn table(title: &str, headers: &[&str], rows: &[Vec<String>]) -> String {
        let mut output = Vec::new();
        output.push(String::new());
        output.push(title.to_string());
        output.push("=".repeat(title.len().min(100)));
        output.push(String::new());

        // Calculate column widths
        let mut col_widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
        for row in rows {
            for (i, cell) in row.iter().enumerate() {
                if i < col_widths.len() {
                    col_widths[i] = col_widths[i].max(cell.len());
                }
            }
        }

        // Header row
        let header_row: String = headers
            .iter()
            .enumerate()
            .map(|(i, h)| format!("{:width$}", h, width = col_widths.get(i).copied().unwrap_or(h.len())))
            .collect::<Vec<_>>()
            .join(" | ");
        output.push(header_row.clone());
        output.push("-".repeat(header_row.len()));

        // Data rows
        for row in rows {
            let data_row: String = row
                .iter()
                .enumerate()
                .map(|(i, cell)| format!("{:width$}", cell, width = col_widths.get(i).copied().unwrap_or(cell.len())))
                .collect::<Vec<_>>()
                .join(" | ");
            output.push(data_row);
        }

        output.push(String::new());
        output.join("\n")
    }

    /// Format a bulleted list
    pub fn list(title: &str, items: &[&str]) -> String {
        let dot = Symbols::get("dot");
        let mut output = Vec::new();
        output.push(title.to_string());
        for item in items {
            output.push(format!("  {} {}", dot, item));
        }
        output.push(String::new());
        output.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_fallback() {
        // Just ensure it doesn't panic
        let _ = Symbols::get("success");
        let _ = Symbols::get("error");
        let _ = Symbols::get("unknown");
    }

    #[test]
    fn test_count_summary() {
        let summary = OutputFormatter::count_summary(&[("Notes", 5), ("Pinned", 2)]);
        assert!(summary.contains("Notes: 5"));
        assert!(summary.contains("Pinned: 2"));
    }

    #[test]
    fn test_pipe_row() {
        let row = OutputFormatter::pipe_row(&["a", "b", "c"]);
        assert_eq!(row, "a|b|c");
    }
}
