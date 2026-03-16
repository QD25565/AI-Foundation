//! Firebase Crashlytics API client
//!
//! Access crash reports, issues, and trends from Firebase Crashlytics.
//! This is the PRIMARY use case for MyApp debugging.

use crate::client::FirebaseClient;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Crashlytics API client
pub struct CrashlyticsClient {
    client: Arc<FirebaseClient>,
}

/// Crash issue summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashIssue {
    /// Issue ID
    pub name: String,
    /// Issue title/summary
    pub title: String,
    /// Issue subtitle (often stack trace snippet)
    pub subtitle: String,
    /// App version affected
    #[serde(rename = "appVersion")]
    pub app_version: Option<String>,
    /// Number of events/crashes
    #[serde(rename = "eventCount")]
    pub event_count: Option<i64>,
    /// Number of affected users
    #[serde(rename = "userCount")]
    pub user_count: Option<i64>,
    /// Issue state (OPEN, CLOSED, etc.)
    pub state: Option<String>,
    /// First seen timestamp
    #[serde(rename = "firstSeenTime")]
    pub first_seen: Option<String>,
    /// Last seen timestamp
    #[serde(rename = "lastSeenTime")]
    pub last_seen: Option<String>,
    /// Issue type (CRASH, NON_FATAL, ANR)
    #[serde(rename = "issueType")]
    pub issue_type: Option<String>,
}

/// Crash event details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashEvent {
    /// Event name/ID
    pub name: String,
    /// Event timestamp
    #[serde(rename = "createTime")]
    pub create_time: Option<String>,
    /// Device info
    pub device: Option<DeviceInfo>,
    /// Stack trace frames
    #[serde(rename = "stackTrace")]
    pub stack_trace: Option<StackTrace>,
    /// App version
    #[serde(rename = "appVersion")]
    pub app_version: Option<String>,
    /// OS version
    #[serde(rename = "osVersion")]
    pub os_version: Option<String>,
}

/// Device information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Device model
    pub model: Option<String>,
    /// Device manufacturer
    pub manufacturer: Option<String>,
    /// Device architecture
    pub architecture: Option<String>,
    /// RAM in bytes
    #[serde(rename = "ramMb")]
    pub ram_mb: Option<i64>,
    /// Disk space
    #[serde(rename = "diskSpaceFree")]
    pub disk_space_free: Option<i64>,
}

/// Stack trace information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackTrace {
    /// Exception type
    #[serde(rename = "exceptionType")]
    pub exception_type: Option<String>,
    /// Exception message
    #[serde(rename = "exceptionMessage")]
    pub exception_message: Option<String>,
    /// Stack frames
    pub frames: Option<Vec<StackFrame>>,
}

/// Single stack frame
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackFrame {
    /// Symbol/method name
    pub symbol: Option<String>,
    /// File name
    pub file: Option<String>,
    /// Line number
    pub line: Option<i64>,
    /// Library/module
    pub library: Option<String>,
    /// Is blamed frame
    pub blamed: Option<bool>,
}

/// List issues response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ListIssuesResponse {
    issues: Option<Vec<CrashIssue>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

/// List events response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ListEventsResponse {
    events: Option<Vec<CrashEvent>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

impl CrashlyticsClient {
    /// Create new Crashlytics client
    pub fn new(client: Arc<FirebaseClient>) -> Self {
        Self { client }
    }

    /// List crash issues (most recent first)
    ///
    /// # Arguments
    /// * `limit` - Maximum number of issues to return
    /// * `app_id` - Optional Android app ID (e.g., "1:123456789:android:abc123")
    pub async fn list_issues(&self, limit: usize, app_id: Option<&str>) -> Result<Vec<CrashIssue>> {
        // Note: Firebase Crashlytics REST API requires app_id
        // We'll use the Firebase Data API format
        let app_path = if let Some(app) = app_id {
            format!("apps/{}", app)
        } else {
            // Try to get first app
            "apps/-".to_string()
        };

        let url = format!(
            "{}?pageSize={}",
            self.client.api_url("crashlytics", &format!("{}/issues", app_path)),
            limit
        );

        let response: ListIssuesResponse = self.client.get_json(&url).await?;
        Ok(response.issues.unwrap_or_default())
    }

    /// Get specific issue details
    pub async fn get_issue(&self, issue_id: &str) -> Result<CrashIssue> {
        let url = self.client.api_url("crashlytics", &format!("issues/{}", issue_id));
        self.client.get_json(&url).await
    }

