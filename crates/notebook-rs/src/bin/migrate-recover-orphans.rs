//! One-shot migration: recover orphan rows (rows that fail to decrypt under the
//! current AI_ID) by trying candidate ai_ids / alt device parameters, then
//! re-encrypting under the current key.
//!
//! History: Engram derives its content key from device_secret (home_dir + salt +
//! COMPUTERNAME) + ai_id. Early bootstraps, renames, and cross-environment runs
//! (WSL vs Windows; different COMPUTERNAME) wrote rows under a different key than
//! the folder-name-derived id the DB is opened under today. Those rows are dead
//! weight to list()/recall(). This tool recovers what it can.
//!
//! Candidate cipher set:
//!   - Each candidate ai_id (user-supplied + auto list) is tried under:
//!     (current home, current COMPUTERNAME),
//!     (current home, no hostname),
//!     every (alt_home, alt_host) combination the user supplies via --alt-homes / --alt-hosts,
//!     and each alt_home with no hostname.
//!   - Auto candidates: "default", "unknown", "" (empty), every ai_id found in
//!     ~/.ai-foundation/agents/ (so cross-ai-written rows can be recovered),
//!     and the current ai_id with trailing -N suffix stripped.
//!
//! Safety:
//!   - Backup to <path>.bak-recover-orphans-<epoch> before any write.
//!   - Idempotent: second run finds zero newly-recoverable rows.
//!   - Unrecoverable rows remain untouched on disk (list paths skip them via
//!     get_skip_orphan).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use engram::crypto::{derive_encryption_key, derive_encryption_key_with_device, EngramCipher};
use engram::{Engram, RecoveryResult};

const AUTO_CANDIDATE_AI_IDS: &[&str] = &["default", "unknown", ""];

fn usage() {
    eprintln!(
        "usage: migrate-recover-orphans --db <path> --ai-id <id> [options]\n\
         \n\
         Options:\n\
           --db <path>             notebook.engram path (required)\n\
           --ai-id <id>            current AI identity that owns this notebook (required)\n\
           --candidates <a,b,c>    extra candidate ai_ids to try (comma-separated)\n\
           --agents-root <path>    dir containing per-ai folders; every folder name is\n\
                                    added as a candidate ai_id\n\
                                    (default: <USERPROFILE>\\.ai-foundation\\agents)\n\
           --alt-homes <a,b,c>     additional home_dir values to try in device_secret\n\
                                    (comma-separated; useful for WSL vs Windows path)\n\
           --alt-hosts <a,b,c>     additional hostnames to try in device_secret\n\
                                    (comma-separated)\n\
           --key-files-dir <path>  dir containing legacy <ai_id>.engram-key files\n\
                                    (raw 32-byte keys); each file loaded as an additional\n\
                                    candidate cipher. Covers v2 file-stored-key scheme\n\
                                    written by legacy notebook.exe.\n\
           --limit <N>             only attempt the first N orphans\n\
           --dry-run               probe only; report what would recover; no writes\n"
    );
}

struct Args {
    db: PathBuf,
    ai_id: String,
    user_candidates: Vec<String>,
    agents_root: Option<PathBuf>,
    alt_homes: Vec<String>,
    alt_hosts: Vec<String>,
    key_files_dir: Option<PathBuf>,
    limit: Option<usize>,
    dry_run: bool,
}

fn parse_list(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().to_string()).collect()
}

