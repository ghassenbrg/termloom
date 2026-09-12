//! Command execution.
//!
//! Every command id from [`super::commands`] is handled here. Keeping them in
//! one place means a keybinding, a palette entry and a mouse click all run the
//! same code path.

use std::path::{Path, PathBuf};

use crate::domain::agent::{AgentKind, AgentState};
use crate::domain::diagnostics::Position;
use crate::services::agents::{local::request_from_preset, SpawnAgentRequest};
use crate::services::dap::{self, DebugCommand, DebugSession};
use crate::services::lsp::{RequestExtra, RequestKind};
use crate::services::terminal::SpawnSpec;
use crate::services::workspace::ScanOptions;
use crate::services::{fs_ops, git};

use super::events::Notice;
use super::focus::{Direction, FocusTarget};
use super::services::Services;
use super::state::{
    AgentDetailTab, AppState, ConfirmAction, PaletteMode, PromptPurpose, SelectPurpose,
};

/// Run a command by id. Unknown ids are reported rather than ignored.
pub fn execute(state: &mut AppState, services: &mut Services, id: &str) {
    match id {
        // ── file ──────────────────────────────────────────────────────────
        "file.save" => save_active(state, services, false),
        "file.save_all" => save_all(state, services),
        "file.reload" => reload_active(state),
        "file.new" => {
            let dir = state.explorer_target_dir();
            state.prompt("New file", "", PromptPurpose::NewFile(dir));
        }
        "file.new_folder" => {
            let dir = state.explorer_target_dir();
            state.prompt("New folder", "", PromptPurpose::NewFolder(dir));
        }
        "file.rename" => match state.explorer_selection() {
            Some(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                state.prompt("Rename", name, PromptPurpose::Rename(path));
            }
            None => state.warn("select something in the explorer first"),
        },
        "file.delete" => match state.explorer_selection() {
            Some(path) => state.confirm(
                "Delete",
                format!("Delete {}? This cannot be undone.", path.display()),
                ConfirmAction::DeletePath(path),
            ),
            None => state.warn("select something in the explorer first"),
        },
        "file.reveal" => {
            if let Some(path) = state.active_document().and_then(|d| d.path.clone()) {
                state.reveal_in_explorer(&path);
                state.focus = FocusTarget::Explorer;
            }
        }

        // ── editor ────────────────────────────────────────────────────────
        "editor.close_tab" => close_active_tab(state, services),
        "editor.close_others" => {
            let keep = state.active_tab;
            let ids: Vec<_> = state
                .documents
                .iter()
                .filter(|d| Some(d.id) != keep && !d.is_dirty())
                .map(|d| d.id)
                .collect();
            for id in ids {
                close_tab(state, services, id, false);
            }
        }
        "editor.next_tab" => state.next_tab(1),
        "editor.prev_tab" => state.next_tab(-1),
        "editor.reopen_tab" => {
            if let Some(path) = state.recently_closed.pop_back() {
                if let Err(err) = state.open_file(&path) {
                    state.error(format!("{err:#}"));
                }
            } else {
                state.info("no recently closed files");
            }
        }
        "editor.undo" => {
            if let Some(document) = state.active_document_mut() {
                if !document.buffer.undo() {
                    state.info("nothing to undo");
                }
            }
        }
        "editor.redo" => {
            if let Some(document) = state.active_document_mut() {
                if !document.buffer.redo() {
                    state.info("nothing to redo");
                }
            }
        }
        "editor.find" => {
            let seed = state
                .active_document()
                .and_then(|d| d.buffer.selected_text())
                .or_else(|| {
                    state
                        .active_document()
                        .and_then(|d| d.buffer.word_at(d.buffer.cursor()))
                })
                .unwrap_or_default();
            state.prompt("Find", seed, PromptPurpose::FindInFile);
        }
        "editor.find_next" => find_step(state, true),
        "editor.find_prev" => find_step(state, false),
        "editor.goto_line" => state.prompt("Go to line", "", PromptPurpose::GotoLine),
        "editor.copy" => copy_selection(state, false),
        "editor.cut" => copy_selection(state, true),
        "editor.paste" => paste(state),
        "editor.select_all" => {
            if let Some(document) = state.active_document_mut() {
                document.buffer.select_all();
            }
        }
        "editor.toggle_comment" => toggle_comment(state),

        // ── view ──────────────────────────────────────────────────────────
        "view.toggle.explorer" => state.layout.explorer = !state.layout.explorer,
        "view.toggle.outline" => state.layout.outline = !state.layout.outline,
        "view.toggle.git" => state.layout.git = !state.layout.git,
        "view.toggle.agents" => state.layout.agents = !state.layout.agents,
        "view.toggle.terminals" => state.layout.terminals = !state.layout.terminals,
        "view.toggle.problems" => {
            state.layout.problems = !state.layout.problems;
            if state.layout.problems {
                state.focus = FocusTarget::Problems;
            }
        }
        "view.toggle.debug" => {
            state.layout.debug = !state.layout.debug;
            if state.layout.debug {
                state.focus = FocusTarget::Debug;
            }
        }
        "view.toggle.extensions" | "extensions.list" => {
            state.layout.extensions = !state.layout.extensions;
            if state.layout.extensions {
                reload_extensions(state);
            }
        }
        "extensions.install" => state.prompt("Install VSIX", "", PromptPurpose::InstallVsix),
        "extensions.inspect" => state.prompt("Inspect VSIX", "", PromptPurpose::InspectVsix),
        "extensions.remove" => match state
            .extensions
            .installed
            .get(state.extensions.selected)
            .map(|report| report.id.clone())
        {
            Some(id) => state.confirm(
                "Remove extension",
                format!("Remove {id} and its installed declarative assets?"),
                ConfirmAction::RemoveExtension(id),
            ),
            None => state.warn("no extension selected"),
        },
        "view.focus.explorer" => {
            state.layout.explorer = true;
            state.focus = FocusTarget::Explorer;
        }
        "view.focus.outline" => {
            state.layout.outline = true;
            state.focus = FocusTarget::Outline;
        }
        "view.focus.git" => {
            state.layout.git = true;
            state.focus = FocusTarget::Git;
        }
        "view.focus.agents" => {
            state.layout.agents = true;
            state.focus = FocusTarget::AgentList;
        }
        "view.focus.editor" => {
            if let Some(id) = state.active_tab {
                state.focus = FocusTarget::Editor(id);
            }
        }
        "view.focus.terminal" => {
            if let Some(id) = state
                .focused_terminal
                .or(state.terminal_ids().first().copied())
            {
                state.focus_terminal(id);
            }
        }
        "focus.left" => state.move_focus(Direction::Left),
        "focus.right" => state.move_focus(Direction::Right),
        "focus.up" => state.move_focus(Direction::Up),
        "focus.down" => state.move_focus(Direction::Down),

        // ── terminals ─────────────────────────────────────────────────────
        "terminal.new" => new_terminal(state),
        "terminal.next" => state.cycle_terminal(1),
        "terminal.prev" => state.cycle_terminal(-1),
        "terminal.kill" => with_focused_terminal(state, |session| {
            let _ = session.kill();
        }),
        "terminal.restart" => restart_terminal(state),
        "terminal.close" => {
            if let Some(id) = state.focused_terminal {
                if let Ok(mut terminals) = state.terminals.lock() {
                    terminals.close(id);
                }
                state.reconcile_terminal_focus();
            }
        }
        "terminal.clear_scroll" => with_focused_terminal(state, |session| {
            session.set_scroll_offset(0);
        }),

        // ── agents ────────────────────────────────────────────────────────
        "agent.new.claude" => spawn_preset(state, services, "claude"),
        "agent.new.codex" => spawn_preset(state, services, "codex"),
        "agent.new.shell" => spawn_shell_agent(state, services),
        "agent.new.custom" => state.prompt(
            "Run command as agent",
            "",
            PromptPurpose::CustomAgentCommand,
        ),
        "agent.focus_terminal" => {
            match state
                .selected_agent()
                .map(|agent| (agent.id, agent.terminal_id))
            {
                Some((_, Some(terminal))) => state.focus_terminal(terminal),
                Some((id, None)) => {
                    let result = services.runtime.block_on(async {
                        match services.agents.owner(id).await {
                            Some(backend) => backend.focus(id).await,
                            None => Err(anyhow::anyhow!("no backend owns this agent")),
                        }
                    });
                    match result {
                        Ok(()) => state.info("focused the agent in Herdr"),
                        Err(err) => state.warn(format!("could not focus agent: {err:#}")),
                    }
                }
                None => state.warn("no agent selected"),
            }
        }
        "agent.send_input" => match state.selected_agent_id() {
            Some(id) => state.prompt("Send to agent", "", PromptPurpose::AgentInput(id)),
            None => state.warn("no agent selected"),
        },
        "agent.stop" => match state.selected_agent() {
            Some(agent) => {
                let (id, label) = (agent.id, agent.label.clone());
                state.confirm(
                    "Stop agent",
                    format!("Stop {label}? The process will be terminated."),
                    ConfirmAction::StopAgent(id),
                );
            }
            None => state.warn("no agent selected"),
        },
        "agent.restart" => {
            if let Some(id) = state.selected_agent_id() {
                let registry = &services.agents;
                let result = services.runtime.block_on(async {
                    match registry.owner(id).await {
                        Some(backend) => backend.restart(id).await.map(|_| ()),
                        None => Err(anyhow::anyhow!("no backend owns this agent")),
                    }
                });
                match result {
                    Ok(()) => state.info("agent restarted"),
                    Err(err) => state.error(format!("restart failed: {err:#}")),
                }
                refresh_agents(state, services);
            }
        }
        "agent.rename" => match state.selected_agent() {
            Some(agent) => {
                let (id, label) = (agent.id, agent.label.clone());
                state.prompt("Rename agent", label, PromptPurpose::AgentRename(id));
            }
            None => state.warn("no agent selected"),
        },
        "agent.remove" => {
            if let Some(id) = state.selected_agent_id() {
                let registry = &services.agents;
                let _ = services.runtime.block_on(async {
                    match registry.owner(id).await {
                        Some(backend) => backend.remove(id).await,
                        None => Ok(()),
                    }
                });
                state.agent_tasks.remove(&id);
                refresh_agents(state, services);
                state.agent_selection.clamp(state.agents.len());
                state.reconcile_terminal_focus();
            }
        }
        "agent.task.add" => match state.selected_agent_id() {
            Some(id) => state.prompt("New task", "", PromptPurpose::AgentTask(id)),
            None => state.warn("no agent selected"),
        },
        "agent.task.toggle" => match state.toggle_selected_task() {
            Some(done) => {
                state.agent_tab = AgentDetailTab::Tasks;
                state.info(if done { "task done" } else { "task reopened" });
            }
            None => state.info("no task selected — add one with agent.task.add"),
        },

        // ── git ───────────────────────────────────────────────────────────
        "git.refresh" => services.refresh_git(state.git_root()),
        "git.stage" => git_operation(state, services, "stage"),
        "git.unstage" => git_operation(state, services, "unstage"),
        "git.stage_all" => git_operation(state, services, "stage_all"),
        "git.unstage_all" => git_operation(state, services, "unstage_all"),
        "git.discard" => match state.selected_git_path() {
            Some(path) => state.confirm(
                "Discard changes",
                format!(
                    "Discard all changes to {}? This cannot be undone.",
                    path.display()
                ),
                ConfirmAction::DiscardGitChange(path),
            ),
            None => state.warn("no changed file selected"),
        },
        "git.open_diff" => open_diff(state),
        "git.open_file" => {
            if let Some(path) = state
                .selected_git_path()
                .and_then(|rel| state.git_absolute(&rel))
            {
                if let Err(err) = state.open_file(&path) {
                    state.error(format!("{err:#}"));
                }
            }
        }

        // ── palette / workspace / app ─────────────────────────────────────
        "palette.commands" => state.open_palette(PaletteMode::Commands),
        "palette.quick_open" => state.open_palette(PaletteMode::Files),
        "workspace.reload" => reload_workspace(state, services),
        "workspace.open_config" => open_config(state),
        "help.toggle" => state.help_visible = !state.help_visible,
        "app.quit" => quit(state),
        "workbench.prefix" => state.prefix_active = true,

        // ── debugging ─────────────────────────────────────────────────────
        "debug.start" => start_debugging(state, services),
        "debug.stop" => debug_command(state, services, DebugCommand::Stop),
        "debug.continue" => debug_command(state, services, DebugCommand::Continue),
        "debug.pause" => debug_command(state, services, DebugCommand::Pause),
        "debug.step_over" => debug_command(state, services, DebugCommand::StepOver),
        "debug.step_in" => debug_command(state, services, DebugCommand::StepIn),
        "debug.step_out" => debug_command(state, services, DebugCommand::StepOut),
        "debug.evaluate" => state.prompt("Evaluate expression", "", PromptPurpose::DebugEvaluate),
        "debug.toggle_breakpoint" => toggle_breakpoint(state, services),
        "debug.clear_breakpoints" => {
            let paths: Vec<PathBuf> = state
                .documents
                .iter()
                .filter(|d| !d.breakpoints.is_empty())
                .filter_map(|d| d.path.clone())
                .collect();
            for document in &mut state.documents {
                document.breakpoints.clear();
            }
            for path in paths {
                sync_breakpoints(state, services, &path);
            }
            state.info("all breakpoints removed");
        }
        "workspace.run_task" => run_task(state),

        // ── language services ─────────────────────────────────────────────
        "lsp.start" => start_language_server(state, services),
        "lsp.restart" => restart_language_server(state, services),
        "lsp.stop" => stop_language_server(state, services),
        "lsp.hover" => lsp_request(state, services, RequestKind::Hover, RequestExtra::None),
        "lsp.completion" => {
            let local = show_snippet_completions(state);
            if let Err(err) =
                send_lsp_request(state, services, RequestKind::Completion, RequestExtra::None)
            {
                if !local {
                    state.warn(format!("{err:#}"));
                }
            }
        }
        "lsp.definition" => {
            lsp_request(state, services, RequestKind::Definition, RequestExtra::None)
        }
        "lsp.references" => {
            lsp_request(state, services, RequestKind::References, RequestExtra::None)
        }
        "lsp.rename" => match state
            .active_document()
            .and_then(|d| d.buffer.word_at(d.buffer.cursor()))
        {
            Some(word) => {
                let position = state
                    .active_document()
                    .map(|d| d.buffer.cursor())
                    .unwrap_or_default();
                state.prompt("Rename symbol", word, PromptPurpose::RenameSymbol(position));
            }
            None => state.warn("place the cursor on a symbol first"),
        },
        "lsp.format" => {
            let extra = RequestExtra::Formatting {
                tab_size: state.config.editor.tab_width,
                spaces: state.config.editor.insert_spaces,
            };
            lsp_request(state, services, RequestKind::Formatting, extra);
        }
        "lsp.back" => match state.navigation_history.pop() {
            Some((path, position)) => match state.open_file(&path) {
                Ok(id) => {
                    if let Some(document) = state.document_mut(id) {
                        document.goto(position);
                    }
                }
                Err(err) => state.error(format!("{err:#}")),
            },
            None => state.info("nowhere to go back to"),
        },

        // ── not yet connected to a running service ────────────────────────
        other => unavailable(state, other),
    }
}

