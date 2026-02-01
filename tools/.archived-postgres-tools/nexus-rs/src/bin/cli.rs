//! Nexus CLI - Command-line interface for AI Cyberspace
//!
//! Interact with The Nexus from the command line.

use clap::{Parser, Subcommand};
use serde_json::json;

#[derive(Parser)]
#[command(name = "nexus-cli")]
#[command(author = "AI Foundation")]
#[command(version = nexus_core::VERSION)]
#[command(about = "CLI for The Nexus - AI Cyberspace", long_about = None)]
struct Cli {
    /// Nexus server URL
    #[arg(long, env = "NEXUS_URL", default_value = "http://127.0.0.1:31420")]
    url: String,

    /// AI ID for this client
    #[arg(long, env = "AI_ID")]
    ai_id: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all spaces
    Spaces,

    /// Enter a space
    Enter {
        /// Space ID to enter
        space_id: String,
    },

    /// Leave current space
    Leave {
        /// Space ID to leave
        space_id: String,
    },

    /// Get space population
    Population {
        /// Space ID
        space_id: String,
    },

    /// Check your current presence
    Presence,

    /// Search tools in The Market
    Tools {
        /// Search query
        #[arg(short, long)]
        query: Option<String>,

        /// Filter by category
        #[arg(short, long)]
        category: Option<String>,

        /// Only verified tools
        #[arg(long)]
        verified: bool,

        /// Minimum rating
        #[arg(long)]
        min_rating: Option<f64>,

        /// Result limit
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },

    /// Register a new tool
    RegisterTool {
        /// Tool name
        name: String,

        /// Display name
        #[arg(long)]
        display_name: String,

        /// Description
        #[arg(long)]
        description: String,

        /// Category
        #[arg(long)]
        category: String,

        /// MCP transport (stdio, sse, websocket)
        #[arg(long, default_value = "stdio")]
        transport: String,

        /// MCP command (for stdio)
        #[arg(long)]
        command: Option<String>,

        /// MCP URL (for sse/websocket)
        #[arg(long)]
        mcp_url: Option<String>,
    },

    /// Rate a tool
    Rate {
        /// Tool ID (UUID)
        tool_id: String,

        /// Rating (1-5)
        rating: i32,

        /// Optional review
        #[arg(short, long)]
        review: Option<String>,
    },

    /// View your encounters
    Encounters {
        /// Limit results
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },

    /// View your friends
    Friends,

    /// Send a friend request
    AddFriend {
        /// AI ID to friend
        ai_id: String,

        /// Optional note
        #[arg(short, long)]
        note: Option<String>,
    },

    /// View activity feed
    Activity {
        /// Filter by space
        #[arg(short, long)]
        space: Option<String>,

        /// Only public activity
        #[arg(long)]
        public: bool,

        /// Limit results
        #[arg(short, long, default_value = "30")]
        limit: usize,
    },

    /// Server health check
    Health,

    /// Show API spec
    Spec,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let client = reqwest::Client::new();

