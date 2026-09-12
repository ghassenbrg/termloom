//! Terminal lifecycle and the main event loop.

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::config::Config;
use crate::domain::terminal::TerminalKind;
use crate::services::agents::LocalAgentBackend;
use crate::services::persistence::{self, LayoutRecord, TerminalRecord, WorkspaceState};
use crate::services::terminal::{EventSink, SpawnSpec, TerminalEvent, TerminalManager};
use crate::services::workspace::{ScanOptions, Workspace};
use crate::ui::{self, Theme, WorkbenchLayout};

use super::actions;
use super::events::{AppEvent, EventBus};
use super::input;
use super::state::AppState;

/// How often the tick fires (agent state, durations, external file checks).
const TICK: Duration = Duration::from_millis(400);
/// How long toasts stay on screen.
const TOAST_LIFETIME: Duration = Duration::from_secs(5);
/// Minimum gap between automatic git refreshes.
const GIT_REFRESH_INTERVAL: Duration = Duration::from_secs(3);

type Backend = CrosstermBackend<Stdout>;

/// The running application.
pub struct App {
    state: AppState,
    services: super::services::Services,
    bus: EventBus,
    theme: Theme,
    layout: WorkbenchLayout,
    last_git_refresh: Instant,
    mouse_enabled: bool,
}

impl App {
    /// Build the workbench for a workspace.
    pub fn new(workspace: Workspace, initial_file: Option<PathBuf>) -> Result<App> {
        let (config, config_diagnostics) = Config::load(Some(&workspace.root));
        let bus = EventBus::new();

        // Terminal sessions publish into the application channel.
        let sender = bus.sender.clone();
        let sink: EventSink = Arc::new(move |event| {
            let app_event = match event {
                TerminalEvent::Output(id) => AppEvent::TerminalOutput(id),
                TerminalEvent::Exited(id, code) => AppEvent::TerminalExited(id, code),
            };
            let _ = sender.send(app_event);
        });
        let mut manager = TerminalManager::new(sink);
        manager.default_scrollback = config.terminal.scrollback;
        let terminals = Arc::new(Mutex::new(manager));

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .context("starting the async runtime")?;

        let mut services = super::services::Services::new(bus.sender.clone(), runtime);
        services.agents.push(Arc::new(LocalAgentBackend::new(
            Arc::clone(&terminals),
            Duration::from_secs(config.terminal.idle_after_secs),
        )));

        let theme = Theme::by_name(&config.ui.theme);
        let mut state = AppState::new(workspace, config, config_diagnostics, terminals);

        for diagnostic in state.config_diagnostics.clone() {
            state.error(format!(
                "{}: {}",
                diagnostic.path.display(),
                diagnostic.message
            ));
        }

        let mut app = App {
            state,
            services,
            bus,
            theme,
            layout: WorkbenchLayout::default(),
            last_git_refresh: Instant::now(),
            mouse_enabled: false,
        };

        app.restore_session();
        if let Some(path) = initial_file {
            if let Err(err) = app.state.open_file(&path) {
                app.state.error(format!("{err:#}"));
            }
        }
        app.services.refresh_git(app.state.git_root());
        let options = ScanOptions::from_config(&app.state.config.workspace);
        app.services.reindex(&app.state.workspace.root, options);
        let _ = persistence::RecentWorkspaces::record(&app.state.workspace.root);

        Ok(app)
    }

    /// Enter the alternate screen, run the loop, and always restore the
    /// terminal afterwards.
    pub fn run(&mut self) -> Result<()> {
        let mut terminal = setup_terminal(self.state.config.ui.mouse)?;
        self.mouse_enabled = self.state.config.ui.mouse;
        super::events::spawn_input_reader(self.bus.sender.clone());
        super::events::spawn_ticker(self.bus.sender.clone(), TICK);

        let result = self.event_loop(&mut terminal);
        restore_terminal(&mut terminal, self.mouse_enabled).ok();
        self.shutdown();
        result
    }

