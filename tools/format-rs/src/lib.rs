//! Token-Efficient Formatting for AI Context
//!
//! Reduces token usage in AI conversations by:
//! 1. Compact headers: |HEADER| instead of underlined headers
//! 2. Time deduplication: Only show time when minute changes
//! 3. AI name abbreviation: "cascade-230" → "Ca"
//! 4. Efficient data presentation

use chrono::{DateTime, Utc};
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::Mutex;

lazy_static! {
    /// Last formatted time (for deduplication)
    static ref LAST_TIME: Mutex<Option<String>> = Mutex::new(None);
    
    /// AI name abbreviation cache
    static ref AI_ABBREV_CACHE: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
}

/// Format header with minimal tokens
///
/// Before: "THIS IS A HEADER\n________________\n" (30+ tokens)
/// After: "|THIS IS A HEADER|\n" (5-6 tokens)
///
/// # Example
/// ```
/// use format_rs::format_header;
/// let header = format_header("TEAM ACTIVITY");
/// assert_eq!(header, "|TEAM ACTIVITY|");
/// ```
pub fn format_header(text: &str) -> String {
    format!("|{}|", text.to_uppercase())
}

/// Format subheader with minimal tokens
///
/// For nested sections, uses single pipe
///
/// # Example
/// ```
/// use format_rs::format_subheader;
/// let subheader = format_subheader("Recent Actions");
/// assert_eq!(subheader, "|Recent Actions|");
/// ```
pub fn format_subheader(text: &str) -> String {
    format!("|{}|", text)
}

/// Format time with deduplication
///
/// Only shows time if minute has changed since last call.
/// Reduces "2025-11-21 14:30 UTC" spam from 5-40 repetitions to 1.
///
/// # Example
/// ```
/// use format_rs::format_time_dedupe;
/// use chrono::Utc;
///
/// let time1 = format_time_dedupe(&Utc::now());
/// assert!(time1.is_some()); // First call always returns Some
///
/// let time2 = format_time_dedupe(&Utc::now()); // Same minute
/// assert!(time2.is_none()); // Deduplicated
/// ```
pub fn format_time_dedupe(time: &DateTime<Utc>) -> Option<String> {
    let formatted = time.format("%H:%M").to_string();
    
    let mut last_time = LAST_TIME.lock().unwrap();
    
    if let Some(ref last) = *last_time {
        if last == &formatted {
            return None; // Same minute, don't repeat
        }
    }
    
    *last_time = Some(formatted.clone());
    Some(formatted)
}

/// Format time compactly (always show, but minimal)
///
/// Shows HH:MM without date or timezone spam
///
/// # Example
/// ```
/// use format_rs::format_time_compact;
/// use chrono::Utc;
/// let time = format_time_compact(&Utc::now());
/// assert_eq!(time.len(), 5); // "HH:MM" format
/// ```
pub fn format_time_compact(time: &DateTime<Utc>) -> String {
    time.format("%H:%M").to_string()
}

/// Format time with date when needed
///
/// Shows date only if not today
///
/// # Example
/// ```
/// use format_rs::format_time_smart;
/// use chrono::Utc;
/// let time = format_time_smart(&Utc::now());
/// assert_eq!(time.len(), 5); // "HH:MM" for today
/// ```
pub fn format_time_smart(time: &DateTime<Utc>) -> String {
    let now = Utc::now();
    
    if time.date_naive() == now.date_naive() {
        // Today: just time
        time.format("%H:%M").to_string()
    } else {
        // Other day: abbreviated date + time
        time.format("%b%d %H:%M").to_string()
    }
}

/// Abbreviate AI name for token efficiency
///
/// Converts full AI ID to 2-letter abbreviation using first letter
/// of each component (or first 2 letters if single component)
///
/// # Examples
/// ```
/// use format_rs::abbreviate_ai;
/// assert_eq!(abbreviate_ai("cascade-230"), "Ca");
/// assert_eq!(abbreviate_ai("sage-724"), "Sa");
/// assert_eq!(abbreviate_ai("resonance-684"), "Re");
/// assert_eq!(abbreviate_ai("lyra-584"), "Ly");
/// assert_eq!(abbreviate_ai("nova"), "No");
/// ```
pub fn abbreviate_ai(ai_id: &str) -> String {
    // Check cache first
    {
        let cache = AI_ABBREV_CACHE.lock().unwrap();
        if let Some(abbrev) = cache.get(ai_id) {
            return abbrev.clone();
        }
    }
    
    // Generate abbreviation
    let abbrev = if ai_id.contains('-') {
        // Multi-component: take first letter of first component
        let first_component = ai_id.split('-').next().unwrap_or(ai_id);
        let first_char = first_component.chars().next().unwrap_or('?');
        format!("{}{}", 
            first_char.to_uppercase(),
            first_component.chars().nth(1).unwrap_or(first_char).to_lowercase()
        )
    } else {
        // Single component: take first 2 letters
        let mut chars = ai_id.chars();
        format!("{}{}",
            chars.next().unwrap_or('?').to_uppercase(),
            chars.next().unwrap_or('?').to_lowercase()
        )
    };
    
    // Cache it
    {
        let mut cache = AI_ABBREV_CACHE.lock().unwrap();
        cache.insert(ai_id.to_string(), abbrev.clone());
    }
    
    abbrev
}