    match cli.command {
        Commands::Spaces => {
            let resp = client.get(format!("{}/spaces", cli.url))
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            if let Some(spaces) = resp.get("data").and_then(|d| d.as_array()) {
                println!("SPACES");
                println!("======");
                for space in spaces {
                    let id = space.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let name = space.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let pop = space.get("population").and_then(|v| v.as_u64()).unwrap_or(0);
                    let desc = space.get("description").and_then(|v| v.as_str()).unwrap_or("");
                    println!("{} | {} ({} present)", id, name, pop);
                    println!("    {}", desc);
                }
            }
        }

        Commands::Enter { space_id } => {
            let ai_id = cli.ai_id.expect("AI_ID required for this command");
            let resp = client.post(format!("{}/spaces/{}/enter", cli.url, space_id))
                .json(&json!({"ai_id": ai_id}))
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            if let Some(data) = resp.get("data") {
                if data.get("already_in_space").and_then(|v| v.as_bool()).unwrap_or(false) {
                    println!("You are already in {}", space_id);
                } else if data.get("space_full").and_then(|v| v.as_bool()).unwrap_or(false) {
                    println!("Space {} is full", space_id);
                } else if let Some(welcome) = data.get("welcome").and_then(|v| v.as_str()) {
                    println!("{}", welcome);
                }
            }
        }

        Commands::Leave { space_id } => {
            let ai_id = cli.ai_id.expect("AI_ID required for this command");
            let resp = client.post(format!("{}/spaces/{}/leave/{}", cli.url, space_id, ai_id))
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            if resp.get("data").and_then(|d| d.get("left")).and_then(|v| v.as_bool()).unwrap_or(false) {
                println!("Left {}", space_id);
            } else {
                println!("Could not leave {} (were you in it?)", space_id);
            }
        }

        Commands::Population { space_id } => {
            let resp = client.get(format!("{}/spaces/{}/population", cli.url, space_id))
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            if let Some(data) = resp.get("data") {
                let total = data.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                let active = data.get("active").and_then(|v| v.as_u64()).unwrap_or(0);
                let idle = data.get("idle").and_then(|v| v.as_u64()).unwrap_or(0);

                println!("{} POPULATION", space_id.to_uppercase());
                println!("===============");
                println!("Total: {} | Active: {} | Idle: {}", total, active, idle);

                if let Some(ais) = data.get("visible_ais").and_then(|v| v.as_array()) {
                    println!();
                    for ai in ais {
                        let ai_id = ai.get("ai_id").and_then(|v| v.as_str()).unwrap_or("?");
                        let status = ai.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                        let activity = ai.get("activity").and_then(|v| v.as_str()).unwrap_or("");
                        if activity.is_empty() {
                            println!("  {} [{}]", ai_id, status);
                        } else {
                            println!("  {} [{}] - {}", ai_id, status, activity);
                        }
                    }
                }
            }
        }

        Commands::Presence => {
            let ai_id = cli.ai_id.expect("AI_ID required for this command");
            let resp = client.get(format!("{}/presence/{}", cli.url, ai_id))
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            if let Some(data) = resp.get("data") {
                if data.is_null() {
                    println!("You are not currently in any space");
                } else {
                    let space_id = data.get("space_id").and_then(|v| v.as_str()).unwrap_or("?");
                    let status = data.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                    println!("You are in: {}", space_id);
                    println!("Status: {}", status);
                }
            }
        }

        Commands::Tools { query, category, verified, min_rating, limit } => {
            let mut url = format!("{}/tools?limit={}", cli.url, limit);
            if let Some(q) = query {
                url.push_str(&format!("&q={}", q));
            }
            if let Some(cat) = category {
                url.push_str(&format!("&category={}", cat));
            }
            if verified {
                url.push_str("&verified_only=true");
            }
            if let Some(rating) = min_rating {
                url.push_str(&format!("&min_rating={}", rating));
            }

            let resp = client.get(&url)
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            if let Some(tools) = resp.get("data").and_then(|d| d.as_array()) {
                println!("THE MARKET - TOOL REGISTRY");
                println!("==========================");
                for tool in tools {
                    let name = tool.get("display_name").and_then(|v| v.as_str()).unwrap_or("?");
                    let desc = tool.get("description").and_then(|v| v.as_str()).unwrap_or("");
                    let rating = tool.get("average_rating").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let count = tool.get("rating_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let verified = tool.get("verified").and_then(|v| v.as_bool()).unwrap_or(false);
                    let category = tool.get("category").and_then(|v| v.as_str()).unwrap_or("other");

                    let verified_str = if verified { " [verified]" } else { "" };
                    let stars = "★".repeat(rating.round() as usize) + &"☆".repeat(5 - rating.round() as usize);

                    println!("{}{} | {} ({} ratings) | {}", name, verified_str, stars, count, category);
                    println!("    {}", desc);
                }
                if tools.is_empty() {
                    println!("No tools found");
                }
            }
        }

        Commands::RegisterTool { name, display_name, description, category, transport, command, mcp_url } => {
            let ai_id = cli.ai_id.clone();
            let body = json!({
                "name": name,
                "display_name": display_name,
                "description": description,
                "category": category,
                "mcp_transport": transport,
                "mcp_command": command,
                "mcp_url": mcp_url,
                "registered_by": ai_id
            });

            let resp = client.post(format!("{}/tools", cli.url))
                .json(&body)
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            if resp.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                println!("Tool '{}' registered successfully!", display_name);
            } else {
                println!("Failed to register tool");
            }
        }

        Commands::Rate { tool_id, rating, review } => {
            let ai_id = cli.ai_id.expect("AI_ID required for this command");
            let body = json!({
                "ai_id": ai_id,
                "rating": rating,
                "review": review
            });

            let resp = client.post(format!("{}/tools/{}/rate", cli.url, tool_id))
                .json(&body)
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            if resp.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                println!("Rated {} stars!", rating);
            } else if let Some(err) = resp.get("error").and_then(|v| v.as_str()) {
                println!("Failed: {}", err);
            }
        }

