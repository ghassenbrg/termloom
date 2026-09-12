//! Small shared drawing helpers.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};

use super::Theme;

/// Standard panel frame: thin border, compact uppercase title, focus accent.
pub fn panel<'a>(title: &'a str, focused: bool, theme: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(theme.panel_border(focused))
        .title(Span::styled(
            format!(" {title} "),
            theme.panel_title(focused),
        ))
}

/// Panel frame with an extra right-aligned hint in the title bar.
pub fn panel_with_hint<'a>(
    title: &'a str,
    hint: impl Into<String>,
    focused: bool,
    theme: &Theme,
) -> Block<'a> {
    panel(title, focused, theme).title_bottom(
        Line::from(Span::styled(format!(" {} ", hint.into()), theme.dim())).right_aligned(),
    )
}

/// Scroll offset that keeps `selected` visible inside `height` rows.
pub fn window(selected: usize, len: usize, height: usize, stored: usize) -> usize {
    if height == 0 || len == 0 {
        return 0;
    }
    let max_offset = len.saturating_sub(height);
    let mut offset = stored.min(max_offset);
    if selected < offset {
        offset = selected;
    } else if selected >= offset + height {
        offset = selected + 1 - height;
    }
    offset.min(max_offset)
}

/// A dimmed, centred "nothing here" message.
pub fn empty_state<'a>(message: &'a str, theme: &Theme) -> Line<'a> {
    Line::from(Span::styled(message, theme.dim()))
}

/// `label` + value pair used across the detail panels.
pub fn field<'a>(label: &'a str, value: impl Into<String>, theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{label:<11}"), theme.dim()),
        Span::styled(value.into(), Style::default().fg(theme.text)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_follows_the_selection() {
        assert_eq!(window(0, 100, 10, 0), 0);
        assert_eq!(window(15, 100, 10, 0), 6, "scrolls down to reveal");
        assert_eq!(window(3, 100, 10, 20), 3, "scrolls up to reveal");
        assert_eq!(window(99, 100, 10, 0), 90);
    }

    #[test]
    fn window_clamps_to_the_end_of_the_list() {
        assert_eq!(window(5, 6, 10, 4), 0, "short lists never scroll");
        assert_eq!(window(0, 0, 10, 3), 0);
        assert_eq!(window(5, 100, 0, 0), 0);
    }
}
