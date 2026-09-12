//! Owns every live terminal session and keeps their display order stable.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::domain::ids::TerminalId;
use crate::domain::terminal::{TerminalKind, TerminalMeta};

use super::session::{EventSink, SpawnSpec, TerminalSession};

/// Registry of PTY sessions.
pub struct TerminalManager {
    sessions: Vec<TerminalSession>,
    sink: EventSink,
    /// Applied to specs that do not set their own scrollback.
    pub default_scrollback: usize,
    /// Counter used to label new shells (`Terminal 1`, `Terminal 2`, ...).
    next_shell_number: usize,
}

impl TerminalManager {
    pub fn new(sink: EventSink) -> TerminalManager {
        TerminalManager {
            sessions: Vec::new(),
            sink,
            default_scrollback: 5_000,
            next_shell_number: 1,
        }
    }

    /// Label for the next plain shell, e.g. `Terminal 3`.
    pub fn next_shell_label(&mut self) -> String {
        let label = format!("Terminal {}", self.next_shell_number);
        self.next_shell_number += 1;
        label
    }

    /// Start a session. The id is allocated here so callers can reference the
    /// session before its first output arrives.
    pub fn spawn(&mut self, mut spec: SpawnSpec) -> Result<TerminalId> {
        if spec.scrollback == 0 {
            spec.scrollback = self.default_scrollback;
        }
        let id = TerminalId::next();
        let session = TerminalSession::spawn(id, spec, self.sink.clone())?;
        self.sessions.push(session);
        Ok(id)
    }

    /// Kill and drop a session.
    pub fn close(&mut self, id: TerminalId) -> bool {
        let Some(index) = self.index_of(id) else {
            return false;
        };
        let mut session = self.sessions.remove(index);
        let _ = session.kill();
        true
    }

    /// Restart a finished (or running) session with its original command.
    /// The terminal keeps its position in the list but gets a new id.
    pub fn restart(&mut self, id: TerminalId) -> Result<TerminalId> {
        let index = self
            .index_of(id)
            .ok_or_else(|| anyhow!("terminal {id} is unknown"))?;
        let spec = self.sessions[index].spec();
        let mut old = std::mem::replace(
            &mut self.sessions[index],
            TerminalSession::spawn(TerminalId::next(), spec, self.sink.clone())?,
        );
        let _ = old.kill();
        Ok(self.sessions[index].id())
    }

    pub fn get(&self, id: TerminalId) -> Option<&TerminalSession> {
        self.sessions.iter().find(|s| s.id() == id)
    }

