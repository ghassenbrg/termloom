//! The debug session driver.
//!
//! A dedicated thread owns the adapter and runs the DAP handshake
//! (`initialize` → `launch`/`attach` → breakpoints → `configurationDone`),
//! turning protocol traffic into [`DebugEvent`]s and UI commands into
//! requests. The workbench never blocks on the adapter.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};

use crate::domain::debug::{DebugCapabilities, DebugThread, StackFrame, Variable};

use super::client::{AdapterMessage, DapClient};
use super::config::DebugLaunchConfig;
use super::protocol;

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// What the UI asks the session to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugCommand {
    Continue,
    Pause,
    StepOver,
    StepIn,
    StepOut,
    Stop,
    /// Replace the breakpoints of one file (0-based lines).
    SetBreakpoints(PathBuf, Vec<usize>),
    SelectFrame(i64),
    Evaluate(String),
}

/// What the session tells the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugEvent {
    /// Adapter initialised and reported its capabilities.
    Initialized {
        adapter: String,
        capabilities: DebugCapabilities,
    },
    /// The program is running.
    Running,
    /// Execution stopped; frames are ordered innermost first.
    Stopped {
        reason: String,
        thread_id: i64,
        frames: Vec<StackFrame>,
        threads: Vec<DebugThread>,
    },
    /// Variables for the selected frame.
    Variables(Vec<Variable>),
    /// Adapter or debuggee output.
    Output { category: String, text: String },
    /// Breakpoints the adapter accepted (0-based lines).
    BreakpointsVerified { path: PathBuf, lines: Vec<usize> },
    /// Result of an `evaluate` request.
    Evaluated { expression: String, value: String },
    /// The program finished.
    Exited { code: i64 },
    /// The session ended.
    Terminated { session_id: u64 },
    /// Something went wrong; the message is user-facing. A rejected optional
    /// request (for example `evaluate`) is recoverable and must not make the
    /// whole session appear dead.
    Failed {
        session_id: u64,
        message: String,
        fatal: bool,
    },
}

/// Callback used to publish [`DebugEvent`]s.
pub type DebugSink = Arc<dyn Fn(DebugEvent) + Send + Sync>;

/// Handle to a running session.
pub struct DebugSession {
    commands: Sender<DebugCommand>,
    session_id: u64,
    /// The adapter command line, for display and consent.
    pub command_line: String,
    pub configuration: String,
}

impl DebugSession {
    /// Start an adapter and drive it to a running program.
    pub fn start(config: DebugLaunchConfig, sink: DebugSink) -> Result<DebugSession> {
        let (message_tx, message_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);

        let client = DapClient::start(
            &config.adapter_command,
            &config.adapter_args,
            &config.cwd,
            &config.adapter_env,
            message_tx,
        )?;
        let command_line = client.command_line.clone();
        let configuration = config.name.clone();

        std::thread::Builder::new()
            .name("termloom-debug".into())
            .spawn(move || {
                let mut driver = Driver::new(client, config, sink, session_id);
                driver.run(message_rx, command_rx);
            })
            .expect("spawning the debug driver thread");

        Ok(DebugSession {
            commands: command_tx,
            session_id,
            command_line,
            configuration,
        })
    }

    /// Queue a command. Fails only when the session already ended.
    pub fn send(&self, command: DebugCommand) -> Result<()> {
        self.commands
            .send(command)
            .map_err(|_| anyhow::anyhow!("the debug session has ended"))
    }

    pub fn session_id(&self) -> u64 {
        self.session_id
    }
}

/// What a pending response is for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Initialize,
    Launch,
    ConfigurationDone,
    Breakpoints(PathBuf),
    Threads,
    StackTrace,
    Scopes,
    Variables(usize),
    Evaluate(String),
    Other(String),
}

