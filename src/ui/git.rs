//! Git panel: branch summary plus the changed-file list.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::focus::FocusTarget;
use crate::app::state::AppState;

use super::widgets::{panel, window};
use super::{truncate_path, Theme};

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let focused = state.focus == FocusTarget::Git;
    let block = panel("GIT", focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    if state.git_root().is_none() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "not a git repository",
                theme.dim(),
            ))),
            inner,
        );
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Summary row: branch, upstream distance, change count.
    let mut summary = vec![
        Span::styled("⎇ ", Style::default().fg(theme.accent)),
        Span::styled(
            state.git.head_label(),
            Style::default()
                .fg(theme.text_bright)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some((ahead, behind)) = state.git.ahead_behind {
        if ahead > 0 || behind > 0 {
            summary.push(Span::styled(
                format!("  ↑{ahead} ↓{behind}"),
                Style::default().fg(theme.accent_alt),
            ));
        }
    }
    let count = state.git.changes.len();
    summary.push(Span::styled(
        if count == 0 {
            "  clean".to_string()
        } else {
            format!("  {count} changed")
        },
        Style::default().fg(if count == 0 {
            theme.success
        } else {
            theme.warning
        }),
    ));
    lines.push(Line::from(summary));

    let list_height = (inner.height as usize).saturating_sub(1);
    if state.git.changes.is_empty() {
        lines.push(Line::from(Span::styled("working tree clean", theme.dim())));
    } else if list_height > 0 {
        let offset = window(
            state.git_selection.selected,
            state.git.changes.len(),
            list_height,
            state.git_selection.offset,
        );
        for (index, change) in state
            .git
            .changes
            .iter()
            .enumerate()
            .skip(offset)
            .take(list_height)
        {
            let status = change.display_status();
            let code = change.code();
            let staged = change.is_staged();
            let budget = (inner.width as usize).saturating_sub(4);
            let path = truncate_path(&change.path.to_string_lossy(), budget);
            let mut spans = vec![
                Span::styled(
                    code,
                    Style::default()
                        .fg(theme.git_status(status))
                        .add_modifier(if staged {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::raw(" "),
                Span::styled(path, Style::default().fg(theme.text)),
            ];
            if index == state.git_selection.selected {
                spans = spans
                    .into_iter()
                    .map(|span| {
                        Span::styled(span.content, span.style.patch(theme.selected_row(focused)))
                    })
                    .collect();
            }
            lines.push(Line::from(spans));
        }
    }

    frame.render_widget(Paragraph::new(lines), inner);
}
