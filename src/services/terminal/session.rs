//! A single interactive PTY session.
//!
//! Pipeline: `portable-pty` spawns the child on a real pty, a reader thread
//! feeds the bytes into a `vt100` parser, and the UI renders a
//! [`ScreenSnapshot`] taken from that parser. Input travels the other way,
//! encoded by [`super::input`].

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::domain::ids::TerminalId;
use crate::domain::terminal::{TerminalKind, TerminalMeta, TerminalState};

use super::screen::ScreenSnapshot;

/// Something a session wants the application to know about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// New bytes were rendered into the screen.
    Output(TerminalId),
    /// The child exited with an optional status code.
    Exited(TerminalId, Option<i32>),
}

/// Callback used to publish [`TerminalEvent`]s into the application loop.
pub type EventSink = Arc<dyn Fn(TerminalEvent) + Send + Sync>;

/// How to start a session.
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub kind: TerminalKind,
    pub rows: u16,
    pub cols: u16,
    pub scrollback: usize,
}

impl SpawnSpec {
    /// A shell session in `cwd`.
    pub fn shell(
        label: impl Into<String>,
        program: &str,
        args: &[String],
        cwd: &Path,
    ) -> SpawnSpec {
        SpawnSpec {
            label: label.into(),
            program: program.to_string(),
            args: args.to_vec(),
            cwd: cwd.to_path_buf(),
            env: Vec::new(),
            kind: TerminalKind::Shell,
            rows: 24,
            cols: 80,
            scrollback: 5_000,
        }
    }

    /// Full command line, for display.
    pub fn command_vec(&self) -> Vec<String> {
        let mut out = vec![self.program.clone()];
        out.extend(self.args.iter().cloned());
        out
    }
}

/// Live terminal session: pty master, parser and child handle.
pub struct TerminalSession {
    meta: TerminalMeta,
    parser: Arc<Mutex<vt100::Parser>>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    child: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    /// Bumped by the reader thread; lets the UI skip redraws.
    output_seq: Arc<AtomicU64>,
    /// Unix-epoch millis of the last byte received.
    last_output_ms: Arc<AtomicU64>,
    exited: Arc<AtomicBool>,
    /// The spec, kept so the session can be restarted.
    spec: SpawnSpec,
    /// Lines the view is scrolled back by (0 = live).
    scroll_offset: usize,
}

