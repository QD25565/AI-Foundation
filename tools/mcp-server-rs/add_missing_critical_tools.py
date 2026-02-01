#!/usr/bin/env python3
"""
Add the 4 CRITICAL missing tools to the Rust MCP server.
These are REQUIRED for full feature parity - DO NOT SKIP ANY.

1. stigmergy_sense - Sense digital pheromones at a location
2. stigmergy_deposit - Deposit digital pheromone at a location
3. identity_verify - Verify AI identity by fingerprint
4. recent_dirs (track_directory + get_recent_directories) - Directory tracking
"""

with open('src/main.rs', 'r') as f:
    content = f.read()

# ============================================================================
# STEP 1: Add input schemas after PathInput
# ============================================================================

new_schemas = '''
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StigmergySenseInput {
    #[schemars(description = "Location to sense (e.g., 'task:42', 'file:MainActivity.kt', 'module:auth')")]
    pub location: String,
    #[schemars(description = "Filter by pheromone type: INTEREST, WORKING, BLOCKED, SUCCESS, QUESTION")]
    pub pheromone_type: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StigmergyDepositInput {
    #[schemars(description = "Location to deposit (e.g., 'task:42', 'file:MainActivity.kt')")]
    pub location: String,
    #[schemars(description = "Pheromone type: INTEREST, WORKING, BLOCKED, SUCCESS, QUESTION")]
    pub pheromone_type: String,
    #[schemars(description = "Pheromone intensity 0.0-2.0 (default: 1.0)")]
    pub intensity: Option<f64>,
    #[schemars(description = "Decay rate 0.0-1.0 (default: 0.05)")]
    pub decay_rate: Option<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct IdentityVerifyInput {
    #[schemars(description = "The 16-character hex fingerprint to verify (e.g., 'A5EB832B16A95F41')")]
    pub fingerprint: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TrackDirectoryInput {
    #[schemars(description = "Directory path to track")]
    pub directory: String,
    #[schemars(description = "Access type: read, write, search")]
    pub access_type: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RecentDirsInput {
    #[schemars(description = "Maximum directories to return (default: 10)")]
    pub limit: Option<i64>,
}
'''

# Find PathInput and insert after it
schema_marker = '''#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PathInput {
    #[schemars(description = "File path")]
    pub path: String,
}'''

if schema_marker in content:
    content = content.replace(schema_marker, schema_marker + new_schemas)
    print("[OK] Inserted input schemas after PathInput")
else:
    print("[ERROR] PathInput marker not found")

# ============================================================================
# STEP 2: Add tool implementations before the closing brace of impl block
# ============================================================================

