///! Identity CLI - AI Cryptographic Identity Management
///!
///! Replaces identity_cli.py, canonical_identity.py, and regenerate_identity.py
///!
///! Usage:
///!   identity-cli show
///!   identity-cli set-name "New Name"
///!   identity-cli generate --ai-id nova-123 --name "Resonance"
///!   identity-cli verify --fingerprint E0DE57E6241AF9BE
///!   identity-cli sign "message to sign"

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::fs;
use chrono::{DateTime, Utc};

#[derive(Parser)]
#[command(name = "identity-cli")]
#[command(about = "AI cryptographic identity management", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Identity file path (default: data/identity/ai_identity.json)
    #[arg(long, global = true)]
    identity_file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Display current identity
    Show {
        /// Show verbose output including public key
        #[arg(long)]
        verbose: bool,
    },

    /// Change display name (keeps crypto identity)
    SetName {
        /// New display name
        name: String,

        /// Skip confirmation
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Generate new identity
    Generate {
        /// AI identifier (e.g., nova-293)
        #[arg(long)]
        ai_id: String,

        /// Display name
        #[arg(long)]
        name: String,

        /// Overwrite existing identity
        #[arg(long)]
        force: bool,
    },

    /// Verify a fingerprint
    Verify {
        /// Fingerprint to verify (16 hex chars)
        #[arg(long)]
        fingerprint: String,
    },

    /// Sign a message
    Sign {
        /// Message to sign
        message: String,
    },

    /// Verify a signature
    VerifySignature {
        /// Message that was signed
        #[arg(long)]
        message: String,

        /// Signature (hex)
        #[arg(long)]
        signature: String,

        /// Public key (hex)
        #[arg(long)]
        public_key: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Identity {
    ai_id: String,
    display_name: String,
    fingerprint: String,
    public_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    signing_key: Option<String>,
    #[serde(default = "default_created_at")]
    created_at: DateTime<Utc>,
    #[serde(default = "default_updated_at")]
    updated_at: DateTime<Utc>,
    #[serde(default = "default_source")]
    source: String,
    #[serde(default = "default_metadata_version")]
    metadata_version: String,
}

fn default_created_at() -> DateTime<Utc> {
    Utc::now()
}

fn default_updated_at() -> DateTime<Utc> {
    Utc::now()
}

fn default_source() -> String {
    "legacy".to_string()
}

fn default_metadata_version() -> String {
    "1.0.0".to_string()
}

fn get_identity_path(cli_path: Option<PathBuf>) -> PathBuf {
    cli_path.unwrap_or_else(|| PathBuf::from("data/identity/ai_identity.json"))
}

fn load_identity(path: &Path) -> Result<Identity> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read identity file: {}", path.display()))?;

    let identity: Identity = serde_json::from_str(&contents)
        .with_context(|| "Failed to parse identity JSON")?;

    Ok(identity)
}

fn save_identity(path: &Path, identity: &Identity) -> Result<()> {
    // Ensure directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    let json = serde_json::to_string_pretty(identity)
        .with_context(|| "Failed to serialize identity")?;

    fs::write(path, json)
        .with_context(|| format!("Failed to write identity file: {}", path.display()))?;

    Ok(())
}

fn generate_identity(ai_id: String, display_name: String) -> Result<Identity> {
    use ed25519_dalek::SigningKey;
    use sha3::{Sha3_256, Digest};
    use rand::{RngCore, rngs::OsRng};

    // Generate new Ed25519 keypair
    let mut secret_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut secret_bytes);
    let signing_key = SigningKey::from_bytes(&secret_bytes);
    let verifying_key = signing_key.verifying_key();

    // Encode keys as hex
    let signing_key_hex = hex::encode(signing_key.to_bytes());
    let public_key_hex = hex::encode(verifying_key.to_bytes());

    // Generate fingerprint: SHA3-256 of public key, first 16 hex chars
    let mut hasher = Sha3_256::new();
    hasher.update(verifying_key.as_bytes());
    let hash = hasher.finalize();
    let fingerprint = hex::encode(&hash[..8]).to_uppercase();

    let now = Utc::now();
    Ok(Identity {
        ai_id,
        display_name,
        fingerprint,
        public_key: public_key_hex,
        signing_key: Some(signing_key_hex),
        created_at: now,
        updated_at: now,
        source: "identity-cli".to_string(),
        metadata_version: "1.0.0".to_string(),
    })
}

fn sign_message(identity: &Identity, message: &str) -> Result<String> {
    use ed25519_dalek::{SigningKey, Signer};

    let signing_key_hex = identity.signing_key.as_ref()
        .context("Identity has no signing key (public identity only)")?;

    let signing_key_bytes = hex::decode(signing_key_hex)
        .context("Invalid signing key hex")?;

    let signing_key_array: [u8; 32] = signing_key_bytes.try_into()
        .map_err(|_| anyhow::anyhow!("Signing key must be 32 bytes"))?;

    let signing_key = SigningKey::from_bytes(&signing_key_array);
    let signature = signing_key.sign(message.as_bytes());

    Ok(hex::encode(signature.to_bytes()))
}

