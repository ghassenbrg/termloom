//! Confirmation dialogs, prompts and message boxes.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::state::Modal;

use super::{centered_rect, Theme};

pub fn draw(frame: &mut Frame, area: Rect, modal: &Modal, theme: &Theme) {
    let (title, body, footer, accent) = match modal {
        Modal::Confirm { title, message, .. } => (
            title.clone(),
            // Messages are written with line breaks (a command line on its
            // own line, for instance); keep them.
            message
                .split('\n')
                .map(|line| {
                    Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(theme.text),
                    ))
                })
                .collect(),
            " Enter / y confirm · Esc cancel ".to_string(),
            theme.danger,
        ),
        Modal::Prompt { title, value, .. } => (
            title.clone(),
            vec![Line::from(vec![
                Span::styled("› ", Style::default().fg(theme.accent)),
                Span::styled(value.clone(), Style::default().fg(theme.text_bright)),
                Span::styled("▏", Style::default().fg(theme.accent)),
            ])],
            " Enter confirm · Esc cancel ".to_string(),
            theme.accent,
        ),
        Modal::Select {
            title,
            options,
            selected,
            ..
        } => (
            title.clone(),
            options
                .iter()
                .enumerate()
                .map(|(index, option)| {
                    let style = if index == *selected {
                        Style::default()
                            .fg(theme.text_bright)
                            .bg(theme.selection)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text)
                    };
                    Line::from(Span::styled(format!(" {option}"), style))
                })
                .collect(),
            " ↑↓ select · Enter choose · Esc cancel ".to_string(),
            theme.accent,
        ),
        Modal::Message { title, body } => (
            title.clone(),
            body.iter()
                .map(|line| Line::from(Span::styled(line.clone(), Style::default().fg(theme.text))))
                .collect(),
            " Esc close ".to_string(),
            theme.accent_alt,
        ),
    };

    let width = (area.width as f32 * 0.6).max(40.0) as u16;
    let height = (body.len() as u16 + 4)
        .min(area.height.saturating_sub(2))
        .max(6);
    let rect = centered_rect(area, width, height);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
        .style(Style::default().bg(theme.surface))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(footer, theme.dim())).right_aligned());

    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(body)
            .block(block)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(theme.surface)),
        rect,
    );
}
