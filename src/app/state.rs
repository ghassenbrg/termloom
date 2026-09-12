//! Workbench state.
//!
//! One value holds everything the UI draws and the commands mutate. It owns no
//! background threads and performs no rendering: services push events into the
//! loop, the loop mutates this state, the UI reads it.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};

use crate::config::{Config, ConfigDiagnostic, Keymap};
use crate::domain::agent::{AgentSession, AgentState};
use crate::domain::debug::{DebugCapabilities, DebugStatus, DebugThread, StackFrame, Variable};
use crate::domain::diagnostics::{Diagnostic, Position, Severity};
use crate::domain::extensions::CompatibilityReport;
use crate::domain::git::{FileDiff, GitSnapshot};
use crate::domain::ids::{AgentId, EditorTabId, TerminalId};
use crate::editor::Document;
use crate::domain::diagnostics::Location;
use crate::services::lsp::{CompletionItem, ServerStatus};
use crate::services::syntax::{Highlighter, LanguageRegistry, OutlineExtractor};
use crate::services::terminal::TerminalManager;
use crate::services::workspace::{FileIndex, FileTree, ScanOptions, Workspace};

use super::commands::{Command, CommandRegistry, Requirement};
use super::events::{Notice, NoticeLevel};
use super::focus::{Direction, FocusTarget, Region};

/// Which panels are visible and how much room they get.
#[derive(Debug, Clone)]
pub struct LayoutState {
    pub explorer: bool,
    pub outline: bool,
    pub git: bool,
    pub agents: bool,
    pub terminals: bool,
    pub problems: bool,
    pub debug: bool,
    pub extensions: bool,
    pub sidebar_width: u16,
    pub agents_width: u16,
    pub terminal_height_pct: u16,
}

impl LayoutState {
    pub fn from_config(config: &Config) -> LayoutState {
        LayoutState {
            explorer: config.ui.show_explorer,
            outline: config.ui.show_outline,
            git: config.ui.show_git,
            agents: config.ui.show_agents,
            terminals: config.ui.show_terminals,
            problems: false,
            debug: false,
            extensions: false,
            sidebar_width: config.ui.sidebar_width,
            agents_width: config.ui.agents_width,
            terminal_height_pct: config.ui.terminal_height_pct.clamp(10, 70),
        }
    }

    /// Regions currently on screen, for directional focus movement.
    pub fn visible_regions(&self) -> Vec<Region> {
        let mut out = vec![Region::Center];
        if self.explorer || self.outline || self.git {
            out.insert(0, Region::Sidebar);
        }
        if self.agents {
            out.push(Region::Agents);
        }
        if self.terminals {
            out.push(Region::Terminals);
        }
        out
    }
}

/// Which tab of the agent detail panel is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentDetailTab {
    Status,
    Files,
    Tasks,
    Output,
}

impl AgentDetailTab {
    pub const ALL: [AgentDetailTab; 4] = [
        AgentDetailTab::Status,
        AgentDetailTab::Files,
        AgentDetailTab::Tasks,
        AgentDetailTab::Output,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AgentDetailTab::Status => "Status",
            AgentDetailTab::Files => "Files",
            AgentDetailTab::Tasks => "Tasks",
            AgentDetailTab::Output => "Output",
        }
    }

    pub fn next(self) -> AgentDetailTab {
        let index = AgentDetailTab::ALL
            .iter()
            .position(|t| *t == self)
            .unwrap_or(0);
        AgentDetailTab::ALL[(index + 1) % AgentDetailTab::ALL.len()]
    }
}

/// Palette mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteMode {
    Commands,
    Files,
}

/// One palette row.
#[derive(Debug, Clone)]
pub enum PaletteItem {
    Command(Command),
    File(PathBuf),
}

/// Quick open / command palette.
#[derive(Debug, Clone)]
pub struct PaletteState {
    pub mode: PaletteMode,
    pub query: String,
    pub selected: usize,
    pub items: Vec<PaletteItem>,
}

impl PaletteState {
    pub fn new(mode: PaletteMode) -> PaletteState {
        PaletteState {
            mode,
            query: String::new(),
            selected: 0,
            items: Vec::new(),
        }
    }
}