// ── debugging ─────────────────────────────────────────────────────────────

/// Offer the available launch configurations, then ask for consent.
fn start_debugging(state: &mut AppState, services: &mut Services) {
    if services.debug.is_some() {
        state.warn("a debug session is already running");
        return;
    }
    let entries = dap::available(&state.config, &state.workspace.root);
    if entries.is_empty() {
        state.error(
            "no debug configuration found — add one under [dap] in config.toml or a .vscode/launch.json",
        );
        return;
    }
    if entries.len() == 1 {
        confirm_debug_start(state, entries[0].clone());
        return;
    }
    let options = entries
        .iter()
        .map(|entry| format!("{}  ({})", entry.name, entry.source.label()))
        .collect();
    state.select(
        "Start debugging",
        options,
        SelectPurpose::DebugConfiguration(entries),
    );
}

/// Show the exact adapter command before anything is executed.
fn confirm_debug_start(state: &mut AppState, entry: dap::LaunchEntry) {
    let breakpoints = collect_breakpoints(state);
    match dap::resolve(&state.config, &state.workspace.root, &entry, breakpoints) {
        Ok(config) => {
            let unresolved = dap::config::unresolved_variables(&config.arguments);
            if !unresolved.is_empty() {
                state.warn(format!(
                    "unsupported launch variables left as-is: {}",
                    unresolved.join(", ")
                ));
            }
            state.confirm(
                "Run debug adapter",
                format!(
                    "{} will run this debug adapter:\n\n{}\n\nworking directory: {}",
                    config.name,
                    config.command_line(),
                    config.cwd.display()
                ),
                ConfirmAction::StartDebugSession(Box::new(config)),
            );
        }
        Err(err) => state.error(format!("{err:#}")),
    }
}

