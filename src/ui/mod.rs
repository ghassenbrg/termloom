//! Ratatui rendering.
//!
//! Drawing is a pure function of [`AppState`]: `draw` computes a
//! [`WorkbenchLayout`] for the current terminal size, renders each panel into
//! it and returns the layout so the event loop can hit-test mouse clicks and
//! resize pseudo terminals to match their panes.

pub mod agents;
pub mod debug;
pub mod editor;
pub mod explorer;
pub mod extensions;
pub mod git;
pub mod help;
pub mod modal;
pub mod outline;
pub mod palette;
pub mod problems;
pub mod statusbar;
pub mod terminals;
pub mod theme;
pub mod widgets;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::Clear;
use ratatui::Frame;

use crate::app::focus::FocusTarget;
use crate::app::state::AppState;
use crate::domain::ids::TerminalId;

pub use theme::Theme;

/// How much of the workbench fits on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// Sidebar + editor + agents + terminal strip.
    Large,
    /// Sidebar + editor + terminal strip; agents only when there is room.
    Medium,
    /// One dominant surface at a time.
    Small,
}

impl LayoutMode {
    pub fn for_size(width: u16, height: u16) -> LayoutMode {
        if width >= 120 && height >= 28 {
            LayoutMode::Large
        } else if width >= 80 && height >= 18 {
            LayoutMode::Medium
        } else {
            LayoutMode::Small
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LayoutMode::Large => "wide",
            LayoutMode::Medium => "compact",
            LayoutMode::Small => "minimal",
        }
    }
}

/// Where every panel ended up this frame.
#[derive(Debug, Clone, Default)]
pub struct WorkbenchLayout {
    pub mode: Option<LayoutModeWrapper>,
    pub header: Rect,
    pub explorer: Option<Rect>,
    pub outline: Option<Rect>,
    pub git: Option<Rect>,
    pub editor_tabs: Option<Rect>,
    pub editor: Option<Rect>,
    pub bottom_panel: Option<Rect>,
    pub agents: Option<Rect>,
    pub agent_detail: Option<Rect>,
    /// Visible terminal panes and their areas.
    pub terminals: Vec<(TerminalId, Rect)>,
    pub status: Rect,
}

/// Newtype so [`WorkbenchLayout`] can derive `Default`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutModeWrapper(pub LayoutMode);

impl WorkbenchLayout {
    /// Inner text area of a panel (inside its border).
    pub fn inner(rect: Rect) -> Rect {
        Rect {
            x: rect.x.saturating_add(1),
            y: rect.y.saturating_add(1),
            width: rect.width.saturating_sub(2),
            height: rect.height.saturating_sub(2),
        }
    }

    /// Which focus target owns the point, for click-to-focus.
    pub fn hit_test(&self, x: u16, y: u16) -> Option<FocusTarget> {
        let hit = |rect: Option<Rect>| rect.is_some_and(|r| contains(r, x, y));
        if hit(self.explorer) {
            return Some(FocusTarget::Explorer);
        }
        if hit(self.outline) {
            return Some(FocusTarget::Outline);
        }
        if hit(self.git) {
            return Some(FocusTarget::Git);
        }
        if hit(self.agents) {
            return Some(FocusTarget::AgentList);
        }
        if hit(self.agent_detail) {
            return Some(FocusTarget::AgentDetail);
        }
        for (id, rect) in &self.terminals {
            if contains(*rect, x, y) {
                return Some(FocusTarget::Terminal(*id));
            }
        }
        if hit(self.editor) || hit(self.editor_tabs) {
            return Some(FocusTarget::Editor(crate::domain::ids::EditorTabId(0)));
        }
        None
    }
}

fn contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

/// Render the whole workbench.
pub fn draw(frame: &mut Frame, state: &AppState, theme: &Theme) -> WorkbenchLayout {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let mode = LayoutMode::for_size(area.width, area.height);

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    let mut layout = WorkbenchLayout {
        mode: Some(LayoutModeWrapper(mode)),
        header: vertical[0],
        status: vertical[2],
        ..Default::default()
    };

    statusbar::draw_header(frame, vertical[0], state, theme);

    match mode {
        LayoutMode::Small => draw_small(frame, vertical[1], state, theme, &mut layout),
        _ => draw_wide(frame, vertical[1], state, theme, &mut layout, mode),
    }

    statusbar::draw_status(frame, vertical[2], state, theme, mode);

    // Overlays, drawn last so they sit on top.
    if state.help_visible {
        help::draw(frame, area, state, theme);
    }
    if state.layout.extensions {
        extensions::draw(frame, area, state, theme);
    }
    if let Some(palette) = &state.palette {
        palette::draw(frame, area, state, palette, theme);
    }
    if let Some(modal) = &state.modal {
        modal::draw(frame, area, modal, theme);
    }
    statusbar::draw_toasts(frame, area, state, theme);

    layout
}

