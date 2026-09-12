//! Optional Herdr-backed agent discovery and control.
//!
//! Herdr remains an independent program. This adapter uses its documented
//! CLI/JSON surface and degrades cleanly when the binary or server disappears.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::process::Command;

use crate::domain::agent::{
    AgentBackendKind, AgentKind, AgentSession, AgentState, ObservationSource,
};
use crate::domain::ids::AgentId;

use super::{AgentBackend, BackendCapabilities, SpawnAgentRequest, TerminalSnapshot};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Default)]
struct Cache {
    sessions: Vec<AgentSession>,
    ids: HashMap<String, AgentId>,
    revisions: HashMap<String, (u64, u64)>,
    hidden: HashSet<String>,
    last_refresh: Option<Instant>,
}

/// A connected Herdr CLI/server pair.
pub struct HerdrAgentBackend {
    command: String,
    version: String,
    cache: Mutex<Cache>,
}

impl HerdrAgentBackend {
    /// Probe the configured binary and running server, then load its agents.
    pub async fn connect(command: Option<&str>) -> Result<HerdrAgentBackend> {
        let command = command.unwrap_or("herdr").to_string();
        let status = run_json(&command, &["status", "--json"]).await?;
        let running = status
            .pointer("/server/running")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let compatible = status
            .pointer("/server/compatible")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if !running {
            return Err(anyhow!("Herdr server is not running"));
        }
        if !compatible {
            return Err(anyhow!(
                "Herdr client and server protocols are incompatible"
            ));
        }
        let version = status
            .pointer("/server/version")
            .or_else(|| status.pointer("/client/version"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let backend = HerdrAgentBackend {
            command,
            version,
            cache: Mutex::new(Cache::default()),
        };
        backend.refresh_now().await?;
        Ok(backend)
    }

    async fn refresh_now(&self) -> Result<()> {
        let value = run_json(&self.command, &["agent", "list"]).await?;
        let rows = value
            .pointer("/result/agents")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("Herdr agent list returned an unexpected response"))?;
        let now = SystemTime::now();
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow!("Herdr cache poisoned"))?;
        let old: HashMap<AgentId, AgentSession> = cache
            .sessions
            .drain(..)
            .map(|session| (session.id, session))
            .collect();
        let mut sessions = Vec::new();
        for row in rows {
            let Some(pane_id) = row.get("pane_id").and_then(Value::as_str) else {
                continue;
            };
            if cache.hidden.contains(pane_id) {
                continue;
            }
            let id = *cache
                .ids
                .entry(pane_id.to_string())
                .or_insert_with(AgentId::next);
            let previous = old.get(&id);
            let revision = row.get("revision").and_then(Value::as_u64).unwrap_or(0);
            let state_sequence = row
                .get("state_change_seq")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let changed = cache
                .revisions
                .insert(pane_id.to_string(), (revision, state_sequence))
                .is_none_or(|prior| prior != (revision, state_sequence));
            let agent_name = row.get("agent").and_then(Value::as_str).unwrap_or("agent");
            let kind = AgentKind::from_command(agent_name);
            let label = row
                .get("display_agent")
                .or_else(|| row.get("name"))
                .or_else(|| row.get("terminal_title_stripped"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| previous.map(|session| session.label.clone()))
                .unwrap_or_else(|| kind.label().to_string());
            let backend_session_id = row
                .pointer("/agent_session/value")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| Some(pane_id.to_string()));
            sessions.push(AgentSession {
                id,
                kind,
                label,
                terminal_id: None,
                cwd: row
                    .get("foreground_cwd")
                    .or_else(|| row.get("cwd"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .unwrap_or_default(),
                command: vec![agent_name.to_string()],
                state: map_state(
                    row.get("agent_status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown"),
                ),
                state_source: ObservationSource::Backend,
                confidence: 0.95,
                task_summary: row
                    .pointer("/tokens/summary")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                started_at: previous.map_or(now, |session| session.started_at),
                last_activity_at: if changed {
                    now
                } else {
                    previous.map_or(now, |session| session.last_activity_at)
                },
                backend: AgentBackendKind::Herdr,
                backend_session_id,
                tasks: previous
                    .map(|session| session.tasks.clone())
                    .unwrap_or_default(),
            });
        }
        cache.sessions = sessions;
        cache.last_refresh = Some(Instant::now());
        Ok(())
    }

    fn pane_for(&self, id: AgentId) -> Result<String> {
        self.cache
            .lock()
            .map_err(|_| anyhow!("Herdr cache poisoned"))?
            .ids
            .iter()
            .find_map(|(pane, candidate)| (*candidate == id).then(|| pane.clone()))
            .ok_or_else(|| anyhow!("unknown Herdr agent {id}"))
    }

    fn session_for(&self, id: AgentId) -> Result<AgentSession> {
        self.cache
            .lock()
            .map_err(|_| anyhow!("Herdr cache poisoned"))?
            .sessions
            .iter()
            .find(|session| session.id == id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown Herdr agent {id}"))
    }
}

#[async_trait]
impl AgentBackend for HerdrAgentBackend {
    fn backend_kind(&self) -> AgentBackendKind {
        AgentBackendKind::Herdr
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            // Local spawning stays the default. Discovery/control does not
            // create or steal a pane in the user's Herdr layout.
            spawn: false,
            send_input: true,
            stop: true,
            restart: true,
            snapshot: true,
            embedded_terminal: false,
        }
    }

    fn status(&self) -> String {
        format!("connected to Herdr {}", self.version)
    }

    async fn list_agents(&self) -> Result<Vec<AgentSession>> {
        Ok(self
            .cache
            .lock()
            .map_err(|_| anyhow!("Herdr cache poisoned"))?
            .sessions
            .clone())
    }

    async fn spawn_agent(&self, _request: SpawnAgentRequest) -> Result<AgentSession> {
        Err(anyhow!(
            "Herdr spawning is unavailable from this context; local PTY spawning remains active"
        ))
    }

    async fn send_input(&self, id: AgentId, input: &[u8]) -> Result<()> {
        let pane = self.pane_for(id)?;
        let text = String::from_utf8_lossy(input)
            .trim_end_matches(['\r', '\n'])
            .to_string();
        run(&self.command, &["agent", "prompt", &pane, &text]).await?;
        Ok(())
    }

    async fn stop(&self, id: AgentId) -> Result<()> {
        let pane = self.pane_for(id)?;
        run(&self.command, &["agent", "send-keys", &pane, "ctrl+c"]).await?;
        Ok(())
    }

    async fn restart(&self, id: AgentId) -> Result<AgentSession> {
        let session = self.session_for(id)?;
        let pane = self.pane_for(id)?;
        let kind = match session.kind {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::OpenCode => "opencode",
            _ => return Err(anyhow!("this Herdr agent kind cannot be restarted safely")),
        };
        self.stop(id).await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let name = format!("termloom-{}", id.raw());
        run(
            &self.command,
            &["agent", "start", &name, "--kind", kind, "--pane", &pane],
        )
        .await?;
        self.refresh_now().await?;
        self.session_for(id)
    }

    async fn rename(&self, id: AgentId, label: &str) -> Result<()> {
        let pane = self.pane_for(id)?;
        run(&self.command, &["agent", "rename", &pane, label]).await?;
        self.refresh_now().await
    }

    async fn focus(&self, id: AgentId) -> Result<()> {
        let pane = self.pane_for(id)?;
        run(&self.command, &["agent", "focus", &pane]).await?;
        Ok(())
    }

    async fn remove(&self, id: AgentId) -> Result<()> {
        let pane = self.pane_for(id)?;
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow!("Herdr cache poisoned"))?;
        cache.hidden.insert(pane);
        cache.sessions.retain(|session| session.id != id);
        Ok(())
    }

    async fn snapshot(&self, id: AgentId) -> Result<TerminalSnapshot> {
        let pane = self.pane_for(id)?;
        let output = run(
            &self.command,
            &[
                "agent",
                "read",
                &pane,
                "--source",
                "recent-unwrapped",
                "--lines",
                "200",
                "--format",
                "text",
            ],
        )
        .await?;
        Ok(TerminalSnapshot {
            lines: output.lines().map(str::to_string).collect(),
            unavailable: false,
        })
    }

    async fn refresh(&self) -> Result<()> {
        let due = self
            .cache
            .lock()
            .map_err(|_| anyhow!("Herdr cache poisoned"))?
            .last_refresh
            .is_none_or(|last| last.elapsed() >= REFRESH_INTERVAL);
        if due {
            self.refresh_now().await?;
        }
        Ok(())
    }
}

fn map_state(state: &str) -> AgentState {
    match state.to_ascii_lowercase().as_str() {
        "starting" => AgentState::Starting,
        "working" => AgentState::Working,
        "blocked" | "waiting" | "waiting_for_input" => AgentState::WaitingForInput,
        "idle" => AgentState::Idle,
        "done" => AgentState::Done,
        "failed" => AgentState::Failed,
        "exited" => AgentState::Exited,
        _ => AgentState::Unknown,
    }
}

async fn run_json(command: &str, args: &[&str]) -> Result<Value> {
    let output = run(command, args).await?;
    serde_json::from_str(&output).context("parsing Herdr JSON response")
}

async fn run(command: &str, args: &[&str]) -> Result<String> {
    let output = tokio::time::timeout(COMMAND_TIMEOUT, Command::new(command).args(args).output())
        .await
        .map_err(|_| anyhow!("Herdr command timed out"))?
        .with_context(|| format!("starting {command}"))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Herdr command failed: {}", message.trim()));
    }
    String::from_utf8(output.stdout).context("Herdr returned non-UTF-8 output")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_every_documented_state() {
        assert_eq!(map_state("working"), AgentState::Working);
        assert_eq!(map_state("blocked"), AgentState::WaitingForInput);
        assert_eq!(map_state("idle"), AgentState::Idle);
        assert_eq!(map_state("done"), AgentState::Done);
        assert_eq!(map_state("something-new"), AgentState::Unknown);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn adapter_maps_and_controls_a_fake_cli() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("herdr-fake");
        let log = dir.path().join("calls");
        let body = format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> '{}'
if [ "$1" = status ]; then
  printf '%s\n' '{{"client":{{"version":"0.9"}},"server":{{"running":true,"compatible":true,"version":"0.9"}}}}'
elif [ "$1 $2" = 'agent list' ]; then
  printf '%s\n' '{{"result":{{"agents":[{{"agent":"codex","agent_status":"blocked","cwd":"/tmp/project","pane_id":"w1:p2","revision":3,"state_change_seq":4}}]}}}}'
elif [ "$1 $2" = 'agent read' ]; then
  printf 'recent output\nsecond line\n'
else
  printf '%s\n' '{{"result":{{}}}}'
fi
"#,
            log.display()
        );
        std::fs::write(&script, body).unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();

        let backend = HerdrAgentBackend::connect(script.to_str()).await.unwrap();
        assert_eq!(backend.status(), "connected to Herdr 0.9");
        let agents = backend.list_agents().await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].kind, AgentKind::Codex);
        assert_eq!(agents[0].state, AgentState::WaitingForInput);
        let id = agents[0].id;
        backend.send_input(id, b"continue\r").await.unwrap();
        backend.focus(id).await.unwrap();
        backend.stop(id).await.unwrap();
        let snapshot = backend.snapshot(id).await.unwrap();
        assert_eq!(snapshot.lines, ["recent output", "second line"]);
        let calls = std::fs::read_to_string(log).unwrap();
        assert!(calls.contains("agent prompt w1:p2 continue"));
        assert!(calls.contains("agent focus w1:p2"));
        assert!(calls.contains("agent send-keys w1:p2 ctrl+c"));
    }
}