/// Breakpoints across all open documents, keyed by file.
fn collect_breakpoints(state: &AppState) -> std::collections::HashMap<PathBuf, Vec<usize>> {
    let mut out = std::collections::HashMap::new();
    for document in &state.documents {
        if document.breakpoints.is_empty() {
            continue;
        }
        if let Some(path) = &document.path {
            out.insert(
                path.clone(),
                document.breakpoints.iter().copied().collect::<Vec<_>>(),
            );
        }
    }
    out
}

/// Launch the adapter the user approved.
pub fn launch_debug_session(
    state: &mut AppState,
    services: &mut Services,
    config: dap::DebugLaunchConfig,
) {
    let sender = services.events.clone();
    let sink: crate::services::dap::DebugSink = std::sync::Arc::new(move |event| {
        let _ = sender.send(crate::app::events::AppEvent::Debug(Box::new(event)));
    });
    match DebugSession::start(config, sink) {
        Ok(session) => {
            state.debug = crate::app::state::DebugUiState {
                status: crate::domain::debug::DebugStatus::Starting,
                adapter: Some(session.command_line.clone()),
                ..Default::default()
            };
            state.layout.debug = true;
            services.debug = Some(session);
            state.info("debug session starting");
        }
        Err(err) => {
            state.debug.last_error = Some(format!("{err:#}"));
            state.error(format!("could not start the debug adapter: {err:#}"));
        }
    }
}

fn debug_command(state: &mut AppState, services: &mut Services, command: DebugCommand) {
    let Some(session) = services.debug.as_ref() else {
        state.warn("no debug session is running");
        return;
    };
    let stopping = command == DebugCommand::Stop;
    if let Err(err) = session.send(command) {
        state.warn(format!("{err:#}"));
    }
    if stopping {
        services.debug = None;
        state.debug.status = crate::domain::debug::DebugStatus::Terminated;
    }
}

fn toggle_breakpoint(state: &mut AppState, services: &mut Services) {
    let Some(document) = state.active_document_mut() else {
        state.warn("open a file first");
        return;
    };
    let line = document.buffer.cursor().line;
    let enabled = document.toggle_breakpoint(line);
    let path = document.path.clone();
    if let Some(path) = path {
        sync_breakpoints(state, services, &path);
        state.info(format!(
            "breakpoint {} at line {}",
            if enabled { "set" } else { "cleared" },
            line + 1
        ));
    }
}