fn parse_args() -> Option<Args> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut db: Option<PathBuf> = None;
    let mut ai_id: Option<String> = None;
    let mut user_candidates: Vec<String> = Vec::new();
    let mut agents_root: Option<PathBuf> = None;
    let mut alt_homes: Vec<String> = Vec::new();
    let mut alt_hosts: Vec<String> = Vec::new();
    let mut key_files_dir: Option<PathBuf> = None;
    let mut limit: Option<usize> = None;
    let mut dry_run = false;
    let mut auto_agents = true;
    let mut i = 0;
    while i < raw.len() {
        let a = raw[i].as_str();
        let need = |j: usize| -> Option<&str> {
            if j >= raw.len() {
                eprintln!("error: {} requires a value", raw[j-1]);
                None
            } else { Some(raw[j].as_str()) }
        };
        match a {
            "--db" => { i += 1; db = Some(PathBuf::from(need(i)?)); }
            "--ai-id" => { i += 1; ai_id = Some(need(i)?.to_string()); }
            "--candidates" => { i += 1; user_candidates = parse_list(need(i)?); }
            "--agents-root" => { i += 1; agents_root = Some(PathBuf::from(need(i)?)); }
            "--no-agents-root" => { auto_agents = false; }
            "--alt-homes" => { i += 1; alt_homes = parse_list(need(i)?); }
            "--alt-hosts" => { i += 1; alt_hosts = parse_list(need(i)?); }
            "--key-files-dir" => { i += 1; key_files_dir = Some(PathBuf::from(need(i)?)); }
            "--limit" => {
                i += 1;
                limit = Some(match need(i)?.parse() {
                    Ok(v) => v,
                    Err(e) => { eprintln!("error: --limit: {}", e); return None; }
                });
            }
            "--dry-run" => dry_run = true,
            "-h" | "--help" => { usage(); std::process::exit(0); }
            other => { eprintln!("error: unknown flag {}", other); return None; }
        }
        i += 1;
    }
    let db = db.or_else(|| { eprintln!("error: --db is required"); None })?;
    let ai_id = match ai_id {
        Some(s) if !s.is_empty() => s,
        _ => { eprintln!("error: --ai-id is required"); return None; }
    };
    if agents_root.is_none() && auto_agents {
        if let Some(home) = dirs::home_dir() {
            let p = home.join(".ai-foundation").join("agents");
            if p.is_dir() { agents_root = Some(p); }
        }
    }
    Some(Args { db, ai_id, user_candidates, agents_root, alt_homes, alt_hosts, key_files_dir, limit, dry_run })
}

fn build_ai_id_candidates(args: &Args) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for c in &args.user_candidates {
        if seen.insert(c.clone()) { out.push(c.clone()); }
    }
    for c in AUTO_CANDIDATE_AI_IDS {
        let s = c.to_string();
        if seen.insert(s.clone()) { out.push(s); }
    }
    if let Some(pos) = args.ai_id.rfind('-') {
        let base = &args.ai_id[..pos];
        if !base.is_empty() && base.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
            let s = base.to_string();
            if seen.insert(s.clone()) { out.push(s); }
        }
    }
    if let Some(ref root) = args.agents_root {
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    let s = name.to_string();
                    if seen.insert(s.clone()) { out.push(s); }
                }
            }
        }
    }
    out.retain(|c| c != &args.ai_id);
    out
}