/// Actions a confirmation dialog can approve. Destructive operations always go
/// through one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    DeletePath(PathBuf),
    DiscardGitChange(PathBuf),
    CloseDirtyTab(EditorTabId),
    OverwriteExternalChange(EditorTabId),
    ReloadExternalChange(EditorTabId),
    RemoveExtension(String),
    StopAgent(AgentId),
    QuitWithUnsavedChanges,
}

/// What a text prompt is collecting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptPurpose {
    NewFile(PathBuf),
    NewFolder(PathBuf),
    Rename(PathBuf),
    GotoLine,
    FindInFile,
    AgentRename(AgentId),
    AgentInput(AgentId),
    AgentTask(AgentId),
    CustomAgentCommand,
    InstallVsix,
    InspectVsix,
    RenameSymbol(Position),
}

/// A blocking dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    Confirm {
        title: String,
        message: String,
        action: ConfirmAction,
    },
    Prompt {
        title: String,
        value: String,
        purpose: PromptPurpose,
    },
    Message {
        title: String,
        body: Vec<String>,
    },
}

/// A transient status message.
#[derive(Debug, Clone)]
pub struct Toast {
    pub level: NoticeLevel,
    pub text: String,
    pub created_at: Instant,
}

impl Toast {
    pub fn is_expired(&self, lifetime: Duration) -> bool {
        self.created_at.elapsed() > lifetime
    }
}

/// Selection state shared by simple list panels.
#[derive(Debug, Clone, Default)]
pub struct ListSelection {
    pub selected: usize,
    pub offset: usize,
}

impl ListSelection {
    /// Move the selection, clamped to `len`.
    pub fn move_by(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.selected = 0;
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, len as isize - 1) as usize;
    }

    /// Keep the selection inside a viewport of `height` rows.
    pub fn scroll_into_view(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + height {
            self.offset = self.selected + 1 - height;
        }
    }

    pub fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
            self.offset = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }
}

/// Hover documentation shown next to the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverPopup {
    pub lines: Vec<String>,
    /// Buffer position the hover was requested for.
    pub position: Position,
}

/// Completion popup anchored at the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionPopup {
    pub items: Vec<CompletionItem>,
    pub selected: usize,
    /// Position the request was made from.
    pub position: Position,
    /// Word already typed, replaced when an item is accepted.
    pub prefix: String,
}

impl CompletionPopup {
    /// Items matching the typed prefix.
    pub fn filtered(&self) -> Vec<&CompletionItem> {
        if self.prefix.is_empty() {
            return self.items.iter().collect();
        }
        let prefix = self.prefix.to_lowercase();
        self.items
            .iter()
            .filter(|item| item.label.to_lowercase().starts_with(&prefix))
            .collect()
    }

    pub fn selection(&self) -> Option<CompletionItem> {
        self.filtered().get(self.selected).map(|item| (*item).clone())
    }
}

/// Results of `lsp.references`, shown in the bottom panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencesView {
    pub query: String,
    pub locations: Vec<Location>,
    pub selected: usize,
}

/// Debug panel state, filled by the DAP client.
#[derive(Debug, Default)]
pub struct DebugUiState {
    pub status: DebugStatus,
    pub adapter: Option<String>,
    pub capabilities: DebugCapabilities,
    pub threads: Vec<DebugThread>,
    pub frames: Vec<StackFrame>,
    pub selected_frame: usize,
    pub variables: Vec<Variable>,
    pub output: Vec<String>,
    pub last_error: Option<String>,
    /// Source location of the current execution point.
    pub current_location: Option<(PathBuf, usize)>,
}

/// Extensions screen state.
#[derive(Debug, Default)]
pub struct ExtensionsUiState {
    pub installed: Vec<CompatibilityReport>,
    pub selected: usize,
}

/// Everything the workbench knows.
pub struct AppState {
    pub workspace: Workspace,
    pub config: Config,
    pub keymap: Keymap,
    pub config_diagnostics: Vec<ConfigDiagnostic>,

    // ── explorer ──────────────────────────────────────────────────────────
    pub tree: FileTree,
    pub explorer: ListSelection,
    pub file_index: FileIndex,
    pub indexing: bool,