/// Push a file's breakpoints to a running adapter.
fn sync_breakpoints(state: &mut AppState, services: &mut Services, path: &Path) {
    let Some(session) = services.debug.as_ref() else {
        return;
    };
    let lines = state
        .document_for_path(path)
        .map(|document| document.breakpoints.iter().copied().collect::<Vec<_>>())
        .unwrap_or_default();
    if let Err(err) = session.send(DebugCommand::SetBreakpoints(path.to_path_buf(), lines)) {
        tracing::debug!(error = %err, "could not update breakpoints");
    }
}

/// Offer the repository's tasks; running one is always explicit.
fn run_task(state: &mut AppState) {
    let tasks = crate::services::vscode::read_tasks(&state.workspace.root);
    if tasks.is_empty() {
        state.info("no tasks defined in .vscode/tasks.json");
        return;
    }
    let options = tasks
        .iter()
        .map(|task| format!("{}  ({})", task.label, task.command_line()))
        .collect();
    state.select("Run task", options, SelectPurpose::Task(tasks));
}

/// Start a task in a new terminal after the user confirmed it.
pub fn start_task(state: &mut AppState, task: &crate::services::vscode::VsCodeTask) {
    let cwd = task
        .cwd
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| state.workspace.root.clone());
    let (rows, cols) = state.terminal_pane_size;
    let result = {
        let Ok(mut terminals) = state.terminals.lock() else {
            return;
        };
        let mut spec = crate::services::terminal::SpawnSpec::shell(
            task.label.clone(),
            &task.command,
            &task.args,
            &cwd,
        );
        spec.kind = crate::domain::terminal::TerminalKind::Task;
        spec.rows = rows;
        spec.cols = cols;
        terminals.spawn(spec)
    };
    match result {
        Ok(id) => {
            state.layout.terminals = true;
            state.focus_terminal(id);
        }
        Err(err) => state.error(format!("could not run {}: {err:#}", task.label)),
    }
}

// ── language services ─────────────────────────────────────────────────────

fn start_language_server(state: &mut AppState, services: &mut Services) {
    let Some(language) = state.active_document().map(|d| d.language.clone()) else {
        state.warn("open a file first");
        return;
    };
    match services.lsp.ensure_started(&language) {
        Ok(name) => {
            state.info(format!("starting {name}"));
            state.lsp_statuses = services.lsp.statuses();
        }
        Err(err) => state.error(format!("{err:#}")),
    }
}

fn restart_language_server(state: &mut AppState, services: &mut Services) {
    let Some(name) = state
        .active_document()
        .and_then(|d| services.lsp.server_for(&d.language))
    else {
        state.warn("no language server is configured for this file");
        return;
    };
    match services.lsp.restart(&name) {
        Ok(()) => {
            state.info(format!("restarting {name}"));
            state.problems.retain(|d| d.path != PathBuf::new());
            state.lsp_statuses = services.lsp.statuses();
        }
        Err(err) => state.error(format!("{err:#}")),
    }
}

fn stop_language_server(state: &mut AppState, services: &mut Services) {
    let Some(name) = state
        .active_document()
        .and_then(|d| services.lsp.server_for(&d.language))
    else {
        return;
    };
    services.lsp.stop(&name);
    state.lsp_ready = services.lsp.any_ready();
    state.lsp_statuses = services.lsp.statuses();
    // Diagnostics from a stopped server are no longer trustworthy.
    state.problems.clear();
    for document in &mut state.documents {
        document.diagnostics.clear();
        document.synced_version = -1;
    }
    state.info(format!("stopped {name}"));
}

/// Send a position-based request for the active document.
fn lsp_request(
    state: &mut AppState,
    services: &mut Services,
    kind: RequestKind,
    extra: RequestExtra,
) {
    let Some(document) = state.active_document() else {
        state.warn("open a file first");
        return;
    };
    let (Some(path), language, position) = (
        document.path.clone(),
        document.language.clone(),
        document.buffer.cursor(),
    ) else {
        state.warn("save this buffer to a file first");
        return;
    };
    if let Err(err) = services
        .lsp
        .request(&language, kind, &path, position, extra)
    {
        state.warn(format!("{err:#}"));
    }
}

/// The completion command also works without an LSP when an installed VSIX
/// contributes snippets. If an LSP is live its response replaces/extends this
/// popup shortly afterwards.
fn show_snippet_completions(state: &mut AppState) -> bool {
    let Some(document) = state.active_document() else {
        return false;
    };
    let position = document.buffer.cursor();
    let prefix = document.buffer.word_at(position).unwrap_or_default();
    let Some(items) = state.snippets.get(document.language.as_str()).cloned() else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    let popup = crate::app::state::CompletionPopup {
        items,
        selected: 0,
        position,
        prefix,
    };
    // An empty filtered list would draw an invisible popup that swallows the
    // next keystrokes; say nothing matched instead.
    if popup.filtered().is_empty() {
        return false;
    }
    state.completion = Some(popup);
    true
}

fn send_lsp_request(
    state: &AppState,
    services: &mut Services,
    kind: RequestKind,
    extra: RequestExtra,
) -> anyhow::Result<()> {
    let document = state
        .active_document()
        .ok_or_else(|| anyhow::anyhow!("open a file first"))?;
    let path = document
        .path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("save this buffer to a file first"))?;
    services.lsp.request(
        &document.language,
        kind,
        path,
        document.buffer.cursor(),
        extra,
    )
}

/// Commands whose service is not running report exactly why.
fn unavailable(state: &mut AppState, id: &str) {
    let message = match id {
        id if id.starts_with("lsp.") => {
            "no language server is running for this file (configure one under [lsp] in config.toml)"
                .to_string()
        }
        id if id.starts_with("extensions.") => {
            "use `termloom extension install <file.vsix>` to add a package".to_string()
        }
        other => format!("unknown command {other}"),
    };
    state.warn(message);
}

// ── file helpers ──────────────────────────────────────────────────────────

fn save_active(state: &mut AppState, services: &mut Services, force: bool) {
    let Some(id) = state.active_tab else {
        return;
    };
    let config = state.config.editor.clone();
    let Some(document) = state.document_mut(id) else {
        return;
    };
    if !force {
        document.check_external_change();
        if document.save_would_conflict() {
            state.confirm(
                "File changed on disk",
                "This file changed outside TermLoom. Overwrite it with your version?",
                ConfirmAction::OverwriteExternalChange(id),
            );
            return;
        }
    }
    let result = document.save(&config).map(|path| {
        let language = document.language.clone();
        let text = document.buffer.to_text();
        (path, language, text)
    });
    match result {
        Ok((path, language, text)) => {
            services.lsp.did_save(&path, &language, &text);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            state.info(format!("saved {name}"));
        }
        Err(err) => state.error(format!("{err:#}")),
    }
}

