//! The terminal strip: up to three live PTY panes side by side.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use crate::app::focus::FocusTarget;
use crate::app::state::AppState;
use crate::domain::ids::TerminalId;
use crate::services::terminal::ScreenSnapshot;

use super::widgets::panel;
use super::{truncate, Theme, WorkbenchLayout};

/// How many panes fit side by side at this width.
pub fn visible_pane_count(width: u16, total: usize) -> usize {
    if total == 0 {
        return 0;
    }
    let capacity = match width {
        0..=79 => 1,
        80..=139 => 2,
        _ => 3,
    };
    capacity.min(total)
}

/// Draw the strip and record pane rectangles in the layout.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    layout: &mut WorkbenchLayout,
) {
    let Ok(terminals) = state.terminals.lock() else {
        return;
    };
    let ids = terminals.ids();

    if ids.is_empty() {
        let focused = matches!(state.focus, FocusTarget::Terminal(_));
        let block = panel("TERMINAL", focused, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let hint = state
            .keymap
            .hint_for("terminal.new")
            .unwrap_or_else(|| "—".into());
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("no terminals running", theme.dim())),
                Line::from(Span::styled(
                    format!("{hint}  new terminal"),
                    Style::default().fg(theme.text_dim),
                )),
            ]),
            inner,
        );
        return;
    }

    let count = visible_pane_count(area.width, ids.len());
    // Keep the focused pane inside the visible window.
    let focused_index = state
        .focused_terminal
        .and_then(|id| ids.iter().position(|i| *i == id))
        .unwrap_or(0);
    let mut offset = state.terminal_view_offset.min(ids.len().saturating_sub(1));
    if focused_index < offset {
        offset = focused_index;
    } else if focused_index >= offset + count {
        offset = focused_index + 1 - count;
    }

    let constraints: Vec<Constraint> = (0..count)
        .map(|_| Constraint::Percentage((100 / count.max(1)) as u16))
        .collect();
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (slot, id) in ids.iter().skip(offset).take(count).enumerate() {
        let rect = columns[slot];
        layout.terminals.push((*id, rect));
        let Some(session) = terminals.get(*id) else {
            continue;
        };
        let focused = state.focus == FocusTarget::Terminal(*id);
        let more_left = offset > 0 && slot == 0;
        let more_right = slot + 1 == count && offset + count < ids.len();

        let index = ids.iter().position(|i| i == id).unwrap_or(0) + 1;
        let mut title = format!("{index}:{}", session.label());
        if more_left {
            title = format!("‹ {title}");
        }
        if more_right {
            title = format!("{title} ›");
        }

        let state_label = session.state().label();
        let scroll = session.scroll_offset();
        let hint = if scroll > 0 {
            format!("{state_label} · scrollback -{scroll}")
        } else {
            state_label
        };

        let block = super::widgets::panel_with_hint(&title, hint, focused, theme);
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        if inner.width == 0 || inner.height == 0 {
            continue;
        }

        let snapshot = session.snapshot();
        frame.render_widget(
            TerminalWidget {
                snapshot: &snapshot,
                theme,
            },
            inner,
        );

        if focused && snapshot.cursor_visible && session.is_running() {
            let (row, col) = snapshot.cursor;
            let x = inner.x + col.min(inner.width.saturating_sub(1));
            let y = inner.y + row.min(inner.height.saturating_sub(1));
            frame.set_cursor_position(Position::new(x, y));
        }

        if !session.is_running() {
            let notice = format!(
                " {} · {} restart ",
                session.state().label(),
                state
                    .keymap
                    .hint_for("terminal.restart")
                    .unwrap_or_else(|| "—".into())
            );
            let rect = Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(1),
                width: inner.width,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    truncate(&notice, inner.width as usize),
                    Style::default()
                        .fg(theme.background)
                        .bg(theme.warning)
                        .add_modifier(Modifier::BOLD),
                ))),
                rect,
            );
        }
    }
}

/// Renders a parsed terminal screen into a Ratatui buffer, cell by cell.
struct TerminalWidget<'a> {
    snapshot: &'a ScreenSnapshot,
    theme: &'a Theme,
}

impl Widget for TerminalWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for row in 0..area.height {
            let Some(cells) = self.snapshot.rows.get(row as usize) else {
                break;
            };
            for col in 0..area.width {
                let Some(cell) = cells.get(col as usize) else {
                    break;
                };
                let target = &mut buf[(area.x + col, area.y + row)];
                let mut style = Style::default()
                    .fg(self.theme.terminal_color(cell.fg, true))
                    .bg(self.theme.terminal_color(cell.bg, false));
                if cell.bold {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if cell.italic {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                if cell.underline {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                if cell.inverse {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                target.set_symbol(if cell.text.is_empty() {
                    " "
                } else {
                    &cell.text
                });
                target.set_style(style);
            }
        }
    }
}

/// Size of the PTY grid for a pane of this rectangle (inside its border).
pub fn pty_size(rect: Rect) -> (u16, u16) {
    (
        rect.height.saturating_sub(2).max(1),
        rect.width.saturating_sub(2).max(1),
    )
}

/// Pane areas for the ids that should be visible, without drawing.
pub fn pane_geometry(area: Rect, ids: &[TerminalId], offset: usize) -> Vec<(TerminalId, Rect)> {
    let count = visible_pane_count(area.width, ids.len());
    if count == 0 {
        return Vec::new();
    }
    let constraints: Vec<Constraint> = (0..count)
        .map(|_| Constraint::Percentage((100 / count) as u16))
        .collect();
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);
    ids.iter()
        .skip(offset)
        .take(count)
        .enumerate()
        .map(|(slot, id)| (*id, columns[slot]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_count_scales_with_width() {
        assert_eq!(visible_pane_count(60, 4), 1);
        assert_eq!(visible_pane_count(100, 4), 2);
        assert_eq!(visible_pane_count(200, 4), 3);
        assert_eq!(visible_pane_count(200, 1), 1);
        assert_eq!(visible_pane_count(200, 0), 0);
    }

    #[test]
    fn pty_size_excludes_the_border() {
        let (rows, cols) = pty_size(Rect::new(0, 0, 40, 12));
        assert_eq!((rows, cols), (10, 38));
        let (rows, cols) = pty_size(Rect::new(0, 0, 1, 1));
        assert_eq!((rows, cols), (1, 1), "never zero");
    }

    #[test]
    fn geometry_covers_the_visible_window() {
        let ids: Vec<TerminalId> = (0..4).map(|_| TerminalId::next()).collect();
        let geometry = pane_geometry(Rect::new(0, 0, 200, 10), &ids, 1);
        assert_eq!(geometry.len(), 3);
        assert_eq!(geometry[0].0, ids[1]);
        assert!(geometry[0].1.width > 0);
    }
}