struct Driver {
    client: DapClient,
    config: DebugLaunchConfig,
    sink: DebugSink,
    pending: HashMap<i64, Pending>,
    capabilities: DebugCapabilities,
    threads: Vec<DebugThread>,
    frames: Vec<StackFrame>,
    current_thread: i64,
    selected_frame: Option<i64>,
    variables: Vec<Variable>,
    /// Scope references still to fetch for the selected frame.
    /// Scopes still to fetch for the selected frame, in adapter order.
    scope_queue: std::collections::VecDeque<(String, i64)>,
    breakpoints: HashMap<PathBuf, Vec<usize>>,
    launched: bool,
    /// Set once the adapter answered `initialize`.
    initialised: bool,
    /// Reason reported by the last `stopped` event.
    last_stop_reason: String,
    session_id: u64,
    ended: bool,
}

impl Driver {
    fn new(
        client: DapClient,
        config: DebugLaunchConfig,
        sink: DebugSink,
        session_id: u64,
    ) -> Driver {
        Driver {
            client,
            breakpoints: config.breakpoints.clone(),
            last_stop_reason: "paused".to_string(),
            config,
            sink,
            pending: HashMap::new(),
            capabilities: DebugCapabilities::default(),
            threads: Vec::new(),
            frames: Vec::new(),
            current_thread: 1,
            selected_frame: None,
            variables: Vec::new(),
            scope_queue: std::collections::VecDeque::new(),
            launched: false,
            initialised: false,
            session_id,
            ended: false,
        }
    }