impl TerminalSession {
    /// Spawn a child on a fresh pty.
    pub fn spawn(id: TerminalId, spec: SpawnSpec, sink: EventSink) -> Result<TerminalSession> {
        let pty_system = portable_pty::native_pty_system();
        let size = PtySize {
            rows: spec.rows.max(1),
            cols: spec.cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = pty_system
            .openpty(size)
            .context("allocating a pseudo terminal")?;

        let mut command = CommandBuilder::new(&spec.program);
        command.args(&spec.args);
        command.cwd(&spec.cwd);
        // A sane TERM keeps colour output working in child programs.
        command.env(
            "TERM",
            std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".into()),
        );
        command.env("TERMLOOM", "1");
        for (key, value) in &spec.env {
            command.env(key, value);
        }

        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("spawning {}", spec.program))?;
        drop(pair.slave);

        let killer = child.clone_killer();
        let reader = pair
            .master
            .try_clone_reader()
            .context("cloning the pty reader")?;
        let writer = pair.master.take_writer().context("taking the pty writer")?;

        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            spec.rows.max(1),
            spec.cols.max(1),
            spec.scrollback,
        )));
        let output_seq = Arc::new(AtomicU64::new(0));
        let last_output_ms = Arc::new(AtomicU64::new(now_ms()));
        let exited = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(Some(child)));

        spawn_reader(
            id,
            reader,
            Arc::clone(&parser),
            Arc::clone(&output_seq),
            Arc::clone(&last_output_ms),
            Arc::clone(&exited),
            Arc::clone(&child),
            sink,
        );

        let now = SystemTime::now();
        Ok(TerminalSession {
            meta: TerminalMeta {
                id,
                label: spec.label.clone(),
                cwd: spec.cwd.clone(),
                command: spec.command_vec(),
                kind: spec.kind,
                state: TerminalState::Running,
                created_at: now,
                last_output_at: now,
                error: None,
            },
            parser,
            master: pair.master,
            writer,
            killer,
            child,
            output_seq,
            last_output_ms,
            exited,
            spec,
            scroll_offset: 0,
        })
    }

    pub fn id(&self) -> TerminalId {
        self.meta.id
    }

    /// Metadata refreshed with the latest activity timestamp.
    pub fn meta(&self) -> TerminalMeta {
        let mut meta = self.meta.clone();
        meta.last_output_at = self.last_output_at();
        meta
    }

    pub fn label(&self) -> &str {
        &self.meta.label
    }

    pub fn set_label(&mut self, label: impl Into<String>) {
        self.meta.label = label.into();
    }

    pub fn kind(&self) -> TerminalKind {
        self.meta.kind
    }

    pub fn state(&self) -> TerminalState {
        self.meta.state
    }

    pub fn cwd(&self) -> &Path {
        &self.meta.cwd
    }

    pub fn command(&self) -> &[String] {
        &self.meta.command
    }

    /// Monotonic counter of output batches; cheap change detection.
    pub fn output_seq(&self) -> u64 {
        self.output_seq.load(Ordering::Relaxed)
    }

    pub fn last_output_at(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_millis(self.last_output_ms.load(Ordering::Relaxed))
    }

    /// Time since the child last wrote anything.
    pub fn quiet_for(&self) -> Duration {
        SystemTime::now()
            .duration_since(self.last_output_at())
            .unwrap_or_default()
    }

    /// Mark the session as exited (called from the event loop).
    pub fn mark_exited(&mut self, code: Option<i32>) {
        self.meta.state = TerminalState::Exited(code);
        self.exited.store(true, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        !self.exited.load(Ordering::Relaxed) && self.meta.state.is_running()
    }

    /// Current screen contents.
    pub fn snapshot(&self) -> ScreenSnapshot {
        match self.parser.lock() {
            Ok(parser) => ScreenSnapshot::capture(parser.screen()),
            // A poisoned parser means a reader thread panicked; render blank
            // rather than taking the workbench down with it.
            Err(_) => ScreenSnapshot::default(),
        }
    }

    /// Write raw bytes to the child.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if !self.is_running() {
            return Ok(());
        }
        // Any input returns the view to the live screen.
        self.set_scroll_offset(0);
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Send text (newlines become carriage returns).
    pub fn send_text(&mut self, text: &str) -> Result<()> {
        let bytes = super::input::encode_text(text);
        self.write(&bytes)
    }

    /// Resize both the parser grid and the pty.
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        let (rows, cols) = (rows.max(1), cols.max(1));
        if let Ok(mut parser) = self.parser.lock() {
            let (current_rows, current_cols) = parser.screen().size();
            if current_rows == rows && current_cols == cols {
                return Ok(());
            }
            parser.set_size(rows, cols);
        }
        self.spec.rows = rows;
        self.spec.cols = cols;
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    /// Scroll the view back by `lines` (positive scrolls into history).
    pub fn scroll(&mut self, lines: isize) {
        let next = (self.scroll_offset as isize + lines).max(0) as usize;
        self.set_scroll_offset(next);
    }

    pub fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset;
        if let Ok(mut parser) = self.parser.lock() {
            parser.set_scrollback(offset);
            // The parser clamps; read back so the UI shows the real position.
            self.scroll_offset = parser.screen().scrollback();
        }
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Terminate the child process.
    pub fn kill(&mut self) -> Result<()> {
        self.killer.kill()?;
        Ok(())
    }

    /// Reap the child, returning its exit code when it has finished.
    pub fn try_wait(&mut self) -> Option<i32> {
        let mut guard = self.child.lock().ok()?;
        let child = guard.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code() as i32),
            _ => None,
        }
    }

    /// The spec needed to restart this session with the same command.
    pub fn spec(&self) -> SpawnSpec {
        self.spec.clone()
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        // Never leave orphaned children behind when a pane is closed.
        let _ = self.killer.kill();
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_reader(
    id: TerminalId,
    mut reader: Box<dyn Read + Send>,
    parser: Arc<Mutex<vt100::Parser>>,
    output_seq: Arc<AtomicU64>,
    last_output_ms: Arc<AtomicU64>,
    exited: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    sink: EventSink,
) {
    std::thread::Builder::new()
        .name(format!("termloom-pty-{}", id.raw()))
        .spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut parser) = parser.lock() {
                            parser.process(&buffer[..n]);
                        }
                        last_output_ms.store(now_ms(), Ordering::Relaxed);
                        output_seq.fetch_add(1, Ordering::Relaxed);
                        sink(TerminalEvent::Output(id));
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }

            exited.store(true, Ordering::Relaxed);
            let code = child
                .lock()
                .ok()
                .and_then(|mut guard| guard.as_mut().map(|c| c.wait()))
                .and_then(|status| status.ok())
                .map(|status| status.exit_code() as i32);
            sink(TerminalEvent::Exited(id, code));
        })
        .expect("spawning a pty reader thread");
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn spec(program: &str, args: &[&str]) -> SpawnSpec {
        SpawnSpec {
            label: "test".into(),
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: std::env::temp_dir(),
            env: Vec::new(),
            kind: TerminalKind::Task,
            rows: 10,
            cols: 40,
            scrollback: 100,
        }
    }

    fn sink(tx: mpsc::Sender<TerminalEvent>) -> EventSink {
        Arc::new(move |event| {
            let _ = tx.send(event);
        })
    }

    /// Wait until `predicate` holds or the deadline passes.
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
    fn runs_a_command_and_renders_its_output() {
        let (tx, rx) = mpsc::channel();
        let session =
            TerminalSession::spawn(TerminalId::next(), spec("echo", &["hello-pty"]), sink(tx))
                .unwrap();
        assert!(wait_until(|| session
            .snapshot()
            .to_text()
            .contains("hello-pty")));
        // The exit event must arrive too.
        let saw_exit = std::iter::from_fn(|| rx.recv_timeout(Duration::from_secs(5)).ok())
            .any(|e| matches!(e, TerminalEvent::Exited(_, _)));
        assert!(saw_exit, "expected an exit event");
    }

    #[test]
    fn interactive_input_reaches_the_child() {
        let (tx, _rx) = mpsc::channel();
        let mut session =
            TerminalSession::spawn(TerminalId::next(), spec("cat", &[]), sink(tx)).unwrap();
        session.send_text("ping\n").unwrap();
        assert!(
            wait_until(|| session.snapshot().to_text().contains("ping")),
            "cat should echo what we typed"
        );
        session.kill().unwrap();
    }

    #[test]
    fn ansi_colour_output_is_parsed_not_shown_raw() {
        let (tx, _rx) = mpsc::channel();
        let session = TerminalSession::spawn(
            TerminalId::next(),
            spec("printf", &["\\033[31mRED\\033[0m"]),
            sink(tx),
        )
        .unwrap();
        assert!(wait_until(|| session.snapshot().to_text().contains("RED")));
        let snap = session.snapshot();
        assert!(
            !snap.to_text().contains("\u{1b}["),
            "escape sequences must be consumed by the parser"
        );
        assert_eq!(
            snap.rows[0][0].fg,
            crate::services::terminal::screen::CellColor::Indexed(1)
        );
    }

    #[test]
    fn resizing_updates_the_parser_grid() {
        let (tx, _rx) = mpsc::channel();
        let mut session =
            TerminalSession::spawn(TerminalId::next(), spec("cat", &[]), sink(tx)).unwrap();
        session.resize(30, 100).unwrap();
        let snap = session.snapshot();
        assert_eq!(snap.rows.len(), 30);
        assert_eq!(snap.rows[0].len(), 100);
        session.kill().unwrap();
    }

    #[test]
    fn killing_a_session_reports_an_exit() {
        let (tx, rx) = mpsc::channel();
        let mut session =
            TerminalSession::spawn(TerminalId::next(), spec("sleep", &["30"]), sink(tx)).unwrap();
        session.kill().unwrap();
        let event = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(event, TerminalEvent::Exited(_, _)), "{event:?}");
    }

    #[test]
    fn spawning_a_missing_program_is_an_error_not_a_panic() {
        let (tx, _rx) = mpsc::channel();
        let result = TerminalSession::spawn(
            TerminalId::next(),
            spec("definitely-not-a-real-binary-xyz", &[]),
            sink(tx),
        );
        assert!(result.is_err());
    }

    #[test]
    fn scrollback_offset_is_clamped_to_available_history() {
        let (tx, _rx) = mpsc::channel();
        let mut session =
            TerminalSession::spawn(TerminalId::next(), spec("cat", &[]), sink(tx)).unwrap();
        session.scroll(50);
        assert_eq!(
            session.scroll_offset(),
            0,
            "no history yet, so the view stays live"
        );
        session.kill().unwrap();
    }
}