    pub fn get_mut(&mut self, id: TerminalId) -> Option<&mut TerminalSession> {
        self.sessions.iter_mut().find(|s| s.id() == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &TerminalSession> {
        self.sessions.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut TerminalSession> {
        self.sessions.iter_mut()
    }

    pub fn ids(&self) -> Vec<TerminalId> {
        self.sessions.iter().map(TerminalSession::id).collect()
    }

    pub fn metas(&self) -> Vec<TerminalMeta> {
        self.sessions.iter().map(TerminalSession::meta).collect()
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Position of a session in display order.
    pub fn index_of(&self, id: TerminalId) -> Option<usize> {
        self.sessions.iter().position(|s| s.id() == id)
    }

    /// Session at a display position.
    pub fn id_at(&self, index: usize) -> Option<TerminalId> {
        self.sessions.get(index).map(TerminalSession::id)
    }

    /// Record an exit reported by the event loop.
    pub fn mark_exited(&mut self, id: TerminalId, code: Option<i32>) {
        if let Some(session) = self.get_mut(id) {
            session.mark_exited(code);
        }
    }

    /// Resize every session to the same grid (the panes share a strip).
    pub fn resize_all(&mut self, rows: u16, cols: u16) {
        for session in &mut self.sessions {
            let _ = session.resize(rows, cols);
        }
    }

    /// Terminals of a given kind, in display order.
    pub fn of_kind(&self, kind: TerminalKind) -> Vec<TerminalId> {
        self.sessions
            .iter()
            .filter(|s| s.kind() == kind)
            .map(TerminalSession::id)
            .collect()
    }

    /// Kill everything; called on shutdown.
    pub fn shutdown(&mut self) {
        for session in &mut self.sessions {
            let _ = session.kill();
        }
        self.sessions.clear();
    }

    /// Convenience used by the `terminal.new` command.
    pub fn spawn_shell(
        &mut self,
        program: &str,
        args: &[String],
        cwd: &Path,
    ) -> Result<TerminalId> {
        let label = self.next_shell_label();
        let mut spec = SpawnSpec::shell(label, program, args, cwd);
        spec.scrollback = self.default_scrollback;
        self.spawn(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::terminal::session::TerminalEvent;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn manager() -> (TerminalManager, mpsc::Receiver<TerminalEvent>) {
        let (tx, rx) = mpsc::channel();
        let sink: EventSink = Arc::new(move |event| {
            let _ = tx.send(event);
        });
        (TerminalManager::new(sink), rx)
    }

    fn spec(program: &str, args: &[&str]) -> SpawnSpec {
        SpawnSpec {
            label: program.into(),
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: std::env::temp_dir(),
            env: Vec::new(),
            kind: TerminalKind::Shell,
            rows: 10,
            cols: 40,
            scrollback: 0,
        }
    }

    fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if predicate() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn runs_four_sessions_at_once() {
        let (mut manager, _rx) = manager();
        let ids: Vec<TerminalId> = (0..4)
            .map(|i| {
                manager
                    .spawn(spec("echo", &[&format!("session-{i}")]))
                    .unwrap()
            })
            .collect();
        assert_eq!(manager.len(), 4);
        assert!(wait_until(|| ids.iter().all(|id| manager
            .get(*id)
            .map(|s| s.snapshot().to_text().contains("session-"))
            .unwrap_or(false))));
    }

    #[test]
    fn labels_shells_sequentially() {
        let (mut manager, _rx) = manager();
        assert_eq!(manager.next_shell_label(), "Terminal 1");
        assert_eq!(manager.next_shell_label(), "Terminal 2");
    }

    #[test]
    fn closing_removes_the_session() {
        let (mut manager, _rx) = manager();
        let id = manager.spawn(spec("cat", &[])).unwrap();
        assert!(manager.close(id));
        assert!(manager.get(id).is_none());
        assert!(!manager.close(id), "closing twice is a no-op");
    }

    #[test]
    fn restart_keeps_the_position_and_reruns_the_command() {
        let (mut manager, _rx) = manager();
        let first = manager.spawn(spec("echo", &["first"])).unwrap();
        let other = manager.spawn(spec("cat", &[])).unwrap();
        assert!(wait_until(|| manager
            .get(first)
            .map(|s| s.snapshot().to_text().contains("first"))
            .unwrap_or(false)));

        let restarted = manager.restart(first).unwrap();
        assert_ne!(restarted, first, "a restart gets a fresh id");
        assert_eq!(manager.index_of(restarted), Some(0), "position is kept");
        assert_eq!(manager.index_of(other), Some(1));
        assert!(wait_until(|| manager
            .get(restarted)
            .map(|s| s.snapshot().to_text().contains("first"))
            .unwrap_or(false)));
    }

    #[test]
    fn exit_status_is_recorded() {
        let (mut manager, rx) = manager();
        let id = manager.spawn(spec("false", &[])).unwrap();
        let code = loop {
            match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
                TerminalEvent::Exited(exited, code) if exited == id => break code,
                _ => continue,
            }
        };
        manager.mark_exited(id, code);
        assert!(!manager.get(id).unwrap().is_running());
        assert_eq!(manager.get(id).unwrap().state().label(), "exited 1");
    }

    #[test]
    fn resize_all_applies_to_every_session() {
        let (mut manager, _rx) = manager();
        manager.spawn(spec("cat", &[])).unwrap();
        manager.spawn(spec("cat", &[])).unwrap();
        manager.resize_all(12, 60);
        for session in manager.iter() {
            let snap = session.snapshot();
            assert_eq!(snap.rows.len(), 12);
            assert_eq!(snap.rows[0].len(), 60);
        }
        manager.shutdown();
        assert!(manager.is_empty());
    }
}
