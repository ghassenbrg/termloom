//! The single application event channel.
//!
//! Background work (terminal output, filesystem watching, git refreshes,
//! language servers) never touches application state directly: everything
//! becomes an [`AppEvent`] handled by the main loop, which keeps state
//! mutation single-threaded and rendering predictable.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use crossterm::event::{KeyEvent, MouseEvent};

use crate::domain::agent::AgentSession;
use crate::domain::diagnostics::Diagnostic;
use crate::domain::git::GitSnapshot;
use crate::domain::ids::TerminalId;
use crate::services::workspace::FileIndex;

/// Everything that can move the application forward.
#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    /// Periodic tick used for state refreshes and duration displays.
    Tick,
    /// Paths changed on disk.
    FileSystem(Vec<PathBuf>),
    /// A terminal produced output.
    TerminalOutput(TerminalId),
    /// A terminal's child exited.
    TerminalExited(TerminalId, Option<i32>),
    /// A recomputed git snapshot.
    GitUpdated(Box<GitSnapshot>),
    /// Refreshed agent list from the backends.
    AgentsUpdated(Vec<AgentSession>),
    /// Quick-open index finished scanning.
    FileIndexed(FileIndex),
    /// Diagnostics published by a language server.
    Diagnostics {
        server: String,
        path: PathBuf,
        diagnostics: Vec<Diagnostic>,
    },
    /// A language server changed state (started, crashed, stopped).
    LanguageServer(LanguageServerEvent),
    /// A debug adapter event.
    Debug(DebugEvent),
    /// Something to show the user.
    Notice(Notice),
    /// Quit requested by a background task.
    Quit,
}

/// Language server lifecycle notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageServerEvent {
    Initialized { server: String },
    Stopped { server: String },
    Crashed { server: String, message: String },
    Progress { server: String, message: String },
}

/// Debug session notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugEvent {
    Initialized,
    Stopped { reason: String, thread_id: i64 },
    Continued,
    Output { category: String, text: String },
    Terminated,
    Exited { code: i64 },
    Failed { message: String },
}

/// Severity of a user-facing notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// A transient message rendered as a toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: NoticeLevel,
    pub text: String,
}

impl Notice {
    pub fn info(text: impl Into<String>) -> Notice {
        Notice {
            level: NoticeLevel::Info,
            text: text.into(),
        }
    }

    pub fn warning(text: impl Into<String>) -> Notice {
        Notice {
            level: NoticeLevel::Warning,
            text: text.into(),
        }
    }

    pub fn error(text: impl Into<String>) -> Notice {
        Notice {
            level: NoticeLevel::Error,
            text: text.into(),
        }
    }
}

/// Sending half of the application channel, cloned into background threads.
pub type EventSender = Sender<AppEvent>;

/// The application's event bus.
pub struct EventBus {
    pub sender: EventSender,
    pub receiver: Receiver<AppEvent>,
}

impl EventBus {
    pub fn new() -> EventBus {
        let (sender, receiver) = std::sync::mpsc::channel();
        EventBus { sender, receiver }
    }

    /// Block for the next event, up to `timeout`.
    pub fn next(&self, timeout: Duration) -> Option<AppEvent> {
        self.receiver.recv_timeout(timeout).ok()
    }

    /// Drain everything queued without blocking. Used to coalesce bursts of
    /// terminal output into a single redraw.
    pub fn drain(&self) -> Vec<AppEvent> {
        self.receiver.try_iter().collect()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        EventBus::new()
    }
}

/// Spawn the crossterm input reader. Terminal events arrive on their own
/// thread so a slow render never drops keystrokes.
pub fn spawn_input_reader(sender: EventSender) {
    std::thread::Builder::new()
        .name("termloom-input".into())
        .spawn(move || loop {
            match crossterm::event::read() {
                Ok(crossterm::event::Event::Key(key)) => {
                    // Key release events would double every keystroke.
                    if key.kind == crossterm::event::KeyEventKind::Release {
                        continue;
                    }
                    if sender.send(AppEvent::Key(key)).is_err() {
                        break;
                    }
                }
                Ok(crossterm::event::Event::Mouse(mouse)) => {
                    if sender.send(AppEvent::Mouse(mouse)).is_err() {
                        break;
                    }
                }
                Ok(crossterm::event::Event::Resize(cols, rows)) => {
                    if sender.send(AppEvent::Resize(cols, rows)).is_err() {
                        break;
                    }
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        })
        .expect("spawning the input thread");
}

/// Spawn the periodic tick used for agent/git refreshes.
pub fn spawn_ticker(sender: EventSender, interval: Duration) {
    std::thread::Builder::new()
        .name("termloom-tick".into())
        .spawn(move || loop {
            std::thread::sleep(interval);
            if sender.send(AppEvent::Tick).is_err() {
                break;
            }
        })
        .expect("spawning the tick thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_round_trip_through_the_bus() {
        let bus = EventBus::new();
        bus.sender.send(AppEvent::Tick).unwrap();
        assert!(matches!(
            bus.next(Duration::from_millis(100)),
            Some(AppEvent::Tick)
        ));
        assert!(bus.next(Duration::from_millis(10)).is_none());
    }

    #[test]
    fn drain_collects_a_burst() {
        let bus = EventBus::new();
        for _ in 0..5 {
            bus.sender
                .send(AppEvent::TerminalOutput(TerminalId::next()))
                .unwrap();
        }
        assert_eq!(bus.drain().len(), 5);
        assert!(bus.drain().is_empty());
    }

    #[test]
    fn ticker_produces_events() {
        let bus = EventBus::new();
        spawn_ticker(bus.sender.clone(), Duration::from_millis(10));
        assert!(matches!(
            bus.next(Duration::from_secs(2)),
            Some(AppEvent::Tick)
        ));
    }

    #[test]
    fn notices_carry_their_level() {
        assert_eq!(Notice::error("boom").level, NoticeLevel::Error);
        assert_eq!(Notice::info("hi").text, "hi");
    }
}
