//! Language-server status block in the sidebar.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::state::AppState;
use crate::services::lsp::ClientStatus;

use super::widgets::panel;
use super::{truncate, Theme};

/// Show each configured server and whether it is actually running.
pub fn draw(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let block = panel("LSP", false, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    if state.lsp_statuses.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("no servers configured", theme.dim())),
                Line::from(Span::styled("add [lsp.<name>] to config", theme.dim())),
            ]),
            inner,
        );
        return;
    }

    let lines: Vec<Line> = state
        .lsp_statuses
        .iter()
        .take(inner.height as usize)
        .map(|server| {
            let (glyph, label, colour) = match (server.running, server.status) {
                (true, ClientStatus::Ready) => ("●", "Running", theme.success),
                (true, ClientStatus::Starting) => ("◌", "Starting", theme.accent),
                (true, ClientStatus::Failed) => ("×", "Failed", theme.danger),
                (true, ClientStatus::Exited) => ("▫", "Stopped", theme.text_dim),
                // Not running: either it failed to start, or it simply has
                // not been needed yet. Never imply the first is the second.
                (false, _) if server.failure.is_some() => ("×", "Failed", theme.danger),
                (false, _) => ("○", "Available", theme.text_dim),
            };
            let budget = (inner.width as usize).saturating_sub(label.len() + 3);
            let name = truncate(&server.name, budget);
            let pad = budget.saturating_sub(name.chars().count());
            Line::from(vec![
                Span::styled(format!("{glyph} "), Style::default().fg(colour)),
                Span::styled(name, Style::default().fg(theme.text)),
                Span::raw(" ".repeat(pad)),
                Span::styled(label, Style::default().fg(colour)),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}
