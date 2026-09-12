//! Keyboard shortcut overlay (F1).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::state::AppState;
use crate::config::keys::Scope;

use super::{centered_rect, Theme};

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let rect = centered_rect(
        area,
        (area.width as f32 * 0.8) as u16,
        (area.height as f32 * 0.85) as u16,
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("{} — {}", crate::PRODUCT_NAME, crate::TAGLINE),
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        format!(
            "Press {} then a key for workbench commands. Terminal panes keep every other key.",
            state.keymap.prefix
        ),
        theme.dim(),
    )));
    lines.push(Line::from(""));

    for (scope, title) in [
        (Scope::Global, "GLOBAL"),
        (Scope::Prefix, "PREFIX"),
        (Scope::Editor, "EDITOR"),
    ] {
        lines.push(Line::from(Span::styled(
            title,
            Style::default()
                .fg(theme.text_bright)
                .add_modifier(Modifier::BOLD),
        )));
        for binding in state.keymap.all().iter().filter(|b| b.scope == scope) {
            let command = state.commands.get(&binding.command);
            let title = command
                .map(|c| c.display())
                .unwrap_or_else(|| binding.command.clone());
            let chord = if scope == Scope::Prefix {
                format!("{} {}", state.keymap.prefix, binding.chord)
            } else {
                binding.chord.to_string()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {chord:<18}"), Style::default().fg(theme.accent)),
                Span::styled(title, Style::default().fg(theme.text)),
            ]));
        }
        lines.push(Line::from(""));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.surface))
        .title(Span::styled(
            " Keyboard Shortcuts ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(" Esc / F1 close ", theme.dim())).right_aligned());

    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(Style::default().bg(theme.surface)),
        rect,
    );
}
