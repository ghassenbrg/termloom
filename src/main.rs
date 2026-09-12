//! `termloom` binary entry point.

use std::io::IsTerminal;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use termloom::app::runtime::App;
use termloom::cli::{self, Cli};
use termloom::services::workspace::Workspace;

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(&cli);

    match run(cli) {
        Ok(code) => ExitCode::from(code as u8),
        Err(err) => {
            eprintln!("termloom: {err:#}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<i32> {
    if let Some(command) = cli.command {
        return cli::run_subcommand(command);
    }

    if !std::io::stdout().is_terminal() {
        anyhow::bail!(
            "termloom needs an interactive terminal; run `termloom doctor` to check this environment"
        );
    }

    let (workspace, initial_file) = Workspace::discover(&cli.path)?;
    tracing::info!(root = %workspace.root.display(), "opening workspace");

    let mut app = App::new(workspace, initial_file)?;
    app.run()?;
    Ok(0)
}

/// Logs go to a file, never to the screen: stdout belongs to the TUI.
fn init_logging(cli: &Cli) {
    use tracing_subscriber::EnvFilter;

    let path = cli.log_file.clone().or_else(termloom::config::log_path);
    let Some(path) = path else { return };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };

    let filter = EnvFilter::try_from_env("TERMLOOM_LOG")
        .or_else(|_| EnvFilter::try_new(&cli.log_level))
        .unwrap_or_else(|_| EnvFilter::new("warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(file)
        .with_ansi(false)
        .with_target(true)
        .init();
}
