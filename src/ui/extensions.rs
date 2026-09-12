//! Extensions screen: what TermLoom can actually use from installed packages.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::state::AppState;
use crate::domain::extensions::{CapabilitySupport, CompatibilityClass};

use super::widgets::window;
use super::{centered_rect, truncate, Theme};

/// The extensions screen is an overlay rather than a permanent panel: it is
/// consulted occasionally, unlike the explorer or the agents list.
pub fn draw(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let rect = centered_rect(
        area,
        (area.width as f32 * 0.8) as u16,
        (area.height as f32 * 0.8) as u16,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.surface))
        .title(Span::styled(
            " Extensions ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(
            Line::from(Span::styled(
                " ↑↓ select · i install vsix · x remove · Esc close ",
                theme.dim(),
            ))
            .right_aligned(),
        );
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    if inner.height < 3 {
        return;
    }

    if state.extensions.installed.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("no extensions installed", theme.dim())),
                Line::from(""),
                Line::from(Span::styled(
                    "install a local package with:  termloom extension install ./ext.vsix",
                    theme.dim(),
                )),
                Line::from(Span::styled(
                    "TermLoom reuses declarative assets only; extension code is never executed.",
                    theme.dim(),
                )),
            ]),
            inner,
        );
        return;
    }

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(inner);

    let height = columns[0].height as usize;
    let offset = window(
        state.extensions.selected,
        state.extensions.installed.len(),
        height,
        0,
    );
    let list: Vec<Line> = state
        .extensions
        .installed
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, report)| {
            let colour = class_colour(report.class, theme);
            let label = format!(
                "{} {}  {}",
                report.class.glyph(),
                truncate(
                    &report.display_name,
                    (columns[0].width as usize).saturating_sub(14)
                ),
                report.class.label()
            );
            let mut span = Span::styled(label, Style::default().fg(colour));
            if index == state.extensions.selected {
                span = Span::styled(span.content, span.style.patch(theme.selected_row(true)));
            }
            Line::from(span)
        })
        .collect();
    frame.render_widget(Paragraph::new(list), columns[0]);

    let Some(report) = state.extensions.installed.get(state.extensions.selected) else {
        return;
    };
    let mut detail = vec![
        Line::from(Span::styled(
            format!("{} {}", report.display_name, report.version),
            Style::default()
                .fg(theme.text_bright)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(report.id.clone(), theme.dim())),
        Line::from(""),
    ];
    for capability in &report.capabilities {
        let colour = match capability.support {
            CapabilitySupport::Supported => theme.success,
            CapabilitySupport::NeedsAdaptation => theme.warning,
            CapabilitySupport::Unsupported => theme.danger,
        };
        detail.push(Line::from(vec![
            Span::styled(
                format!("{} ", capability.support.glyph()),
                Style::default().fg(colour),
            ),
            Span::styled(capability.name.clone(), Style::default().fg(theme.text)),
        ]));
        detail.push(Line::from(Span::styled(
            format!("    {}", capability.detail),
            theme.dim(),
        )));
    }
    frame.render_widget(Paragraph::new(detail), columns[1]);
}

fn class_colour(class: CompatibilityClass, theme: &Theme) -> ratatui::style::Color {
    match class {
        CompatibilityClass::Full => theme.success,
        CompatibilityClass::Partial => theme.warning,
        CompatibilityClass::Unsupported => theme.text_dim,
        CompatibilityClass::Broken => theme.danger,
    }
}
