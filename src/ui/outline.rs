//! Outline panel: document symbols from LSP, or Tree-sitter as a fallback.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::focus::FocusTarget;
use crate::app::state::AppState;

use super::widgets::{panel, window};
use super::{truncate, Theme};

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let focused = state.focus == FocusTarget::Outline;
    let block = panel("OUTLINE", focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let Some(document) = state.active_document() else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("no file open", theme.dim()))),
            inner,
        );
        return;
    };

    if document.symbols.is_empty() {
        let message = if crate::services::syntax::outline::supports(&document.language) {
            "no symbols found"
        } else {
            "outline unavailable for this language"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(message, theme.dim()))),
            inner,
        );
        return;
    }

    let height = inner.height as usize;
    let offset = window(
        state.outline_selection.selected,
        document.symbols.len(),
        height,
        state.outline_selection.offset,
    );

    let lines: Vec<Line> = document
        .symbols
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, symbol)| {
            let indent = "  ".repeat(symbol.depth.min(4));
            let kind = symbol.kind.label();
            let budget =
                (inner.width as usize).saturating_sub(indent.chars().count() + kind.len() + 2);
            let name = truncate(&symbol.name, budget);
            let pad = budget.saturating_sub(name.chars().count());
            let mut spans = vec![
                Span::styled(indent, theme.dim()),
                Span::styled(name, Style::default().fg(theme.text)),
                Span::raw(" ".repeat(pad + 1)),
                Span::styled(kind, theme.dim()),
            ];
            if index == state.outline_selection.selected {
                spans = spans
                    .into_iter()
                    .map(|span| {
                        Span::styled(span.content, span.style.patch(theme.selected_row(focused)))
                    })
                    .collect();
            }
            Line::from(spans)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}
