//! Normalised agent model.
//!
//! TermLoom supervises independent CLI programs (Claude Code, Codex, a shell,
//! or any user command). Their behaviour differs wildly, so every backend maps
//! into the vocabulary below. Rule: when the truth is not knowable, report the
//! conservative state rather than inventing one.

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use super::ids::{AgentId, TerminalId};

/// Which CLI a session is running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentKind {
    Claude,
    Codex,
    OpenCode,
    Shell,
    Custom(String),
    Unknown,
}

impl AgentKind {
    /// Short label used in dense list rows.
    pub fn label(&self) -> &str {
        match self {
            AgentKind::Claude => "Claude",
            AgentKind::Codex => "Codex",
            AgentKind::OpenCode => "OpenCode",
            AgentKind::Shell => "Shell",
            AgentKind::Custom(name) => name,
            AgentKind::Unknown => "Agent",
        }
    }

    /// Whether this kind is a coding agent (as opposed to a plain shell/task).
    pub fn is_coding_agent(&self) -> bool {
        matches!(
            self,
            AgentKind::Claude | AgentKind::Codex | AgentKind::OpenCode
        )
    }

    /// Best-effort classification from the program that was launched.
    pub fn from_command(program: &str) -> AgentKind {
        let base = std::path::Path::new(program)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(program)
            .to_ascii_lowercase();
        match base.as_str() {
            "claude" => AgentKind::Claude,
            "codex" => AgentKind::Codex,
            "opencode" => AgentKind::OpenCode,
            "sh" | "bash" | "zsh" | "fish" | "nu" | "dash" => AgentKind::Shell,
            other => AgentKind::Custom(other.to_string()),
        }
    }
}

/// The normalised lifecycle state shown in the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AgentState {
    /// Process spawned, nothing observed yet.
    Starting,
    /// Produced output recently; presumed busy.
    Working,
    /// Looks like it is blocked on the user (prompt detected, quiet TTY).
    WaitingForInput,
    /// Alive but quiet for a while.
    Idle,
    /// Exited successfully.
    Done,
    /// Exited with a failure status.
    Failed,
    /// Exited, status unknown or signalled.
    Exited,
    /// No usable observation.
    Unknown,
}

impl AgentState {
    /// Human label.
    pub fn label(self) -> &'static str {
        match self {
            AgentState::Starting => "Starting",
            AgentState::Working => "Working",
            AgentState::WaitingForInput => "Waiting",
            AgentState::Idle => "Idle",
            AgentState::Done => "Done",
            AgentState::Failed => "Failed",
            AgentState::Exited => "Exited",
            AgentState::Unknown => "Unknown",
        }
    }

    /// Glyph used next to the label. Colour is never the only signal.
    pub fn glyph(self) -> &'static str {
        match self {
            AgentState::Starting => "◌",
            AgentState::Working => "●",
            AgentState::WaitingForInput => "!",
            AgentState::Idle => "○",
            AgentState::Done => "✓",
            AgentState::Failed => "×",
            AgentState::Exited => "▫",
            AgentState::Unknown => "?",
        }
    }

    /// True when the session still has a live process.
    pub fn is_live(self) -> bool {
        matches!(
            self,
            AgentState::Starting
                | AgentState::Working
                | AgentState::WaitingForInput
                | AgentState::Idle
        )
    }

    /// True when the user should probably look at this agent.
    pub fn needs_attention(self) -> bool {
        matches!(self, AgentState::WaitingForInput | AgentState::Failed)
    }
}

/// Where an observation came from. Used to keep confidence honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObservationSource {
    /// Derived from the OS process (alive / exit status). Highly reliable.
    Process,
    /// Derived from PTY output timing. Heuristic.
    OutputActivity,
    /// Derived from recognised text markers in the terminal screen. Heuristic.
    OutputMarker,
    /// Reported by an external backend such as Herdr.
    Backend,
    /// Set by the user (e.g. manual rename/marking).
    User,
}

/// A single state reading with its provenance.
#[derive(Debug, Clone, Copy)]
pub struct AgentStateObservation {
    pub state: AgentState,
    pub source: ObservationSource,
    /// 0.0–1.0. Process-derived observations are 1.0; heuristics are lower.
    pub confidence: f32,
    pub observed_at: SystemTime,
}

impl AgentStateObservation {
    pub fn new(state: AgentState, source: ObservationSource, confidence: f32) -> Self {
        Self {
            state,
            source,
            confidence: confidence.clamp(0.0, 1.0),
            observed_at: SystemTime::now(),
        }
    }
}

/// Which backend owns the session's process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentBackendKind {
    /// A PTY owned by this TermLoom process.
    Local,
    /// A pane owned by a Herdr session.
    Herdr,
}

impl AgentBackendKind {
    pub fn label(self) -> &'static str {
        match self {
            AgentBackendKind::Local => "local",
            AgentBackendKind::Herdr => "herdr",
        }
    }
}

