//! Command palette and quick open.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::state::{AppState, PaletteItem, PaletteMode, PaletteState};

use super::widgets::window;
use super::{centered_rect, truncate_path, Theme};

pub fn draw(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    palette: &PaletteState,
    theme: &Theme,
) {
    let width = (area.width as f32 * 0.7) as u16;
    let height = (area.height as f32 * 0.6) as u16;
    let rect = centered_rect(area, width.max(30), height.max(8));

    let title = match palette.mode {
        PaletteMode::Commands => " Commands ",
        PaletteMode::Files => " Open File ",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.surface))
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(
            Line::from(Span::styled(
                " ↑↓ select · Enter run · Esc cancel ",
                theme.dim(),
            ))
            .right_aligned(),
        );
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    if inner.height < 2 {
        return;
    }

    // Query line.
    let prompt = match palette.mode {
        PaletteMode::Commands => ">",
        PaletteMode::Files => "▸",
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{prompt} "), Style::default().fg(theme.accent)),
            Span::styled(
                palette.query.clone(),
                Style::default().fg(theme.text_bright),
            ),
            Span::styled("▏", Style::default().fg(theme.accent)),
        ])),
        Rect { height: 1, ..inner },
    );

    let list_area = Rect {
        y: inner.y + 1,
        height: inner.height - 1,
        ..inner
    };
    if palette.items.is_empty() {
        let message = match palette.mode {
            PaletteMode::Files if state.indexing => "indexing workspace…",
            _ => "no matches",
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(message, theme.dim()))),
            list_area,
        );
        return;
    }

    let height = list_area.height as usize;
    let offset = window(palette.selected, palette.items.len(), height, 0);
    let lines: Vec<Line> = palette
        .items
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(index, item)| {
            let selected = index == palette.selected;
            let mut spans = match item {
                PaletteItem::Command(command) => {
                    let hint = state.keymap.hint_for(command.id).unwrap_or_default();
                    let title = command.display();
                    let pad = (list_area.width as usize)
                        .saturating_sub(title.chars().count() + hint.chars().count() + 2);
                    vec![
                        Span::styled(format!(" {title}"), Style::default().fg(theme.text)),
                        Span::raw(" ".repeat(pad)),
                        Span::styled(hint, theme.dim()),
                    ]
                }
                PaletteItem::File(path) => {
                    let text = path.to_string_lossy().to_string();
                    vec![Span::styled(
                        format!(" {}", truncate_path(&text, list_area.width as usize - 2)),
                        Style::default().fg(theme.text),
                    )]
                }
            };
            if selected {
                spans = spans
                    .into_iter()
                    .map(|span| {
                        Span::styled(span.content, span.style.patch(theme.selected_row(true)))
                    })
                    .collect();
            }
            Line::from(spans)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), list_area);
}