fn save_all(state: &mut AppState, services: &mut Services) {
    let config = state.config.editor.clone();
    let mut saved = 0;
    let mut errors = Vec::new();
    let mut conflicts = Vec::new();
    let mut notifications = Vec::new();
    for document in state.documents.iter_mut().filter(|d| d.is_dirty()) {
        document.check_external_change();
        if document.save_would_conflict() {
            conflicts.push(
                document
                    .path
                    .as_deref()
                    .map(Path::display)
                    .map(|path| path.to_string())
                    .unwrap_or_else(|| document.display_name()),
            );
            continue;
        }
        match document.save(&config) {
            Ok(path) => {
                notifications.push((path, document.language.clone(), document.buffer.to_text()));
                saved += 1;
            }
            Err(err) => errors.push(format!("{err:#}")),
        }
    }
    for (path, language, text) in notifications {
        services.lsp.did_save(&path, &language, &text);
    }
    for error in errors {
        state.error(error);
    }
    if !conflicts.is_empty() {
        state.warn(format!(
            "skipped {} externally changed file(s): {}; save them individually to review the overwrite",
            conflicts.len(),
            conflicts.join(", ")
        ));
    }
    if saved > 0 {
        state.info(format!("saved {saved} file(s)"));
    }
}

fn reload_active(state: &mut AppState) {
    let Some(id) = state.active_tab else { return };
    let dirty = state.document(id).is_some_and(|d| d.is_dirty());
    if dirty {
        state.confirm(
            "Reload file",
            "Discard your unsaved changes and reload from disk?",
            ConfirmAction::ReloadExternalChange(id),
        );
        return;
    }
    if let Some(document) = state.document_mut(id) {
        if let Err(err) = document.reload() {
            state.error(format!("{err:#}"));
        }
    }
}

fn close_active_tab(state: &mut AppState, services: &mut Services) {
    let Some(id) = state.active_tab else { return };
    if !close_tab(state, services, id, false) {
        state.confirm(
            "Unsaved changes",
            "This file has unsaved changes. Close it anyway?",
            ConfirmAction::CloseDirtyTab(id),
        );
    }
}

fn close_tab(
    state: &mut AppState,
    services: &mut Services,
    id: crate::domain::ids::EditorTabId,
    force: bool,
) -> bool {
    let lifecycle = state
        .document(id)
        .and_then(|document| Some((document.path.clone()?, document.language.clone())));
    if !state.close_tab(id, force) {
        return false;
    }
    if let Some((path, language)) = lifecycle {
        services.lsp.did_close(&path, &language);
    }
    true
}

fn find_step(state: &mut AppState, forward: bool) {
    let Some(document) = state.active_document_mut() else {
        return;
    };
    if !document.search.is_active() {
        return;
    }
    let cursor = document.buffer.cursor();
    let hit = if forward {
        document.search.next_from(cursor)
    } else {
        document.search.prev_from(cursor)
    };
    match hit {
        Some(range) => {
            document.buffer.move_to(range.start, false);
            document.buffer.move_to(range.end, true);
        }
        None => state.info("no matches"),
    }
}

fn copy_selection(state: &mut AppState, cut: bool) {
    let Some(document) = state.active_document_mut() else {
        return;
    };
    let text = match document.buffer.selected_text() {
        Some(text) => {
            if cut {
                document.buffer.delete_selection();
            }
            text
        }
        None => {
            // No selection: operate on the whole line, like most editors.
            document.buffer.select_line();
            let text = document.buffer.selected_text().unwrap_or_default();
            if cut {
                document.buffer.delete_selection();
            } else {
                document.buffer.clear_selection();
            }
            text
        }
    };
    set_clipboard(state, text);
}

fn set_clipboard(state: &mut AppState, text: String) {
    state.clipboard = text.clone();
    match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
        Ok(()) => {}
        Err(err) => {
            tracing::debug!(error = %err, "system clipboard unavailable, using internal buffer")
        }
    }
}

fn paste(state: &mut AppState) {
    let text = arboard::Clipboard::new()
        .and_then(|mut c| c.get_text())
        .unwrap_or_else(|_| state.clipboard.clone());
    if text.is_empty() {
        return;
    }
    if let Some(document) = state.active_document_mut() {
        document.buffer.insert(&text);
    }
}

fn toggle_comment(state: &mut AppState) {
    let Some(document) = state.active_document_mut() else {
        return;
    };
    let Some(token) = document.language.line_comment() else {
        state.warn("no line comment syntax for this language");
        return;
    };
    let cursor = document.buffer.cursor();
    let (first, last) = match document.buffer.selection() {
        Some(range) => (range.start.line, range.end.line),
        None => (cursor.line, cursor.line),
    };
    let all_commented =
        (first..=last).all(|line| document.buffer.line(line).trim_start().starts_with(token));

    for line in first..=last {
        let text = document.buffer.line(line).to_string();
        let indent = text.chars().take_while(|c| c.is_whitespace()).count();
        if all_commented {
            let rest = text.chars().skip(indent).collect::<String>();
            let stripped = rest
                .strip_prefix(token)
                .map(|s| s.strip_prefix(' ').unwrap_or(s))
                .unwrap_or(&rest);
            let removed = rest.chars().count() - stripped.chars().count();
            if removed > 0 {
                document.buffer.replace_range(
                    crate::domain::diagnostics::Range::single_line(line, indent, indent + removed),
                    "",
                );
            }
        } else if !text.trim().is_empty() {
            document.buffer.replace_range(
                crate::domain::diagnostics::Range::single_line(line, indent, indent),
                &format!("{token} "),
            );
        }
    }
    document.buffer.move_to(cursor, false);
}

// ── terminals ─────────────────────────────────────────────────────────────

fn with_focused_terminal(
    state: &mut AppState,
    f: impl FnOnce(&mut crate::services::terminal::TerminalSession),
) {
    let Some(id) = state.focused_terminal else {
        return;
    };
    if let Ok(mut terminals) = state.terminals.lock() {
        if let Some(session) = terminals.get_mut(id) {
            f(session);
        }
    }
}

fn new_terminal(state: &mut AppState) {
    let (shell, args) = state.config.shell_command();
    let cwd = state.workspace.root.clone();
    let (rows, cols) = state.terminal_pane_size;
    let scrollback = state.config.terminal.scrollback;

    let result = {
        let Ok(mut terminals) = state.terminals.lock() else {
            state.error("terminal manager unavailable");
            return;
        };
        let label = terminals.next_shell_label();
        let mut spec = SpawnSpec::shell(label, &shell, &args, &cwd);
        spec.rows = rows;
        spec.cols = cols;
        spec.scrollback = scrollback;
        terminals.spawn(spec)
    };

    match result {
        Ok(id) => {
            state.layout.terminals = true;
            state.focus_terminal(id);
        }
        Err(err) => state.error(format!("could not start a terminal: {err:#}")),
    }
}

