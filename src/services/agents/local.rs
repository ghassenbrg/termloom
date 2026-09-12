//! Agents backed by pseudo terminals owned by this TermLoom process.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use crate::domain::agent::{
    AgentBackendKind, AgentKind, AgentSession, AgentState, ObservationSource,
};
use crate::domain::ids::AgentId;
use crate::domain::terminal::{TerminalKind, TerminalState};
use crate::services::terminal::{SpawnSpec, TerminalManager};

use super::detect::{observe, ActivityInput};
use super::{AgentBackend, BackendCapabilities, SpawnAgentRequest, TerminalSnapshot};

/// Local backend: one PTY per agent.
pub struct LocalAgentBackend {
    terminals: Arc<Mutex<TerminalManager>>,
    sessions: Mutex<Vec<AgentSession>>,
    /// Silence after which a live agent counts as idle.
    idle_after: Duration,
}

impl LocalAgentBackend {
    pub fn new(terminals: Arc<Mutex<TerminalManager>>, idle_after: Duration) -> LocalAgentBackend {
        LocalAgentBackend {
            terminals,
            sessions: Mutex::new(Vec::new()),
            idle_after,
        }
    }

    fn with_session<T>(&self, id: AgentId, f: impl FnOnce(&mut AgentSession) -> T) -> Result<T> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| anyhow!("agent registry lock poisoned"))?;
        let session = sessions
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| anyhow!("unknown agent {id}"))?;
        Ok(f(session))
    }
}

#[async_trait]
impl AgentBackend for LocalAgentBackend {
    fn backend_kind(&self) -> AgentBackendKind {
        AgentBackendKind::Local
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            spawn: true,
            send_input: true,
            stop: true,
            restart: true,
            snapshot: true,
            embedded_terminal: true,
        }
    }

    fn status(&self) -> String {
        "local pty".to_string()
    }

    async fn list_agents(&self) -> Result<Vec<AgentSession>> {
        Ok(self
            .sessions
            .lock()
            .map_err(|_| anyhow!("agent registry lock poisoned"))?
            .clone())
    }

    async fn spawn_agent(&self, request: SpawnAgentRequest) -> Result<AgentSession> {
        let spec = SpawnSpec {
            label: request.label.clone(),
            program: request.program.clone(),
            args: request.args.clone(),
            cwd: request.cwd.clone(),
            env: request.env.clone(),
            kind: TerminalKind::Agent,
            rows: request.rows,
            cols: request.cols,
            scrollback: 0,
        };
        let terminal_id = {
            let mut terminals = self
                .terminals
                .lock()
                .map_err(|_| anyhow!("terminal manager lock poisoned"))?;
            terminals.spawn(spec)?
        };

        let now = SystemTime::now();
        let session = AgentSession {
            id: AgentId::next(),
            kind: request.kind.clone(),
            label: request.label,
            terminal_id: Some(terminal_id),
            cwd: request.cwd,
            command: std::iter::once(request.program)
                .chain(request.args)
                .collect(),
            state: AgentState::Starting,
            state_source: ObservationSource::Process,
            confidence: 0.9,
            task_summary: None,
            started_at: now,
            last_activity_at: now,
            backend: AgentBackendKind::Local,
            backend_session_id: None,
            tasks: Vec::new(),
        };
        self.sessions
            .lock()
            .map_err(|_| anyhow!("agent registry lock poisoned"))?
            .push(session.clone());
        Ok(session)
    }

    async fn send_input(&self, id: AgentId, input: &[u8]) -> Result<()> {
        let terminal_id = self
            .with_session(id, |s| s.terminal_id)?
            .ok_or_else(|| anyhow!("agent {id} has no terminal"))?;
        let mut terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal manager lock poisoned"))?;
        let session = terminals
            .get_mut(terminal_id)
            .ok_or_else(|| anyhow!("agent {id} has no terminal"))?;
        session.write(input)
    }

    async fn stop(&self, id: AgentId) -> Result<()> {
        let terminal_id = self
            .with_session(id, |s| s.terminal_id)?
            .ok_or_else(|| anyhow!("agent {id} has no terminal"))?;
        let mut terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal manager lock poisoned"))?;
        if let Some(session) = terminals.get_mut(terminal_id) {
            session.kill()?;
        }
        Ok(())
    }

    async fn restart(&self, id: AgentId) -> Result<AgentSession> {
        let terminal_id = self
            .with_session(id, |s| s.terminal_id)?
            .ok_or_else(|| anyhow!("agent {id} has no terminal"))?;
        let new_terminal = {
            let mut terminals = self
                .terminals
                .lock()
                .map_err(|_| anyhow!("terminal manager lock poisoned"))?;
            terminals.restart(terminal_id)?
        };
        let now = SystemTime::now();
        self.with_session(id, |session| {
            session.terminal_id = Some(new_terminal);
            session.state = AgentState::Starting;
            session.state_source = ObservationSource::Process;
            session.confidence = 0.9;
            session.started_at = now;
            session.last_activity_at = now;
            session.clone()
        })
    }

    async fn rename(&self, id: AgentId, label: &str) -> Result<()> {
        let terminal_id = self.with_session(id, |session| {
            session.label = label.to_string();
            session.terminal_id
        })?;
        if let Some(terminal_id) = terminal_id {
            if let Ok(mut terminals) = self.terminals.lock() {
                if let Some(terminal) = terminals.get_mut(terminal_id) {
                    terminal.set_label(label);
                }
            }
        }
        Ok(())
    }

    async fn remove(&self, id: AgentId) -> Result<()> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| anyhow!("agent registry lock poisoned"))?;
        let Some(index) = sessions.iter().position(|s| s.id == id) else {
            return Ok(());
        };
        let session = sessions.remove(index);
        drop(sessions);
        if let Some(terminal_id) = session.terminal_id {
            if let Ok(mut terminals) = self.terminals.lock() {
                terminals.close(terminal_id);
            }
        }
        Ok(())
    }

    async fn snapshot(&self, id: AgentId) -> Result<TerminalSnapshot> {
        let terminal_id = self
            .with_session(id, |s| s.terminal_id)?
            .ok_or_else(|| anyhow!("agent {id} has no terminal"))?;
        let terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal manager lock poisoned"))?;
        let Some(session) = terminals.get(terminal_id) else {
            return Ok(TerminalSnapshot {
                lines: Vec::new(),
                unavailable: true,
            });
        };
        Ok(TerminalSnapshot {
            lines: session.snapshot().tail(200),
            unavailable: false,
        })
    }

    async fn refresh(&self) -> Result<()> {
        let mut terminals = self
            .terminals
            .lock()
            .map_err(|_| anyhow!("terminal manager lock poisoned"))?;
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| anyhow!("agent registry lock poisoned"))?;

        for session in sessions.iter_mut() {
            let Some(terminal_id) = session.terminal_id else {
                continue;
            };
            // Reap first so a finished child is reported as exited.
            if let Some(code) = terminals.get_mut(terminal_id).and_then(|t| t.try_wait()) {
                terminals.mark_exited(terminal_id, Some(code));
            }
            let Some(terminal) = terminals.get(terminal_id) else {
                session.apply(crate::domain::agent::AgentStateObservation::new(
                    AgentState::Exited,
                    ObservationSource::Process,
                    1.0,
                ));
                continue;
            };

            let exit_code = match terminal.state() {
                TerminalState::Exited(code) => code,
                _ => None,
            };
            let input = ActivityInput {
                running: terminal.is_running(),
                exit_code,
                quiet_for: terminal.quiet_for(),
                idle_after: self.idle_after,
                age: SystemTime::now()
                    .duration_since(session.started_at)
                    .unwrap_or_default(),
                screen_tail: terminal.snapshot().tail(6),
            };
            session.last_activity_at = terminal.last_output_at();
            session.apply(observe(&input));
        }
        Ok(())
    }
}

