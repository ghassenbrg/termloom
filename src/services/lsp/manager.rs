//! Routes documents and requests to the right language server.
//!
//! Servers are started lazily, one per configured entry, and only when a file
//! of a language they declare is opened. A crashed server is reported and can
//! be restarted; it never takes the workbench down with it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::config::{Config, LspServerConfig};
use crate::domain::diagnostics::Position;
use crate::services::syntax::LanguageId;

use super::client::{ClientStatus, LspClient, LspSink, RequestContext, RequestKind};
use super::protocol::{self, ServerCapabilities};

/// A row for the status panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerStatus {
    pub name: String,
    pub languages: Vec<String>,
    pub status: ClientStatus,
    /// `None` until the server is configured but not started.
    pub running: bool,
    pub command: String,
}

/// Owns every language server client.
pub struct LspManager {
    clients: HashMap<String, LspClient>,
    configs: HashMap<String, LspServerConfig>,
    root: PathBuf,
    sink: LspSink,
    /// Servers that failed to start, with the reason, so the UI can explain.
    failures: HashMap<String, String>,
}

impl LspManager {
    pub fn new(config: &Config, root: &Path, sink: LspSink) -> LspManager {
        LspManager {
            clients: HashMap::new(),
            configs: config
                .lsp
                .iter()
                .map(|(name, config)| (name.clone(), config.clone()))
                .collect(),
            root: root.to_path_buf(),
            sink,
            failures: HashMap::new(),
        }
    }

    /// Replace the configuration (after `workspace.reload`).
    pub fn set_config(&mut self, config: &Config) {
        self.configs = config
            .lsp
            .iter()
            .map(|(name, config)| (name.clone(), config.clone()))
            .collect();
    }

    /// Name of the configured server for a language, if any.
    pub fn server_for(&self, language: &LanguageId) -> Option<String> {
        self.configs
            .iter()
            .find(|(_, config)| config.languages.iter().any(|l| l == language.as_str()))
            .map(|(name, _)| name.clone())
    }

    /// Is any server ready?
    pub fn any_ready(&self) -> bool {
        self.clients
            .values()
            .any(|client| client.status() == ClientStatus::Ready)
    }

    /// Capabilities of the server handling a language.
    pub fn capabilities_for(&self, language: &LanguageId) -> Option<ServerCapabilities> {
        let name = self.server_for(language)?;
        self.clients.get(&name)?.capabilities()
    }

    /// Rows for the status panel: configured servers plus their live state.
    pub fn statuses(&self) -> Vec<ServerStatus> {
        let mut rows: Vec<ServerStatus> = self
            .configs
            .iter()
            .map(|(name, config)| {
                let client = self.clients.get(name);
                ServerStatus {
                    name: name.clone(),
                    languages: config.languages.clone(),
                    status: client
                        .map(LspClient::status)
                        .unwrap_or(ClientStatus::Exited),
                    running: client.is_some(),
                    command: config.command.clone(),
                }
            })
            .collect();
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        rows
    }

    /// Why a server could not start.
    pub fn failure(&self, name: &str) -> Option<&String> {
        self.failures.get(name)
    }

    /// Start the server for a language if it is configured and not running.
    pub fn ensure_started(&mut self, language: &LanguageId) -> Result<String> {
        let name = self
            .server_for(language)
            .ok_or_else(|| anyhow!("no language server configured for {language}"))?;
        if let Some(client) = self.clients.get_mut(&name) {
            if client.is_alive() {
                return Ok(name);
            }
            self.clients.remove(&name);
        }
        let config = self.configs.get(&name).cloned().unwrap();
        if crate::app::actions::which(&config.command).is_none() {
            let message = format!("`{}` was not found on PATH", config.command);
            self.failures.insert(name.clone(), message.clone());
            return Err(anyhow!(message));
        }
        let root = self.project_root(&config);
        match LspClient::start(&name, &config, &root, self.sink.clone()) {
            Ok(client) => {
                self.failures.remove(&name);
                self.clients.insert(name.clone(), client);
                Ok(name)
            }
            Err(err) => {
                let message = format!("{err:#}");
                self.failures.insert(name.clone(), message.clone());
                Err(anyhow!(message))
            }
        }
    }

