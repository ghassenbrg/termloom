//! Problems panel: diagnostics from language servers.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::focus::FocusTarget;
use crate::app::state::AppState;

use super::widgets::{panel_with_hint, window};
use super::{truncate, Theme};

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    // The same slot shows references when a lookup produced some.
    if state.references.is_some() {
        draw_references(frame, area, state, theme);
        return;
    }
    let focused = state.focus == FocusTarget::Problems;
    let (errors, warnings) = state.problem_counts();
    let block = panel_with_hint(
        "PROBLEMS",
        format!("{errors} errors · {warnings} warnings"),
        focused,
        theme,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    if state.problems.is_empty() {
        let message = if state.lsp_ready {
            "no problems detected"
        } else {
            "no language server running — start one with lsp.start"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(message, theme.dim()))),
            inner,
        );
        return;
    }

    let height = inner.height as usize;
    let offset = window(
        state.problems_selection.selected,
        state.problems.len(),
        height,
        state.problems_selection.offset,
    );
    let lines: Vec<Line> = state
        .problems
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, diagnostic)| {
            let name = diagnostic
                .path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| diagnostic.path.display().to_string());
            let location = format!("{name}:{}", diagnostic.range.start.line + 1);
            let source = diagnostic
                .source
                .clone()
                .map(|s| format!(" [{s}]"))
                .unwrap_or_default();
            let text = format!(
                "{} {location}{source} {}",
                diagnostic.severity.glyph(),
                diagnostic.message.lines().next().unwrap_or("")
            );
            let mut span = Span::styled(
                truncate(&text, inner.width as usize),
                Style::default().fg(theme.severity(diagnostic.severity)),
            );
            if index == state.problems_selection.selected {
                span = Span::styled(span.content, span.style.patch(theme.selected_row(focused)));
            }
            Line::from(span)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Results of `lsp.references`.
fn draw_references(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let Some(references) = &state.references else {
        return;
    };
    let focused = state.focus == FocusTarget::Problems;
    let block = panel_with_hint(
        "REFERENCES",
        format!(
            "{} · {} results · Esc back to problems",
            references.query,
            references.locations.len()
        ),
        focused,
        theme,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let height = inner.height as usize;
    let offset = window(references.selected, references.locations.len(), height, 0);
    let lines: Vec<Line> = references
        .locations
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, location)| {
            let name = location
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let text = format!(
                "{name}:{}:{}",
                location.range.start.line + 1,
                location.range.start.character + 1
            );
            let mut span = Span::styled(
                truncate(&text, inner.width as usize),
                Style::default().fg(theme.text),
            );
            if index == references.selected {
                span = Span::styled(span.content, span.style.patch(theme.selected_row(focused)));
            }
            Line::from(span)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}
