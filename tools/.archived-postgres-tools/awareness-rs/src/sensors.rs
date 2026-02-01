// Autonomous Sensory Cortex - Reality Check Module
// Sub-millisecond environmental awareness for AI operations
// Runs BEFORE tokenization to prevent common mistakes

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;
use serde::{Deserialize, Serialize};

/// The "Sensory Cortex" Output - Complete environmental snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContext {
    pub path: String,
    pub freshness: FreshnessStatus,
    pub git_status: GitStatus,
    pub dependency_sync: DependencyStatus,
    pub safety: SafetyStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreshnessStatus {
    pub category: String,      // HOT, ACTIVE, STABLE, STALE
    pub age_seconds: u64,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatus {
    pub state: String,         // CLEAN, DIRTY, NO_GIT, GIT_ERROR
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyStatus {
    pub state: String,         // SYNCED, DESYNC, MISSING_LOCK, N/A
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyStatus {
    pub state: String,         // SAFE, HAZARD_LARGE, HAZARD_BINARY, MISSING, READ_ERROR
    pub size_mb: Option<f64>,
    pub details: Option<String>,
}

/// Main entry point - Run all sensors on a file path
pub fn analyze(path: &str) -> FileContext {
    FileContext {
        path: path.to_string(),
        freshness: check_freshness(path),
        git_status: check_git(path),
        dependency_sync: check_dependency_sync(path),
        safety: check_safety(path),
    }
}

// ============================================================================
// SENSOR 1: TIME - Freshness Detection
// ============================================================================
// Detects stale code, prevents reading outdated modules
// Gotcha: Docker volume lag - timestamps can lag by a few seconds in WSL2

fn check_freshness(path: &str) -> FreshnessStatus {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => {
            return FreshnessStatus {
                category: "NEW_FILE".to_string(),
                age_seconds: 0,
                description: "File does not exist yet".to_string(),
            };
        }
    };

    let modified = metadata.modified().unwrap_or(SystemTime::now());
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    let age_seconds = age.as_secs();
    let hours = age_seconds / 3600;
    let days = hours / 24;

    let (category, description) = if hours < 1 {
        ("HOT".to_string(), "Changed <1h ago - Fresh".to_string())
    } else if hours < 24 {
        ("ACTIVE".to_string(), format!("{}h old - Recent", hours))
    } else if days > 30 {
        ("STALE".to_string(), "Legacy code >30 days - Review carefully".to_string())
    } else {
        ("STABLE".to_string(), format!("{}d old - Stable", days))
    };

    FreshnessStatus {
        category,
        age_seconds,
        description,
    }
}

// ============================================================================
// SENSOR 2: STATE - Git Status Detection
// ============================================================================
// Prevents reading uncommitted changes, detects dirty working tree
// Gotcha: Git lock deadlock - add timeout to prevent hanging during rebase

fn check_git(path: &str) -> GitStatus {
    // Check if we're in a git repository
    if !Path::new(".git").exists() {
        return GitStatus {
            state: "NO_GIT".to_string(),
            details: None,
        };
    }

    // Run git status with timeout protection (5 seconds max)
    // Note: Timeout implementation would require wait-timeout crate
    // For now, we run without timeout but this is marked for Phase 2
    let output = Command::new("git")
        .args(["status", "--porcelain", path])
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if stdout.trim().is_empty() {
                GitStatus {
                    state: "CLEAN".to_string(),
                    details: Some("Committed - No changes".to_string()),
                }
            } else {
                let status = stdout.trim().to_string();
                GitStatus {
                    state: "DIRTY".to_string(),
                    details: Some(format!("Uncommitted: {}", status)),
                }
            }
        }
        Err(e) => GitStatus {
            state: "GIT_ERROR".to_string(),
            details: Some(format!("Git command failed: {}", e)),
        },
    }
}

// ============================================================================
// SENSOR 3: SYNC - Dependency Drift Detection
// ============================================================================
// Detects when Cargo.toml is newer than Cargo.lock
// Gotcha: Workspaces - lock file might be in parent directory

