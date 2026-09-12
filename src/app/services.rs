//! Long-lived services the commands operate on.
//!
//! Kept separate from [`AppState`] so state stays plain data: anything that
//! owns a thread, a child process or a socket lives here.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::services::agents::AgentRegistry;
use crate::services::dap::DebugSession;
use crate::services::lsp::LspManager;
use crate::services::workspace::{FileIndex, ScanOptions};

use super::events::{AppEvent, EventSender};

/// Background services and the handles used to talk to them.
pub struct Services {
    pub events: EventSender,
    pub runtime: tokio::runtime::Runtime,
    pub agents: AgentRegistry,
    /// Language servers. Present even when nothing is configured.
    pub lsp: LspManager,
    /// The current debug session, if one is running.
    pub debug: Option<DebugSession>,
    /// Set while a git refresh is in flight, so ticks do not pile up.
    git_refreshing: Arc<AtomicBool>,
    /// Set while the quick-open index is being rebuilt.
    indexing: Arc<AtomicBool>,
    /// Set while backend state is being refreshed away from the UI thread.
    agent_refreshing: Arc<AtomicBool>,
}

impl Services {
    pub fn new(events: EventSender, runtime: tokio::runtime::Runtime, lsp: LspManager) -> Services {
        Services {
            events,
            runtime,
            agents: AgentRegistry::new(),
            lsp,
            debug: None,
            git_refreshing: Arc::new(AtomicBool::new(false)),
            indexing: Arc::new(AtomicBool::new(false)),
            agent_refreshing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Recompute the git snapshot off the UI thread. No-op when one is running
    /// or when the workspace is not a repository.
    pub fn refresh_git(&self, root: Option<PathBuf>) {
        let Some(root) = root else { return };
        if self.git_refreshing.swap(true, Ordering::SeqCst) {
            return;
        }
        let events = self.events.clone();
        let flag = Arc::clone(&self.git_refreshing);
        std::thread::Builder::new()
            .name("termloom-git".into())
            .spawn(move || {
                match crate::services::git::snapshot(&root) {
                    Ok(snapshot) => {
                        let _ = events.send(AppEvent::GitUpdated(Box::new(snapshot)));
                    }
                    Err(err) => {
                        tracing::debug!(error = %err, "git refresh failed");
                    }
                }
                flag.store(false, Ordering::SeqCst);
            })
            .ok();
    }

    /// Rebuild the quick-open file index off the UI thread.
    pub fn reindex(&self, root: &Path, options: ScanOptions) {
        if self.indexing.swap(true, Ordering::SeqCst) {
            return;
        }
        let events = self.events.clone();
        let flag = Arc::clone(&self.indexing);
        let root = root.to_path_buf();
        std::thread::Builder::new()
            .name("termloom-index".into())
            .spawn(move || {
                let index = FileIndex::scan(&root, &options);
                let _ = events.send(AppEvent::FileIndexed(index));
                flag.store(false, Ordering::SeqCst);
            })
            .ok();
    }

    pub fn is_indexing(&self) -> bool {
        self.indexing.load(Ordering::SeqCst)
    }

    /// Refresh agent backends asynchronously. External backends may invoke a
    /// CLI/socket with a timeout, so this work must never block a draw tick.
    pub fn request_agent_refresh(&self) {
        if self.agent_refreshing.swap(true, Ordering::SeqCst) {
            return;
        }
        let registry = self.agents.clone();
        let events = self.events.clone();
        let flag = Arc::clone(&self.agent_refreshing);
        self.runtime.spawn(async move {
            registry.refresh_all().await;
            let agents = registry.list_all().await;
            let _ = events.send(AppEvent::AgentsUpdated(agents));
            flag.store(false, Ordering::SeqCst);
        });
    }
}

#[cfg(test)]
pub use test_support::TestServices;

#[cfg(test)]
mod test_support {
    use std::path::Path;
    use std::sync::Arc;

    use crate::config::Config;
    use crate::services::agents::LocalAgentBackend;
    use crate::services::lsp::{LspManager, LspSink};
    use crate::services::terminal::{EventSink, TerminalManager};

    use super::*;

    /// Services wired for tests: real local agent backend, no language server
    /// processes, and an event channel the test can drain.
    pub struct TestServices {
        pub services: Services,
        pub terminals: Arc<std::sync::Mutex<TerminalManager>>,
        pub events: std::sync::mpsc::Receiver<AppEvent>,
    }

    impl Services {
        /// Build services suitable for unit tests.
        pub fn for_test(config: &Config, root: &Path) -> TestServices {
            let (sender, events) = std::sync::mpsc::channel();
            let terminal_sender = sender.clone();
            let sink: EventSink = Arc::new(move |event| {
                let app_event = match event {
                    crate::services::terminal::TerminalEvent::Output(id) => {
                        AppEvent::TerminalOutput(id)
                    }
                    crate::services::terminal::TerminalEvent::Exited(id, code) => {
                        AppEvent::TerminalExited(id, code)
                    }
                };
                let _ = terminal_sender.send(app_event);
            });
            let terminals = Arc::new(std::sync::Mutex::new(TerminalManager::new(sink)));
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime");
            let lsp_sink: LspSink = Arc::new(|_| {});
            let lsp = LspManager::new(config, root, lsp_sink);
            let mut services = Services::new(sender, runtime, lsp);
            services.agents.push(Arc::new(LocalAgentBackend::new(
                Arc::clone(&terminals),
                std::time::Duration::from_secs(20),
            )));
            TestServices {
                services,
                terminals,
                events,
            }
        }
    }
}
