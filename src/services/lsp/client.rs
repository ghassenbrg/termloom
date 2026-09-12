//! A language server process and its JSON-RPC session.
//!
//! One client owns one child process. Requests are asynchronous: callers get
//! back a request id, and the answer arrives later as an [`LspEvent`] on the
//! sink, so a slow server can never block the UI thread.

use std::collections::HashMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::config::LspServerConfig;
use crate::domain::diagnostics::{Diagnostic, Location, Position, Symbol};

use super::framing;
use super::protocol::{self, CompletionItem, ServerCapabilities, TextEdit};

/// Which request a response belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Hover,
    Completion,
    Definition,
    References,
    DocumentSymbols,
    WorkspaceSymbols,
    Rename,
    Formatting,
}

impl RequestKind {
    pub fn label(self) -> &'static str {
        match self {
            RequestKind::Hover => "hover",
            RequestKind::Completion => "completion",
            RequestKind::Definition => "definition",
            RequestKind::References => "references",
            RequestKind::DocumentSymbols => "document symbols",
            RequestKind::WorkspaceSymbols => "workspace symbols",
            RequestKind::Rename => "rename",
            RequestKind::Formatting => "formatting",
        }
    }
}

/// A converted response payload.
#[derive(Debug, Clone)]
pub enum LspResult {
    Hover(Vec<String>),
    Completion(Vec<CompletionItem>),
    Locations(Vec<Location>),
    Symbols(Vec<Symbol>),
    Edits(Vec<(PathBuf, Vec<TextEdit>)>),
    Empty,
}

/// Everything a client tells the application.
#[derive(Debug, Clone)]
pub enum LspEvent {
    Initialized {
        server: String,
        capabilities: Box<ServerCapabilities>,
    },
    Diagnostics {
        server: String,
        path: PathBuf,
        diagnostics: Vec<Diagnostic>,
    },
    Response {
        server: String,
        kind: RequestKind,
        /// Where the request was made, so popups can be placed correctly.
        context: RequestContext,
        result: LspResult,
    },
    RequestFailed {
        server: String,
        kind: RequestKind,
        message: String,
    },
    /// Server-reported progress or log message worth surfacing.
    Status {
        server: String,
        message: String,
    },
    Exited {
        server: String,
        status: Option<i32>,
        /// True when the process died without us asking it to.
        crashed: bool,
    },
}

/// Where a request came from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RequestContext {
    pub path: PathBuf,
    pub position: Position,
}

/// Callback used to publish [`LspEvent`]s.
pub type LspSink = Arc<dyn Fn(LspEvent) + Send + Sync>;

/// Lifecycle of a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientStatus {
    Starting,
    Ready,
    Failed,
    Exited,
}

impl ClientStatus {
    pub fn label(self) -> &'static str {
        match self {
            ClientStatus::Starting => "starting",
            ClientStatus::Ready => "running",
            ClientStatus::Failed => "failed",
            ClientStatus::Exited => "exited",
        }
    }
}

struct Pending {
    kind: RequestKind,
    context: RequestContext,
}

/// A running language server.
pub struct LspClient {
    pub name: String,
    pub root: PathBuf,
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: AtomicI64,
    pending: Arc<Mutex<HashMap<i64, Pending>>>,
    capabilities: Arc<Mutex<Option<ServerCapabilities>>>,
    status: Arc<Mutex<ClientStatus>>,
    shutting_down: Arc<AtomicBool>,
    open_documents: HashMap<PathBuf, i64>,
    config: LspServerConfig,
}