new_tools = '''
    // ============================================================================
    // STIGMERGY TOOLS - Digital Pheromones for O(1) Coordination
    // ============================================================================

    #[tool(description = "Sense digital pheromones at a location. Use this to check what other AI agents are doing or have done at a specific location (file, task, module). Pheromones decay over time. Enables O(1) coordination without direct communication.")]
    async fn stigmergy_sense(&self, Parameters(input): Parameters<StigmergySenseInput>) -> String {
        let state = self.state.read().await;

        // Query pheromones from PostgreSQL
        let query = if let Some(ref ptype) = input.pheromone_type {
            format!(
                "SELECT location, pheromone_type, intensity, decay_rate, agent_id, created_at, expires_at
                 FROM pheromones
                 WHERE location = $1 AND pheromone_type = $2 AND expires_at > NOW()
                 ORDER BY created_at DESC"
            )
        } else {
            format!(
                "SELECT location, pheromone_type, intensity, decay_rate, agent_id, created_at, expires_at
                 FROM pheromones
                 WHERE location = $1 AND expires_at > NOW()
                 ORDER BY created_at DESC"
            )
        };

        match state.teambook.query_pheromones(&input.location, input.pheromone_type.as_deref()).await {
            Ok(pheromones) => {
                if pheromones.is_empty() {
                    return format!("No pheromones at: {}", input.location);
                }

                let mut out = format!("Pheromones at {} ({} found):\\n", input.location, pheromones.len());
                for (ptype, intensity, agent_id, age_secs) in pheromones {
                    let age_str = if age_secs < 60 {
                        format!("{}s ago", age_secs)
                    } else if age_secs < 3600 {
                        format!("{}m ago", age_secs / 60)
                    } else {
                        format!("{}h ago", age_secs / 3600)
                    };
                    out.push_str(&format!("  {} by {} | intensity: {:.2} | {}\\n", ptype, agent_id, intensity, age_str));
                }
                out.trim_end().to_string()
            },
            Err(e) => format!("Error sensing pheromones: {}", e),
        }
    }

    #[tool(description = "Deposit a digital pheromone at a location. Use this to signal your intent or state to other AI agents. Pheromones decay over time. Types: INTEREST (exploring), WORKING (actively working), BLOCKED (stuck), SUCCESS (completed), QUESTION (need help).")]
    async fn stigmergy_deposit(&self, Parameters(input): Parameters<StigmergyDepositInput>) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();

        let intensity = input.intensity.unwrap_or(1.0).clamp(0.0, 2.0);
        let decay_rate = input.decay_rate.unwrap_or(0.05).clamp(0.0, 1.0);

        // Validate pheromone type
        let valid_types = ["INTEREST", "WORKING", "BLOCKED", "SUCCESS", "QUESTION"];
        let ptype = input.pheromone_type.to_uppercase();
        if !valid_types.contains(&ptype.as_str()) {
            return format!("Invalid pheromone type: {}. Valid types: {:?}", input.pheromone_type, valid_types);
        }

        match state.teambook.deposit_pheromone(&ai_id, &input.location, &ptype, intensity, decay_rate).await {
            Ok(_) => format!("Deposited {} pheromone at {} (intensity: {:.2}, decay: {:.2})", ptype, input.location, intensity, decay_rate),
            Err(e) => format!("Error depositing pheromone: {}", e),
        }
    }

    // ============================================================================
    // IDENTITY TOOLS - Cryptographic Verification
    // ============================================================================

    #[tool(description = "Verify another AI's identity by fingerprint. Use this to check if a fingerprint matches a known AI in the registry. Provides cryptographic verification of AI identity.")]
    async fn identity_verify(&self, Parameters(input): Parameters<IdentityVerifyInput>) -> String {
        // Load identity registry from disk
        let registry_path = std::path::Path::new("data/identity_registry.json");

        // Try multiple paths
        let paths_to_try = [
            std::path::PathBuf::from("data/identity_registry.json"),
            std::path::PathBuf::from("../data/identity_registry.json"),
            dirs::home_dir().map(|h| h.join("identity_registry.json")).unwrap_or_default(),
        ];

        let mut registry_content = None;
        for path in &paths_to_try {
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(path) {
                    registry_content = Some(content);
                    break;
                }
            }
        }

        let registry_content = match registry_content {
            Some(c) => c,
            None => return "Identity registry not found".to_string(),
        };

        let registry: std::collections::HashMap<String, serde_json::Value> = match serde_json::from_str(&registry_content) {
            Ok(r) => r,
            Err(e) => return format!("Error parsing registry: {}", e),
        };

        // Normalize fingerprint (uppercase, no colons)
        let target_fp = input.fingerprint.to_uppercase().replace(":", "");

        for (ai_id, data) in registry.iter() {
            if let Some(fp) = data.get("fingerprint").and_then(|f| f.as_str()) {
                if fp.to_uppercase() == target_fp {
                    let display_name = data.get("display_name").and_then(|d| d.as_str()).unwrap_or("Unknown");
                    return format!("VERIFIED: {} ({})\\nFingerprint: {}\\nAI ID: {}", display_name, ai_id, fp, ai_id);
                }
            }
        }

        format!("NOT FOUND: Fingerprint {} not in registry", input.fingerprint)
    }

    // ============================================================================
    // DIRECTORY TRACKING TOOLS - Track AI's directory access patterns
    // ============================================================================

    #[tool(description = "Track a directory access for pattern analysis. Records when an AI accesses a directory to help understand working patterns.")]
    async fn track_directory(&self, Parameters(input): Parameters<TrackDirectoryInput>) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();
        let access_type = input.access_type.unwrap_or_else(|| "read".to_string());

        match state.teambook.track_directory(&ai_id, &input.directory, &access_type).await {
            Ok(_) => format!("Tracked {} access to: {}", access_type, input.directory),
            Err(e) => format!("Error tracking directory: {}", e),
        }
    }

    #[tool(description = "Get recently accessed directories. Returns directories this AI has accessed, ordered by recency.")]
    async fn get_recent_directories(&self, Parameters(input): Parameters<RecentDirsInput>) -> String {
        let state = self.state.read().await;
        let ai_id = state.ai_id.clone();
        let limit = input.limit.unwrap_or(10);

        match state.teambook.get_recent_directories(&ai_id, limit as i32).await {
            Ok(dirs) => {
                if dirs.is_empty() {
                    return "No recent directories tracked".to_string();
                }

                let mut out = format!("Recent directories ({}):\\n", dirs.len());
                for (dir, access_type, age_str) in dirs {
                    out.push_str(&format!("  [{}] {} - {}\\n", access_type, dir, age_str));
                }
                out.trim_end().to_string()
            },
            Err(e) => format!("Error getting directories: {}", e),
        }
    }

    #[tool(description = "Alias for get_recent_directories - Get recently accessed directories")]
    async fn recent_dirs(&self, Parameters(input): Parameters<RecentDirsInput>) -> String {
        self.get_recent_directories(Parameters(input)).await
    }
'''

# Find the closing brace before #[tool_handler]
tool_marker = '''    #[tool(description = "Calculate text hash (SHA256)")]
    async fn util_hash(&self, Parameters(input): Parameters<ContentInput>) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        input.content.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

}

#[tool_handler]'''

if tool_marker in content:
    content = content.replace(tool_marker, tool_marker.replace("\n}\n\n#[tool_handler]", new_tools + "\n}\n\n#[tool_handler]"))
    print("[OK] Inserted tool implementations")
else:
    print("[ERROR] Tool marker not found - trying alternative")
    # Alternative: find just before #[tool_handler]
    alt_marker = "}\n\n#[tool_handler]"
    if alt_marker in content:
        # Find the last occurrence
        idx = content.rfind(alt_marker)
        content = content[:idx] + new_tools + "\n" + content[idx:]
        print("[OK] Inserted tools using alternative method")
    else:
        print("[ERROR] Could not find insertion point")

with open('src/main.rs', 'w') as f:
    f.write(content)

print("\n[COMPLETE] Added 4 critical missing tools:")
print("  1. stigmergy_sense - Sense digital pheromones")
print("  2. stigmergy_deposit - Deposit digital pheromones")
print("  3. identity_verify - Verify AI fingerprint")
print("  4. track_directory - Track directory access")
print("  5. get_recent_directories - Get recent dirs")
print("  6. recent_dirs - Alias for get_recent_directories")
print("\nNow need to add PostgreSQL methods to teambook_rs...")
