//! Firebase CLI for AI-Foundation
//!
//! Command-line interface for Firebase services:
//! - Crashlytics: View and search crash reports
//! - Firestore: Read documents and run queries
//! - Auth: Look up user information
//!
//! Part of AI-Foundation - True AI Empowerment

use clap::{Parser, Subcommand};
use colored::*;
use firebase_rs::{
    auth::FirebaseAuth,
    client::FirebaseClient,
    crashlytics::CrashlyticsClient,
    firestore::FirestoreClient,
    play_vitals::PlayVitalsClient,
    Result,
};
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "firebase")]
#[command(author = "AI-Foundation Team")]
#[command(version = "0.1.0")]
#[command(about = "Firebase CLI for AI-Foundation - Access Crashlytics, Firestore, Auth")]
#[command(long_about = None)]
struct Cli {
    /// Path to service account JSON file
    #[arg(short = 'k', long, env = "GOOGLE_APPLICATION_CREDENTIALS")]
    credentials: Option<String>,

    /// Firebase project ID (overrides service account)
    #[arg(short = 'p', long, env = "FIREBASE_PROJECT_ID")]
    project: Option<String>,

    /// Output format: text, json
    #[arg(short = 'o', long, default_value = "text")]
    output: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Crashlytics operations - view crash reports
    #[command(alias = "crash")]
    Crashlytics {
        #[command(subcommand)]
        action: CrashlyticsAction,
    },

    /// Firestore operations - read documents
    #[command(alias = "fs")]
    Firestore {
        #[command(subcommand)]
        action: FirestoreAction,
    },

    /// Authentication - check status
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },

    /// List apps in the Firebase project
    Apps,

    /// Google Play Developer Reporting - crashes, ANRs from Play Console
    #[command(alias = "play")]
    Vitals {
        /// Android package name
        #[arg(short = 'a', long, env = "ANDROID_PACKAGE_NAME")]
        package: String,

        #[command(subcommand)]
        action: VitalsAction,
    },

    /// Show current configuration
    Status,
}

#[derive(Subcommand)]
enum CrashlyticsAction {
    /// List recent crash issues
    #[command(alias = "ls")]
    List {
        /// Maximum number of issues to show
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,

        /// App ID to filter by
        #[arg(short = 'a', long)]
        app: Option<String>,
    },

    /// Get details for a specific issue
    Get {
        /// Issue ID
        issue_id: String,

        /// Show stack traces
        #[arg(short = 's', long)]
        stacks: bool,
    },

    /// Search issues by text
    Search {
        /// Search query
        query: String,

        /// Maximum results
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,
    },

    /// Show crash trends summary
    Trends {
        /// App ID
        #[arg(short = 'a', long)]
        app: Option<String>,
    },

    /// List events for an issue
    Events {
        /// Issue ID
        issue_id: String,

        /// Maximum events
        #[arg(short = 'n', long, default_value = "5")]
        limit: usize,
    },
}

#[derive(Subcommand)]
enum FirestoreAction {
    /// Get a document by path
    Get {
        /// Document path (e.g., users/abc123)
        path: String,
    },

    /// List documents in a collection
    #[command(alias = "ls")]
    List {
        /// Collection path
        collection: String,

        /// Maximum documents
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,
    },

    /// Query documents with a filter
    Query {
        /// Collection name
        collection: String,

        /// Field to filter on
        #[arg(short = 'f', long)]
        field: String,

        /// Operator: EQUAL, NOT_EQUAL, LESS_THAN, GREATER_THAN, etc.
        #[arg(short = 'o', long, default_value = "EQUAL")]
        op: String,

        /// Value to compare
        #[arg(short = 'v', long)]
        value: String,

        /// Maximum results
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,
    },
}

#[derive(Subcommand)]
enum AuthAction {
    /// Verify authentication is working
    Check,

    /// Show service account info
    Info,
}

#[derive(Subcommand)]
enum VitalsAction {
    /// List recent crashes from Play Console
    Crashes {
        /// Maximum issues to show
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,
    },

    /// List recent ANRs from Play Console
    Anrs {
        /// Maximum issues to show
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,
    },

    /// List all error issues (crashes + ANRs + non-fatal)
    All {
        /// Maximum issues to show
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,
    },

    /// Search issues by text in cause/location
    Search {
        /// Search query
        query: String,

        /// Maximum results
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,
    },

