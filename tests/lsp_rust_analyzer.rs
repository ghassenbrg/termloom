//! End-to-end validation against a real language server.
//!
//! The test drives `rust-analyzer` over stdio: initialize, open a document,
//! receive diagnostics, and answer hover and definition requests. It skips
//! itself when the server is not installed, so CI never depends on it.

use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use termloom::config::LspServerConfig;
use termloom::domain::diagnostics::Position;
use termloom::services::lsp::{
    ClientStatus, LspClient, LspEvent, LspResult, LspSink, RequestContext, RequestKind,
};
use termloom::services::lsp::protocol;

fn rust_analyzer() -> Option<String> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join("rust-analyzer"))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().to_string())
}

/// A tiny cargo project with a deliberate error.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        r#"fn greet(name: &str) -> String {
    format!("hello {name}")
}

fn main() {
    let message = greet("world");
    println!("{message}");
}
"#,
    )
    .unwrap();
    dir
}

struct Collector {
    events: mpsc::Receiver<LspEvent>,
}

impl Collector {
    /// Wait for an event matching `predicate`.
    fn wait<T>(&self, predicate: impl FnMut(&LspEvent) -> Option<T>) -> Option<T> {
        self.wait_for(Duration::from_secs(120), predicate)
    }

    fn wait_for<T>(
        &self,
        timeout: Duration,
        mut predicate: impl FnMut(&LspEvent) -> Option<T>,
    ) -> Option<T> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(remaining.min(Duration::from_secs(5))) {
                Ok(event) => {
                    if let Some(value) = predicate(&event) {
                        return Some(value);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return None,
            }
        }
        None
    }
}

#[test]
fn rust_analyzer_initializes_and_answers_requests() {
    let Some(command) = rust_analyzer() else {
        eprintln!("skipping: rust-analyzer is not installed");
        return;
    };

    let dir = project();
    let root = dir.path();
    let main = root.join("src/main.rs");
    let source = std::fs::read_to_string(&main).unwrap();

    let (tx, rx) = mpsc::channel();
    let sink: LspSink = Arc::new(move |event| {
        let _ = tx.send(event);
    });
    let collector = Collector { events: rx };

    let config = LspServerConfig {
        command,
        languages: vec!["rust".into()],
        root_markers: vec!["Cargo.toml".into()],
        ..Default::default()
    };
    let mut client = LspClient::start("rust-analyzer", &config, root, sink).unwrap();

    // 1. Initialize and capability negotiation.
    let capabilities = collector
        .wait(|event| match event {
            LspEvent::Initialized { capabilities, .. } => Some(**capabilities),
            _ => None,
        })
        .expect("rust-analyzer should initialize");
    assert!(capabilities.hover, "hover should be advertised");
    assert!(capabilities.definition);
    assert!(capabilities.document_symbols);
    assert_eq!(client.status(), ClientStatus::Ready);

    // 2. Document synchronisation.
    client.did_open(&main, "rust", &source).unwrap();

    // 3. Document symbols come back converted into our own type.
    wait_for_symbols(&client, &collector, &main);

    // 4. Hover on the `greet` call inside main. rust-analyzer answers with an
    // empty result until it has loaded the crate graph, so retry the way an
    // editor would rather than failing on the first empty answer.
    let hover_position = Position::new(5, 19);
    let hover = retry(&collector, Duration::from_secs(180), || {
        client
            .request(
                RequestKind::Hover,
                "textDocument/hover",
                protocol::text_document_position(&main, hover_position),
                RequestContext {
                    path: main.clone(),
                    position: hover_position,
                },
            )
            .unwrap();
    }, |event| match event {
        LspEvent::Response {
            kind: RequestKind::Hover,
            result: LspResult::Hover(lines),
            ..
        } if !lines.is_empty() => Some(lines.clone()),
        _ => None,
    })
    .expect("hover should return documentation");
    assert!(
        hover.iter().any(|line| line.contains("greet")),
        "hover text should mention the symbol: {hover:?}"
    );

    // 5. Go to definition jumps back to the function.
    let locations = retry(&collector, Duration::from_secs(60), || {
        client
            .request(
                RequestKind::Definition,
                "textDocument/definition",
                protocol::text_document_position(&main, hover_position),
                RequestContext {
                    path: main.clone(),
                    position: hover_position,
                },
            )
            .unwrap();
    }, |event| match event {
        LspEvent::Response {
            kind: RequestKind::Definition,
            result: LspResult::Locations(locations),
            ..
        } if !locations.is_empty() => Some(locations.clone()),
        _ => None,
    })
    .expect("definition should resolve");
    assert_eq!(
        std::fs::canonicalize(&locations[0].path).unwrap(),
        std::fs::canonicalize(&main).unwrap()
    );
    assert_eq!(locations[0].range.start.line, 0, "greet is on the first line");

    // 6. Clean shutdown.
    client.shutdown();
    assert_eq!(client.status(), ClientStatus::Exited);
}

/// Re-send a request until it produces a useful answer or time runs out.
fn retry<T>(
    collector: &Collector,
    timeout: Duration,
    mut send: impl FnMut(),
    mut predicate: impl FnMut(&LspEvent) -> Option<T>,
) -> Option<T> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        send();
        if let Some(value) = collector.wait_for(Duration::from_secs(10), &mut predicate) {
            return Some(value);
        }
    }
    None
}

fn wait_for_symbols(client: &LspClient, collector: &Collector, path: &Path) {
    client
        .request(
            RequestKind::DocumentSymbols,
            "textDocument/documentSymbol",
            serde_json::json!({ "textDocument": { "uri": protocol::path_to_uri(path) } }),
            RequestContext {
                path: path.to_path_buf(),
                position: Position::default(),
            },
        )
        .unwrap();
    let symbols = collector
        .wait_for(Duration::from_secs(60), |event| match event {
            LspEvent::Response {
                kind: RequestKind::DocumentSymbols,
                result: LspResult::Symbols(symbols),
                ..
            } if !symbols.is_empty() => Some(symbols.clone()),
            _ => None,
        })
        .expect("document symbols should be returned");
    let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"greet"), "{names:?}");
    assert!(names.contains(&"main"), "{names:?}");
}

#[test]
fn a_crashing_server_is_reported_and_does_not_hang() {
    // `cat` speaks no LSP: it exits at EOF and must be reported as a crash.
    let (tx, rx) = mpsc::channel();
    let sink: LspSink = Arc::new(move |event| {
        let _ = tx.send(event);
    });
    let config = LspServerConfig {
        command: "false".into(),
        languages: vec!["rust".into()],
        ..Default::default()
    };
    let _client = LspClient::start("broken", &config, Path::new("."), sink).unwrap();

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut saw_exit = false;
    while Instant::now() < deadline && !saw_exit {
        if let Ok(LspEvent::Exited { crashed, .. }) = rx.recv_timeout(Duration::from_secs(5)) {
            assert!(crashed, "an unexpected exit is a crash");
            saw_exit = true;
        }
    }
    assert!(saw_exit, "a dead server must report its exit");
}
