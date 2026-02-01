//! AI-Foundation IDE (afide)
//! Developer environment for managing AI agent instances

mod config;
mod registry;
mod drift;
mod deploy;
mod health;
mod display;

use clap::{Parser, Subcommand};
use colored::*;
use anyhow::Result;

#[derive(Parser)]
#[command(name = "afide")]
#[command(about = "AI-Foundation IDE - Manage AI agent instances", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show status of all registered instances
    Status {
        /// Show detailed information
        #[arg(short, long)]
        verbose: bool,
    },

    /// Detect drift between source and instances
    Drift {
        /// Only check specific instance
        #[arg(short, long)]
        instance: Option<String>,
    },

    /// Deploy binaries to instances
    Deploy {
        /// Files to deploy (deploys all if --all is set)
        files: Vec<String>,

        /// Deploy all binaries
        #[arg(long)]
        all: bool,

        /// Target specific instances (comma-separated)
        #[arg(long, value_delimiter = ',')]
        to: Option<Vec<String>>,

        /// Force deployment even if processes are running
        #[arg(short, long)]
        force: bool,

        /// Dry run - show what would be deployed
        #[arg(long)]
        dry_run: bool,
    },

    /// Check health of all instances
    Health,

    /// List all registered instances
    List,

    /// Show detailed info about an instance
    Info {
        /// Instance name
        name: String,
    },

    /// Register a new instance
    Register {
        /// Instance name
        name: String,

        /// Path to instance directory
        path: String,

        /// AI ID for this instance
        #[arg(long)]
        ai_id: Option<String>,

        /// Instance type (claude-code, gemini-cli, myapp-agent, custom)
        #[arg(long, default_value = "custom")]
        instance_type: String,
    },

    /// Unregister an instance
    Unregister {
        /// Instance name
        name: String,
    },

    /// Compare two instances
    Compare {
        /// First instance
        instance1: String,
        /// Second instance
        instance2: String,
    },

    /// Initialize config with default instances
    Init {
        /// Overwrite existing config
        #[arg(long)]
        force: bool,
    },

    /// Sync configuration files across instances
    SyncConfig {
        /// Dry run
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Status { verbose } => {
            display::print_header("Instance Status");
            let config = config::load_or_create()?;
            let instances = registry::get_instances(&config)?;
            let source_bins = registry::get_source_binaries(&config)?;

            display::print_status_table(&instances, &source_bins, verbose)?;
        }

        Commands::Drift { instance } => {
            display::print_header("Drift Detection");
            let config = config::load_or_create()?;
            let reports = drift::detect_drift(&config, instance.as_deref())?;
            display::print_drift_report(&reports)?;
        }

        Commands::Deploy { files, all, to, force, dry_run } => {
            display::print_header("Deployment");
            if dry_run {
                println!("{}", "[DRY RUN MODE]".yellow());
            }
            let config = config::load_or_create()?;
            deploy::deploy(&config, &files, all, to.as_deref(), force, dry_run)?;
        }

        Commands::Health => {
            display::print_header("Health Check");
            let config = config::load_or_create()?;
            health::check_health(&config)?;
        }

        Commands::List => {
            display::print_header("Registered Instances");
            let config = config::load_or_create()?;
            display::print_instance_list(&config)?;
        }

        Commands::Info { name } => {
            let config = config::load_or_create()?;
            display::print_instance_info(&config, &name)?;
        }

        Commands::Register { name, path, ai_id, instance_type } => {
            let mut config = config::load_or_create()?;
            registry::register_instance(&mut config, &name, &path, ai_id.as_deref(), &instance_type)?;
            config::save(&config)?;
            println!("{} Registered instance: {}", "✓".green(), name.cyan());
        }

        Commands::Unregister { name } => {
            let mut config = config::load_or_create()?;
            registry::unregister_instance(&mut config, &name)?;
            config::save(&config)?;
            println!("{} Unregistered instance: {}", "✓".green(), name.cyan());
        }

        Commands::Compare { instance1, instance2 } => {
            display::print_header(&format!("Comparing {} vs {}", instance1, instance2));
            let config = config::load_or_create()?;
            drift::compare_instances(&config, &instance1, &instance2)?;
        }

        Commands::Init { force } => {
            config::init_default_config(force)?;
            println!("{} Configuration initialized at ~/.ai-foundation/instances.toml", "✓".green());
        }

        Commands::SyncConfig { dry_run } => {
            display::print_header("Config Sync");
            let config = config::load_or_create()?;
            deploy::sync_configs(&config, dry_run)?;
        }
    }

    Ok(())
}
