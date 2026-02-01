//! Instance registry management

use crate::config::{Config, Instance};
use anyhow::{Result, bail};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use chrono::{DateTime, Utc};
use md5::{Md5, Digest};

#[derive(Debug, Clone)]
pub struct BinaryInfo {
    pub name: String,
    pub hash: String,
    pub size: u64,
    pub modified: DateTime<Utc>,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct InstanceStatus {
    pub instance: Instance,
    pub exists: bool,
    pub bin_dir_exists: bool,
    pub binary_count: usize,
    pub drift_count: usize,
}

pub fn get_instances(config: &Config) -> Result<Vec<InstanceStatus>> {
    let source_bins = get_source_binaries(config)?;

    let mut statuses = Vec::new();

    for instance in &config.instances {
        if !instance.enabled {
            continue;
        }

        let path = Path::new(&instance.path);
        let bin_path = path.join("bin");

        let exists = path.exists();
        let bin_dir_exists = bin_path.exists();

        let (binary_count, drift_count) = if bin_dir_exists {
            let target_bins = get_binaries_in_dir(&bin_path)?;
            let drift = count_drift(&source_bins, &target_bins);
            (target_bins.len(), drift)
        } else {
            (0, source_bins.len())
        };

        statuses.push(InstanceStatus {
            instance: instance.clone(),
            exists,
            bin_dir_exists,
            binary_count,
            drift_count,
        });
    }

    Ok(statuses)
}

pub fn get_source_binaries(config: &Config) -> Result<HashMap<String, BinaryInfo>> {
    let source_path = Path::new(&config.source.path);
    get_binaries_in_dir(source_path)
}

pub fn get_binaries_in_dir(dir: &Path) -> Result<HashMap<String, BinaryInfo>> {
    let mut binaries = HashMap::new();

    if !dir.exists() {
        return Ok(binaries);
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "exe" {
                    let name = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();

                    if let Ok(info) = get_binary_info(&path) {
                        binaries.insert(name, info);
                    }
                }
            }
        }
    }

    Ok(binaries)
}

pub fn get_binary_info(path: &Path) -> Result<BinaryInfo> {
    let metadata = fs::metadata(path)?;
    let modified = metadata.modified()?;
    let modified: DateTime<Utc> = modified.into();

    // Calculate MD5 hash
    let content = fs::read(path)?;
    let mut hasher = Md5::new();
    hasher.update(&content);
    let hash = format!("{:x}", hasher.finalize());

    Ok(BinaryInfo {
        name: path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string(),
        hash,
        size: metadata.len(),
        modified,
        path: path.to_path_buf(),
    })
}

fn count_drift(source: &HashMap<String, BinaryInfo>, target: &HashMap<String, BinaryInfo>) -> usize {
    let mut drift = 0;

    // Count missing files
    for name in source.keys() {
        if !target.contains_key(name) {
            drift += 1;
        }
    }

    // Count outdated files (different hash)
    for (name, source_info) in source {
        if let Some(target_info) = target.get(name) {
            if source_info.hash != target_info.hash {
                drift += 1;
            }
        }
    }

    drift
}

pub fn register_instance(
    config: &mut Config,
    name: &str,
    path: &str,
    ai_id: Option<&str>,
    instance_type: &str,
) -> Result<()> {
    // Check if already exists
    if config.instances.iter().any(|i| i.name == name) {
        bail!("Instance '{}' already exists", name);
    }

    let instance = Instance {
        name: name.to_string(),
        path: path.to_string(),
        ai_id: ai_id.unwrap_or("unknown").to_string(),
        instance_type: instance_type.to_string(),
        enabled: true,
    };

    config.instances.push(instance);
    Ok(())
}

pub fn unregister_instance(config: &mut Config, name: &str) -> Result<()> {
    let initial_len = config.instances.len();
    config.instances.retain(|i| i.name != name);

    if config.instances.len() == initial_len {
        bail!("Instance '{}' not found", name);
    }

    Ok(())
}

pub fn get_instance<'a>(config: &'a Config, name: &str) -> Option<&'a Instance> {
    config.instances.iter().find(|i| i.name == name)
}