impl LspClient {
    /// Spawn the server and send `initialize`.
    pub fn start(
        name: &str,
        config: &LspServerConfig,
        root: &Path,
        sink: LspSink,
    ) -> Result<LspClient> {
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &config.env {
            command.env(key, value);
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("starting language server `{}`", config.command))?;

        let stdin = child.stdin.take().context("language server stdin")?;
        let stdout = child.stdout.take().context("language server stdout")?;
        let stderr = child.stderr.take().context("language server stderr")?;

        let stdin = Arc::new(Mutex::new(stdin));
        let pending: Arc<Mutex<HashMap<i64, Pending>>> = Arc::new(Mutex::new(HashMap::new()));
        let capabilities = Arc::new(Mutex::new(None));
        let status = Arc::new(Mutex::new(ClientStatus::Starting));
        let shutting_down = Arc::new(AtomicBool::new(false));

        spawn_reader(
            name.to_string(),
            BufReader::new(stdout),
            Arc::clone(&stdin),
            Arc::clone(&pending),
            Arc::clone(&capabilities),
            Arc::clone(&status),
            Arc::clone(&shutting_down),
            sink.clone(),
        );
        spawn_stderr_logger(name.to_string(), stderr);

        let client = LspClient {
            name: name.to_string(),
            root: root.to_path_buf(),
            child,
            stdin,
            next_id: AtomicI64::new(1),
            pending,
            capabilities,
            status,
            shutting_down,
            open_documents: HashMap::new(),
            config: config.clone(),
        };

        let options = config
            .initialization_options
            .clone()
            .map(toml_to_json)
            .filter(|value| !value.is_null());
        let params = protocol::initialize_params(root, options);
        client.send_request_raw(0, "initialize", params)?;
        Ok(client)
    }