/// Build (label, cipher) pairs covering every combination of:
///   - candidate ai_id
///   - device-secret mode: (a) process default (cipher from ai_id alone);
///                         (b) explicit (home, host) with host=None or host=Some(h)
///     where homes are [current process home + alt_homes] and hosts are
///     [None, current COMPUTERNAME, ...alt_hosts].
fn build_cipher_candidates(args: &Args, ai_ids: &[String]) -> Vec<(String, EngramCipher)> {
    let mut candidates: Vec<(String, EngramCipher)> = Vec::new();

    let current_home: Option<String> = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string());
    let current_host: Option<String> = std::env::var("COMPUTERNAME").ok()
        .or_else(|| std::env::var("HOSTNAME").ok());

    // Home set: current + alt_homes (dedup).
    let mut homes: Vec<Option<String>> = Vec::new();
    let mut home_seen: HashSet<String> = HashSet::new();
    homes.push(None); // "default" = let the process decide (matches EngramCipher::new)
    if let Some(ref h) = current_home {
        if home_seen.insert(h.clone()) { homes.push(Some(h.clone())); }
    }
    for h in &args.alt_homes {
        if home_seen.insert(h.clone()) { homes.push(Some(h.clone())); }
    }

    // Host set: None (env-var-unset), current, alt_hosts.
    let mut hosts: Vec<Option<String>> = vec![None];
    let mut host_seen: HashSet<String> = HashSet::new();
    if let Some(ref h) = current_host {
        if host_seen.insert(h.clone()) { hosts.push(Some(h.clone())); }
    }
    for h in &args.alt_hosts {
        if host_seen.insert(h.clone()) { hosts.push(Some(h.clone())); }
    }

    let mut key_seen: HashSet<[u8; 32]> = HashSet::new();

    // Load legacy v2 file-stored keys (raw 32 bytes per file at
    // <key_files_dir>/<ai_id>.engram-key). Legacy notebook.exe did NOT derive
    // keys at runtime — it generated a random 32-byte key once and persisted it
    // here. These keys are not reconstructible from environment, so without
    // loading the files we cannot decrypt rows written by that scheme.
    if let Some(ref dir) = args.key_files_dir {
        match fs::read_dir(dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = match path.file_name().and_then(|n| n.to_str()) {
                        Some(n) => n.to_string(),
                        None => continue,
                    };
                    if !name.ends_with(".engram-key") { continue; }
                    let ai_id_label = name.trim_end_matches(".engram-key").to_string();
                    let bytes = match fs::read(&path) {
                        Ok(b) => b,
                        Err(e) => {
                            eprintln!("warn: read {}: {}", path.display(), e);
                            continue;
                        }
                    };
                    if bytes.len() != 32 {
                        eprintln!("warn: {} is {} bytes, expected 32; skipping",
                            path.display(), bytes.len());
                        continue;
                    }
                    let mut key = [0u8; 32];
                    key.copy_from_slice(&bytes);
                    if key_seen.insert(key) {
                        candidates.push((
                            format!("key-file={:?}(ai_id={:?})", path.display().to_string(), ai_id_label),
                            EngramCipher::from_key(key),
                        ));
                    }
                }
            }
            Err(e) => {
                eprintln!("warn: --key-files-dir {}: {}", dir.display(), e);
            }
        }
    }

    for ai_id in ai_ids {
        for home in &homes {
            for host in &hosts {
                let (label, key): (String, [u8; 32]) = match home {
                    None => {
                        // Only valid when host is also None (host=None means "let process env decide"
                        // via derive_encryption_key which calls get_device_secret). Skip other
                        // combinations to avoid duplicating the env-default key under misleading labels.
                        if host.is_some() { continue; }
                        let k = derive_encryption_key(ai_id);
                        (format!("ai_id={:?},device=process-default", ai_id), k)
                    }
                    Some(h) => {
                        let k = derive_encryption_key_with_device(ai_id, h, host.as_deref());
                        let host_tag = host.as_deref().unwrap_or("(none)");
                        (format!("ai_id={:?},home={:?},host={:?}", ai_id, h, host_tag), k)
                    }
                };
                if key_seen.insert(key) {
                    candidates.push((label, EngramCipher::from_key(key)));
                }
            }
        }
    }
    candidates
}

fn backup_db(db_path: &Path) -> std::io::Result<PathBuf> {
    let ts = chrono::Utc::now().timestamp();
    let bak = db_path.with_extension(format!("engram.bak-recover-orphans-{}", ts));
    fs::copy(db_path, &bak)?;
    Ok(bak)
}

