//! Debug panel: breakpoints, call stack and variables.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::focus::FocusTarget;
use crate::app::state::AppState;

use super::widgets::panel_with_hint;
use super::{truncate, Theme};

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let focused = state.focus == FocusTarget::Debug;
    let block = panel_with_hint("DEBUG", state.debug.status.label(), focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    if !state.debug.status.is_active() {
        let mut lines = vec![Line::from(Span::styled(
            match state.debug.last_error.as_deref() {
                Some(error) => error,
                None => "no debug session",
            },
            if state.debug.last_error.is_some() {
                Style::default().fg(theme.danger)
            } else {
                theme.dim()
            },
        ))];
        let breakpoints = breakpoint_lines(state, inner.width as usize, theme);
        if !breakpoints.is_empty() {
            lines.push(Line::from(""));
            lines.extend(breakpoints);
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                "{} start · {} toggle breakpoint",
                state
                    .keymap
                    .hint_for("debug.start")
                    .unwrap_or_else(|| "palette".into()),
                state
                    .keymap
                    .hint_for("debug.toggle_breakpoint")
                    .unwrap_or_else(|| "F9".into())
            ),
            theme.dim(),
        )));
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    // Call stack.
    let mut stack: Vec<Line> = vec![Line::from(Span::styled(
        "CALL STACK",
        Style::default()
            .fg(theme.text_dim)
            .add_modifier(Modifier::BOLD),
    ))];
    if state.debug.frames.is_empty() {
        stack.push(Line::from(Span::styled("running…", theme.dim())));
    } else {
        for (index, frame_info) in state.debug.frames.iter().enumerate() {
            let selected = index == state.debug.selected_frame;
            let location = frame_info
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| format!("{}:{}", n.to_string_lossy(), frame_info.line))
                .unwrap_or_default();
            let text = format!(
                "{} {}  {}",
                if selected { "▸" } else { " " },
                frame_info.name,
                location
            );
            let style = if selected {
                Style::default()
                    .fg(theme.text_bright)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            stack.push(Line::from(Span::styled(
                truncate(&text, columns[0].width as usize),
                style,
            )));
        }
    }
    frame.render_widget(Paragraph::new(stack), columns[0]);

    // Variables.
    let mut variables: Vec<Line> = vec![Line::from(Span::styled(
        "VARIABLES",
        Style::default()
            .fg(theme.text_dim)
            .add_modifier(Modifier::BOLD),
    ))];
    if state.debug.variables.is_empty() {
        variables.push(Line::from(Span::styled("no variables", theme.dim())));
    } else {
        for variable in &state.debug.variables {
            let indent = "  ".repeat(variable.depth);
            variables.push(Line::from(vec![
                Span::styled(
                    format!("{indent}{}", variable.name),
                    Style::default().fg(theme.syn_property),
                ),
                Span::raw(" "),
                Span::styled(
                    truncate(
                        &variable.value,
                        (columns[1].width as usize)
                            .saturating_sub(variable.name.len() + indent.len() + 2),
                    ),
                    Style::default().fg(theme.text),
                ),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(variables), columns[1]);
}

fn breakpoint_lines<'a>(state: &'a AppState, width: usize, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    for document in &state.documents {
        let Some(path) = &document.path else { continue };
        for line in &document.breakpoints {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled("● ", Style::default().fg(theme.danger)),
                Span::styled(
                    truncate(&format!("{name}:{}", line + 1), width.saturating_sub(2)),
                    Style::default().fg(theme.text),
                ),
            ]));
        }
    }
    lines
}