    pub fn status(&self) -> ClientStatus {
        *self.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn capabilities(&self) -> Option<ServerCapabilities> {
        *self.capabilities.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn config(&self) -> &LspServerConfig {
        &self.config
    }

    /// Languages this server was configured for.
    pub fn languages(&self) -> &[String] {
        &self.config.languages
    }

    /// Whether the process is still alive.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn next_id(&self) -> i64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn send(&self, message: Value) -> Result<()> {
        let mut stdin = self
            .stdin
            .lock()
            .map_err(|_| anyhow::anyhow!("language server stdin lock poisoned"))?;
        framing::write_message(&mut *stdin, &message)
    }

    fn send_request_raw(&self, id: i64, method: &str, params: Value) -> Result<()> {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
    }

    /// Send a notification.
    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send(json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
    }

    /// Send a request; the response arrives on the sink.
    pub fn request(
        &self,
        kind: RequestKind,
        method: &str,
        params: Value,
        context: RequestContext,
    ) -> Result<i64> {
        let id = self.next_id();
        self.pending
            .lock()
            .map_err(|_| anyhow::anyhow!("pending map poisoned"))?
            .insert(id, Pending { kind, context });
        self.send_request_raw(id, method, params)?;
        Ok(id)
    }

    // ── document synchronisation ──────────────────────────────────────────

    pub fn did_open(&mut self, path: &Path, language_id: &str, text: &str) -> Result<()> {
        let version = 1;
        self.open_documents.insert(path.to_path_buf(), version);
        self.notify(
            "textDocument/didOpen",
            protocol::did_open(path, language_id, version, text),
        )
    }

    pub fn did_change(&mut self, path: &Path, text: &str) -> Result<()> {
        let version = self
            .open_documents
            .get(path)
            .copied()
            .unwrap_or(1)
            .saturating_add(1);
        self.open_documents.insert(path.to_path_buf(), version);
        self.notify(
            "textDocument/didChange",
            protocol::did_change_full(path, version, text),
        )
    }

    pub fn did_save(&self, path: &Path, text: Option<&str>) -> Result<()> {
        self.notify("textDocument/didSave", protocol::did_save(path, text))
    }

    pub fn did_close(&mut self, path: &Path) -> Result<()> {
        self.open_documents.remove(path);
        self.notify("textDocument/didClose", protocol::did_close(path))
    }

    pub fn is_open(&self, path: &Path) -> bool {
        self.open_documents.contains_key(path)
    }

    /// Politely shut the server down, then make sure it is gone.
    pub fn shutdown(&mut self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let id = self.next_id();
        let _ = self.send_request_raw(id, "shutdown", Value::Null);
        let _ = self.notify("exit", Value::Null);

        // Give it a moment to exit on its own before killing it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
        while std::time::Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                *self.status.lock().unwrap_or_else(|e| e.into_inner()) = ClientStatus::Exited;
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let _ = self.child.kill();
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = ClientStatus::Exited;
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let _ = self.child.kill();
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_reader(
    name: String,
    mut reader: BufReader<std::process::ChildStdout>,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<i64, Pending>>>,
    capabilities: Arc<Mutex<Option<ServerCapabilities>>>,
    status: Arc<Mutex<ClientStatus>>,
    shutting_down: Arc<AtomicBool>,
    sink: LspSink,
) {
    std::thread::Builder::new()
        .name(format!("termloom-lsp-{name}"))
        .spawn(move || {
            loop {
                match framing::read_message(&mut reader) {
                    Ok(Some(message)) => {
                        handle_message(
                            &name,
                            message,
                            &stdin,
                            &pending,
                            &capabilities,
                            &status,
                            &sink,
                        );
                    }
                    Ok(None) => break,
                    Err(err) => {
                        tracing::warn!(server = %name, error = %err, "malformed language server message");
                        break;
                    }
                }
            }

            let crashed = !shutting_down.load(Ordering::SeqCst);
            if crashed {
                *status.lock().unwrap_or_else(|e| e.into_inner()) = ClientStatus::Failed;
            } else {
                *status.lock().unwrap_or_else(|e| e.into_inner()) = ClientStatus::Exited;
            }
            sink(LspEvent::Exited {
                server: name,
                status: None,
                crashed,
            });
        })
        .expect("spawning the language server reader thread");
}

fn handle_message(
    name: &str,
    message: Value,
    stdin: &Arc<Mutex<ChildStdin>>,
    pending: &Arc<Mutex<HashMap<i64, Pending>>>,
    capabilities: &Arc<Mutex<Option<ServerCapabilities>>>,
    status: &Arc<Mutex<ClientStatus>>,
    sink: &LspSink,
) {
    // A response to one of our requests.
    if let Some(id) = message.get("id").and_then(Value::as_i64) {
        if message.get("method").is_none() {
            if id == 0 {
                // initialize
                if let Some(result) = message.get("result") {
                    let parsed = ServerCapabilities::from_initialize(result);
                    *capabilities.lock().unwrap_or_else(|e| e.into_inner()) = Some(parsed);
                    *status.lock().unwrap_or_else(|e| e.into_inner()) = ClientStatus::Ready;
                    // The spec requires `initialized` before anything else.
                    if let Ok(mut stdin) = stdin.lock() {
                        let _ = framing::write_message(
                            &mut *stdin,
                            &json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
                        );
                    }
                    sink(LspEvent::Initialized {
                        server: name.to_string(),
                        capabilities: Box::new(parsed),
                    });
                } else if let Some(error) = message.get("error") {
                    *status.lock().unwrap_or_else(|e| e.into_inner()) = ClientStatus::Failed;
                    sink(LspEvent::Status {
                        server: name.to_string(),
                        message: format!("initialize failed: {error}"),
                    });
                }
                return;
            }

            let Some(entry) = pending
                .lock()
                .ok()
                .and_then(|mut map| map.remove(&id))
            else {
                return;
            };
            if let Some(error) = message.get("error") {
                sink(LspEvent::RequestFailed {
                    server: name.to_string(),
                    kind: entry.kind,
                    message: error["message"]
                        .as_str()
                        .unwrap_or("request failed")
                        .to_string(),
                });
                return;
            }
            let result = message.get("result").cloned().unwrap_or(Value::Null);
            sink(LspEvent::Response {
                server: name.to_string(),
                kind: entry.kind,
                context: entry.context,
                result: convert(entry.kind, &result),
            });
            return;
        }

        // A request *from* the server: answer so it does not block.
        let method = message["method"].as_str().unwrap_or_default();
        let result = match method {
            "workspace/configuration" => {
                // One null entry per requested item.
                let count = message["params"]["items"]
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or(1);
                Value::Array(vec![Value::Null; count])
            }
            "window/workDoneProgress/create" | "client/registerCapability"
            | "client/unregisterCapability" => Value::Null,
            "workspace/applyEdit" => json!({ "applied": false }),
            _ => Value::Null,
        };
        if let Ok(mut stdin) = stdin.lock() {
            let _ = framing::write_message(
                &mut *stdin,
                &json!({"jsonrpc":"2.0","id":id,"result":result}),
            );
        }
        return;
    }

    // Notifications.
    let method = message["method"].as_str().unwrap_or_default();
    let params = &message["params"];
    match method {
        "textDocument/publishDiagnostics" => {
            if let Some((path, diagnostics)) = protocol::parse_diagnostics(params) {
                sink(LspEvent::Diagnostics {
                    server: name.to_string(),
                    path,
                    diagnostics,
                });
            }
        }
        "window/showMessage" => {
            if let Some(text) = params["message"].as_str() {
                sink(LspEvent::Status {
                    server: name.to_string(),
                    message: text.to_string(),
                });
            }
        }
        "window/logMessage" => {
            tracing::debug!(server = %name, message = %params["message"], "lsp log");
        }
        "$/progress" => {
            let value = &params["value"];
            if let Some(title) = value["title"].as_str() {
                sink(LspEvent::Status {
                    server: name.to_string(),
                    message: title.to_string(),
                });
            }
        }
        _ => {}
    }
}

fn convert(kind: RequestKind, result: &Value) -> LspResult {
    match kind {
        RequestKind::Hover => LspResult::Hover(protocol::parse_hover(result)),
        RequestKind::Completion => LspResult::Completion(protocol::parse_completion(result)),
        RequestKind::Definition | RequestKind::References => {
            LspResult::Locations(protocol::parse_locations(result))
        }
        RequestKind::DocumentSymbols | RequestKind::WorkspaceSymbols => {
            LspResult::Symbols(protocol::parse_symbols(result))
        }
        RequestKind::Rename => LspResult::Edits(protocol::parse_workspace_edit(result)),
        RequestKind::Formatting => {
            let edits = protocol::parse_text_edits(result);
            if edits.is_empty() {
                LspResult::Empty
            } else {
                LspResult::Edits(vec![(PathBuf::new(), edits)])
            }
        }
    }
}

fn spawn_stderr_logger(name: String, stderr: std::process::ChildStderr) {
    std::thread::Builder::new()
        .name(format!("termloom-lsp-err-{name}"))
        .spawn(move || {
            use std::io::BufRead;
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                tracing::debug!(server = %name, "{line}");
            }
        })
        .ok();
}

/// Convert a TOML value from the config file into JSON for the protocol.
pub fn toml_to_json(value: toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::Number(i.into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(items) => Value::Array(items.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(table) => Value::Object(
            table
                .into_iter()
                .map(|(key, value)| (key, toml_to_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_values_convert_to_json() {
        let value: toml::Value = toml::from_str("a = 1\nb = [true, \"x\"]\n[c]\nd = 2.5\n")
            .unwrap();
        let json = toml_to_json(value);
        assert_eq!(json["a"], 1);
        assert_eq!(json["b"][1], "x");
        assert_eq!(json["c"]["d"], 2.5);
    }

    #[test]
    fn starting_a_missing_server_is_an_error() {
        let config = LspServerConfig {
            command: "definitely-not-a-language-server".into(),
            ..Default::default()
        };
        let sink: LspSink = Arc::new(|_| {});
        let result = LspClient::start("missing", &config, Path::new("/tmp"), sink);
        assert!(result.is_err());
    }

    #[test]
    fn request_kinds_convert_their_results() {
        let hover = convert(
            RequestKind::Hover,
            &serde_json::json!({"contents": "text"}),
        );
        assert!(matches!(hover, LspResult::Hover(lines) if lines == vec!["text"]));

        let empty = convert(RequestKind::Formatting, &Value::Null);
        assert!(matches!(empty, LspResult::Empty));
    }
}