fn restart_terminal(state: &mut AppState) {
    let Some(id) = state.focused_terminal else {
        return;
    };
    let result = state
        .terminals
        .lock()
        .map_err(|_| anyhow::anyhow!("terminal manager unavailable"))
        .and_then(|mut terminals| terminals.restart(id));
    match result {
        Ok(new_id) => state.focus_terminal(new_id),
        Err(err) => state.error(format!("restart failed: {err:#}")),
    }
}

// ── agents ────────────────────────────────────────────────────────────────

/// Refresh the agent list from every backend.
pub fn refresh_agents(state: &mut AppState, services: &mut Services) {
    let registry = &services.agents;
    let mut agents = services.runtime.block_on(async {
        registry.refresh_all().await;
        registry.list_all().await
    });
    for agent in &mut agents {
        if let Some(tasks) = state.agent_tasks.get(&agent.id) {
            agent.tasks.clone_from(tasks);
        }
    }
    state.agents = agents;
    state.agent_selection.clamp(state.agents.len());
}

fn spawn_preset(state: &mut AppState, services: &mut Services, name: &str) {
    let Some(preset) = state.config.agents.get(name).cloned() else {
        state.error(format!("no `[agents.{name}]` preset is configured"));
        return;
    };
    if which(&preset.command).is_none() {
        state.error(format!(
            "`{}` was not found on PATH — install it or set [agents.{name}] command",
            preset.command
        ));
        return;
    }
    let mut request = request_from_preset(name, &preset, state.workspace.root.clone());
    let (rows, cols) = state.terminal_pane_size;
    request.rows = rows;
    request.cols = cols;
    spawn_agent(state, services, request);
}

fn spawn_shell_agent(state: &mut AppState, services: &mut Services) {
    let (shell, args) = state.config.shell_command();
    let (rows, cols) = state.terminal_pane_size;
    let request = SpawnAgentRequest {
        kind: AgentKind::Shell,
        label: "Shell".into(),
        program: shell,
        args,
        cwd: state.workspace.root.clone(),
        env: Vec::new(),
        rows,
        cols,
    };
    spawn_agent(state, services, request);
}

/// Launch an agent through the primary backend.
pub fn spawn_agent(state: &mut AppState, services: &mut Services, request: SpawnAgentRequest) {
    let label = request.label.clone();
    let Some(backend) = services.agents.primary().cloned() else {
        state.error("no agent backend is available");
        return;
    };
    match services.runtime.block_on(backend.spawn_agent(request)) {
        Ok(session) => {
            state.layout.terminals = true;
            state.layout.agents = true;
            if let Some(terminal) = session.terminal_id {
                state.focus_terminal(terminal);
            }
            // Snapshot the current git changes so the Files tab can show what
            // appeared *after* the session started.
            let baseline = state
                .git
                .changes
                .iter()
                .map(|c| c.path.clone())
                .collect::<Vec<_>>();
            state.agent_files_baseline = Some((session.id, baseline));
            refresh_agents(state, services);
            if let Some(index) = state.agents.iter().position(|a| a.id == session.id) {
                state.agent_selection.selected = index;
            }
            state.info(format!("started {label}"));
        }
        Err(err) => state.error(format!("could not start {label}: {err:#}")),
    }
}

/// Find a program on `PATH` (also accepts an explicit path).
pub fn which(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.is_absolute() || program.contains('/') {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|dir| {
        let full = dir.join(program);
        full.is_file().then_some(full)
    })
}

// ── git ───────────────────────────────────────────────────────────────────

fn git_operation(state: &mut AppState, services: &mut Services, operation: &str) {
    let Some(root) = state.git_root() else {
        state.warn("not a git repository");
        return;
    };
    let path = state.selected_git_path();
    let result = match (operation, path.as_ref()) {
        ("stage", Some(path)) => git::stage(&root, path),
        ("unstage", Some(path)) => git::unstage(&root, path),
        ("stage_all", _) => git::stage_all(&root),
        ("unstage_all", _) => git::unstage_all(&root),
        _ => {
            state.warn("no changed file selected");
            return;
        }
    };
    match result {
        Ok(()) => services.refresh_git(Some(root)),
        Err(err) => state.error(format!("{err:#}")),
    }
}

fn open_diff(state: &mut AppState) {
    let (Some(root), Some(path)) = (state.git_root(), state.selected_git_path()) else {
        state.warn("no changed file selected");
        return;
    };
    let staged = state
        .git
        .changes
        .get(state.git_selection.selected)
        .is_some_and(|c| c.is_staged());
    match git::diff_file(&root, &path, staged) {
        Ok(diff) => {
            state.diff = Some(diff);
            state.diff_scroll = 0;
        }
        Err(err) => state.error(format!("{err:#}")),
    }
}

// ── workspace ─────────────────────────────────────────────────────────────

fn reload_workspace(state: &mut AppState, services: &mut Services) {
    let (mut config, mut diagnostics) = crate::config::Config::load(Some(&state.workspace.root));
    let vscode_settings = crate::services::vscode::read_settings(&state.workspace.root);
    crate::services::vscode::apply_settings(&mut config, &vscode_settings);
    if !vscode_settings.ignored.is_empty() {
        diagnostics.push(crate::config::ConfigDiagnostic {
            path: state.workspace.root.join(".vscode/settings.json"),
            message: format!(
                "ignored unsupported VS Code settings: {}",
                vscode_settings.ignored.join(", ")
            ),
        });
    }
    state.keymap = config.keymap();
    state.config = config;
    services.lsp.set_config(&state.config);
    reload_extensions(state);
    state.config_diagnostics = diagnostics;
    state.refresh_tree();
    services.refresh_git(state.git_root());
    services.reindex(
        &state.workspace.root,
        ScanOptions::from_config(&state.config.workspace),
    );
    state.indexing = true;
    for diagnostic in state.config_diagnostics.clone() {
        state.error(format!(
            "{}: {}",
            diagnostic.path.display(),
            diagnostic.message
        ));
    }
    state.info("workspace reloaded");
}

fn open_config(state: &mut AppState) {
    let project = state
        .workspace
        .root
        .join(crate::config::PROJECT_CONFIG_FILE);
    let path = if project.exists() {
        Some(project)
    } else {
        crate::config::global_config_path().filter(|p| p.exists())
    };
    match path {
        Some(path) => {
            if let Err(err) = state.open_file(&path) {
                state.error(format!("{err:#}"));
            }
        }
        None => state.info(
            "no configuration file yet — create .termloom.toml in the project or run `termloom config --write-default`",
        ),
    }
}