fn run(args: Args) -> Result<(), String> {
    if !args.db.exists() { return Err(format!("db does not exist: {}", args.db.display())); }
    if !args.db.is_file() { return Err(format!("db is not a regular file: {}", args.db.display())); }

    std::env::set_var("AI_ID", &args.ai_id);

    let ai_ids = build_ai_id_candidates(&args);
    let candidates = build_cipher_candidates(&args, &ai_ids);

    println!("db:              {}", args.db.display());
    println!("ai_id:           {}", args.ai_id);
    println!("ai_id candidates: {} [{}]", ai_ids.len(),
        ai_ids.iter().map(|s| if s.is_empty() { "(empty)".into() } else { format!("{:?}", s) })
            .collect::<Vec<_>>().join(", "));
    println!("alt homes:       {:?}", args.alt_homes);
    println!("alt hosts:       {:?}", args.alt_hosts);
    println!("key-files dir:   {:?}", args.key_files_dir);
    println!("cipher probes:   {} distinct keys", candidates.len());
    println!("dry_run:         {}", args.dry_run);
    if let Some(n) = args.limit { println!("limit:           {}", n); }

    let orphan_ids: Vec<u64> = {
        let mut db = Engram::open_readonly(&args.db)
            .map_err(|e| format!("open read-only: {}", e))?;
        let stats = db.stats();
        let scan_upper = stats.note_count.saturating_add(64);
        println!("scanning:        ids 1..={} (note_count={}, active={})",
            scan_upper, stats.note_count, stats.active_notes);
        let mut orphans: Vec<u64> = Vec::new();
        for id in 1..=scan_upper {
            match db.get(id) {
                Ok(Some(_)) => {}
                Ok(None) => {}
                Err(_) => orphans.push(id),
            }
        }
        orphans
    };
    println!("orphans:         {} rows undecryptable under current key", orphan_ids.len());

    if orphan_ids.is_empty() {
        println!("nothing to do.");
        return Ok(());
    }

    let work_ids: Vec<u64> = match args.limit {
        Some(n) => orphan_ids.iter().take(n).copied().collect(),
        None => orphan_ids.clone(),
    };

    let mut by_candidate: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut unrecoverable: Vec<u64> = Vec::new();
    {
        let mut db = Engram::open_readonly(&args.db)
            .map_err(|e| format!("open read-only for probe: {}", e))?;
        for id in &work_ids {
            match db.probe_orphan(*id, &candidates).map_err(|e| format!("probe id={}: {}", id, e))? {
                RecoveryResult::Recovered(c) => { *by_candidate.entry(c).or_insert(0) += 1; }
                RecoveryResult::Unrecoverable => unrecoverable.push(*id),
                RecoveryResult::NotOrphan => {}
                RecoveryResult::Missing => {}
            }
        }
    }
    println!("probe result:");
    if by_candidate.is_empty() {
        println!("  (no candidate recovered any row)");
    } else {
        for (c, n) in &by_candidate {
            println!("  {} rows ← {}", n, c);
        }
    }
    println!("  unrecoverable: {} rows", unrecoverable.len());

    if args.dry_run {
        println!("dry-run: no writes.");
        return Ok(());
    }

    let recoverable_total: usize = by_candidate.values().sum();
    if recoverable_total == 0 {
        println!("no candidates produced a recovery; skipping write phase.");
        return Ok(());
    }

    let bak = backup_db(&args.db).map_err(|e| format!("backup failed: {}", e))?;
    println!("backup:          {}", bak.display());

    let mut recovered = 0usize;
    let mut still_orphan = 0usize;
    let mut errors = 0usize;
    {
        let mut db = Engram::open(&args.db)
            .map_err(|e| format!("open read-write: {}", e))?;
        for id in &work_ids {
            match db.recover_orphan(*id, &candidates) {
                Ok(RecoveryResult::Recovered(_)) => recovered += 1,
                Ok(RecoveryResult::Unrecoverable) => still_orphan += 1,
                Ok(RecoveryResult::NotOrphan) => {}
                Ok(RecoveryResult::Missing) => {}
                Err(e) => { errors += 1; eprintln!("  id={} recover failed: {}", id, e); }
            }
        }
    }
    println!("recovered:       {}", recovered);
    println!("still orphan:    {}", still_orphan);
    if errors > 0 { println!("errors:          {} (see stderr)", errors); }
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Some(a) => a,
        None => { usage(); return ExitCode::from(2); }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => { eprintln!("error: {}", e); ExitCode::from(1) }
    }
}