    fn event_loop(&mut self, terminal: &mut Terminal<Backend>) -> Result<()> {
        self.draw(terminal)?;
        loop {
            let Some(event) = self.bus.next(Duration::from_millis(250)) else {
                continue;
            };
            let mut events = vec![event];
            events.extend(self.bus.drain());

            let mut redraw = false;
            for event in events {
                redraw |= self.handle_event(event);
                if self.state.should_quit {
                    return Ok(());
                }
            }
            if redraw {
                self.draw(terminal)?;
            }
        }
    }

    /// Apply one event. Returns whether the screen needs redrawing.
    fn handle_event(&mut self, event: AppEvent) -> bool {
        match event {
            AppEvent::Key(key) => {
                input::handle_key(&mut self.state, &mut self.services, key);
                true
            }
            AppEvent::Mouse(mouse) => {
                if self.mouse_enabled {
                    input::handle_mouse(&mut self.state, &mut self.services, mouse, &self.layout);
                }
                true
            }
            AppEvent::Resize(cols, rows) => {
                self.state.terminal_size = (cols, rows);
                true
            }
            AppEvent::Tick => self.on_tick(),
            AppEvent::TerminalOutput(_) => true,
            AppEvent::TerminalExited(id, code) => {
                actions::handle_terminal_exit(&mut self.state, &mut self.services, id, code);
                true
            }
            AppEvent::GitUpdated(snapshot) => {
                self.state.git = *snapshot;
                self.state.git_selection.clamp(self.state.git.changes.len());
                actions::refresh_agent_files(&mut self.state);
                true
            }
            AppEvent::AgentsUpdated(agents) => {
                self.state.agents = agents;
                self.state.agent_selection.clamp(self.state.agents.len());
                true
            }
            AppEvent::FileIndexed(index) => {
                if index.truncated {
                    self.state.warn(format!(
                        "quick open indexed the first {} files only",
                        index.files.len()
                    ));
                }
                self.state.file_index = index;
                self.state.indexing = false;
                true
            }
            AppEvent::FileSystem(paths) => {
                self.on_filesystem_change(&paths);
                true
            }
            AppEvent::Diagnostics {
                path, diagnostics, ..
            } => {
                self.state.set_diagnostics(&path, diagnostics);
                true
            }
            AppEvent::LanguageServer(_) | AppEvent::Debug(_) => true,
            AppEvent::Notice(notice) => {
                self.state.notify(notice);
                true
            }
            AppEvent::Quit => {
                self.state.should_quit = true;
                false
            }
        }
    }

    /// Periodic housekeeping.
    fn on_tick(&mut self) -> bool {
        actions::refresh_agents(&mut self.state, &mut self.services);
        self.state.expire_toasts(TOAST_LIFETIME);
        self.state.reconcile_terminal_focus();
        self.state.indexing = self.services.is_indexing();

        // Notice files edited underneath us (by an agent, for example).
        for document in &mut self.state.documents {
            document.check_external_change();
        }

        if self.last_git_refresh.elapsed() >= GIT_REFRESH_INTERVAL {
            self.services.refresh_git(self.state.git_root());
            self.last_git_refresh = Instant::now();
        }
        true
    }

    fn on_filesystem_change(&mut self, paths: &[PathBuf]) {
        self.state.refresh_tree();
        for document in &mut self.state.documents {
            if document
                .path
                .as_ref()
                .is_some_and(|p| paths.iter().any(|changed| changed == p))
            {
                document.check_external_change();
            }
        }
        self.services.refresh_git(self.state.git_root());
    }

    fn draw(&mut self, terminal: &mut Terminal<Backend>) -> Result<()> {
        // Refresh derived data the renderer needs.
        self.state.refresh_syntax();
        self.refresh_agent_output();

        let state = &self.state;
        let theme = &self.theme;
        let mut layout = WorkbenchLayout::default();
        terminal.draw(|frame| {
            layout = ui::draw(frame, state, theme);
        })?;

        self.sync_pane_sizes(&layout);
        self.sync_scroll(&layout);
        self.layout = layout;
        Ok(())
    }

