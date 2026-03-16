//! Google Play Developer Reporting API client
//!
//! Access crash reports, ANRs, and error data from Google Play Console.
//! This provides ACTUAL working REST API access unlike Firebase Crashlytics.
//!
//! Docs: https://developers.google.com/play/developer/reporting

use crate::client::FirebaseClient;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Play Vitals API client for crash/ANR reporting
pub struct PlayVitalsClient {
    client: Arc<FirebaseClient>,
    /// App package name (e.g., "com.myapp")
    package_name: String,
}

/// Error issue type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorType {
    /// Application crash
    Crash,
    /// Application Not Responding
    Anr,
    /// Non-fatal error (logged exception)
    NonFatal,
    /// Unknown type
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for ErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorType::Crash => write!(f, "CRASH"),
            ErrorType::Anr => write!(f, "ANR"),
            ErrorType::NonFatal => write!(f, "NON_FATAL"),
            ErrorType::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

/// App version info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppVersion {
    /// Version code (e.g., 123)
    #[serde(rename = "versionCode")]
    pub version_code: Option<String>,
}

/// OS version info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsVersion {
    /// API level (e.g., 33)
    #[serde(rename = "apiLevel")]
    pub api_level: Option<String>,
}

/// Decimal representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decimal {
    pub value: Option<String>,
}

/// Issue annotation (insights from Google)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueAnnotation {
    pub category: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
}

/// Error issue from Play Console
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorIssue {
    /// Resource name (e.g., "apps/com.example/errorIssues/abc123")
    pub name: String,

    /// Error type (CRASH, ANR, NON_FATAL)
    #[serde(rename = "type")]
    pub error_type: Option<ErrorType>,

    /// Root cause (exception class/type)
    pub cause: Option<String>,

    /// Location where error occurred
    pub location: Option<String>,

    /// Total error report count
    #[serde(rename = "errorReportCount")]
    pub error_report_count: Option<String>,

    /// Number of distinct affected users
    #[serde(rename = "distinctUsers")]
    pub distinct_users: Option<String>,

    /// Percentage of users affected
    #[serde(rename = "distinctUsersPercent")]
    pub distinct_users_percent: Option<Decimal>,

    /// Last occurrence time
    #[serde(rename = "lastErrorReportTime")]
    pub last_error_report_time: Option<String>,

    /// Link to Play Console
    #[serde(rename = "issueUri")]
    pub issue_uri: Option<String>,

    /// First OS version where issue appeared
    #[serde(rename = "firstOsVersion")]
    pub first_os_version: Option<OsVersion>,

    /// Last OS version where issue appeared
    #[serde(rename = "lastOsVersion")]
    pub last_os_version: Option<OsVersion>,

    /// First app version where issue appeared
    #[serde(rename = "firstAppVersion")]
    pub first_app_version: Option<AppVersion>,

    /// Last app version where issue appeared
    #[serde(rename = "lastAppVersion")]
    pub last_app_version: Option<AppVersion>,

    /// Google's automated insights
    pub annotations: Option<Vec<IssueAnnotation>>,

    /// Sample error report resource names
    #[serde(rename = "sampleErrorReports")]
    pub sample_error_reports: Option<Vec<String>>,
}

/// Error report with stack trace details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorReport {
    /// Resource name
    pub name: String,

    /// Report type
    #[serde(rename = "type")]
    pub report_type: Option<ErrorType>,

    /// When the error occurred
    #[serde(rename = "reportTime")]
    pub report_time: Option<String>,

    /// Device info
    #[serde(rename = "deviceModel")]
    pub device_model: Option<DeviceModelSummary>,

    /// OS version
    #[serde(rename = "osVersion")]
    pub os_version: Option<String>,

    /// App version code
    #[serde(rename = "versionCode")]
    pub version_code: Option<String>,

    /// Issue this report belongs to
    pub issue: Option<String>,
}

/// Device model summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceModelSummary {
    /// Device codename
    pub device: Option<String>,
    /// Device ID
    #[serde(rename = "deviceId")]
    pub device_id: Option<String>,
    /// URI for device details
    #[serde(rename = "deviceUri")]
    pub device_uri: Option<String>,
}

/// Search response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SearchErrorIssuesResponse {
    #[serde(rename = "errorIssues")]
    error_issues: Option<Vec<ErrorIssue>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

impl PlayVitalsClient {
    /// Create new Play Vitals client
    ///
    /// # Arguments
    /// * `client` - Firebase client for auth
    /// * `package_name` - Android package (e.g., "com.myapp")
    pub fn new(client: Arc<FirebaseClient>, package_name: String) -> Self {
        Self { client, package_name }
    }

    /// Build API URL for Play Developer Reporting
    fn api_url(&self, path: &str) -> String {
        format!(
            "https://playdeveloperreporting.googleapis.com/v1alpha1/apps/{}/{}",
            self.package_name, path
        )
    }

