//! Drift detection between source and instances

use crate::config::Config;
use crate::registry::{self, BinaryInfo};
use anyhow::{Result, bail};
use colored::*;
use std::path::Path;

#[derive(Debug)]
pub struct DriftReport {
    pub instance_name: String,
    pub missing: Vec<String>,
    pub outdated: Vec<OutdatedFile>,
    pub extra: Vec<String>,
}

#[derive(Debug)]
pub struct OutdatedFile {
    pub name: String,
    pub source_modified: String,
    pub target_modified: String,
    pub source_size: u64,
    pub target_size: u64,
}

impl DriftReport {
    pub fn has_drift(&self) -> bool {
        !self.missing.is_empty() || !self.outdated.is_empty()
    }

    pub fn total_drift(&self) -> usize {
        self.missing.len() + self.outdated.len()
    }
}

pub fn detect_drift(config: &Config, instance_filter: Option<&str>) -> Result<Vec<DriftReport>> {
    let source_bins = registry::get_source_binaries(config)?;
    let mut reports = Vec::new();

    for instance in &config.instances {
        if !instance.enabled {
            continue;
        }

        if let Some(filter) = instance_filter {
            if instance.name != filter {
                continue;
            }
        }

        let bin_path = Path::new(&instance.path).join("bin");
        let target_bins = registry::get_binaries_in_dir(&bin_path)?;

        let mut missing = Vec::new();
        let mut outdated = Vec::new();
        let mut extra = Vec::new();

        // Check for missing and outdated files
        for (name, source_info) in &source_bins {
            match target_bins.get(name) {
                None => {
                    missing.push(name.clone());
                }
                Some(target_info) => {
                    if source_info.hash != target_info.hash {
                        outdated.push(OutdatedFile {
                            name: name.clone(),
                            source_modified: source_info.modified.format("%Y-%m-%d %H:%M").to_string(),
                            target_modified: target_info.modified.format("%Y-%m-%d %H:%M").to_string(),
                            source_size: source_info.size,
                            target_size: target_info.size,
                        });
                    }
                }
            }
        }

        // Check for extra files (in target but not in source)
        for name in target_bins.keys() {
            if !source_bins.contains_key(name) {
                extra.push(name.clone());
            }
        }

        // Sort for consistent output
        missing.sort();
        outdated.sort_by(|a, b| a.name.cmp(&b.name));
        extra.sort();

        reports.push(DriftReport {
            instance_name: instance.name.clone(),
            missing,
            outdated,
            extra,
        });
    }

    Ok(reports)
}

pub fn compare_instances(config: &Config, name1: &str, name2: &str) -> Result<()> {
    let instance1 = registry::get_instance(config, name1)
        .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", name1))?;
    let instance2 = registry::get_instance(config, name2)
        .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", name2))?;

    let bin_path1 = Path::new(&instance1.path).join("bin");
    let bin_path2 = Path::new(&instance2.path).join("bin");

    let bins1 = registry::get_binaries_in_dir(&bin_path1)?;
    let bins2 = registry::get_binaries_in_dir(&bin_path2)?;

    // Files only in instance1
    let only_in_1: Vec<_> = bins1.keys()
        .filter(|k| !bins2.contains_key(*k))
        .cloned()
        .collect();

    // Files only in instance2
    let only_in_2: Vec<_> = bins2.keys()
        .filter(|k| !bins1.contains_key(*k))
        .cloned()
        .collect();

    // Files with different hashes
    let mut different: Vec<(&String, &BinaryInfo, &BinaryInfo)> = Vec::new();
    for (name, info1) in &bins1 {
        if let Some(info2) = bins2.get(name) {
            if info1.hash != info2.hash {
                different.push((name, info1, info2));
            }
        }
    }

    // Print results
    println!("\n{}: {} binaries", name1.cyan(), bins1.len());
    println!("{}: {} binaries\n", name2.cyan(), bins2.len());

    if only_in_1.is_empty() && only_in_2.is_empty() && different.is_empty() {
        println!("{}", "Instances are identical!".green());
        return Ok(());
    }

    if !only_in_1.is_empty() {
        println!("{} (only in {}):", "UNIQUE".yellow(), name1);
        for name in &only_in_1 {
            println!("  {}", name);
        }
        println!();
    }

    if !only_in_2.is_empty() {
        println!("{} (only in {}):", "UNIQUE".yellow(), name2);
        for name in &only_in_2 {
            println!("  {}", name);
        }
        println!();
    }

    if !different.is_empty() {
        println!("{} (different versions):", "DIFFERENT".red());
        for (name, info1, info2) in &different {
            println!("  {}:", name);
            println!("    {} {} ({} bytes)", name1, info1.modified.format("%Y-%m-%d %H:%M"), info1.size);
            println!("    {} {} ({} bytes)", name2, info2.modified.format("%Y-%m-%d %H:%M"), info2.size);
        }
    }

    Ok(())
}