    /// Nearest ancestor containing one of the server's root markers.
    fn project_root(&self, config: &LspServerConfig) -> PathBuf {
        if config.root_markers.is_empty() {
            return self.root.clone();
        }
        let mut current = Some(self.root.as_path());
        while let Some(dir) = current {
            if config
                .root_markers
                .iter()
                .any(|marker| dir.join(marker).exists())
            {
                return dir.to_path_buf();
            }
            current = dir.parent();
        }
        self.root.clone()
    }

    /// Whether a server should start automatically for this language.
    pub fn auto_start(&self, language: &LanguageId) -> bool {
        self.server_for(language)
            .and_then(|name| self.configs.get(&name))
            .map(|config| config.auto_start)
            .unwrap_or(false)
    }

    fn client_for(&mut self, language: &LanguageId) -> Option<&mut LspClient> {
        let name = self.server_for(language)?;
        self.clients.get_mut(&name)
    }

    // ── document lifecycle ────────────────────────────────────────────────

    pub fn did_open(&mut self, path: &Path, language: &LanguageId, text: &str) {
        let language_id = language.as_str().to_string();
        if let Some(client) = self.client_for(language) {
            if !client.is_open(path) {
                if let Err(err) = client.did_open(path, &language_id, text) {
                    tracing::warn!(error = %err, "didOpen failed");
                }
            }
        }
    }

    pub fn did_change(&mut self, path: &Path, language: &LanguageId, text: &str) {
        if let Some(client) = self.client_for(language) {
            if client.is_open(path) {
                if let Err(err) = client.did_change(path, text) {
                    tracing::warn!(error = %err, "didChange failed");
                }
            }
        }
    }

    pub fn did_save(&mut self, path: &Path, language: &LanguageId, text: &str) {
        if let Some(client) = self.client_for(language) {
            if client.is_open(path) {
                if let Err(err) = client.did_save(path, Some(text)) {
                    tracing::warn!(error = %err, "didSave failed");
                }
            }
        }
    }

    pub fn did_close(&mut self, path: &Path, language: &LanguageId) {
        if let Some(client) = self.client_for(language) {
            if client.is_open(path) {
                let _ = client.did_close(path);
            }
        }
    }

    // ── requests ──────────────────────────────────────────────────────────

    /// Send a position-based request, refusing when the server does not
    /// advertise the capability.
    pub fn request(
        &mut self,
        language: &LanguageId,
        kind: RequestKind,
        path: &Path,
        position: Position,
        extra: RequestExtra,
    ) -> Result<()> {
        let Some(client) = self.client_for(language) else {
            return Err(anyhow!("no language server running for {language}"));
        };
        let capabilities = client
            .capabilities()
            .ok_or_else(|| anyhow!("language server is still starting"))?;

        let supported = match kind {
            RequestKind::Hover => capabilities.hover,
            RequestKind::Completion => capabilities.completion,
            RequestKind::Definition => capabilities.definition,
            RequestKind::References => capabilities.references,
            RequestKind::DocumentSymbols => capabilities.document_symbols,
            RequestKind::WorkspaceSymbols => capabilities.workspace_symbols,
            RequestKind::Rename => capabilities.rename,
            RequestKind::Formatting => capabilities.formatting,
        };
        if !supported {
            return Err(anyhow!("{} does not support {}", client.name, kind.label()));
        }

        let (method, params) = match (kind, &extra) {
            (RequestKind::Hover, _) => (
                "textDocument/hover",
                protocol::text_document_position(path, position),
            ),
            (RequestKind::Completion, _) => (
                "textDocument/completion",
                protocol::text_document_position(path, position),
            ),
            (RequestKind::Definition, _) => (
                "textDocument/definition",
                protocol::text_document_position(path, position),
            ),
            (RequestKind::References, _) => (
                "textDocument/references",
                protocol::references_params(path, position, true),
            ),
            (RequestKind::DocumentSymbols, _) => (
                "textDocument/documentSymbol",
                serde_json::json!({ "textDocument": { "uri": protocol::path_to_uri(path) } }),
            ),
            (RequestKind::WorkspaceSymbols, RequestExtra::Query(query)) => {
                ("workspace/symbol", serde_json::json!({ "query": query }))
            }
            (RequestKind::WorkspaceSymbols, _) => {
                ("workspace/symbol", serde_json::json!({ "query": "" }))
            }
            (RequestKind::Rename, RequestExtra::NewName(name)) => (
                "textDocument/rename",
                protocol::rename_params(path, position, name),
            ),
            (RequestKind::Rename, _) => return Err(anyhow!("rename needs a new name")),
            (RequestKind::Formatting, RequestExtra::Formatting { tab_size, spaces }) => (
                "textDocument/formatting",
                protocol::formatting_params(path, *tab_size, *spaces),
            ),
            (RequestKind::Formatting, _) => (
                "textDocument/formatting",
                protocol::formatting_params(path, 4, true),
            ),
        };

        client.request(
            kind,
            method,
            params,
            RequestContext {
                path: path.to_path_buf(),
                position,
            },
        )?;
        Ok(())
    }