/// Format AI list compactly
///
/// Before: "cascade-230, sage-724, resonance-684" (30 tokens)
/// After: "Ca Sa Re" (3 tokens)
///
/// # Example
/// ```
/// use format_rs::format_ai_list;
/// let ais = vec!["cascade-230", "sage-724", "resonance-684"];
/// let compact = format_ai_list(&ais);
/// assert_eq!(compact, "Ca Sa Re");
/// ```
pub fn format_ai_list(ai_ids: &[&str]) -> String {
    ai_ids.iter()
        .map(|id| abbreviate_ai(id))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Format key-value pair compactly
///
/// Before: "Status: Active\n" (3 tokens)
/// After: "Status:Active\n" (2 tokens)
///
/// Removes space after colon for common fields
///
/// # Example
/// ```
/// use format_rs::format_kv;
/// let kv = format_kv("Status", "Active");
/// assert_eq!(kv, "Status:Active");
/// ```
pub fn format_kv(key: &str, value: &str) -> String {
    format!("{}:{}", key, value)
}

/// Format list item compactly
///
/// Before: "  - Item here\n" (4 tokens)
/// After: "•Item here\n" (2 tokens)
///
/// # Example
/// ```
/// use format_rs::format_list_item;
/// let item = format_list_item("Team activity");
/// assert_eq!(item, "•Team activity");
/// ```
pub fn format_list_item(text: &str) -> String {
    format!("•{}", text)
}

/// Format section with header and content
///
/// # Example
/// ```
/// use format_rs::format_section;
/// let section = format_section("TEAM STATUS", vec![
///     "Ca:working",
///     "Sa:reviewing",
///     "Re:testing"
/// ]);
/// assert!(section.contains("|TEAM STATUS|"));
/// assert!(section.contains("Ca:working"));
/// ```
pub fn format_section(header: &str, lines: Vec<&str>) -> String {
    let mut output = format!("{}\n", format_header(header));
    for line in lines {
        output.push_str(line);
        output.push('\n');
    }
    output
}

/// Python bindings using PyO3
#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
#[pyfunction]
fn py_format_header(text: &str) -> String {
    format_header(text)
}

#[cfg(feature = "python")]
#[pyfunction]
fn py_format_subheader(text: &str) -> String {
    format_subheader(text)
}

#[cfg(feature = "python")]
#[pyfunction]
fn py_format_time_compact(timestamp: &str) -> PyResult<String> {
    // Parse ISO 8601 timestamp
    let time = DateTime::parse_from_rfc3339(timestamp)
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("Invalid timestamp: {}", e)))?;
    Ok(format_time_compact(&time.with_timezone(&Utc)))
}

#[cfg(feature = "python")]
#[pyfunction]
fn py_abbreviate_ai(ai_id: &str) -> String {
    abbreviate_ai(ai_id)
}

#[cfg(feature = "python")]
#[pyfunction]
fn py_format_ai_list(ai_ids: Vec<String>) -> String {
    let refs: Vec<&str> = ai_ids.iter().map(|s| s.as_str()).collect();
    format_ai_list(&refs)
}

#[cfg(feature = "python")]
#[pyfunction]
fn py_format_kv(key: &str, value: &str) -> String {
    format_kv(key, value)
}

#[cfg(feature = "python")]
#[pyfunction]
fn py_format_list_item(text: &str) -> String {
    format_list_item(text)
}

#[cfg(feature = "python")]
#[pyfunction]
fn py_format_section(header: &str, lines: Vec<String>) -> String {
    let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    format_section(header, refs)
}

#[cfg(feature = "python")]
#[pymodule]
fn format_rs(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(py_format_header, m)?)?;
    m.add_function(wrap_pyfunction!(py_format_subheader, m)?)?;
    m.add_function(wrap_pyfunction!(py_format_time_compact, m)?)?;
    m.add_function(wrap_pyfunction!(py_abbreviate_ai, m)?)?;
    m.add_function(wrap_pyfunction!(py_format_ai_list, m)?)?;
    m.add_function(wrap_pyfunction!(py_format_kv, m)?)?;
    m.add_function(wrap_pyfunction!(py_format_list_item, m)?)?;
    m.add_function(wrap_pyfunction!(py_format_section, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_header() {
        assert_eq!(format_header("test"), "|TEST|");
        assert_eq!(format_header("Team Activity"), "|TEAM ACTIVITY|");
    }

    #[test]
    fn test_abbreviate_ai() {
        assert_eq!(abbreviate_ai("cascade-230"), "Ca");
        assert_eq!(abbreviate_ai("sage-724"), "Sa");
        assert_eq!(abbreviate_ai("resonance-684"), "Re");
        assert_eq!(abbreviate_ai("lyra-584"), "Ly");
        assert_eq!(abbreviate_ai("nova"), "No");
    }

    #[test]
    fn test_format_ai_list() {
        let ais = vec!["cascade-230", "sage-724", "resonance-684"];
        assert_eq!(format_ai_list(&ais), "Ca Sa Re");
    }

    #[test]
    fn test_format_kv() {
        assert_eq!(format_kv("Status", "Active"), "Status:Active");
    }

    #[test]
    fn test_format_list_item() {
        assert_eq!(format_list_item("Item"), "•Item");
    }
}