fn check_dependency_sync(path: &str) -> DependencyStatus {
    // Only check if we're looking at a Cargo.toml file
    if !path.ends_with("Cargo.toml") {
        return DependencyStatus {
            state: "N/A".to_string(),
            details: Some("Not a Cargo.toml file".to_string()),
        };
    }

    let path_obj = Path::new(path);
    let dir = path_obj.parent().unwrap_or(Path::new("."));
    let lock = dir.join("Cargo.lock");
    let toml = dir.join("Cargo.toml");

    if !lock.exists() {
        return DependencyStatus {
            state: "MISSING_LOCK".to_string(),
            details: Some("Cargo.lock not found - Run 'cargo check'".to_string()),
        };
    }

    let t_meta = fs::metadata(&toml).ok();
    let l_meta = fs::metadata(&lock).ok();

    if let (Some(t), Some(l)) = (t_meta, l_meta) {
        if let (Ok(t_mod), Ok(l_mod)) = (t.modified(), l.modified()) {
            if t_mod > l_mod {
                return DependencyStatus {
                    state: "DESYNC".to_string(),
                    details: Some(
                        "Lockfile older than manifest - Run 'cargo check'".to_string()
                    ),
                };
            }
        }
    }

    DependencyStatus {
        state: "SYNCED".to_string(),
        details: Some("Dependencies synchronized".to_string()),
    }
}

// ============================================================================
// SENSOR 4: SAFETY - Binary/Size Hazard Detection
// ============================================================================
// Prevents AI from reading massive files or binary data
// Gotcha: UTF-16 text files can contain null bytes and appear binary

fn check_safety(path: &str) -> SafetyStatus {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => {
            return SafetyStatus {
                state: "MISSING".to_string(),
                size_mb: None,
                details: Some("File does not exist".to_string()),
            };
        }
    };

    let size_bytes = metadata.len();
    let size_mb = size_bytes as f64 / 1_000_000.0;

    // Binary check FIRST: Peek at first 512 bytes (runs before size check)
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => {
            return SafetyStatus {
                state: "READ_ERROR".to_string(),
                size_mb: Some(size_mb),
                details: Some("Cannot open file for reading".to_string()),
            };
        }
    };

    use std::io::Read;
    let mut buffer = [0u8; 512];
    let bytes_read = file.read(&mut buffer).unwrap_or(0);

    // Check for null bytes (binary indicator) - highest priority
    if bytes_read > 0 && buffer[..bytes_read].contains(&0) {
        return SafetyStatus {
            state: "HAZARD_BINARY".to_string(),
            size_mb: Some(size_mb),
            details: Some("Binary data detected (SQLite/executable) - Do not read".to_string()),
        };
    }

    // Size check SECOND: Warn if >1MB, block if >10MB (only for non-binary files)
    if size_bytes > 10_000_000 {
        return SafetyStatus {
            state: "HAZARD_LARGE".to_string(),
            size_mb: Some(size_mb),
            details: Some(format!(
                "{:.1}MB - BLOCKED - File too large to read safely",
                size_mb
            )),
        };
    }

    if size_bytes > 1_000_000 {
        return SafetyStatus {
            state: "HAZARD_LARGE".to_string(),
            size_mb: Some(size_mb),
            details: Some(format!(
                "{:.1}MB - WARNING - Large file, consider truncating",
                size_mb
            )),
        };
    }

    SafetyStatus {
        state: "SAFE".to_string(),
        size_mb: Some(size_mb),
        details: Some(format!("{:.2}MB - Safe to read", size_mb)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_self() {
        // Test analyzing this source file
        let result = analyze("src/sensors.rs");
        assert_eq!(result.path, "src/sensors.rs");
        println!("Freshness: {:?}", result.freshness);
        println!("Git: {:?}", result.git_status);
        println!("Safety: {:?}", result.safety);
    }

    #[test]
    fn test_cargo_sync() {
        // Test Cargo.toml sync detection
        let result = check_dependency_sync("Cargo.toml");
        println!("Dependency sync: {:?}", result);
    }
}