    /// Show crash/ANR summary statistics
    Summary,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{} {}", "Error:".red().bold(), e);
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    // Try to load credentials
    let auth = if let Some(ref creds_path) = cli.credentials {
        FirebaseAuth::from_file(creds_path)?
    } else {
        FirebaseAuth::from_env()?
    };

    let client = Arc::new(FirebaseClient::new(auth));

    match cli.command {
        Commands::Crashlytics { action } => {
            handle_crashlytics(client, action, &cli.output).await?;
        }
        Commands::Firestore { action } => {
            handle_firestore(client, action, &cli.output).await?;
        }
        Commands::Auth { action } => {
            handle_auth(client, action).await?;
        }
        Commands::Apps => {
            handle_apps(client).await?;
        }
        Commands::Vitals { package, action } => {
            handle_vitals(client, package, action, &cli.output).await?;
        }
        Commands::Status => {
            println!("{}", "|STATUS|".cyan().bold());
            println!("Project:{}", client.project_id());
            println!("Auth:OK");
        }
    }

    Ok(())
}

async fn handle_apps(client: Arc<FirebaseClient>) -> Result<()> {    println!("{}", "|FIREBASE APPS|".cyan().bold());    println!("Project:{}", client.project_id());    let apps = client.list_android_apps().await?;    if apps.is_empty() {        println!("  No Android apps found");    } else {        println!("
|ANDROID APPS|{}", apps.len());        for app in apps {            println!("  AppID:{}", app.app_id);            if let Some(ref name) = app.display_name {                println!("    Name:{}", name);            }            if let Some(ref pkg) = app.package_name {                println!("    Package:{}", pkg);            }        }    }    Ok(())}
async fn handle_crashlytics(
    client: Arc<FirebaseClient>,
    action: CrashlyticsAction,
    output: &str,
) -> Result<()> {
    let crashlytics = CrashlyticsClient::new(client);

    match action {
        CrashlyticsAction::List { limit, app } => {
            let issues = crashlytics.list_issues(limit, app.as_deref()).await?;

            if output == "json" {
                println!("{}", serde_json::to_string_pretty(&issues)?);
            } else {
                println!("{}", format!("|CRASH ISSUES|{}", issues.len()).cyan().bold());
                if issues.is_empty() {
                    println!("  No crash issues found");
                }
                for issue in issues {
                    print_issue(&issue);
                }
            }
        }

        CrashlyticsAction::Get { issue_id, stacks } => {
            let issue = crashlytics.get_issue(&issue_id).await?;

            if output == "json" {
                println!("{}", serde_json::to_string_pretty(&issue)?);
            } else {
                println!("{}", "|ISSUE DETAILS|".cyan().bold());
                print_issue(&issue);

                if stacks {
                    let events = crashlytics.list_events(&issue_id, 3).await?;
                    println!("\n{}", "|STACK TRACES|".cyan().bold());
                    for event in events {
                        println!("{}", event);
                    }
                }
            }
        }

        CrashlyticsAction::Search { query, limit } => {
            let issues = crashlytics.search_issues(&query, limit).await?;

            if output == "json" {
                println!("{}", serde_json::to_string_pretty(&issues)?);
            } else {
                println!("{}", format!("|SEARCH RESULTS|{}", issues.len()).cyan().bold());
                println!("Query:{}", query);
                if issues.is_empty() {
                    println!("  No matching issues found");
                }
                for issue in issues {
                    print_issue(&issue);
                }
            }
        }

        CrashlyticsAction::Trends { app } => {
            let trends = crashlytics.get_trends(app.as_deref()).await?;

            if output == "json" {
                println!("{}", serde_json::to_string_pretty(&trends)?);
            } else {
                println!("{}", "|CRASH TRENDS|".cyan().bold());
                println!("TotalCrashes:{}", trends.total_crashes);
                println!("AffectedUsers:{}", trends.total_affected_users);
                println!("OpenIssues:{}", trends.open_issues);
                println!("\n{}", "|TOP ISSUES|".cyan().bold());
                for issue in &trends.top_issues {
                    print_issue_brief(issue);
                }
            }
        }

        CrashlyticsAction::Events { issue_id, limit } => {
            let events = crashlytics.list_events(&issue_id, limit).await?;

            if output == "json" {
                println!("{}", serde_json::to_string_pretty(&events)?);
            } else {
                println!("{}", format!("|CRASH EVENTS|{}", events.len()).cyan().bold());
                for event in events {
                    println!("{}", event);
                }
            }
        }
    }

    Ok(())
}

async fn handle_firestore(
    client: Arc<FirebaseClient>,
    action: FirestoreAction,
    output: &str,
) -> Result<()> {
    let firestore = FirestoreClient::new(client);

    match action {
        FirestoreAction::Get { path } => {
            let doc = firestore.get_document(&path).await?;

            if output == "json" {
                println!("{}", doc);
            } else {
                println!("{}", "|DOCUMENT|".cyan().bold());
                println!("Path:{}", doc.name);
                println!("{}", doc);
            }
        }

        FirestoreAction::List { collection, limit } => {
            let docs = firestore.list_documents(&collection, limit).await?;

            if output == "json" {
                let json: Vec<_> = docs.iter().map(|d| d.to_json()).collect();
                println!("{}", serde_json::to_string_pretty(&json)?);
            } else {
                println!("{}", format!("|DOCUMENTS|{}", docs.len()).cyan().bold());
                println!("Collection:{}", collection);
                for doc in docs {
                    println!("\n  ID:{}", doc.id());
                    // Print first few fields
                    if let Some(ref fields) = doc.fields {
                        for (key, value) in fields.iter().take(5) {
                            let simple = value.to_simple();
                            let display = match &simple {
                                serde_json::Value::String(s) if s.len() > 50 => {
                                    format!("{}...", &s[..50])
                                }
                                other => other.to_string(),
                            };
                            println!("    {}:{}", key, display);
                        }
                        if fields.len() > 5 {
                            println!("    ...and {} more fields", fields.len() - 5);
                        }
                    }
                }
            }
        }

        FirestoreAction::Query { collection, field, op, value, limit } => {
            let docs = firestore.query(&collection, &field, &op, &value, limit).await?;

            if output == "json" {
                let json: Vec<_> = docs.iter().map(|d| d.to_json()).collect();
                println!("{}", serde_json::to_string_pretty(&json)?);
            } else {
                println!("{}", format!("|QUERY RESULTS|{}", docs.len()).cyan().bold());
                println!("Collection:{} WHERE {} {} {}", collection, field, op, value);
                for doc in docs {
                    println!("\n{}", doc);
                }
            }
        }
    }

    Ok(())
}

async fn handle_auth(
    client: Arc<FirebaseClient>,
    action: AuthAction,
) -> Result<()> {
    match action {
        AuthAction::Check => {
            println!("{}", "|AUTH CHECK|".cyan().bold());
            println!("Project:{}", client.project_id());
            println!("Status:{}", "Authenticated".green());
        }
        AuthAction::Info => {
            println!("{}", "|SERVICE ACCOUNT|".cyan().bold());
            println!("Project:{}", client.project_id());
            // Note: Don't expose sensitive details
        }
    }

    Ok(())
}

fn print_issue(issue: &firebase_rs::crashlytics::CrashIssue) {
    let state_color = match issue.state.as_deref() {
        Some("OPEN") => "OPEN".red(),
        Some("CLOSED") => "CLOSED".green(),
        _ => "UNKNOWN".yellow(),
    };

    println!("  {} [{}]", issue.title.white().bold(), state_color);
    println!("    Type:{}", issue.issue_type.as_deref().unwrap_or("?"));
    if let Some(count) = issue.event_count {
        print!("    Events:{}", count);
    }
    if let Some(users) = issue.user_count {
        print!("  Users:{}", users);
    }
    println!();
    if let Some(ref version) = issue.app_version {
        println!("    Version:{}", version);
    }
    if issue.subtitle.len() > 0 {
        let subtitle = if issue.subtitle.len() > 80 {
            format!("{}...", &issue.subtitle[..80])
        } else {
            issue.subtitle.clone()
        };
        println!("    Detail:{}", subtitle.dimmed());
    }
}

fn print_issue_brief(issue: &firebase_rs::crashlytics::CrashIssue) {
    let events = issue.event_count.unwrap_or(0);
    let users = issue.user_count.unwrap_or(0);
    println!("  {} ({}events, {}users)",
        issue.title.white(),
        events,
        users
    );
}

async fn handle_vitals(
    client: Arc<FirebaseClient>,
    package: String,
    action: VitalsAction,
    output: &str,
) -> Result<()> {
    let vitals = PlayVitalsClient::new(client, package);

    match action {
        VitalsAction::Crashes { limit } => {
            let issues = vitals.list_crashes(limit).await?;

            if output == "json" {
                println!("{}", serde_json::to_string_pretty(&issues)?);
            } else {
                println!("{}", format!("|CRASHES|{}", issues.len()).cyan().bold());
                if issues.is_empty() {
                    println!("  No crashes found (or no access to Play Developer Reporting API)");
                }
                for issue in issues {
                    print_vitals_issue(&issue);
                }
            }
        }

        VitalsAction::Anrs { limit } => {
            let issues = vitals.list_anrs(limit).await?;

            if output == "json" {
                println!("{}", serde_json::to_string_pretty(&issues)?);
            } else {
                println!("{}", format!("|ANRS|{}", issues.len()).cyan().bold());
                if issues.is_empty() {
                    println!("  No ANRs found");
                }
                for issue in issues {
                    print_vitals_issue(&issue);
                }
            }
        }

        VitalsAction::All { limit } => {
            let issues = vitals.list_all(limit).await?;

            if output == "json" {
                println!("{}", serde_json::to_string_pretty(&issues)?);
            } else {
                println!("{}", format!("|ALL ERRORS|{}", issues.len()).cyan().bold());
                if issues.is_empty() {
                    println!("  No error issues found");
                }
                for issue in issues {
                    print_vitals_issue(&issue);
                }
            }
        }

        VitalsAction::Search { query, limit } => {
            let issues = vitals.search_by_text(&query, limit).await?;

            if output == "json" {
                println!("{}", serde_json::to_string_pretty(&issues)?);
            } else {
                println!("{}", format!("|SEARCH RESULTS|{}", issues.len()).cyan().bold());
                println!("Query:{}", query);
                if issues.is_empty() {
                    println!("  No matching issues found");
                }
                for issue in issues {
                    print_vitals_issue(&issue);
                }
            }
        }

        VitalsAction::Summary => {
            let summary = vitals.get_crash_summary().await?;

            if output == "json" {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                println!("{}", "|CRASH SUMMARY|".cyan().bold());
                println!("CrashIssues:{}", summary.crash_issue_count);
                println!("ANRIssues:{}", summary.anr_issue_count);
                println!("TotalCrashReports:{}", summary.total_crash_reports);
                println!("TotalANRReports:{}", summary.total_anr_reports);
                println!("CrashUsers:{}", summary.total_crash_users);
                println!("ANRUsers:{}", summary.total_anr_users);

                if !summary.top_crashes.is_empty() {
                    println!("\n{}", "|TOP CRASHES|".cyan().bold());
                    for issue in &summary.top_crashes {
                        print_vitals_issue_brief(issue);
                    }
                }

                if !summary.top_anrs.is_empty() {
                    println!("\n{}", "|TOP ANRS|".cyan().bold());
                    for issue in &summary.top_anrs {
                        print_vitals_issue_brief(issue);
                    }
                }
            }
        }
    }

    Ok(())
}

fn print_vitals_issue(issue: &firebase_rs::play_vitals::ErrorIssue) {
    let etype = issue.error_type.as_ref()
        .map(|t| t.to_string())
        .unwrap_or_else(|| "UNKNOWN".to_string());
    
    let cause = issue.cause.as_deref().unwrap_or("<unknown>");
    let cause_display = if cause.len() > 60 {
        format!("{}...", &cause[..60])
    } else {
        cause.to_string()
    };

    println!("  {} {}", etype.yellow().bold(), cause_display.white());

    if let Some(ref loc) = issue.location {
        let loc_display = if loc.len() > 70 { format!("{}...", &loc[..70]) } else { loc.clone() };
        println!("    Location:{}", loc_display.dimmed());
    }

    if let Some(ref count) = issue.error_report_count {
        print!("    Reports:{}", count);
    }
    if let Some(ref users) = issue.distinct_users {
        print!("  Users:{}", users);
    }
    println!();

    if let Some(ref ver) = issue.last_app_version {
        if let Some(ref code) = ver.version_code {
            println!("    Version:{}", code);
        }
    }

    if let Some(ref uri) = issue.issue_uri {
        println!("    Console:{}", uri.dimmed());
    }
}

fn print_vitals_issue_brief(issue: &firebase_rs::play_vitals::ErrorIssue) {
    let reports = issue.error_report_count.as_ref()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let users = issue.distinct_users.as_ref()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let cause = issue.cause.as_deref().unwrap_or("?");
    let cause_display = if cause.len() > 40 { format!("{}...", &cause[..40]) } else { cause.to_string() };
    
    println!("  {} ({}reports, {}users)", cause_display, reports, users);
}