/// An agent session as the workbench understands it.
#[derive(Debug, Clone)]
pub struct AgentSession {
    pub id: AgentId,
    pub kind: AgentKind,
    /// User-visible name; renameable.
    pub label: String,
    /// The terminal that renders this agent, when the backend exposes one.
    pub terminal_id: Option<TerminalId>,
    pub cwd: std::path::PathBuf,
    /// Program + args actually launched.
    pub command: Vec<String>,
    pub state: AgentState,
    /// Provenance of `state`.
    pub state_source: ObservationSource,
    pub confidence: f32,
    /// Short description of what the session is doing, only when it comes from
    /// a reliable source. Never inferred from model prose.
    pub task_summary: Option<String>,
    pub started_at: SystemTime,
    pub last_activity_at: SystemTime,
    pub backend: AgentBackendKind,
    pub backend_session_id: Option<String>,
    /// Local, user-authored checklist. Not scraped from agent output.
    pub tasks: Vec<AgentTask>,
}

impl AgentSession {
    /// Wall-clock lifetime of the session.
    pub fn duration(&self) -> Duration {
        SystemTime::now()
            .duration_since(self.started_at)
            .unwrap_or_default()
    }

    /// Time since the last observed activity.
    pub fn idle_for(&self) -> Duration {
        SystemTime::now()
            .duration_since(self.last_activity_at)
            .unwrap_or_default()
    }

    /// Apply a fresh observation.
    ///
    /// Every observation describes the session *now*, so the newest one wins
    /// and `confidence` is recorded as provenance for the UI rather than used
    /// as a ratchet — an early high-confidence `Starting` must not freeze the
    /// dashboard for the rest of the session. The one hard rule is that a
    /// process which has finished never goes back to a live state.
    pub fn apply(&mut self, obs: AgentStateObservation) {
        let finished = matches!(
            self.state,
            AgentState::Done | AgentState::Failed | AgentState::Exited
        );
        if finished && obs.state.is_live() {
            return;
        }
        self.state = obs.state;
        self.state_source = obs.source;
        self.confidence = obs.confidence;
    }
}

/// A user-authored checklist entry attached to an agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub text: String,
    pub done: bool,
}

/// Format a duration the way the dashboard shows it (`12m`, `3h04`, `45s`).
pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h{:02}", secs / 3600, (secs % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_agent_commands() {
        assert_eq!(AgentKind::from_command("claude"), AgentKind::Claude);
        assert_eq!(AgentKind::from_command("/usr/bin/codex"), AgentKind::Codex);
        assert_eq!(AgentKind::from_command("/bin/zsh"), AgentKind::Shell);
        assert_eq!(
            AgentKind::from_command("cargo"),
            AgentKind::Custom("cargo".into())
        );
    }

    #[test]
    fn process_observations_beat_heuristics() {
        let mut session = sample();
        session.apply(AgentStateObservation::new(
            AgentState::Working,
            ObservationSource::OutputActivity,
            0.6,
        ));
        assert_eq!(session.state, AgentState::Working);

        session.apply(AgentStateObservation::new(
            AgentState::Done,
            ObservationSource::Process,
            1.0,
        ));
        assert_eq!(session.state, AgentState::Done);
    }

    #[test]
    fn exited_sessions_never_go_back_to_working() {
        let mut session = sample();
        session.apply(AgentStateObservation::new(
            AgentState::Failed,
            ObservationSource::Process,
            1.0,
        ));
        session.apply(AgentStateObservation::new(
            AgentState::Working,
            ObservationSource::OutputActivity,
            0.9,
        ));
        assert_eq!(session.state, AgentState::Failed);
    }

    #[test]
    fn a_newer_reading_replaces_an_older_one() {
        // Observations describe the present, so the latest wins even when it
        // is less certain; its provenance travels with it.
        let mut session = sample();
        session.apply(AgentStateObservation::new(
            AgentState::WaitingForInput,
            ObservationSource::OutputMarker,
            0.8,
        ));
        session.apply(AgentStateObservation::new(
            AgentState::Idle,
            ObservationSource::OutputActivity,
            0.3,
        ));
        assert_eq!(session.state, AgentState::Idle);
        assert_eq!(session.state_source, ObservationSource::OutputActivity);
        assert_eq!(session.confidence, 0.3);
    }

    #[test]
    fn a_starting_session_leaves_that_state_once_it_is_observed() {
        // Regression: a high-confidence `Starting` used to block every later
        // heuristic, so live agents stayed "Starting" forever.
        let mut session = sample();
        session.state = AgentState::Starting;
        session.state_source = ObservationSource::Process;
        session.confidence = 0.9;

        session.apply(AgentStateObservation::new(
            AgentState::Working,
            ObservationSource::OutputActivity,
            0.7,
        ));
        assert_eq!(session.state, AgentState::Working);
    }

    #[test]
    fn durations_render_compactly() {
        assert_eq!(format_duration(Duration::from_secs(9)), "9s");
        assert_eq!(format_duration(Duration::from_secs(12 * 60)), "12m");
        assert_eq!(format_duration(Duration::from_secs(3 * 3600 + 240)), "3h04");
    }

    fn sample() -> AgentSession {
        AgentSession {
            id: AgentId::next(),
            kind: AgentKind::Claude,
            label: "Claude".into(),
            terminal_id: None,
            cwd: std::path::PathBuf::from("/tmp"),
            command: vec!["claude".into()],
            state: AgentState::Starting,
            state_source: ObservationSource::Process,
            confidence: 0.0,
            task_summary: None,
            started_at: SystemTime::now(),
            last_activity_at: SystemTime::now(),
            backend: AgentBackendKind::Local,
            backend_session_id: None,
            tasks: Vec::new(),
        }
    }
}