fn verify_signature(public_key_hex: &str, message: &str, signature_hex: &str) -> Result<bool> {
    use ed25519_dalek::{VerifyingKey, Signature, Verifier};

    let public_key_bytes = hex::decode(public_key_hex)
        .context("Invalid public key hex")?;

    let public_key_array: [u8; 32] = public_key_bytes.try_into()
        .map_err(|_| anyhow::anyhow!("Public key must be 32 bytes"))?;

    let verifying_key = VerifyingKey::from_bytes(&public_key_array)
        .context("Invalid verifying key")?;

    let signature_bytes = hex::decode(signature_hex)
        .context("Invalid signature hex")?;

    let signature_array: [u8; 64] = signature_bytes.try_into()
        .map_err(|_| anyhow::anyhow!("Signature must be 64 bytes"))?;

    let signature = Signature::from_bytes(&signature_array);

    Ok(verifying_key.verify(message.as_bytes(), &signature).is_ok())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let identity_path = get_identity_path(cli.identity_file);

    match cli.command {
        Commands::Show { verbose } => {
            let identity = load_identity(&identity_path)?;

            println!("AI IDENTITY");
            println!("============================================================");
            println!("Crypto AI ID:    {}", identity.ai_id);
            println!("Display Name:    {}", identity.display_name);
            println!("Fingerprint:     {}", identity.fingerprint);
            println!("Created:         {}", identity.created_at.format("%Y-%m-%d %H:%M:%S UTC"));
            println!("Updated:         {}", identity.updated_at.format("%Y-%m-%d %H:%M:%S UTC"));

            if verbose {
                println!("\nPublic Key:      {}...", &identity.public_key[..40]);
                println!("Source:          {}", identity.source);
                println!("Version:         {}", identity.metadata_version);
            }

            println!("\n============================================================");
            println!("Identity is cryptographically signed with Ed25519");
            println!("Use fingerprint {} for verification", identity.fingerprint);
        }

        Commands::SetName { name, yes } => {
            let mut identity = load_identity(&identity_path)?;

            println!("CHANGING DISPLAY NAME");
            println!("============================================================");
            println!("Old Display Name: {}", identity.display_name);
            println!("New Display Name: {}", name);
            println!("\nYour crypto identity will NOT change:");
            println!("   Crypto AI ID:  {}", identity.ai_id);
            println!("   Fingerprint:   {}", identity.fingerprint);

            if !yes {
                print!("\nProceed with name change? (y/N): ");
                use std::io::{self, BufRead};
                let stdin = io::stdin();
                let mut line = String::new();
                stdin.lock().read_line(&mut line)?;

                if line.trim().to_lowercase() != "y" {
                    println!("Aborted");
                    return Ok(());
                }
            }

            identity.display_name = name;
            identity.updated_at = Utc::now();

            save_identity(&identity_path, &identity)?;

            println!("\nDISPLAY NAME UPDATED!");
            println!("============================================================");
            println!("Display Name:    {}", identity.display_name);
            println!("Crypto AI ID:    {}", identity.ai_id);
            println!("Fingerprint:     {}", identity.fingerprint);
        }

        Commands::Generate { ai_id, name, force } => {
            if identity_path.exists() && !force {
                eprintln!("Error: Identity already exists at {}", identity_path.display());
                eprintln!("Use --force to overwrite");
                std::process::exit(1);
            }

            let identity = generate_identity(ai_id, name)?;

            save_identity(&identity_path, &identity)?;

            println!("NEW IDENTITY GENERATED!");
            println!("============================================================");
            println!("Crypto AI ID:    {}", identity.ai_id);
            println!("Display Name:    {}", identity.display_name);
            println!("Fingerprint:     {}", identity.fingerprint);
            println!("Saved to:        {}", identity_path.display());
        }

        Commands::Verify { fingerprint } => {
            let identity = load_identity(&identity_path)?;

            if identity.fingerprint == fingerprint.to_uppercase() {
                println!("VERIFIED");
                println!("Fingerprint matches current identity:");
                println!("  AI ID: {}", identity.ai_id);
                println!("  Name:  {}", identity.display_name);
            } else {
                println!("NOT VERIFIED");
                println!("Fingerprint does not match current identity");
                println!("  Expected: {}", identity.fingerprint);
                println!("  Got:      {}", fingerprint.to_uppercase());
                std::process::exit(1);
            }
        }

        Commands::Sign { message } => {
            let identity = load_identity(&identity_path)?;
            let signature = sign_message(&identity, &message)?;
            println!("{}", signature);
        }

        Commands::VerifySignature { message, signature, public_key } => {
            match verify_signature(&public_key, &message, &signature) {
                Ok(true) => {
                    println!("SIGNATURE VALID");
                    std::process::exit(0);
                }
                Ok(false) => {
                    println!("SIGNATURE INVALID");
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}
