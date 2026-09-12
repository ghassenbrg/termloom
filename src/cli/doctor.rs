//! `termloom doctor` — report what is available on this machine.

use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;

/// One diagnostic row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Ok,
    /// Optional integration missing; TermLoom still works.
    Missing,
    /// Something required is broken.
    Failed,
}

impl CheckStatus {
    pub fn glyph(self) -> &'static str {
        match self {
            CheckStatus::Ok => "✓",
            CheckStatus::Missing => "○",
            CheckStatus::Failed => "✕",
        }
    }
}

/// Locate a program on `PATH`.
pub fn which(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|dir| {
        let candidate = dir.join(program);
        candidate.is_file().then_some(candidate)
    })
}

/// First line of `<program> --version`, when it runs.
fn version_of(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    let text = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).to_string()
    } else {
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    text.lines().next().map(|line| line.trim().to_string())
}

/// Build the full check list. Pure enough to test: it only reads the
/// environment, never mutates anything.
pub fn collect_checks() -> Vec<Check> {
    let mut checks = Vec::new();

    checks.push(Check {
        name: "terminal".into(),
        status: if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            CheckStatus::Ok
        } else {
            CheckStatus::Missing
        },
        detail: format!(
            "TERM={}, colours={}",
            std::env::var("TERM").unwrap_or_else(|_| "unset".into()),
            std::env::var("COLORTERM").unwrap_or_else(|_| "basic".into())
        ),
    });

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    checks.push(Check {
        name: "shell".into(),
        status: if PathBuf::from(&shell).is_file() {
            CheckStatus::Ok
        } else {
            CheckStatus::Failed
        },
        detail: shell,
    });

    checks.push(binary_check("git", &["--version"], true));
    checks.push(binary_check("claude", &["--version"], false));
    checks.push(binary_check("codex", &["--version"], false));
    checks.push(binary_check("herdr", &["--version"], false));
    checks.push(binary_check("rust-analyzer", &["--version"], false));

    let config_path = crate::config::global_config_path();
    checks.push(Check {
        name: "config".into(),
        status: CheckStatus::Ok,
        detail: match &config_path {
            Some(path) if path.exists() => format!("{}", path.display()),
            Some(path) => format!("{} (not created yet)", path.display()),
            None => "no configuration directory".into(),
        },
    });

    checks.push(Check {
        name: "state".into(),
        status: match crate::config::state_dir() {
            Some(_) => CheckStatus::Ok,
            None => CheckStatus::Missing,
        },
        detail: crate::config::state_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "unavailable".into()),
    });

    let cwd = std::env::current_dir().unwrap_or_default();
    let git_root = crate::services::workspace::find_git_root(&cwd);
    checks.push(Check {
        name: "workspace".into(),
        status: CheckStatus::Ok,
        detail: match git_root {
            Some(root) => format!("git repository at {}", root.display()),
            None => format!("{} (not a git repository)", cwd.display()),
        },
    });

    checks
}

fn binary_check(program: &str, version_args: &[&str], required: bool) -> Check {
    match which(program) {
        Some(path) => Check {
            name: program.to_string(),
            status: CheckStatus::Ok,
            detail: version_of(program, version_args).unwrap_or_else(|| path.display().to_string()),
        },
        None => Check {
            name: program.to_string(),
            status: if required {
                CheckStatus::Failed
            } else {
                CheckStatus::Missing
            },
            detail: if required {
                "not found on PATH (required)".into()
            } else {
                "not found on PATH (optional)".into()
            },
        },
    }
}

/// Print the report. Exit code 1 when a required check failed.
pub fn run() -> Result<i32> {
    println!("{} {}", crate::PRODUCT_NAME, crate::VERSION);
    println!();
    let checks = collect_checks();
    let width = checks.iter().map(|c| c.name.len()).max().unwrap_or(8);
    for check in &checks {
        println!(
            "{} {:width$}  {}",
            check.status.glyph(),
            check.name,
            check.detail,
            width = width
        );
    }
    println!();

    let failed = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Failed)
        .count();
    let missing: Vec<&str> = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Missing)
        .map(|c| c.name.as_str())
        .collect();

    if !missing.is_empty() {
        println!(
            "optional integrations not installed: {} — TermLoom works without them",
            missing.join(", ")
        );
    }
    if failed > 0 {
        println!("{failed} required check(s) failed");
        return Ok(1);
    }
    println!("ready");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_expected_checks() {
        let checks = collect_checks();
        let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
        for expected in ["terminal", "shell", "git", "claude", "codex", "herdr"] {
            assert!(
                names.contains(&expected),
                "missing check {expected}: {names:?}"
            );
        }
    }

    #[test]
    fn missing_optional_tools_are_not_failures() {
        let check = binary_check("definitely-not-installed-xyz", &["--version"], false);
        assert_eq!(check.status, CheckStatus::Missing);
        assert!(check.detail.contains("optional"));
    }

    #[test]
    fn missing_required_tools_are_failures() {
        let check = binary_check("definitely-not-installed-xyz", &["--version"], true);
        assert_eq!(check.status, CheckStatus::Failed);
    }

    #[test]
    fn finds_a_program_that_exists() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-installed-xyz").is_none());
    }
}