    // ── editor ────────────────────────────────────────────────────────────
    pub documents: Vec<Document>,
    pub active_tab: Option<EditorTabId>,
    pub recently_closed: VecDeque<PathBuf>,
    /// Go-to-definition history for `lsp.back`.
    pub navigation_history: Vec<(PathBuf, Position)>,
    pub outline_selection: ListSelection,

    // ── terminals ─────────────────────────────────────────────────────────
    pub terminals: Arc<Mutex<TerminalManager>>,
    pub focused_terminal: Option<TerminalId>,
    /// Index of the leftmost visible terminal pane.
    pub terminal_view_offset: usize,

    // ── agents ────────────────────────────────────────────────────────────
    pub agents: Vec<AgentSession>,
    pub agent_selection: ListSelection,
    pub agent_tab: AgentDetailTab,
    /// Cached output lines for the selected agent's detail panel.
    pub agent_output: Vec<String>,
    /// Files changed since the selected agent started, with honest provenance.
    pub agent_files: Vec<PathBuf>,
    pub agent_files_baseline: Option<(AgentId, Vec<PathBuf>)>,

    // ── git ───────────────────────────────────────────────────────────────
    pub git: GitSnapshot,
    pub git_selection: ListSelection,
    pub diff: Option<FileDiff>,
    pub diff_scroll: usize,

    // ── problems ──────────────────────────────────────────────────────────
    pub problems: Vec<Diagnostic>,
    pub problems_selection: ListSelection,

    pub debug: DebugUiState,
    pub extensions: ExtensionsUiState,

    // ── language services ─────────────────────────────────────────────────
    pub hover: Option<HoverPopup>,
    pub completion: Option<CompletionPopup>,
    pub references: Option<ReferencesView>,
    /// Configured servers and their live state, for the sidebar.
    pub lsp_statuses: Vec<ServerStatus>,
    /// Last status message a server reported.
    pub lsp_message: Option<String>,

    // ── overlays ──────────────────────────────────────────────────────────
    pub palette: Option<PaletteState>,
    pub modal: Option<Modal>,
    pub toasts: Vec<Toast>,
    pub help_visible: bool,

    // ── focus and chrome ──────────────────────────────────────────────────
    pub focus: FocusTarget,
    /// True after the command prefix was pressed, waiting for the next key.
    pub prefix_active: bool,
    pub layout: LayoutState,
    pub should_quit: bool,
    pub terminal_size: (u16, u16),
    /// Grid size of a single terminal pane, updated after each draw so new
    /// sessions start at the right size.
    pub terminal_pane_size: (u16, u16),
    /// Fallback clipboard used when the OS clipboard is unavailable.
    pub clipboard: String,
    /// True once at least one language server finished initialising.
    pub lsp_ready: bool,

    // ── services used while rendering/editing ─────────────────────────────
    pub highlighter: Highlighter,
    pub outliner: OutlineExtractor,
    pub languages: LanguageRegistry,
    pub commands: CommandRegistry,
    pub matcher: nucleo_matcher::Matcher,
}