    fn run(&mut self, messages: Receiver<AdapterMessage>, commands: Receiver<DebugCommand>) {
        if let Err(err) = self.send(
            "initialize",
            protocol::initialize_arguments(),
            Pending::Initialize,
        ) {
            // An adapter that died on startup makes this write fail with a
            // bare "broken pipe"; name the adapter instead.
            tracing::debug!(error = %err, "debug adapter initialize failed");
            self.emit(DebugEvent::Failed {
                session_id: self.session_id,
                message: format!(
                    "`{}` exited before the debug session could start",
                    self.config.adapter_command
                ),
                fatal: true,
            });
            self.emit(DebugEvent::Terminated {
                session_id: self.session_id,
            });
            return;
        }

        loop {
            // Adapter traffic first, then queued UI commands.
            match messages.try_recv() {
                Ok(AdapterMessage::Message(message)) => {
                    self.on_message(message);
                    if self.ended {
                        return;
                    }
                    continue;
                }
                Ok(AdapterMessage::Disconnected) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.finish_disconnected();
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }

            match commands.try_recv() {
                Ok(command) => {
                    if self.on_command(command) {
                        return;
                    }
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.client.terminate();
                    self.emit(DebugEvent::Terminated {
                        session_id: self.session_id,
                    });
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }

            // Nothing to do: wait briefly for either source.
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// The adapter's pipe closed. A disconnect before the handshake finished
    /// is a broken adapter, not a finished debug session, and saying only
    /// "terminated" would leave the user guessing.
    fn finish_disconnected(&mut self) {
        if !self.initialised || !self.launched {
            self.emit(DebugEvent::Failed {
                session_id: self.session_id,
                message: format!(
                    "`{}` exited before the debug session could start",
                    self.config.adapter_command
                ),
                fatal: true,
            });
        }
        self.emit(DebugEvent::Terminated {
            session_id: self.session_id,
        });
    }

    fn send(&mut self, command: &str, arguments: Value, pending: Pending) -> Result<()> {
        let seq = self.client.request(command, arguments)?;
        self.pending.insert(seq, pending);
        Ok(())
    }

    fn emit(&self, event: DebugEvent) {
        (self.sink)(event);
    }

    /// Returns true when the session should end.
    fn on_command(&mut self, command: DebugCommand) -> bool {
        let thread = self.current_thread;
        let result = match command {
            DebugCommand::Continue => self.send(
                "continue",
                json!({ "threadId": thread }),
                Pending::Other("continue".into()),
            ),
            DebugCommand::Pause => self.send(
                "pause",
                json!({ "threadId": thread }),
                Pending::Other("pause".into()),
            ),
            DebugCommand::StepOver => self.send(
                "next",
                json!({ "threadId": thread }),
                Pending::Other("next".into()),
            ),
            DebugCommand::StepIn => self.send(
                "stepIn",
                json!({ "threadId": thread }),
                Pending::Other("stepIn".into()),
            ),
            DebugCommand::StepOut => self.send(
                "stepOut",
                json!({ "threadId": thread }),
                Pending::Other("stepOut".into()),
            ),
            DebugCommand::Stop => {
                self.client.terminate();
                self.emit(DebugEvent::Terminated {
                    session_id: self.session_id,
                });
                return true;
            }
            DebugCommand::SetBreakpoints(path, lines) => {
                self.breakpoints.insert(path.clone(), lines.clone());
                self.send(
                    "setBreakpoints",
                    protocol::set_breakpoints(&path, &lines),
                    Pending::Breakpoints(path),
                )
            }
            DebugCommand::SelectFrame(frame_id) => {
                self.selected_frame = Some(frame_id);
                self.variables.clear();
                self.send("scopes", protocol::scopes(frame_id), Pending::Scopes)
            }
            DebugCommand::Evaluate(expression) => self.send(
                "evaluate",
                protocol::evaluate(&expression, self.selected_frame),
                Pending::Evaluate(expression),
            ),
        };
        if let Err(err) = result {
            self.emit(DebugEvent::Failed {
                session_id: self.session_id,
                message: format!("{err:#}"),
                fatal: true,
            });
            self.client.terminate();
            self.emit(DebugEvent::Terminated {
                session_id: self.session_id,
            });
            return true;
        }
        false
    }

    fn on_message(&mut self, message: Value) {
        match message["type"].as_str() {
            Some("response") => self.on_response(message),
            Some("event") => self.on_event(message),
            // Reverse requests (runInTerminal) are declined politely.
            Some("request") => {
                tracing::debug!(command = %message["command"], "declining adapter request");
            }
            _ => {}
        }
    }

    fn on_response(&mut self, message: Value) {
        let seq = message["request_seq"].as_i64().unwrap_or(-1);
        let Some(pending) = self.pending.remove(&seq) else {
            return;
        };
        let success = message["success"].as_bool().unwrap_or(false);
        let body = message.get("body").cloned().unwrap_or(Value::Null);

        if !success {
            let command = message["command"].as_str().unwrap_or("request");
            let reason = message["message"]
                .as_str()
                .unwrap_or("the adapter rejected the request");
            // A failed step or evaluate is not fatal; a failed launch is.
            let fatal = matches!(pending, Pending::Initialize | Pending::Launch);
            self.emit(DebugEvent::Failed {
                session_id: self.session_id,
                message: format!("{command} failed: {reason}"),
                fatal,
            });
            if fatal {
                self.client.terminate();
                self.emit(DebugEvent::Terminated {
                    session_id: self.session_id,
                });
                self.ended = true;
            }
            return;
        }

        match pending {
            Pending::Initialize => {
                self.initialised = true;
                self.capabilities = protocol::parse_capabilities(&body);
                self.emit(DebugEvent::Initialized {
                    adapter: self.config.adapter_command.clone(),
                    capabilities: self.capabilities,
                });
                let request = if self.config.attach {
                    "attach"
                } else {
                    "launch"
                };
                let arguments = self.config.request_arguments();
                if let Err(err) = self.send(request, arguments, Pending::Launch) {
                    self.emit(DebugEvent::Failed {
                        session_id: self.session_id,
                        message: format!("{err:#}"),
                        fatal: true,
                    });
                    self.client.terminate();
                    self.emit(DebugEvent::Terminated {
                        session_id: self.session_id,
                    });
                    self.ended = true;
                }
            }
            Pending::Launch => {
                self.launched = true;
                self.emit(DebugEvent::Running);
            }
            Pending::ConfigurationDone => {}
            Pending::Breakpoints(path) => {
                let verified: Vec<usize> = protocol::parse_breakpoints(&body)
                    .into_iter()
                    .filter(|(_, verified)| *verified)
                    .map(|(line, _)| line)
                    .collect();
                self.emit(DebugEvent::BreakpointsVerified {
                    path,
                    lines: verified,
                });
            }
            Pending::Threads => {
                self.threads = protocol::parse_threads(&body);
            }
            Pending::StackTrace => {
                self.frames = protocol::parse_stack_frames(&body);
                self.selected_frame = self.frames.first().map(|frame| frame.id);
                self.emit(DebugEvent::Stopped {
                    reason: self.last_stop_reason.clone(),
                    thread_id: self.current_thread,
                    frames: self.frames.clone(),
                    threads: self.threads.clone(),
                });
                if let Some(frame) = self.selected_frame {
                    let _ = self.send("scopes", protocol::scopes(frame), Pending::Scopes);
                }
            }
            Pending::Scopes => {
                self.variables.clear();
                // Adapters list the most useful scope first (Locals before
                // Registers); keep that order in the panel.
                self.scope_queue = protocol::parse_scopes(&body).into_iter().collect();
                self.request_next_scope();
            }
            Pending::Variables(depth) => {
                self.variables
                    .extend(protocol::parse_variables(&body, depth));
                if self.scope_queue.is_empty() {
                    self.emit(DebugEvent::Variables(self.variables.clone()));
                } else {
                    self.request_next_scope();
                }
            }
            Pending::Evaluate(expression) => {
                let value = body["result"].as_str().unwrap_or_default().to_string();
                self.emit(DebugEvent::Evaluated { expression, value });
            }
            Pending::Other(_) => {}
        }
    }

    /// Fetch the next scope's variables, adding a header row for it.
    fn request_next_scope(&mut self) {
        let Some((name, reference)) = self.scope_queue.pop_front() else {
            self.emit(DebugEvent::Variables(self.variables.clone()));
            return;
        };
        self.variables.push(Variable {
            name,
            value: String::new(),
            type_name: None,
            variables_reference: 0,
            depth: 0,
        });
        let _ = self.send(
            "variables",
            protocol::variables(reference),
            Pending::Variables(1),
        );
    }

    fn on_event(&mut self, message: Value) {
        let event = message["event"].as_str().unwrap_or_default();
        let body = &message["body"];
        match event {
            "initialized" => {
                // Now the adapter accepts configuration.
                let breakpoints = self.breakpoints.clone();
                for (path, lines) in breakpoints {
                    let _ = self.send(
                        "setBreakpoints",
                        protocol::set_breakpoints(&path, &lines),
                        Pending::Breakpoints(path),
                    );
                }
                if self.capabilities.configuration_done {
                    let _ = self.send("configurationDone", Value::Null, Pending::ConfigurationDone);
                }
            }
            "stopped" => {
                self.current_thread = body["threadId"].as_i64().unwrap_or(self.current_thread);
                self.last_stop_reason = body["reason"].as_str().unwrap_or("paused").to_string();
                let _ = self.send("threads", Value::Null, Pending::Threads);
                let _ = self.send(
                    "stackTrace",
                    protocol::stack_trace(self.current_thread),
                    Pending::StackTrace,
                );
            }
            "continued" => self.emit(DebugEvent::Running),
            "output" => {
                let text = body["output"].as_str().unwrap_or_default().to_string();
                if !text.is_empty() {
                    self.emit(DebugEvent::Output {
                        category: body["category"].as_str().unwrap_or("console").to_string(),
                        text,
                    });
                }
            }
            "exited" => self.emit(DebugEvent::Exited {
                code: body["exitCode"].as_i64().unwrap_or(0),
            }),
            "terminated" => {
                self.emit(DebugEvent::Terminated {
                    session_id: self.session_id,
                });
            }
            "breakpoint" => {
                // A breakpoint was (un)verified after the fact.
                if let Some(path) = body["breakpoint"]["source"]["path"].as_str() {
                    let line = body["breakpoint"]["line"]
                        .as_u64()
                        .unwrap_or(1)
                        .saturating_sub(1) as usize;
                    if body["breakpoint"]["verified"].as_bool().unwrap_or(false) {
                        self.emit(DebugEvent::BreakpointsVerified {
                            path: PathBuf::from(path),
                            lines: vec![line],
                        });
                    }
                }
            }
            _ => {}
        }
    }
}
