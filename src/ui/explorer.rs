//! Project explorer.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::focus::FocusTarget;
use crate::app::state::AppState;

use super::widgets::{panel, window};
use super::{truncate, Theme};

/// Draw the file tree with git decorations.
pub fn draw(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let focused = state.focus == FocusTarget::Explorer;
    let block = panel("EXPLORER", focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let rows = state.tree.rows();
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("empty directory", theme.dim()))),
            inner,
        );
        return;
    }

    let height = inner.height as usize;
    let offset = window(
        state.explorer.selected,
        rows.len(),
        height,
        state.explorer.offset,
    );
    let git_root = state.git.root.clone();

    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, row)| {
            let selected = index == state.explorer.selected;
            let indent = "  ".repeat(row.depth.saturating_sub(1));
            let glyph = if row.is_dir {
                if row.expanded {
                    "▾ "
                } else {
                    "▸ "
                }
            } else {
                "  "
            };

            // Git decoration: a letter code, never colour alone.
            let (code, colour) = match git_root.as_ref().and_then(|root| {
                if row.is_dir {
                    state
                        .git
                        .dir_has_changes(root, &row.path)
                        .then_some((None, theme.warning))
                } else {
                    state
                        .git
                        .status_for(root, &row.path)
                        .map(|status| (Some(status.code()), theme.git_status(status)))
                }
            }) {
                Some((Some(code), colour)) => (code.to_string(), colour),
                Some((None, colour)) => ("•".to_string(), colour),
                None => (" ".to_string(), theme.text_dim),
            };

            let name_style = if row.is_dir {
                Style::default().fg(theme.text_bright)
            } else if code != " " {
                Style::default().fg(colour)
            } else {
                Style::default().fg(theme.text)
            };

            let prefix_width = indent.chars().count() + 2;
            let budget = (inner.width as usize).saturating_sub(prefix_width + 2);
            let name = truncate(&row.name, budget);
            let pad = budget.saturating_sub(name.chars().count());

            let mut spans = vec![
                Span::styled(indent, theme.dim()),
                Span::styled(glyph, Style::default().fg(theme.text_dim)),
                Span::styled(name, name_style),
                Span::raw(" ".repeat(pad)),
                Span::styled(code, Style::default().fg(colour)),
            ];
            if selected {
                spans = spans
                    .into_iter()
                    .map(|span| {
                        let style = span.style.patch(theme.selected_row(focused));
                        Span::styled(span.content, style)
                    })
                    .collect();
            }
            Line::from(spans)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);

    // Scroll indicator, so long trees do not feel bottomless.
    if rows.len() > height {
        let position = format!("{}/{}", state.explorer.selected + 1, rows.len());
        let hint = Paragraph::new(Line::from(Span::styled(
            position,
            Style::default()
                .fg(theme.text_dim)
                .add_modifier(Modifier::DIM),
        )))
        .right_aligned();
        let rect = Rect {
            x: inner.x,
            y: area.y + area.height.saturating_sub(1),
            width: inner.width,
            height: 1,
        };
        frame.render_widget(hint, rect);
    }
}