/// Large/medium layout: sidebar, editor, agents, terminal strip.
fn draw_wide(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    layout: &mut WorkbenchLayout,
    mode: LayoutMode,
) {
    let show_terminals = state.layout.terminals && area.height >= 14;
    let body_terminal = if show_terminals {
        let pct = state.layout.terminal_height_pct.clamp(15, 60);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(100 - pct),
                Constraint::Percentage(pct),
            ])
            .split(area);
        (rows[0], Some(rows[1]))
    } else {
        (area, None)
    };

    let sidebar_visible =
        (state.layout.explorer || state.layout.outline || state.layout.git) && area.width >= 70;
    let agents_visible = state.layout.agents
        && match mode {
            LayoutMode::Large => true,
            _ => area.width >= 110,
        };

    let mut constraints = Vec::new();
    if sidebar_visible {
        constraints.push(Constraint::Length(state.layout.sidebar_width.clamp(18, 50)));
    }
    constraints.push(Constraint::Min(30));
    if agents_visible {
        constraints.push(Constraint::Length(state.layout.agents_width.clamp(24, 60)));
    }

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(body_terminal.0);

    let mut index = 0;
    if sidebar_visible {
        draw_sidebar(frame, columns[index], state, theme, layout, mode);
        index += 1;
    }
    draw_center(frame, columns[index], state, theme, layout);
    index += 1;
    if agents_visible {
        agents::draw(frame, columns[index], state, theme, layout);
    }

    if let Some(strip) = body_terminal.1 {
        terminals::draw(frame, strip, state, theme, layout);
    }
}

/// Sidebar stack: explorer (flexible), outline, git.
fn draw_sidebar(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    layout: &mut WorkbenchLayout,
    mode: LayoutMode,
) {
    // Outline is the first thing to go when vertical room is tight.
    let show_outline = state.layout.outline && mode == LayoutMode::Large && area.height >= 26;
    let show_git = state.layout.git && area.height >= 16;

    let mut constraints = vec![Constraint::Min(6)];
    if show_outline {
        constraints.push(Constraint::Length(area.height / 5));
    }
    if show_git {
        constraints.push(Constraint::Length((area.height / 3).max(6)));
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut index = 0;
    if state.layout.explorer {
        explorer::draw(frame, rows[index], state, theme);
        layout.explorer = Some(rows[index]);
    }
    index += 1;
    if show_outline {
        outline::draw(frame, rows[index], state, theme);
        layout.outline = Some(rows[index]);
        index += 1;
    }
    if show_git {
        git::draw(frame, rows[index], state, theme);
        layout.git = Some(rows[index]);
    }
}

/// Centre column: editor tabs, editor body, optional bottom panel.
fn draw_center(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    layout: &mut WorkbenchLayout,
) {
    let show_bottom = (state.layout.problems || state.layout.debug) && area.height >= 16;
    let rows = if show_bottom {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(area.height / 3)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1)])
            .split(area)
    };

    editor::draw(frame, rows[0], state, theme, layout);

    if show_bottom {
        let panel = rows[1];
        layout.bottom_panel = Some(panel);
        if state.layout.problems && state.layout.debug {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(panel);
            problems::draw(frame, split[0], state, theme);
            debug::draw(frame, split[1], state, theme);
        } else if state.layout.problems {
            problems::draw(frame, panel, state, theme);
        } else {
            debug::draw(frame, panel, state, theme);
        }
    }
}

/// Small layout: whatever currently has focus fills the screen.
fn draw_small(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    layout: &mut WorkbenchLayout,
) {
    match state.focus {
        FocusTarget::Explorer => {
            explorer::draw(frame, area, state, theme);
            layout.explorer = Some(area);
        }
        FocusTarget::Outline => {
            outline::draw(frame, area, state, theme);
            layout.outline = Some(area);
        }
        FocusTarget::Git => {
            git::draw(frame, area, state, theme);
            layout.git = Some(area);
        }
        FocusTarget::AgentList | FocusTarget::AgentDetail => {
            agents::draw(frame, area, state, theme, layout);
        }
        FocusTarget::Terminal(_) => terminals::draw(frame, area, state, theme, layout),
        FocusTarget::Problems => problems::draw(frame, area, state, theme),
        FocusTarget::Debug => debug::draw(frame, area, state, theme),
        _ => editor::draw(frame, area, state, theme, layout),
    }
}

/// Centre a box of the given size inside `area`.
pub fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// Truncate to `width` columns, appending `…` when it does not fit.
pub fn truncate(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width == 1 {
        return "…".to_string();
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.push('…');
    out
}

/// Shorten the middle of a path so both ends stay readable.
pub fn truncate_path(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width || width < 6 {
        return truncate(text, width);
    }
    let keep_end = width / 2 - 1;
    let keep_start = width - keep_end - 1;
    let start: String = text.chars().take(keep_start).collect();
    let end: String = text.chars().skip(count - keep_end).collect();
    format!("{start}…{end}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_mode_follows_terminal_size() {
        assert_eq!(LayoutMode::for_size(200, 60), LayoutMode::Large);
        assert_eq!(LayoutMode::for_size(100, 24), LayoutMode::Medium);
        assert_eq!(LayoutMode::for_size(70, 20), LayoutMode::Small);
        assert_eq!(LayoutMode::for_size(200, 10), LayoutMode::Small);
    }

    #[test]
    fn truncation_keeps_within_the_budget() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("hello", 0), "");
        assert_eq!(truncate("hello", 1), "…");
    }

    #[test]
    fn path_truncation_keeps_both_ends() {
        let out = truncate_path("src/features/auth/auth_provider.dart", 20);
        assert_eq!(out.chars().count(), 20);
        assert!(out.starts_with("src/"), "{out}");
        assert!(out.ends_with(".dart"), "{out}");
    }

    #[test]
    fn centered_rect_stays_inside_the_area() {
        let area = Rect::new(0, 0, 40, 20);
        let rect = centered_rect(area, 100, 100);
        assert_eq!(rect.width, 40);
        assert_eq!(rect.height, 20);
        let rect = centered_rect(area, 20, 10);
        assert_eq!((rect.x, rect.y), (10, 5));
    }
}