impl AppState {
    pub fn new(
        workspace: Workspace,
        config: Config,
        config_diagnostics: Vec<ConfigDiagnostic>,
        terminals: Arc<Mutex<TerminalManager>>,
    ) -> AppState {
        let scan = ScanOptions::from_config(&config.workspace);
        let tree = FileTree::new(workspace.root.clone(), scan);
        let keymap = config.keymap();
        let layout = LayoutState::from_config(&config);

        AppState {
            workspace,
            keymap,
            config_diagnostics,
            tree,
            explorer: ListSelection::default(),
            file_index: FileIndex::default(),
            indexing: true,
            documents: Vec::new(),
            active_tab: None,
            recently_closed: VecDeque::new(),
            navigation_history: Vec::new(),
            outline_selection: ListSelection::default(),
            terminals,
            focused_terminal: None,
            terminal_view_offset: 0,
            agents: Vec::new(),
            agent_selection: ListSelection::default(),
            agent_tab: AgentDetailTab::Status,
            agent_output: Vec::new(),
            agent_files: Vec::new(),
            agent_files_baseline: None,
            git: GitSnapshot::default(),
            git_selection: ListSelection::default(),
            diff: None,
            diff_scroll: 0,
            problems: Vec::new(),
            problems_selection: ListSelection::default(),
            debug: DebugUiState::default(),
            extensions: ExtensionsUiState::default(),
            hover: None,
            completion: None,
            references: None,
            lsp_statuses: Vec::new(),
            lsp_message: None,
            palette: None,
            modal: None,
            toasts: Vec::new(),
            help_visible: false,
            focus: FocusTarget::Explorer,
            prefix_active: false,
            layout,
            should_quit: false,
            terminal_size: (0, 0),
            terminal_pane_size: (24, 80),
            clipboard: String::new(),
            lsp_ready: false,
            highlighter: Highlighter::new(),
            outliner: OutlineExtractor::new(),
            languages: LanguageRegistry::new(),
            commands: CommandRegistry::new(),
            matcher: nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT),
            config,
        }
    }

    // ── notices ───────────────────────────────────────────────────────────

    pub fn notify(&mut self, notice: Notice) {
        tracing::debug!(level = ?notice.level, text = %notice.text, "notice");
        self.toasts.push(Toast {
            level: notice.level,
            text: notice.text,
            created_at: Instant::now(),
        });
        const MAX_TOASTS: usize = 4;
        while self.toasts.len() > MAX_TOASTS {
            self.toasts.remove(0);
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.notify(Notice::info(text));
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.notify(Notice::warning(text));
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.notify(Notice::error(text));
    }

    /// Drop expired toasts; called on every tick.
    pub fn expire_toasts(&mut self, lifetime: Duration) {
        self.toasts.retain(|t| !t.is_expired(lifetime));
    }

    // ── documents ─────────────────────────────────────────────────────────

    pub fn active_document(&self) -> Option<&Document> {
        let id = self.active_tab?;
        self.documents.iter().find(|d| d.id == id)
    }

    pub fn active_document_mut(&mut self) -> Option<&mut Document> {
        let id = self.active_tab?;
        self.documents.iter_mut().find(|d| d.id == id)
    }

    pub fn document(&self, id: EditorTabId) -> Option<&Document> {
        self.documents.iter().find(|d| d.id == id)
    }

    pub fn document_mut(&mut self, id: EditorTabId) -> Option<&mut Document> {
        self.documents.iter_mut().find(|d| d.id == id)
    }

    pub fn document_for_path(&self, path: &Path) -> Option<&Document> {
        self.documents
            .iter()
            .find(|d| d.path.as_deref() == Some(path))
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.documents.iter().any(Document::is_dirty)
    }

    /// Open a file, or focus the tab that already has it.
    ///
    /// Paths are canonicalised first: language servers and git report resolved
    /// paths, and without this the same file could end up in two tabs.
    pub fn open_file(&mut self, path: &Path) -> Result<EditorTabId> {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let path = canonical.as_path();
        if let Some(existing) = self.document_for_path(path).map(|d| d.id) {
            self.activate_tab(existing);
            return Ok(existing);
        }
        let language = self.languages.detect(path);
        let document = Document::open(path, &self.config.editor, language)?;
        let id = document.id;
        self.documents.push(document);
        self.activate_tab(id);
        Ok(id)
    }

    pub fn activate_tab(&mut self, id: EditorTabId) {
        if self.documents.iter().any(|d| d.id == id) {
            self.active_tab = Some(id);
            self.focus = FocusTarget::Editor(id);
            self.outline_selection = ListSelection::default();
        }
    }

    /// Close a tab. Returns false when the tab is dirty and `force` is unset,
    /// so the caller can ask for confirmation.
    pub fn close_tab(&mut self, id: EditorTabId, force: bool) -> bool {
        let Some(index) = self.documents.iter().position(|d| d.id == id) else {
            return true;
        };
        if self.documents[index].is_dirty() && !force {
            return false;
        }
        let document = self.documents.remove(index);
        if let Some(path) = document.path {
            self.recently_closed.push_back(path);
            while self.recently_closed.len() > 16 {
                self.recently_closed.pop_front();
            }
        }
        if self.active_tab == Some(id) {
            let next = self
                .documents
                .get(index)
                .or_else(|| self.documents.get(index.saturating_sub(1)))
                .map(|d| d.id);
            self.active_tab = next;
            self.focus = match next {
                Some(id) => FocusTarget::Editor(id),
                None => FocusTarget::Explorer,
            };
        }
        true
    }

    pub fn next_tab(&mut self, delta: isize) {
        if self.documents.is_empty() {
            return;
        }
        let current = self
            .active_tab
            .and_then(|id| self.documents.iter().position(|d| d.id == id))
            .unwrap_or(0) as isize;
        let len = self.documents.len() as isize;
        let index = ((current + delta) % len + len) % len;
        let id = self.documents[index as usize].id;
        self.activate_tab(id);
    }

    /// Refresh the cached highlight index and outline of the active document.
    pub fn refresh_syntax(&mut self) {
        let Some(id) = self.active_tab else { return };
        let Some(index) = self.documents.iter().position(|d| d.id == id) else {
            return;
        };
        let (version, language, text, needs_syntax, needs_symbols) = {
            let doc = &self.documents[index];
            (
                doc.buffer.version(),
                doc.language.clone(),
                doc.buffer.to_text(),
                doc.syntax_version != doc.buffer.version(),
                doc.symbols_version != doc.buffer.version(),
            )
        };

        if needs_syntax && self.config.editor.syntax_highlighting {
            let syntax = self.highlighter.highlight(&language, &text);
            let doc = &mut self.documents[index];
            doc.syntax = syntax;
            doc.syntax_version = version;
        }
        if needs_symbols {
            let symbols = self.outliner.extract(&language, &text).unwrap_or_default();
            let doc = &mut self.documents[index];
            doc.symbols = symbols;
            doc.symbols_version = version;
        }
    }

    // ── terminals ─────────────────────────────────────────────────────────

    /// Terminal ids in display order.
    pub fn terminal_ids(&self) -> Vec<TerminalId> {
        self.terminals.lock().map(|t| t.ids()).unwrap_or_default()
    }

    pub fn focus_terminal(&mut self, id: TerminalId) {
        self.focused_terminal = Some(id);
        self.focus = FocusTarget::Terminal(id);
        self.layout.terminals = true;
    }

    /// Move terminal focus by `delta` panes.
    pub fn cycle_terminal(&mut self, delta: isize) {
        let ids = self.terminal_ids();
        if ids.is_empty() {
            return;
        }
        let current = self
            .focused_terminal
            .and_then(|id| ids.iter().position(|i| *i == id))
            .unwrap_or(0) as isize;
        let len = ids.len() as isize;
        let index = ((current + delta) % len + len) % len;
        self.focus_terminal(ids[index as usize]);
    }

    /// Drop focus when the focused terminal disappears.
    pub fn reconcile_terminal_focus(&mut self) {
        let ids = self.terminal_ids();
        if let Some(id) = self.focused_terminal {
            if !ids.contains(&id) {
                self.focused_terminal = ids.first().copied();
                if let FocusTarget::Terminal(_) = self.focus {
                    self.focus = match self.focused_terminal {
                        Some(next) => FocusTarget::Terminal(next),
                        None => FocusTarget::Editor(self.active_tab.unwrap_or(EditorTabId(0))),
                    };
                    if self.active_tab.is_none() && self.focused_terminal.is_none() {
                        self.focus = FocusTarget::Explorer;
                    }
                }
            }
        } else {
            self.focused_terminal = ids.first().copied();
        }
        if self.terminal_view_offset >= ids.len() {
            self.terminal_view_offset = ids.len().saturating_sub(1);
        }
    }

    // ── agents ────────────────────────────────────────────────────────────

    pub fn selected_agent(&self) -> Option<&AgentSession> {
        self.agents.get(self.agent_selection.selected)
    }

    pub fn selected_agent_id(&self) -> Option<AgentId> {
        self.selected_agent().map(|a| a.id)
    }

    /// Agents that want the user's attention.
    pub fn agents_needing_attention(&self) -> usize {
        self.agents
            .iter()
            .filter(|a| a.state.needs_attention())
            .count()
    }

    pub fn agents_working(&self) -> usize {
        self.agents
            .iter()
            .filter(|a| a.state == AgentState::Working)
            .count()
    }

    // ── git ───────────────────────────────────────────────────────────────

    pub fn selected_git_path(&self) -> Option<PathBuf> {
        self.git
            .changes
            .get(self.git_selection.selected)
            .map(|c| c.path.clone())
    }

    /// Absolute path of a repository-relative path.
    pub fn git_absolute(&self, relative: &Path) -> Option<PathBuf> {
        self.git.root.as_ref().map(|root| root.join(relative))
    }

    pub fn git_root(&self) -> Option<PathBuf> {
        self.git
            .root
            .clone()
            .or_else(|| self.workspace.git_root.clone())
    }

    // ── problems ──────────────────────────────────────────────────────────

    pub fn problem_counts(&self) -> (usize, usize) {
        let errors = self
            .problems
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        let warnings = self
            .problems
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count();
        (errors, warnings)
    }

    /// Replace the diagnostics of one file, keeping the rest.
    pub fn set_diagnostics(&mut self, path: &Path, diagnostics: Vec<Diagnostic>) {
        self.problems.retain(|d| d.path != path);
        self.problems.extend(diagnostics.iter().cloned());
        self.problems.sort_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then_with(|| a.range.start.line.cmp(&b.range.start.line))
        });
        self.problems_selection.clamp(self.problems.len());
        if let Some(doc) = self
            .documents
            .iter_mut()
            .find(|d| d.path.as_deref() == Some(path))
        {
            doc.diagnostics = diagnostics;
        }
    }

    // ── focus ─────────────────────────────────────────────────────────────

    /// Move focus between regions.
    pub fn move_focus(&mut self, direction: Direction) {
        let visible = self.layout.visible_regions();
        let Some(region) = super::focus::next_region(self.focus.region(), direction, &visible)
        else {
            return;
        };
        self.focus = match region {
            Region::Sidebar => {
                if self.layout.explorer {
                    FocusTarget::Explorer
                } else if self.layout.outline {
                    FocusTarget::Outline
                } else {
                    FocusTarget::Git
                }
            }
            Region::Center => match self.active_tab {
                Some(id) => FocusTarget::Editor(id),
                None if self.layout.problems => FocusTarget::Problems,
                None => FocusTarget::Explorer,
            },
            Region::Agents => FocusTarget::AgentList,
            Region::Terminals => match self
                .focused_terminal
                .or(self.terminal_ids().first().copied())
            {
                Some(id) => {
                    self.focused_terminal = Some(id);
                    FocusTarget::Terminal(id)
                }
                None => self.focus,
            },
            Region::Overlay => self.focus,
        };
    }

    /// Whether a command can run right now.
    pub fn is_available(&self, command: &Command) -> bool {
        match command.requires {
            Requirement::Always => true,
            Requirement::ActiveEditor => self.active_tab.is_some(),
            Requirement::DirtyEditor => self.active_document().is_some_and(Document::is_dirty),
            Requirement::GitRepository => self.git_root().is_some(),
            Requirement::SelectedGitFile => self.selected_git_path().is_some(),
            Requirement::SelectedAgent => self.selected_agent().is_some(),
            Requirement::ActiveTerminal => !self.terminal_ids().is_empty(),
            // Filled in by the language/debug services once they are running.
            Requirement::LanguageServer => self.language_server_available(),
            Requirement::DebugSession => self.debug_session_active(),
        }
    }

    /// True when at least one language server finished initialising.
    pub fn language_server_available(&self) -> bool {
        self.lsp_ready
    }

    /// Dismiss language-service popups (on cursor moves, edits, Esc).
    pub fn dismiss_popups(&mut self) {
        self.hover = None;
        self.completion = None;
    }

    pub fn debug_session_active(&self) -> bool {
        self.debug.status.is_active()
    }

    // ── palette ───────────────────────────────────────────────────────────

    pub fn open_palette(&mut self, mode: PaletteMode) {
        let mut palette = PaletteState::new(mode);
        self.refresh_palette_items(&mut palette);
        self.palette = Some(palette);
        self.focus = FocusTarget::CommandPalette;
    }

    pub fn close_palette(&mut self) {
        self.palette = None;
        self.focus = match self.active_tab {
            Some(id) => FocusTarget::Editor(id),
            None => FocusTarget::Explorer,
        };
    }

    /// Recompute palette results for the current query.
    pub fn refresh_palette_items(&mut self, palette: &mut PaletteState) {
        const LIMIT: usize = 200;
        match palette.mode {
            PaletteMode::Commands => {
                let query = palette.query.trim_start_matches('>').trim().to_string();
                palette.items = self
                    .commands
                    .search(&query)
                    .into_iter()
                    .filter(|m| self.is_available(&m.command))
                    .take(LIMIT)
                    .map(|m| PaletteItem::Command(m.command))
                    .collect();
            }
            PaletteMode::Files => {
                let query = palette.query.clone();
                let strings: Vec<String> = self
                    .file_index
                    .files
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                let items: Vec<PaletteItem> = if query.trim().is_empty() {
                    strings
                        .iter()
                        .take(LIMIT)
                        .map(|s| PaletteItem::File(PathBuf::from(s)))
                        .collect()
                } else {
                    super::commands::fuzzy_rank(
                        &mut self.matcher,
                        &query,
                        strings.iter().map(String::as_str),
                        LIMIT,
                    )
                    .into_iter()
                    .map(|(path, _)| PaletteItem::File(PathBuf::from(path)))
                    .collect()
                };
                palette.items = items;
            }
        }
        if palette.selected >= palette.items.len() {
            palette.selected = 0;
        }
    }

    /// Selected palette row.
    pub fn palette_selection(&self) -> Option<PaletteItem> {
        let palette = self.palette.as_ref()?;
        palette.items.get(palette.selected).cloned()
    }

    // ── modals ────────────────────────────────────────────────────────────

    pub fn confirm(
        &mut self,
        title: impl Into<String>,
        message: impl Into<String>,
        action: ConfirmAction,
    ) {
        self.modal = Some(Modal::Confirm {
            title: title.into(),
            message: message.into(),
            action,
        });
        self.focus = FocusTarget::Modal;
    }

    pub fn prompt(
        &mut self,
        title: impl Into<String>,
        value: impl Into<String>,
        purpose: PromptPurpose,
    ) {
        self.modal = Some(Modal::Prompt {
            title: title.into(),
            value: value.into(),
            purpose,
        });
        self.focus = FocusTarget::Modal;
    }

    pub fn show_message(&mut self, title: impl Into<String>, body: Vec<String>) {
        self.modal = Some(Modal::Message {
            title: title.into(),
            body,
        });
        self.focus = FocusTarget::Modal;
    }

    /// Dismiss the modal and restore a sensible focus.
    pub fn close_modal(&mut self) {
        self.modal = None;
        self.focus = match self.active_tab {
            Some(id) => FocusTarget::Editor(id),
            None => FocusTarget::Explorer,
        };
    }

    // ── explorer ──────────────────────────────────────────────────────────

    /// Path under the explorer cursor.
    pub fn explorer_selection(&self) -> Option<PathBuf> {
        self.tree
            .rows()
            .get(self.explorer.selected)
            .map(|row| row.path.clone())
    }

    /// Directory that new files should be created in.
    pub fn explorer_target_dir(&self) -> PathBuf {
        match self.explorer_selection() {
            Some(path) if path.is_dir() => path,
            Some(path) => path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.workspace.root.clone()),
            None => self.workspace.root.clone(),
        }
    }

    /// Select a path in the tree, expanding ancestors as needed.
    pub fn reveal_in_explorer(&mut self, path: &Path) {
        self.tree.reveal(path);
        if let Some(index) = self.tree.rows().iter().position(|row| row.path == path) {
            self.explorer.selected = index;
        }
    }

    /// Rebuild the tree after external changes, keeping the selection.
    pub fn refresh_tree(&mut self) {
        let selected = self.explorer_selection();
        self.tree.refresh();
        if let Some(path) = selected {
            if let Some(index) = self.tree.rows().iter().position(|row| row.path == path) {
                self.explorer.selected = index;
            } else {
                self.explorer.clamp(self.tree.rows().len());
            }
        }
    }
}

/// Resolve a path against the workspace, rejecting escapes outside it.
///
/// Used by every filesystem mutation so a crafted name cannot write outside
/// the project.
pub fn safe_join(root: &Path, name: &str) -> Result<PathBuf> {
    let candidate = Path::new(name);
    if candidate.is_absolute() {
        return Err(anyhow!("absolute paths are not allowed here"));
    }
    if candidate
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(anyhow!("`..` is not allowed in a name"));
    }
    Ok(root.join(candidate))
}
