//! Local agent state heuristics.
//!
//! With no backend telling us what an agent is doing, TermLoom can only
//! observe a process and the bytes it printed. The rules below are deliberately
//! few and conservative: each observation carries its source and a confidence,
//! and anything uncertain resolves to `Idle` or `Unknown` rather than to an
//! invented "Working on authentication" summary.

use std::time::Duration;

use crate::domain::agent::{AgentState, AgentStateObservation, ObservationSource};

/// What we can see about a local session at one instant.
#[derive(Debug, Clone)]
pub struct ActivityInput {
    /// Whether the child process is still alive.
    pub running: bool,
    /// Exit code once the process finished.
    pub exit_code: Option<i32>,
    /// Time since the last byte of output.
    pub quiet_for: Duration,
    /// Time after which a live, silent session counts as idle.
    pub idle_after: Duration,
    /// Time since the session started (a grace period for `Starting`).
    pub age: Duration,
    /// The last few rendered lines of the screen.
    pub screen_tail: Vec<String>,
}

/// High-precision markers that a CLI is blocked on the user. Kept short on
/// purpose — a false "Waiting" is worse than a vague "Working".
const WAIT_MARKERS: &[&str] = &[
    "(y/n)",
    "(yes/no)",
    "[y/n]",
    "[Y/n]",
    "[y/N]",
    "press enter to continue",
    "password:",
    "passphrase:",
    "do you want to proceed",
    "do you want to continue",
    "continue? ",
    "overwrite?",
    "waiting for your input",
];

/// Time a freshly started session is reported as `Starting`.
const STARTING_GRACE: Duration = Duration::from_millis(800);
/// Output newer than this means the session is actively doing something.
const WORKING_WINDOW: Duration = Duration::from_secs(3);

/// Derive a state observation from what we can see.
pub fn observe(input: &ActivityInput) -> AgentStateObservation {
    if !input.running {
        let state = match input.exit_code {
            Some(0) => AgentState::Done,
            Some(_) => AgentState::Failed,
            None => AgentState::Exited,
        };
        // Process state is ground truth.
        return AgentStateObservation::new(state, ObservationSource::Process, 1.0);
    }

    if input.age < STARTING_GRACE {
        return AgentStateObservation::new(AgentState::Starting, ObservationSource::Process, 0.9);
    }

    if let Some(marker) = waiting_marker(&input.screen_tail) {
        // Only trust a prompt marker once output has settled; a marker that
        // scrolls past mid-run does not mean the agent is blocked.
        if input.quiet_for >= WORKING_WINDOW {
            tracing::trace!(marker, "agent looks blocked on input");
            return AgentStateObservation::new(
                AgentState::WaitingForInput,
                ObservationSource::OutputMarker,
                0.6,
            );
        }
    }

    if input.quiet_for < WORKING_WINDOW {
        return AgentStateObservation::new(
            AgentState::Working,
            ObservationSource::OutputActivity,
            0.7,
        );
    }

    if input.quiet_for >= input.idle_after {
        return AgentStateObservation::new(
            AgentState::Idle,
            ObservationSource::OutputActivity,
            0.5,
        );
    }

    // Between "just printed something" and "clearly idle" we genuinely do not
    // know; keep the previous state by reporting a low-confidence Working.
    AgentStateObservation::new(AgentState::Working, ObservationSource::OutputActivity, 0.4)
}

/// The marker that made us believe the session is waiting, if any.
fn waiting_marker(tail: &[String]) -> Option<&'static str> {
    let haystack: String = tail
        .iter()
        .rev()
        .take(4)
        .map(|line| line.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    WAIT_MARKERS
        .iter()
        .find(|marker| haystack.contains(&marker.to_lowercase()))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(running: bool, quiet_secs: u64, tail: &[&str]) -> ActivityInput {
        ActivityInput {
            running,
            exit_code: None,
            quiet_for: Duration::from_secs(quiet_secs),
            idle_after: Duration::from_secs(20),
            age: Duration::from_secs(60),
            screen_tail: tail.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn exit_status_is_ground_truth() {
        let mut finished = input(false, 5, &[]);
        finished.exit_code = Some(0);
        let obs = observe(&finished);
        assert_eq!(obs.state, AgentState::Done);
        assert_eq!(obs.source, ObservationSource::Process);
        assert_eq!(obs.confidence, 1.0);

        finished.exit_code = Some(2);
        assert_eq!(observe(&finished).state, AgentState::Failed);

        finished.exit_code = None;
        assert_eq!(observe(&finished).state, AgentState::Exited);
    }

    #[test]
    fn young_sessions_report_starting() {
        let mut fresh = input(true, 0, &[]);
        fresh.age = Duration::from_millis(100);
        assert_eq!(observe(&fresh).state, AgentState::Starting);
    }

    #[test]
    fn recent_output_means_working() {
        let obs = observe(&input(true, 0, &["compiling..."]));
        assert_eq!(obs.state, AgentState::Working);
        assert_eq!(obs.source, ObservationSource::OutputActivity);
        assert!(obs.confidence < 1.0, "heuristics never claim certainty");
    }

    #[test]
    fn long_silence_means_idle() {
        assert_eq!(observe(&input(true, 60, &["done"])).state, AgentState::Idle);
    }

    #[test]
    fn settled_prompt_markers_mean_waiting() {
        let obs = observe(&input(true, 10, &["Do you want to proceed? (y/n)"]));
        assert_eq!(obs.state, AgentState::WaitingForInput);
        assert_eq!(obs.source, ObservationSource::OutputMarker);
        assert!(obs.confidence <= 0.6);
    }

    #[test]
    fn markers_are_ignored_while_output_is_still_streaming() {
        // The same marker scrolling past during active output is not a block.
        let obs = observe(&input(true, 0, &["Do you want to proceed? (y/n)"]));
        assert_eq!(obs.state, AgentState::Working);
    }

    #[test]
    fn ordinary_prose_is_not_treated_as_a_prompt() {
        // Requirement: never invent semantic state from model output.
        let obs = observe(&input(
            true,
            10,
            &["I am implementing the authentication flow now"],
        ));
        assert_ne!(obs.state, AgentState::WaitingForInput);
    }

    #[test]
    fn marker_matching_is_case_insensitive() {
        let obs = observe(&input(true, 10, &["PASSWORD:"]));
        assert_eq!(obs.state, AgentState::WaitingForInput);
    }
}
