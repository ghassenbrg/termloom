//! `termloom extension …` subcommands.

use anyhow::Result;

use super::ExtensionCommand;

/// Dispatch an extension subcommand.
pub fn run(command: ExtensionCommand) -> Result<i32> {
    match command {
        ExtensionCommand::Inspect { path, json } => {
            let _ = (path, json);
            println!("extension inspection is not wired up yet");
            Ok(1)
        }
        ExtensionCommand::Install { path } => {
            let _ = path;
            println!("extension installation is not wired up yet");
            Ok(1)
        }
        ExtensionCommand::List { json } => {
            let _ = json;
            println!("extension listing is not wired up yet");
            Ok(1)
        }
        ExtensionCommand::Remove { id } => {
            let _ = id;
            println!("extension removal is not wired up yet");
            Ok(1)
        }
    }
}