    /// List events for an issue (individual crash occurrences)
    pub async fn list_events(&self, issue_id: &str, limit: usize) -> Result<Vec<CrashEvent>> {
        let url = format!(
            "{}?pageSize={}",
            self.client.api_url("crashlytics", &format!("issues/{}/events", issue_id)),
            limit
        );

        let response: ListEventsResponse = self.client.get_json(&url).await?;
        Ok(response.events.unwrap_or_default())
    }

    /// Search issues by title/subtitle text
    pub async fn search_issues(&self, query: &str, limit: usize) -> Result<Vec<CrashIssue>> {
        // Get all issues and filter client-side
        // (Crashlytics API doesn't have server-side search)
        let issues = self.list_issues(100, None).await?;

        let query_lower = query.to_lowercase();
        let filtered: Vec<_> = issues
            .into_iter()
            .filter(|issue| {
                issue.title.to_lowercase().contains(&query_lower)
                    || issue.subtitle.to_lowercase().contains(&query_lower)
            })
            .take(limit)
            .collect();

        Ok(filtered)
    }

    /// Get crash trends summary
    pub async fn get_trends(&self, app_id: Option<&str>) -> Result<CrashTrends> {
        let issues = self.list_issues(50, app_id).await?;

        let total_crashes: i64 = issues.iter()
            .filter_map(|i| i.event_count)
            .sum();

        let total_users: i64 = issues.iter()
            .filter_map(|i| i.user_count)
            .sum();

        let open_issues = issues.iter()
            .filter(|i| i.state.as_deref() == Some("OPEN"))
            .count();

        let top_issues: Vec<_> = issues.into_iter().take(5).collect();

        Ok(CrashTrends {
            total_crashes,
            total_affected_users: total_users,
            open_issues,
            top_issues,
        })
    }
}

/// Crash trends summary
#[derive(Debug, Clone, Serialize)]
pub struct CrashTrends {
    pub total_crashes: i64,
    pub total_affected_users: i64,
    pub open_issues: usize,
    pub top_issues: Vec<CrashIssue>,
}

impl std::fmt::Display for CrashIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Issue: {}", self.title)?;
        writeln!(f, "  Type: {}", self.issue_type.as_deref().unwrap_or("UNKNOWN"))?;
        writeln!(f, "  State: {}", self.state.as_deref().unwrap_or("UNKNOWN"))?;
        if let Some(count) = self.event_count {
            writeln!(f, "  Events: {}", count)?;
        }
        if let Some(users) = self.user_count {
            writeln!(f, "  Users: {}", users)?;
        }
        if let Some(ref version) = self.app_version {
            writeln!(f, "  Version: {}", version)?;
        }
        writeln!(f, "  Detail: {}", self.subtitle)?;
        Ok(())
    }
}

impl std::fmt::Display for CrashEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Event: {}", self.name)?;
        if let Some(ref time) = self.create_time {
            writeln!(f, "  Time: {}", time)?;
        }
        if let Some(ref device) = self.device {
            if let Some(ref model) = device.model {
                writeln!(f, "  Device: {} {}",
                    device.manufacturer.as_deref().unwrap_or(""),
                    model
                )?;
            }
        }
        if let Some(ref version) = self.app_version {
            writeln!(f, "  App Version: {}", version)?;
        }
        if let Some(ref os) = self.os_version {
            writeln!(f, "  OS: {}", os)?;
        }
        if let Some(ref trace) = self.stack_trace {
            if let Some(ref exc_type) = trace.exception_type {
                writeln!(f, "  Exception: {}", exc_type)?;
            }
            if let Some(ref msg) = trace.exception_message {
                writeln!(f, "  Message: {}", msg)?;
            }
            if let Some(ref frames) = trace.frames {
                writeln!(f, "  Stack:")?;
                for frame in frames.iter().take(10) {
                    let symbol = frame.symbol.as_deref().unwrap_or("<unknown>");
                    let file = frame.file.as_deref().unwrap_or("");
                    let line = frame.line.unwrap_or(0);
                    let blamed = if frame.blamed.unwrap_or(false) { " [*]" } else { "" };
                    writeln!(f, "    {} {}:{}{}", symbol, file, line, blamed)?;
                }
            }
        }
        Ok(())
    }
}
