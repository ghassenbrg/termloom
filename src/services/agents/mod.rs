//! Agent sessions and the backend abstraction.
//!
//! The workbench never talks to a PTY or to Herdr directly: it talks to an
//! [`AgentBackend`]. `LocalAgentBackend` supervises processes this instance
//! spawned; `HerdrAgentBackend` proxies panes owned by a Herdr session. Both
//! produce the same [`AgentSession`] values, so no UI code needs to know which
//! one is active beyond displaying the backend name.

pub mod detect;
pub mod herdr;
pub mod local;

use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;

use crate::domain::agent::{AgentBackendKind, AgentKind, AgentSession};
use crate::domain::ids::AgentId;

pub use herdr::HerdrAgentBackend;
pub use local::LocalAgentBackend;

/// What to launch.
#[derive(Debug, Clone)]
pub struct SpawnAgentRequest {
    pub kind: AgentKind,
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// Initial grid; the workbench resizes the pane immediately afterwards.
    pub rows: u16,
    pub cols: u16,
}

impl SpawnAgentRequest {
    pub fn new(
        kind: AgentKind,
        label: impl Into<String>,
        program: impl Into<String>,
        cwd: PathBuf,
    ) -> Self {
        SpawnAgentRequest {
            kind,
            label: label.into(),
            program: program.into(),
            args: Vec::new(),
            cwd,
            env: Vec::new(),
            rows: 24,
            cols: 80,
        }
    }
}

/// Read-only text view of an agent's terminal.
#[derive(Debug, Clone, Default)]
pub struct TerminalSnapshot {
    pub lines: Vec<String>,
    /// True when the backend cannot provide output at all.
    pub unavailable: bool,
}

/// Capabilities a backend may or may not provide. The UI greys out actions
/// instead of failing at call time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendCapabilities {
    pub spawn: bool,
    pub send_input: bool,
    pub stop: bool,
    pub restart: bool,
    pub snapshot: bool,
    /// The backend renders its own terminal inside TermLoom.
    pub embedded_terminal: bool,
}

impl BackendCapabilities {
    pub const NONE: BackendCapabilities = BackendCapabilities {
        spawn: false,
        send_input: false,
        stop: false,
        restart: false,
        snapshot: false,
        embedded_terminal: false,
    };
}

/// Uniform control surface over agent sessions.
#[async_trait]
pub trait AgentBackend: Send + Sync {
    /// Which backend this is, for display.
    fn backend_kind(&self) -> AgentBackendKind;

    /// What this backend can do.
    fn capabilities(&self) -> BackendCapabilities;

    /// Human-readable status (`connected to herdr 0.4`, `local`).
    fn status(&self) -> String {
        self.backend_kind().label().to_string()
    }

    /// Every session this backend knows about.
    async fn list_agents(&self) -> Result<Vec<AgentSession>>;

    /// Launch a new agent.
    async fn spawn_agent(&self, request: SpawnAgentRequest) -> Result<AgentSession>;

    /// Send raw bytes to the agent's terminal.
    async fn send_input(&self, id: AgentId, input: &[u8]) -> Result<()>;

    /// Terminate the agent's process.
    async fn stop(&self, id: AgentId) -> Result<()>;

    /// Re-run the agent's original command.
    async fn restart(&self, id: AgentId) -> Result<AgentSession>;

    /// Change the display label.
    async fn rename(&self, id: AgentId, label: &str) -> Result<()>;

    /// Ask an external backend to focus its terminal. Local sessions are
    /// focused directly by the workbench through their `terminal_id`.
    async fn focus(&self, _id: AgentId) -> Result<()> {
        anyhow::bail!("this backend cannot focus an external terminal")
    }

    /// Forget a finished session.
    async fn remove(&self, id: AgentId) -> Result<()>;

    /// Recent output for the detail panel.
    async fn snapshot(&self, id: AgentId) -> Result<TerminalSnapshot>;

    /// Recompute session states. Called on the app's refresh tick.
    async fn refresh(&self) -> Result<()> {
        Ok(())
    }

    /// True when this backend owns the given session.
    async fn owns(&self, id: AgentId) -> bool {
        self.list_agents()
            .await
            .map(|agents| agents.iter().any(|a| a.id == id))
            .unwrap_or(false)
    }
}

/// Aggregates the active backends and routes calls to the owning one.
#[derive(Clone)]
pub struct AgentRegistry {
    backends: Vec<std::sync::Arc<dyn AgentBackend>>,
}

impl AgentRegistry {
    pub fn new() -> AgentRegistry {
        AgentRegistry {
            backends: Vec::new(),
        }
    }

    pub fn push(&mut self, backend: std::sync::Arc<dyn AgentBackend>) {
        self.backends.push(backend);
    }

    /// Remove a backend (used when Herdr disconnects).
    pub fn remove_kind(&mut self, kind: AgentBackendKind) {
        self.backends.retain(|b| b.backend_kind() != kind);
    }

    pub fn has_kind(&self, kind: AgentBackendKind) -> bool {
        self.backends.iter().any(|b| b.backend_kind() == kind)
    }

    pub fn backends(&self) -> &[std::sync::Arc<dyn AgentBackend>] {
        &self.backends
    }

    /// The backend used for new sessions (the first that can spawn).
    pub fn primary(&self) -> Option<&std::sync::Arc<dyn AgentBackend>> {
        self.backends.iter().find(|b| b.capabilities().spawn)
    }

    /// The backend that owns a session.
    pub async fn owner(&self, id: AgentId) -> Option<&std::sync::Arc<dyn AgentBackend>> {
        for backend in &self.backends {
            if backend.owns(id).await {
                return Some(backend);
            }
        }
        None
    }

    /// Every session from every backend, sorted by start time.
    pub async fn list_all(&self) -> Vec<AgentSession> {
        let mut out = Vec::new();
        for backend in &self.backends {
            match backend.list_agents().await {
                Ok(agents) => out.extend(agents),
                Err(err) => {
                    tracing::warn!(backend = %backend.backend_kind().label(), error = %err, "listing agents failed");
                }
            }
        }
        out.sort_by_key(|a| a.started_at);
        out
    }

    /// Refresh every backend, logging (not propagating) failures so one broken
    /// backend cannot stall the workbench.
    pub async fn refresh_all(&self) {
        for backend in &self.backends {
            if let Err(err) = backend.refresh().await {
                tracing::warn!(backend = %backend.backend_kind().label(), error = %err, "agent refresh failed");
            }
        }
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        AgentRegistry::new()
    }
}