    /// Search error issues (crashes, ANRs, non-fatal)
    ///
    /// # Arguments
    /// * `error_type` - Optional filter: "CRASH", "ANR", or "NON_FATAL"
    /// * `limit` - Maximum results (default 50, max 1000)
    /// * `include_sample` - Include one sample error report per issue
    pub async fn search_issues(
        &self,
        error_type: Option<&str>,
        limit: usize,
        include_sample: bool,
    ) -> Result<Vec<ErrorIssue>> {
        let mut url = format!(
            "{}?pageSize={}",
            self.api_url("errorIssues:search"),
            limit.min(1000)
        );

        // Add filter for error type
        if let Some(etype) = error_type {
            url.push_str(&format!("&filter=errorIssueType%3D{}", etype));
        }

        // Include sample reports
        if include_sample {
            url.push_str("&sampleErrorReportLimit=1");
        }

        // Order by most reports first
        url.push_str("&orderBy=errorReportCount%20desc");

        let response: SearchErrorIssuesResponse = self.client.get_json(&url).await?;
        Ok(response.error_issues.unwrap_or_default())
    }

    /// List recent crashes
    pub async fn list_crashes(&self, limit: usize) -> Result<Vec<ErrorIssue>> {
        self.search_issues(Some("CRASH"), limit, true).await
    }

    /// List recent ANRs
    pub async fn list_anrs(&self, limit: usize) -> Result<Vec<ErrorIssue>> {
        self.search_issues(Some("ANR"), limit, true).await
    }

    /// List all error issues (crashes + ANRs + non-fatal)
    pub async fn list_all(&self, limit: usize) -> Result<Vec<ErrorIssue>> {
        self.search_issues(None, limit, true).await
    }

    /// Search issues by cause/location text
    pub async fn search_by_text(&self, query: &str, limit: usize) -> Result<Vec<ErrorIssue>> {
        // Get all issues and filter client-side
        // (API doesn't support text search directly)
        let issues = self.search_issues(None, 500, true).await?;

        let query_lower = query.to_lowercase();
        let filtered: Vec<_> = issues
            .into_iter()
            .filter(|issue| {
                issue.cause.as_ref().map_or(false, |c| c.to_lowercase().contains(&query_lower))
                    || issue.location.as_ref().map_or(false, |l| l.to_lowercase().contains(&query_lower))
            })
            .take(limit)
            .collect();

        Ok(filtered)
    }

    /// Get crash summary statistics
    pub async fn get_crash_summary(&self) -> Result<CrashSummary> {
        let crashes = self.list_crashes(100).await?;
        let anrs = self.list_anrs(100).await?;

        let total_crash_reports: i64 = crashes.iter()
            .filter_map(|i| i.error_report_count.as_ref()?.parse::<i64>().ok())
            .sum();

        let total_anr_reports: i64 = anrs.iter()
            .filter_map(|i| i.error_report_count.as_ref()?.parse::<i64>().ok())
            .sum();

        let total_crash_users: i64 = crashes.iter()
            .filter_map(|i| i.distinct_users.as_ref()?.parse::<i64>().ok())
            .sum();

        let total_anr_users: i64 = anrs.iter()
            .filter_map(|i| i.distinct_users.as_ref()?.parse::<i64>().ok())
            .sum();

        Ok(CrashSummary {
            crash_issue_count: crashes.len(),
            anr_issue_count: anrs.len(),
            total_crash_reports,
            total_anr_reports,
            total_crash_users,
            total_anr_users,
            top_crashes: crashes.into_iter().take(5).collect(),
            top_anrs: anrs.into_iter().take(3).collect(),
        })
    }
}

/// Crash summary statistics
#[derive(Debug, Clone, Serialize)]
pub struct CrashSummary {
    pub crash_issue_count: usize,
    pub anr_issue_count: usize,
    pub total_crash_reports: i64,
    pub total_anr_reports: i64,
    pub total_crash_users: i64,
    pub total_anr_users: i64,
    pub top_crashes: Vec<ErrorIssue>,
    pub top_anrs: Vec<ErrorIssue>,
}

impl std::fmt::Display for ErrorIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let etype = self.error_type.as_ref().map_or("UNKNOWN".to_string(), |t| t.to_string());
        writeln!(f, "{}: {}", etype, self.cause.as_deref().unwrap_or("<unknown>"))?;

        if let Some(loc) = &self.location {
            writeln!(f, "  Location: {}", loc)?;
        }

        if let Some(count) = &self.error_report_count {
            write!(f, "  Reports: {}", count)?;
        }
        if let Some(users) = &self.distinct_users {
            writeln!(f, "  Users: {}", users)?;
        } else {
            writeln!(f)?;
        }

        if let Some(last) = &self.last_error_report_time {
            writeln!(f, "  Last: {}", last)?;
        }

        if let Some(ver) = &self.last_app_version {
            if let Some(code) = &ver.version_code {
                writeln!(f, "  Version: {}", code)?;
            }
        }

        if let Some(uri) = &self.issue_uri {
            writeln!(f, "  Console: {}", uri)?;
        }

        Ok(())
    }
}
