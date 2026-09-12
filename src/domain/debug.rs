//! Debug session view models.
//!
//! These mirror the parts of DAP the debug panel shows. The DAP client maps
//! protocol JSON into them so no widget parses protocol messages.

use std::path::PathBuf;

/// Lifecycle of a debug session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DebugStatus {
    /// No adapter running.
    #[default]
    Inactive,
    /// Adapter started, waiting for `initialized`.
    Starting,
    /// Program running.
    Running,
    /// Stopped at a breakpoint/step.
    Paused,
    /// Program finished.
    Terminated,
    /// Adapter reported an error.
    Failed,
}

impl DebugStatus {
    pub fn label(self) -> &'static str {
        match self {
            DebugStatus::Inactive => "inactive",
            DebugStatus::Starting => "starting",
            DebugStatus::Running => "running",
            DebugStatus::Paused => "paused",
            DebugStatus::Terminated => "terminated",
            DebugStatus::Failed => "failed",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            DebugStatus::Starting | DebugStatus::Running | DebugStatus::Paused
        )
    }
}

/// One frame of the call stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackFrame {
    pub id: i64,
    pub name: String,
    pub path: Option<PathBuf>,
    /// 1-based line as reported by the adapter.
    pub line: usize,
    pub column: usize,
}

/// A variable (or scope header) in the variables pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub type_name: Option<String>,
    /// Non-zero when the variable can be expanded.
    pub variables_reference: i64,
    pub depth: usize,
}

/// A thread reported by the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugThread {
    pub id: i64,
    pub name: String,
}

/// A breakpoint the user set in the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Breakpoint {
    pub path: PathBuf,
    /// 0-based editor line.
    pub line: usize,
    /// Whether the adapter confirmed it.
    pub verified: bool,
}

/// What a debug adapter said it supports. Commands the adapter cannot do are
/// shown as unavailable rather than failing when invoked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DebugCapabilities {
    pub configuration_done: bool,
    pub terminate: bool,
    pub restart: bool,
    pub step_back: bool,
    pub evaluate_for_hovers: bool,
    pub conditional_breakpoints: bool,
    pub set_variable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_states_are_classified() {
        assert!(DebugStatus::Paused.is_active());
        assert!(!DebugStatus::Terminated.is_active());
        assert_eq!(DebugStatus::Failed.label(), "failed");
    }
}