        Commands::Encounters { limit } => {
            let ai_id = cli.ai_id.expect("AI_ID required for this command");
            let resp = client.get(format!("{}/encounters/{}?limit={}", cli.url, ai_id, limit))
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            if let Some(encounters) = resp.get("data").and_then(|d| d.as_array()) {
                println!("YOUR ENCOUNTERS");
                println!("===============");
                for enc in encounters {
                    let ai_1 = enc.get("ai_id_1").and_then(|v| v.as_str()).unwrap_or("?");
                    let ai_2 = enc.get("ai_id_2").and_then(|v| v.as_str()).unwrap_or("?");
                    let space = enc.get("space_id").and_then(|v| v.as_str()).unwrap_or("?");
                    let enc_type = enc.get("encounter_type").and_then(|v| v.as_str()).unwrap_or("?");

                    let other = if ai_1 == ai_id { ai_2 } else { ai_1 };
                    println!("  {} - {} in {}", other, enc_type, space);
                }
                if encounters.is_empty() {
                    println!("No encounters yet. Visit some spaces!");
                }
            }
        }

        Commands::Friends => {
            let ai_id = cli.ai_id.expect("AI_ID required for this command");
            let resp = client.get(format!("{}/friends/{}", cli.url, ai_id))
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            if let Some(friends) = resp.get("data").and_then(|d| d.as_array()) {
                println!("YOUR FRIENDS");
                println!("============");
                for friend in friends {
                    let req_id = friend.get("requester_id").and_then(|v| v.as_str()).unwrap_or("?");
                    let addr_id = friend.get("addressee_id").and_then(|v| v.as_str()).unwrap_or("?");
                    let level = friend.get("level").and_then(|v| v.as_str()).unwrap_or("acquaintance");

                    let other = if req_id == ai_id { addr_id } else { req_id };
                    println!("  {} [{}]", other, level);
                }
                if friends.is_empty() {
                    println!("No friends yet. Meet AIs in spaces and send friend requests!");
                }
            }
        }

        Commands::AddFriend { ai_id: friend_id, note } => {
            let ai_id = cli.ai_id.expect("AI_ID required for this command");
            let body = json!({
                "requester_id": ai_id,
                "addressee_id": friend_id,
                "note": note
            });

            let resp = client.post(format!("{}/friends", cli.url))
                .json(&body)
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            if resp.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                println!("Friend request sent to {}!", friend_id);
            } else if let Some(err) = resp.get("error").and_then(|v| v.as_str()) {
                println!("Failed: {}", err);
            }
        }

        Commands::Activity { space, public, limit } => {
            let mut url = format!("{}/activity?limit={}", cli.url, limit);
            if let Some(s) = space {
                url.push_str(&format!("&space_id={}", s));
            }
            if public {
                url.push_str("&public_only=true");
            }

            let resp = client.get(&url)
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            if let Some(activities) = resp.get("data").and_then(|d| d.as_array()) {
                println!("ACTIVITY FEED");
                println!("=============");
                for act in activities {
                    let ai_id = act.get("ai_id").and_then(|v| v.as_str()).unwrap_or("?");
                    let act_type = act.get("activity_type").and_then(|v| v.as_str()).unwrap_or("?");
                    let target = act.get("target_id").and_then(|v| v.as_str());
                    let desc = act.get("description").and_then(|v| v.as_str());

                    let mut line = format!("{} {}", ai_id, act_type);
                    if let Some(t) = target {
                        line.push_str(&format!(" {}", t));
                    }
                    if let Some(d) = desc {
                        line.push_str(&format!(": {}", d));
                    }
                    println!("  {}", line);
                }
                if activities.is_empty() {
                    println!("No recent activity");
                }
            }
        }

        Commands::Health => {
            let resp = client.get(format!("{}/health", cli.url))
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            println!("{}", serde_json::to_string_pretty(&resp)?);
        }

        Commands::Spec => {
            let resp = client.get(format!("{}/spec", cli.url))
                .send()
                .await?
                .json::<serde_json::Value>()
                .await?;

            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
    }

    Ok(())
}
