//! Command-line interface.

pub mod doctor;
pub mod extension_cmd;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

/// `termloom` — agent-native development in your terminal.
#[derive(Debug, Parser)]
#[command(
    name = "termloom",
    version,
    about = "Agent-native development environment for your terminal",
    long_about = None,
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Directory (or file) to open. Defaults to the current directory.
    #[arg(value_name = "PATH", default_value = ".")]
    pub path: PathBuf,

    /// Write diagnostic logs to this file instead of the default location.
    #[arg(long, value_name = "FILE", global = true)]
    pub log_file: Option<PathBuf>,

    /// Log level: error, warn, info, debug, trace.
    #[arg(long, value_name = "LEVEL", global = true, default_value = "warn")]
    pub log_level: String,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Check that everything TermLoom integrates with is available.
    Doctor,
    /// Inspect, install, list and remove VS Code extension packages.
    Extension {
        #[command(subcommand)]
        command: ExtensionCommand,
    },
    /// Print or write the default configuration.
    Config {
        /// Write the default configuration to the user config path.
        #[arg(long)]
        write_default: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ExtensionCommand {
    /// Inspect a local `.vsix` without installing or executing anything.
    Inspect {
        /// Path to the `.vsix` package.
        path: PathBuf,
        /// Print the full report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Install the supported declarative assets of a local `.vsix`.
    Install {
        /// Path to the `.vsix` package.
        path: PathBuf,
    },
    /// List installed extensions and their compatibility class.
    List {
        /// Print as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Remove an installed extension by id (`publisher.name`).
    Remove {
        /// Extension id.
        id: String,
    },
}

/// Run a non-TUI subcommand. Returns the process exit code.
pub fn run_subcommand(command: Command) -> Result<i32> {
    match command {
        Command::Doctor => doctor::run(),
        Command::Extension { command } => extension_cmd::run(command),
        Command::Config { write_default } => run_config(write_default),
    }
}

fn run_config(write_default: bool) -> Result<i32> {
    let config = crate::config::Config::with_builtin_defaults();
    let text = config.to_toml()?;
    if !write_default {
        println!("{text}");
        return Ok(0);
    }
    let Some(path) = crate::config::global_config_path() else {
        anyhow::bail!("could not determine a configuration directory");
    };
    if path.exists() {
        println!("{} already exists; not overwriting it", path.display());
        return Ok(1);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, text)?;
    println!("wrote {}", path.display());
    Ok(0)
}