    /// Keep every PTY the same size as the pane that renders it.
    fn sync_pane_sizes(&mut self, layout: &WorkbenchLayout) {
        if layout.terminals.is_empty() {
            return;
        }
        if let Some((_, rect)) = layout.terminals.first() {
            self.state.terminal_pane_size = ui::terminals::pty_size(*rect);
        }
        if let Ok(mut terminals) = self.state.terminals.lock() {
            for (id, rect) in &layout.terminals {
                let (rows, cols) = ui::terminals::pty_size(*rect);
                if let Some(session) = terminals.get_mut(*id) {
                    let _ = session.resize(rows, cols);
                }
            }
        }
    }

    /// Update stored scroll offsets from the drawn geometry.
    fn sync_scroll(&mut self, layout: &WorkbenchLayout) {
        if let Some(rect) = layout.explorer {
            let height = rect.height.saturating_sub(2) as usize;
            self.state.explorer.scroll_into_view(height);
        }
        if let Some(rect) = layout.git {
            let height = rect.height.saturating_sub(3) as usize;
            self.state.git_selection.scroll_into_view(height);
        }
        if let Some(rect) = layout.agents {
            let height = (rect.height.saturating_sub(2) / 2) as usize;
            self.state.agent_selection.scroll_into_view(height);
        }
        if let Some(rect) = layout.editor {
            let gutter = self
                .state
                .active_document()
                .map(|d| d.buffer.line_count().to_string().len() + 2)
                .unwrap_or(0);
            let width = (rect.width as usize).saturating_sub(gutter);
            let height = rect.height as usize;
            if let Some(document) = self.state.active_document_mut() {
                document.ensure_cursor_visible(height, width);
            }
        }
    }

    /// Cache the selected agent's recent output for the detail panel.
    fn refresh_agent_output(&mut self) {
        let Some(agent) = self.state.selected_agent() else {
            self.state.agent_output.clear();
            return;
        };
        let Some(terminal_id) = agent.terminal_id else {
            self.state.agent_output.clear();
            return;
        };
        let lines = self
            .state
            .terminals
            .lock()
            .ok()
            .and_then(|terminals| terminals.get(terminal_id).map(|s| s.snapshot().tail(200)))
            .unwrap_or_default();
        self.state.agent_output = lines;
    }

    // ── session persistence ───────────────────────────────────────────────

    fn restore_session(&mut self) {
        if !self.state.config.workspace.restore_session {
            return;
        }
        let Some(saved) = persistence::load(&self.state.workspace.root) else {
            return;
        };

        self.state.layout.explorer = saved.layout.explorer;
        self.state.layout.outline = saved.layout.outline;
        self.state.layout.git = saved.layout.git;
        self.state.layout.agents = saved.layout.agents;
        self.state.layout.terminals = saved.layout.terminals;
        self.state.layout.problems = saved.layout.problems;
        self.state.layout.debug = saved.layout.debug;
        self.state.layout.sidebar_width = saved.layout.sidebar_width;
        self.state.layout.agents_width = saved.layout.agents_width;
        self.state.layout.terminal_height_pct = saved.layout.terminal_height_pct.clamp(10, 70);

        for dir in &saved.expanded_dirs {
            self.state.tree.expand(dir);
        }

        let mut restored = 0;
        for path in &saved.open_files {
            if !path.exists() {
                continue;
            }
            match self.state.open_file(path) {
                Ok(id) => {
                    restored += 1;
                    let key = self.relative_key(path);
                    if let Some((line, character)) = saved.cursors.get(&key).copied() {
                        if let Some(document) = self.state.document_mut(id) {
                            document
                                .goto(crate::domain::diagnostics::Position::new(line, character));
                        }
                    }
                }
                Err(err) => {
                    tracing::debug!(error = %err, path = %path.display(), "could not restore file")
                }
            }
        }
        if let Some(active) = saved.active_file.as_ref() {
            if let Some(id) = self.state.document_for_path(active).map(|d| d.id) {
                self.state.activate_tab(id);
            }
        }

        // Recreate shells and agent sessions that were running.
        for record in &saved.terminals {
            if record.kind == "agent" {
                continue; // agents are relaunched explicitly by the user
            }
            let Some((program, args)) = record.command.split_first() else {
                continue;
            };
            let cwd = if record.cwd.is_dir() {
                record.cwd.clone()
            } else {
                self.state.workspace.root.clone()
            };
            let mut spec = SpawnSpec::shell(record.label.clone(), program, args, &cwd);
            spec.scrollback = self.state.config.terminal.scrollback;
            if let Ok(mut terminals) = self.state.terminals.lock() {
                let _ = terminals.spawn(spec);
            }
        }
        self.state.reconcile_terminal_focus();

        if restored > 0 {
            self.state
                .info(format!("restored {restored} file(s) from the last session"));
        }
    }

