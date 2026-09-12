//! `termloom extension …` subcommands.

use anyhow::Result;

use super::ExtensionCommand;
use crate::domain::extensions::CompatibilityReport;
use crate::services::extensions;

/// Dispatch an extension subcommand.
pub fn run(command: ExtensionCommand) -> Result<i32> {
    match command {
        ExtensionCommand::Inspect { path, json } => {
            let report = extensions::inspect(&path)?;
            print_report(&report, json)?;
            Ok(0)
        }
        ExtensionCommand::Install { path } => {
            let report = extensions::install(&path)?;
            println!(
                "{} installed {} {} ({})",
                report.class.glyph(),
                report.id,
                report.version,
                report.class.label()
            );
            println!("Extension code was not executed.");
            Ok(0)
        }
        ExtensionCommand::List { json } => {
            let reports = extensions::list()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&reports)?);
            } else if reports.is_empty() {
                println!("No extensions installed.");
            } else {
                for report in reports {
                    println!(
                        "{} {:<36} {:<12} {}",
                        report.class.glyph(),
                        report.id,
                        report.version,
                        report.class.label()
                    );
                }
            }
            Ok(0)
        }
        ExtensionCommand::Remove { id } => {
            if extensions::remove(&id)? {
                println!("removed {id}");
                Ok(0)
            } else {
                eprintln!("extension `{id}` is not installed");
                Ok(1)
            }
        }
    }
}

fn print_report(report: &CompatibilityReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }
    println!("Extension: {} {}", report.display_name, report.version);
    println!("ID:        {}", report.id);
    println!("Support:   {}", report.class.label());
    println!();
    if report.capabilities.is_empty() {
        println!("✕ no portable capabilities declared");
    } else {
        for capability in &report.capabilities {
            println!(
                "{} {} — {}",
                capability.support.glyph(),
                capability.name,
                capability.detail
            );
        }
    }
    println!();
    println!("Security: extension main/browser entrypoints were not executed.");
    Ok(())
}