fn quit(state: &mut AppState) {
    if state.has_unsaved_changes() {
        state.confirm(
            "Unsaved changes",
            "Some files have unsaved changes. Quit anyway?",
            ConfirmAction::QuitWithUnsavedChanges,
        );
        return;
    }
    state.should_quit = true;
}

// ── modal results ─────────────────────────────────────────────────────────

/// Apply a confirmed destructive action.
pub fn apply_confirm(state: &mut AppState, services: &mut Services, action: ConfirmAction) {
    match action {
        ConfirmAction::DeletePath(path) => {
            match fs_ops::delete(&state.workspace.root, &path) {
                Ok(()) => {
                    // Close any tab showing the deleted file.
                    if let Some(id) = state.document_for_path(&path).map(|d| d.id) {
                        close_tab(state, services, id, true);
                    }
                    state.refresh_tree();
                    services.refresh_git(state.git_root());
                    state.info(format!("deleted {}", path.display()));
                }
                Err(err) => state.error(format!("{err:#}")),
            }
        }
        ConfirmAction::DiscardGitChange(path) => {
            let Some(root) = state.git_root() else { return };
            match git::discard(&root, &path) {
                Ok(()) => {
                    let absolute = root.join(&path);
                    if let Some(document) = state
                        .documents
                        .iter_mut()
                        .find(|d| d.path.as_deref() == Some(absolute.as_path()))
                    {
                        let _ = document.reload();
                    }
                    services.refresh_git(Some(root));
                    state.info(format!("discarded changes to {}", path.display()));
                }
                Err(err) => state.error(format!("{err:#}")),
            }
        }
        ConfirmAction::CloseDirtyTab(id) => {
            close_tab(state, services, id, true);
        }
        ConfirmAction::OverwriteExternalChange(id) => {
            state.active_tab = Some(id);
            save_active(state, services, true);
        }
        ConfirmAction::ReloadExternalChange(id) => {
            if let Some(document) = state.document_mut(id) {
                if let Err(err) = document.reload() {
                    state.error(format!("{err:#}"));
                }
            }
        }
        ConfirmAction::StopAgent(id) => {
            let registry = &services.agents;
            let result = services.runtime.block_on(async {
                match registry.owner(id).await {
                    Some(backend) => backend.stop(id).await,
                    None => Ok(()),
                }
            });
            if let Err(err) = result {
                state.error(format!("stop failed: {err:#}"));
            }
            refresh_agents(state, services);
        }
        ConfirmAction::RemoveExtension(id) => match crate::services::extensions::remove(&id) {
            Ok(true) => {
                reload_extensions(state);
                state.info(format!("removed {id}"));
            }
            Ok(false) => state.warn(format!("extension {id} is not installed")),
            Err(err) => state.error(format!("could not remove {id}: {err:#}")),
        },
        ConfirmAction::StartDebugSession(config) => {
            launch_debug_session(state, services, *config);
        }
        ConfirmAction::RunTask(task) => start_task(state, &task),
        ConfirmAction::QuitWithUnsavedChanges => state.should_quit = true,
    }
}

/// Apply a choice from a selection dialog.
pub fn apply_selection(
    state: &mut AppState,
    services: &mut Services,
    purpose: SelectPurpose,
    index: usize,
) {
    match purpose {
        SelectPurpose::DebugConfiguration(entries) => {
            let Some(entry) = entries.get(index).cloned() else {
                return;
            };
            confirm_debug_start(state, entry);
        }
        SelectPurpose::Task(tasks) => {
            let Some(task) = tasks.get(index).cloned() else {
                return;
            };
            // Repository-defined commands always need explicit approval.
            state.confirm(
                "Run task",
                format!("Run `{}` in a new terminal?", task.command_line()),
                ConfirmAction::RunTask(task),
            );
        }
    }
    let _ = services;
}