    fn relative_key(&self, path: &Path) -> String {
        self.state
            .workspace
            .relative(path)
            .to_string_lossy()
            .to_string()
    }

    fn save_session(&self) {
        let layout = LayoutRecord {
            explorer: self.state.layout.explorer,
            outline: self.state.layout.outline,
            git: self.state.layout.git,
            agents: self.state.layout.agents,
            terminals: self.state.layout.terminals,
            problems: self.state.layout.problems,
            debug: self.state.layout.debug,
            sidebar_width: self.state.layout.sidebar_width,
            agents_width: self.state.layout.agents_width,
            terminal_height_pct: self.state.layout.terminal_height_pct,
        };

        let terminals = self
            .state
            .terminals
            .lock()
            .map(|manager| {
                manager
                    .iter()
                    .map(|session| TerminalRecord {
                        label: session.label().to_string(),
                        command: session.command().to_vec(),
                        cwd: session.cwd().to_path_buf(),
                        kind: session.kind().label().to_string(),
                        agent_kind: (session.kind() == TerminalKind::Agent).then(|| {
                            crate::domain::agent::AgentKind::from_command(
                                session.command().first().map(String::as_str).unwrap_or(""),
                            )
                            .label()
                            .to_string()
                        }),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let state = WorkspaceState {
            version: persistence::STATE_VERSION,
            root: self.state.workspace.root.clone(),
            open_files: self
                .state
                .documents
                .iter()
                .filter_map(|d| d.path.clone())
                .collect(),
            active_file: self.state.active_document().and_then(|d| d.path.clone()),
            cursors: self
                .state
                .documents
                .iter()
                .filter_map(|document| {
                    let path = document.path.as_ref()?;
                    let cursor = document.buffer.cursor();
                    Some((self.relative_key(path), (cursor.line, cursor.character)))
                })
                .collect(),
            expanded_dirs: self
                .state
                .tree
                .rows()
                .into_iter()
                .filter(|row| row.is_dir && row.expanded)
                .map(|row| row.path)
                .collect(),
            layout,
            terminals,
            selected_agent: self.state.agent_selection.selected,
            theme: Some(self.state.config.ui.theme.clone()),
        };

        if let Err(err) = persistence::save(&state) {
            tracing::warn!(error = %err, "could not save workspace state");
        }
    }

    fn shutdown(&mut self) {
        self.save_session();
        if let Ok(mut terminals) = self.state.terminals.lock() {
            terminals.shutdown();
        }
    }
}

/// Put the terminal into raw alternate-screen mode.
pub fn setup_terminal(mouse: bool) -> Result<Terminal<Backend>> {
    enable_raw_mode().context("enabling raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("entering the alternate screen")?;
    if mouse {
        execute!(stdout, EnableMouseCapture).ok();
    }
    install_panic_hook(mouse);
    let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    Ok(terminal)
}

/// Undo [`setup_terminal`].
pub fn restore_terminal(terminal: &mut Terminal<Backend>, mouse: bool) -> Result<()> {
    disable_raw_mode().ok();
    if mouse {
        execute!(terminal.backend_mut(), DisableMouseCapture).ok();
    }
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    Ok(())
}

/// Make sure a panic never leaves the user with a broken terminal.
fn install_panic_hook(mouse: bool) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut stdout = io::stdout();
        disable_raw_mode().ok();
        if mouse {
            execute!(stdout, DisableMouseCapture).ok();
        }
        execute!(stdout, LeaveAlternateScreen).ok();
        crossterm::execute!(stdout, crossterm::cursor::Show).ok();
        previous(info);
    }));
}
