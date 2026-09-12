//! Terminal session metadata. The actual PTY lives in
//! [`crate::services::terminal`]; nothing here knows about `portable_pty`.

use std::path::PathBuf;
use std::time::SystemTime;

use super::ids::TerminalId;

/// Why the session exists. Agents get richer treatment in the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    /// Plain interactive shell.
    Shell,
    /// Backing terminal of an agent session.
    Agent,
    /// One-off task (test run, build, ...).
    Task,
}

impl TerminalKind {
    pub fn label(self) -> &'static str {
        match self {
            TerminalKind::Shell => "shell",
            TerminalKind::Agent => "agent",
            TerminalKind::Task => "task",
        }
    }
}

/// Lifecycle of the child process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalState {
    Starting,
    Running,
    /// Child exited; carries the status code when one was reported.
    Exited(Option<i32>),
    /// Spawn failed. The reason is kept on the session for display.
    Failed,
}

impl TerminalState {
    pub fn is_running(self) -> bool {
        matches!(self, TerminalState::Starting | TerminalState::Running)
    }

    pub fn label(self) -> String {
        match self {
            TerminalState::Starting => "starting".into(),
            TerminalState::Running => "running".into(),
            TerminalState::Exited(Some(code)) => format!("exited {code}"),
            TerminalState::Exited(None) => "exited".into(),
            TerminalState::Failed => "failed".into(),
        }
    }
}

/// Everything about a terminal that is safe to copy into the UI layer.
#[derive(Debug, Clone)]
pub struct TerminalMeta {
    pub id: TerminalId,
    pub label: String,
    pub cwd: PathBuf,
    /// Program plus arguments, already split.
    pub command: Vec<String>,
    pub kind: TerminalKind,
    pub state: TerminalState,
    pub created_at: SystemTime,
    pub last_output_at: SystemTime,
    /// Populated when spawning failed.
    pub error: Option<String>,
}

impl TerminalMeta {
    /// Command as a single display string.
    pub fn command_line(&self) -> String {
        self.command.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_status_is_rendered() {
        assert_eq!(TerminalState::Exited(Some(1)).label(), "exited 1");
        assert!(!TerminalState::Exited(None).is_running());
        assert!(TerminalState::Starting.is_running());
    }
}