/// Apply a prompt result.
pub fn apply_prompt(
    state: &mut AppState,
    services: &mut Services,
    purpose: PromptPurpose,
    value: String,
) {
    let value = value.trim().to_string();
    match purpose {
        PromptPurpose::NewFile(dir) => {
            if value.is_empty() {
                return;
            }
            match fs_ops::create_file(&state.workspace.root, &dir.join(&value)) {
                Ok(path) => {
                    state.refresh_tree();
                    state.reveal_in_explorer(&path);
                    if let Err(err) = state.open_file(&path) {
                        state.error(format!("{err:#}"));
                    }
                }
                Err(err) => state.error(format!("{err:#}")),
            }
        }
        PromptPurpose::NewFolder(dir) => {
            if value.is_empty() {
                return;
            }
            match fs_ops::create_dir(&state.workspace.root, &dir.join(&value)) {
                Ok(path) => {
                    state.refresh_tree();
                    state.reveal_in_explorer(&path);
                }
                Err(err) => state.error(format!("{err:#}")),
            }
        }
        PromptPurpose::Rename(from) => {
            if value.is_empty() {
                return;
            }
            let to = from
                .parent()
                .map(|parent| parent.join(&value))
                .unwrap_or_else(|| PathBuf::from(&value));
            match fs_ops::rename(&state.workspace.root, &from, &to) {
                Ok(path) => {
                    if let Some(document) = state
                        .documents
                        .iter_mut()
                        .find(|d| d.path.as_deref() == Some(from.as_path()))
                    {
                        document.path = Some(path.clone());
                    }
                    state.refresh_tree();
                    state.reveal_in_explorer(&path);
                    services.refresh_git(state.git_root());
                }
                Err(err) => state.error(format!("{err:#}")),
            }
        }
        PromptPurpose::GotoLine => {
            let Ok(line) = value.parse::<usize>() else {
                state.warn("enter a line number");
                return;
            };
            if let Some(document) = state.active_document_mut() {
                document.goto(Position::new(line.saturating_sub(1), 0));
            }
        }
        PromptPurpose::FindInFile => {
            let Some(document) = state.active_document_mut() else {
                return;
            };
            document.search.query = value;
            document.search.refresh(&document.buffer);
            let cursor = document.buffer.cursor();
            if let Some(range) = document.search.next_from(cursor) {
                document.buffer.move_to(range.start, false);
                document.buffer.move_to(range.end, true);
            } else if document.search.is_active() {
                state.info("no matches");
            }
        }
        PromptPurpose::AgentRename(id) => {
            if value.is_empty() {
                return;
            }
            let registry = &services.agents;
            let result = services.runtime.block_on(async {
                match registry.owner(id).await {
                    Some(backend) => backend.rename(id, &value).await,
                    None => Ok(()),
                }
            });
            if let Err(err) = result {
                state.error(format!("rename failed: {err:#}"));
            }
            refresh_agents(state, services);
        }
        PromptPurpose::AgentInput(id) => {
            let registry = &services.agents;
            let bytes = crate::services::terminal::input::encode_text(&format!("{value}\n"));
            let result = services.runtime.block_on(async {
                match registry.owner(id).await {
                    Some(backend) => backend.send_input(id, &bytes).await,
                    None => Err(anyhow::anyhow!("no backend owns this agent")),
                }
            });
            match result {
                Ok(()) => state.info("sent"),
                Err(err) => state.error(format!("send failed: {err:#}")),
            }
        }
        PromptPurpose::AgentTask(id) => {
            if value.is_empty() {
                return;
            }
            state.agent_task_selection = state.agent_tasks.get(&id).map(Vec::len).unwrap_or(0);
            let tasks = state.agent_tasks.entry(id).or_default();
            tasks.push(crate::domain::agent::AgentTask {
                text: value,
                done: false,
            });
            if let Some(agent) = state.agents.iter_mut().find(|a| a.id == id) {
                agent.tasks.clone_from(tasks);
                state.agent_tab = AgentDetailTab::Tasks;
            }
        }
        PromptPurpose::CustomAgentCommand => {
            if value.is_empty() {
                return;
            }
            let parts = match shell_words::split(&value) {
                Ok(parts) if !parts.is_empty() => parts,
                _ => {
                    state.warn("could not parse that command");
                    return;
                }
            };
            let (program, args) = parts.split_first().unwrap();
            if which(program).is_none() {
                state.error(format!("`{program}` was not found on PATH"));
                return;
            }
            let (rows, cols) = state.terminal_pane_size;
            let request = SpawnAgentRequest {
                kind: AgentKind::from_command(program),
                label: program.clone(),
                program: program.clone(),
                args: args.to_vec(),
                cwd: state.workspace.root.clone(),
                env: Vec::new(),
                rows,
                cols,
            };
            spawn_agent(state, services, request);
        }
        PromptPurpose::InstallVsix => {
            if value.is_empty() {
                return;
            }
            let path = expand_user_path(&value);
            match crate::services::extensions::install(&path) {
                Ok(report) => {
                    let id = report.id.clone();
                    reload_extensions(state);
                    if let Some(index) = state
                        .extensions
                        .installed
                        .iter()
                        .position(|installed| installed.id == id)
                    {
                        state.extensions.selected = index;
                    }
                    state.layout.extensions = true;
                    state.info(format!(
                        "installed {} ({}) without executing extension code",
                        report.id,
                        report.class.label()
                    ));
                }
                Err(err) => state.error(format!("could not install VSIX: {err:#}")),
            }
        }
        PromptPurpose::InspectVsix => {
            if value.is_empty() {
                return;
            }
            let path = expand_user_path(&value);
            match crate::services::extensions::inspect(&path) {
                Ok(report) => {
                    let mut lines = vec![format!(
                        "{} {} — {}",
                        report.display_name,
                        report.version,
                        report.class.label()
                    )];
                    lines.extend(report.capabilities.into_iter().map(|capability| {
                        format!(
                            "{} {} — {}",
                            capability.support.glyph(),
                            capability.name,
                            capability.detail
                        )
                    }));
                    lines.push("Extension code was not executed.".into());
                    state.show_message("VSIX inspection", lines);
                }
                Err(err) => state.error(format!("could not inspect VSIX: {err:#}")),
            }
        }
        PromptPurpose::DebugEvaluate => {
            if value.is_empty() {
                return;
            }
            debug_command(state, services, DebugCommand::Evaluate(value));
        }
        PromptPurpose::RenameSymbol(position) => {
            if value.is_empty() {
                return;
            }
            let Some(document) = state.active_document() else {
                return;
            };
            let (Some(path), language) = (document.path.clone(), document.language.clone()) else {
                return;
            };
            if let Err(err) = services.lsp.request(
                &language,
                RequestKind::Rename,
                &path,
                position,
                RequestExtra::NewName(value),
            ) {
                state.warn(format!("{err:#}"));
            }
        }
    }
}

/// Reload installed extension reports and register portable language mappings.
pub fn reload_extensions(state: &mut AppState) {
    match crate::services::extensions::list() {
        Ok(mut reports) => {
            reports.retain(|report| {
                !state
                    .config
                    .extensions
                    .disabled
                    .iter()
                    .any(|id| id.eq_ignore_ascii_case(&report.id))
            });
            state.languages = crate::services::syntax::LanguageRegistry::new();
            for report in &reports {
                for asset in &report.assets {
                    if let crate::domain::extensions::ExtensionAsset::Language {
                        id,
                        extensions,
                        filenames,
                        ..
                    } = asset
                    {
                        state.languages.register(id, extensions, filenames);
                    }
                }
            }
            state.extensions.installed = reports;
            state.snippets =
                crate::services::extensions::load_snippets(&state.extensions.installed);
            state.extensions.selected = state
                .extensions
                .selected
                .min(state.extensions.installed.len().saturating_sub(1));
        }
        Err(err) => state.warn(format!("could not load installed extensions: {err:#}")),
    }
}

fn expand_user_path(value: &str) -> PathBuf {
    if value == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(value)
}

/// Refresh the agent Files tab against the baseline captured at launch.
pub fn refresh_agent_files(state: &mut AppState) {
    let Some(agent) = state.selected_agent() else {
        state.agent_files.clear();
        return;
    };
    let baseline = match &state.agent_files_baseline {
        Some((id, paths)) if *id == agent.id => paths.clone(),
        _ => Vec::new(),
    };
    state.agent_files = state
        .git
        .changes
        .iter()
        .map(|c| c.path.clone())
        .filter(|path| !baseline.contains(path))
        .collect();
}

/// Keep the dashboard honest when a terminal exits.
pub fn handle_terminal_exit(
    state: &mut AppState,
    services: &mut Services,
    id: crate::domain::ids::TerminalId,
    code: Option<i32>,
) {
    if let Ok(mut terminals) = state.terminals.lock() {
        terminals.mark_exited(id, code);
    }
    refresh_agents(state, services);
    if let Some(agent) = state
        .agents
        .iter()
        .find(|a| a.terminal_id == Some(id) && a.state == AgentState::Failed)
    {
        let label = agent.label.clone();
        state.notify(Notice::warning(format!("{label} exited with an error")));
    }
}