/// Build a spawn request from a configured preset.
pub fn request_from_preset(
    name: &str,
    preset: &crate::config::AgentPreset,
    cwd: std::path::PathBuf,
) -> SpawnAgentRequest {
    let kind = AgentKind::from_command(&preset.command);
    let label = preset
        .label
        .clone()
        .unwrap_or_else(|| kind_label(&kind, name));
    SpawnAgentRequest {
        kind,
        label,
        program: preset.command.clone(),
        args: preset.args.clone(),
        cwd,
        env: preset
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        rows: 24,
        cols: 80,
    }
}

fn kind_label(kind: &AgentKind, fallback: &str) -> String {
    match kind {
        AgentKind::Custom(_) | AgentKind::Unknown => fallback.to_string(),
        other => other.label().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::terminal::{EventSink, TerminalEvent};
    use std::sync::mpsc;

    fn backend() -> (
        LocalAgentBackend,
        Arc<Mutex<TerminalManager>>,
        mpsc::Receiver<TerminalEvent>,
    ) {
        let (tx, rx) = mpsc::channel();
        let sink: EventSink = Arc::new(move |event| {
            let _ = tx.send(event);
        });
        let terminals = Arc::new(Mutex::new(TerminalManager::new(sink)));
        let backend = LocalAgentBackend::new(Arc::clone(&terminals), Duration::from_secs(20));
        (backend, terminals, rx)
    }

    fn request(program: &str, args: &[&str]) -> SpawnAgentRequest {
        SpawnAgentRequest {
            kind: AgentKind::from_command(program),
            label: program.to_string(),
            program: program.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: std::env::temp_dir(),
            env: Vec::new(),
            rows: 10,
            cols: 40,
        }
    }

    fn block<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if predicate() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn spawning_an_agent_creates_a_real_terminal() {
        let (backend, terminals, _rx) = backend();
        let session = block(backend.spawn_agent(request("echo", &["agent-output"]))).unwrap();
        assert_eq!(session.backend, AgentBackendKind::Local);
        let terminal_id = session.terminal_id.expect("agents get a terminal");
        assert!(wait_until(|| terminals
            .lock()
            .unwrap()
            .get(terminal_id)
            .map(|t| t.snapshot().to_text().contains("agent-output"))
            .unwrap_or(false)));
    }

    #[test]
    fn refresh_maps_a_finished_process_to_done() {
        let (backend, _terminals, _rx) = backend();
        let session = block(backend.spawn_agent(request("true", &[]))).unwrap();
        assert!(wait_until(|| {
            block(backend.refresh()).unwrap();
            let agents = block(backend.list_agents()).unwrap();
            agents[0].state == AgentState::Done
        }));
        let agents = block(backend.list_agents()).unwrap();
        assert_eq!(agents[0].id, session.id);
        assert_eq!(agents[0].state_source, ObservationSource::Process);
        assert_eq!(agents[0].confidence, 1.0);
    }

    #[test]
    fn a_live_agent_leaves_the_starting_state() {
        // Regression: the dashboard used to show every running agent as
        // "Starting" because the initial reading was never replaced.
        let (backend, _terminals, _rx) = backend();
        let session = block(backend.spawn_agent(request("cat", &[]))).unwrap();
        assert!(wait_until(|| {
            block(backend.refresh()).unwrap();
            let state = block(backend.list_agents()).unwrap()[0].state;
            state != AgentState::Starting
        }));
        let agents = block(backend.list_agents()).unwrap();
        assert!(agents[0].state.is_live(), "{:?}", agents[0].state);
        assert_ne!(agents[0].state, AgentState::Starting);
        block(backend.stop(session.id)).unwrap();
    }

    #[test]
    fn refresh_maps_a_failing_process_to_failed() {
        let (backend, _terminals, _rx) = backend();
        block(backend.spawn_agent(request("false", &[]))).unwrap();
        assert!(wait_until(|| {
            block(backend.refresh()).unwrap();
            block(backend.list_agents()).unwrap()[0].state == AgentState::Failed
        }));
    }

    #[test]
    fn input_reaches_the_agent_process() {
        let (backend, _terminals, _rx) = backend();
        let session = block(backend.spawn_agent(request("cat", &[]))).unwrap();
        block(backend.send_input(session.id, b"hello-agent\r")).unwrap();
        assert!(wait_until(|| {
            let snapshot = block(backend.snapshot(session.id)).unwrap();
            snapshot.lines.iter().any(|l| l.contains("hello-agent"))
        }));
        block(backend.stop(session.id)).unwrap();
    }

    #[test]
    fn stop_then_restart_gives_a_new_terminal() {
        let (backend, _terminals, _rx) = backend();
        let session = block(backend.spawn_agent(request("cat", &[]))).unwrap();
        let first_terminal = session.terminal_id.unwrap();
        block(backend.stop(session.id)).unwrap();
        let restarted = block(backend.restart(session.id)).unwrap();
        assert_eq!(restarted.id, session.id, "the agent keeps its identity");
        assert_ne!(restarted.terminal_id.unwrap(), first_terminal);
        assert_eq!(restarted.state, AgentState::Starting);
        block(backend.stop(session.id)).unwrap();
    }

    #[test]
    fn rename_and_remove() {
        let (backend, terminals, _rx) = backend();
        let session = block(backend.spawn_agent(request("cat", &[]))).unwrap();
        block(backend.rename(session.id, "Reviewer")).unwrap();
        assert_eq!(block(backend.list_agents()).unwrap()[0].label, "Reviewer");
        assert_eq!(
            terminals
                .lock()
                .unwrap()
                .get(session.terminal_id.unwrap())
                .unwrap()
                .label(),
            "Reviewer"
        );

        block(backend.remove(session.id)).unwrap();
        assert!(block(backend.list_agents()).unwrap().is_empty());
        assert!(
            terminals.lock().unwrap().is_empty(),
            "terminal is closed too"
        );
    }

    #[test]
    fn unknown_agent_ids_are_errors_not_panics() {
        let (backend, _terminals, _rx) = backend();
        let missing = AgentId::next();
        assert!(block(backend.send_input(missing, b"x")).is_err());
        assert!(block(backend.snapshot(missing)).is_err());
        assert!(
            block(backend.remove(missing)).is_ok(),
            "remove is idempotent"
        );
    }

    #[test]
    fn presets_produce_labelled_requests() {
        let preset = crate::config::AgentPreset {
            label: None,
            command: "claude".into(),
            args: vec!["--help".into()],
            env: Default::default(),
        };
        let request = request_from_preset("claude", &preset, std::env::temp_dir());
        assert_eq!(request.kind, AgentKind::Claude);
        assert_eq!(request.label, "Claude");

        let custom = crate::config::AgentPreset {
            label: None,
            command: "cargo".into(),
            args: vec!["test".into()],
            env: Default::default(),
        };
        let request = request_from_preset("tests", &custom, std::env::temp_dir());
        assert_eq!(request.label, "tests");
    }
}