    /// Restart a server by name.
    pub fn restart(&mut self, name: &str) -> Result<()> {
        if let Some(mut client) = self.clients.remove(name) {
            client.shutdown();
        }
        let config = self
            .configs
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow!("no server named {name}"))?;
        let root = self.project_root(&config);
        let client = LspClient::start(name, &config, &root, self.sink.clone())?;
        self.clients.insert(name.to_string(), client);
        self.failures.remove(name);
        Ok(())
    }

    /// Stop a server by name.
    pub fn stop(&mut self, name: &str) {
        if let Some(mut client) = self.clients.remove(name) {
            client.shutdown();
        }
    }

    /// Names of running servers.
    pub fn running(&self) -> Vec<String> {
        self.clients.keys().cloned().collect()
    }

    /// Stop everything (called at shutdown).
    pub fn shutdown_all(&mut self) {
        for (_, mut client) in self.clients.drain() {
            client.shutdown();
        }
    }

    /// Mark a crashed server so a later request can restart it.
    pub fn forget(&mut self, name: &str) {
        self.clients.remove(name);
    }
}

/// Extra data some requests need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestExtra {
    None,
    Query(String),
    NewName(String),
    Formatting { tab_size: usize, spaces: bool },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn manager(config: Config) -> LspManager {
        let sink: LspSink = Arc::new(|_| {});
        LspManager::new(&config, Path::new("/tmp"), sink)
    }

    #[test]
    fn maps_languages_to_configured_servers() {
        let manager = manager(Config::with_builtin_defaults());
        assert_eq!(
            manager.server_for(&LanguageId::new("rust")).as_deref(),
            Some("rust-analyzer")
        );
        assert_eq!(manager.server_for(&LanguageId::new("dart")), None);
    }

    #[test]
    fn statuses_list_configured_servers_even_when_not_running() {
        let manager = manager(Config::with_builtin_defaults());
        let statuses = manager.statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].name, "rust-analyzer");
        assert!(!statuses[0].running);
    }

    #[test]
    fn missing_binaries_are_reported_not_panicked() {
        let mut config = Config::default();
        config.lsp.insert(
            "fake".into(),
            LspServerConfig {
                command: "definitely-not-a-language-server".into(),
                languages: vec!["rust".into()],
                ..Default::default()
            },
        );
        let mut manager = manager(config);
        let err = manager
            .ensure_started(&LanguageId::new("rust"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not found on PATH"), "{err}");
        assert!(manager.failure("fake").is_some());
    }

    #[test]
    fn requests_without_a_client_are_refused() {
        let mut manager = manager(Config::with_builtin_defaults());
        let err = manager
            .request(
                &LanguageId::new("rust"),
                RequestKind::Hover,
                Path::new("/tmp/a.rs"),
                Position::new(0, 0),
                RequestExtra::None,
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("no language server running"), "{err}");
    }

    #[test]
    fn root_markers_walk_up_from_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let nested = dir.path().join("crates/inner");
        std::fs::create_dir_all(&nested).unwrap();

        let sink: LspSink = Arc::new(|_| {});
        let manager = LspManager::new(&Config::with_builtin_defaults(), &nested, sink);
        let config = LspServerConfig {
            root_markers: vec!["Cargo.toml".into()],
            ..Default::default()
        };
        assert_eq!(
            std::fs::canonicalize(manager.project_root(&config)).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
    }

    #[test]
    fn auto_start_follows_the_configuration() {
        let mut config = Config::with_builtin_defaults();
        assert!(manager(config.clone()).auto_start(&LanguageId::new("rust")));
        config.lsp.get_mut("rust-analyzer").unwrap().auto_start = false;
        assert!(!manager(config).auto_start(&LanguageId::new("rust")));
    }
}
