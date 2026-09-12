//! A debug adapter process.
//!
//! The client owns the child process and the sequence-number bookkeeping.
//! Adapter messages are forwarded to a channel so the session driver can
//! interleave them with commands coming from the UI.

use std::io::BufReader;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::services::framing;

use super::protocol;

/// A raw message from the adapter, or the fact that it disconnected.
#[derive(Debug)]
pub enum AdapterMessage {
    Message(Value),
    Disconnected,
}

/// A running debug adapter.
pub struct DapClient {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    next_seq: AtomicI64,
    stopping: Arc<AtomicBool>,
    /// Command line, shown to the user before the first run.
    pub command_line: String,
}

impl DapClient {
    /// Spawn an adapter and stream its messages into `messages`.
    pub fn start(
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &[(String, String)],
        messages: Sender<AdapterMessage>,
    ) -> Result<DapClient> {
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            command.env(key, value);
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("starting debug adapter `{program}`"))?;
        let stdin = child.stdin.take().context("adapter stdin")?;
        let stdout = child.stdout.take().context("adapter stdout")?;
        let stderr = child.stderr.take().context("adapter stderr")?;

        let stopping = Arc::new(AtomicBool::new(false));
        let reader_stopping = Arc::clone(&stopping);
        std::thread::Builder::new()
            .name("termloom-dap".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    match framing::read_message(&mut reader) {
                        Ok(Some(message)) => {
                            if messages.send(AdapterMessage::Message(message)).is_err() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(err) => {
                            tracing::warn!(error = %err, "malformed debug adapter message");
                            break;
                        }
                    }
                }
                reader_stopping.store(true, Ordering::SeqCst);
                let _ = messages.send(AdapterMessage::Disconnected);
            })
            .expect("spawning the debug adapter reader thread");

        std::thread::Builder::new()
            .name("termloom-dap-err".into())
            .spawn(move || {
                use std::io::BufRead;
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    tracing::debug!(target: "dap", "{line}");
                }
            })
            .ok();

        let command_line = std::iter::once(program.to_string())
            .chain(args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");

        Ok(DapClient {
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            next_seq: AtomicI64::new(1),
            stopping,
            command_line,
        })
    }

    /// Send a request, returning its sequence number.
    pub fn request(&self, command: &str, arguments: Value) -> Result<i64> {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let message = protocol::request(seq, command, arguments);
        let mut stdin = self
            .stdin
            .lock()
            .map_err(|_| anyhow::anyhow!("adapter stdin lock poisoned"))?;
        framing::write_message(&mut *stdin, &message)?;
        Ok(seq)
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Ask the adapter to stop, then make sure the process is gone.
    pub fn terminate(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        let _ = self.request(
            "disconnect",
            serde_json::json!({ "terminateDebuggee": true }),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
        while std::time::Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let _ = self.child.kill();
    }
}

impl Drop for DapClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn starting_a_missing_adapter_is_an_error() {
        let (tx, _rx) = mpsc::channel();
        let result = DapClient::start(
            "definitely-not-a-debug-adapter",
            &[],
            std::path::Path::new("."),
            &[],
            tx,
        );
        assert!(result.is_err());
    }

    #[test]
    fn a_dead_adapter_reports_disconnection() {
        let (tx, rx) = mpsc::channel();
        let mut client = DapClient::start("true", &[], std::path::Path::new("."), &[], tx).unwrap();
        let message = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("a disconnect message");
        assert!(matches!(message, AdapterMessage::Disconnected));
        client.terminate();
    }

    #[test]
    fn sequence_numbers_increase() {
        let (tx, _rx) = mpsc::channel();
        // `cat` echoes nothing useful but accepts writes.
        let client = DapClient::start("cat", &[], std::path::Path::new("."), &[], tx).unwrap();
        assert_eq!(client.request("initialize", Value::Null).unwrap(), 1);
        assert_eq!(client.request("launch", Value::Null).unwrap(), 2);
    }
}
