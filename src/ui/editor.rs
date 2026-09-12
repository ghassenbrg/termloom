//! Editor: tab strip, gutter, highlighted text, find bar and diff view.

use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::focus::FocusTarget;
use crate::app::state::AppState;
use crate::domain::git::DiffLine;
use crate::editor::Document;

use super::widgets::panel;
use super::{truncate, Theme, WorkbenchLayout};

/// Draw the centre column.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    layout: &mut WorkbenchLayout,
) {
    let focused = state.focus.is_editor();
    let block = panel("EDITOR", focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // A diff opened from the Git panel takes over the editor area.
    if let Some(diff) = &state.diff {
        layout.editor = Some(inner);
        draw_diff(frame, inner, state, diff, theme);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    layout.editor_tabs = Some(rows[0]);
    draw_tabs(frame, rows[0], state, theme);

    let Some(document) = state.active_document() else {
        layout.editor = Some(rows[1]);
        draw_welcome(frame, rows[1], state, theme);
        return;
    };

    let show_find = document.search.is_active()
        || matches!(state.focus, FocusTarget::Editor(_)) && !document.search.query.is_empty();
    let body = if show_find {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(rows[1]);
        draw_find_bar(frame, split[1], document, theme);
        split[0]
    } else {
        rows[1]
    };

    layout.editor = Some(body);
    draw_buffer(frame, body, state, document, theme, focused);
}

/// Compact tab strip: `[● main.rs] [app.dart]`.
fn draw_tabs(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    if state.documents.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(" no open editors", theme.dim()))),
            area,
        );
        return;
    }

    let mut spans: Vec<Span> = Vec::new();
    let mut used = 0usize;
    let budget = area.width as usize;
    for document in &state.documents {
        let active = state.active_tab == Some(document.id);
        let name = document.display_name();
        let marker = if document.is_dirty() { "●" } else { " " };
        let label = format!(" {name} {marker}");
        let width = label.chars().count() + 1;
        if used + width > budget {
            spans.push(Span::styled("…", theme.dim()));
            break;
        }
        used += width;
        let style = if active {
            Style::default()
                .fg(theme.text_bright)
                .bg(theme.surface_alt)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_dim)
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::styled("│", Style::default().fg(theme.border)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Shown when no file is open: the product identity plus the keys that matter.
fn draw_welcome(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let keymap = &state.keymap;
    let hint = |command: &str, text: &str| -> Line<'static> {
        let chord = keymap
            .hint_for(command)
            .unwrap_or_else(|| "unbound".to_string());
        Line::from(vec![
            Span::styled(format!("  {chord:<16}"), Style::default().fg(theme.accent)),
            Span::styled(text.to_string(), Style::default().fg(theme.text_dim)),
        ])
    };

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", crate::PRODUCT_NAME),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(format!("  {}", crate::TAGLINE), theme.dim())),
        Line::from(""),
        hint("palette.quick_open", "open a file"),
        hint("palette.commands", "command palette"),
        hint("view.focus.explorer", "explorer"),
        hint("agent.new.claude", "new Claude session"),
        hint("agent.new.codex", "new Codex session"),
        hint("terminal.new", "new terminal"),
        hint("help.toggle", "all keyboard shortcuts"),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

/// The text area: gutter + highlighted source.
fn draw_buffer(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    document: &Document,
    theme: &Theme,
    focused: bool,
) {
    let total_lines = document.buffer.line_count();
    let gutter_width = if state.config.editor.line_numbers {
        (total_lines.to_string().len() as u16).max(2) + 1
    } else {
        0
    };
    // One column for breakpoints/diagnostics markers.
    let marker_width = 1u16;
    let text_x = area.x + gutter_width + marker_width;
    let text_width = area.width.saturating_sub(gutter_width + marker_width) as usize;
    if text_width == 0 {
        return;
    }

    let height = area.height as usize;
    let first = document.scroll.min(total_lines.saturating_sub(1));
    let cursor = document.buffer.cursor();
    let selection = document.buffer.selection();

    let mut lines: Vec<Line> = Vec::with_capacity(height);
    for offset in 0..height {
        let line_index = first + offset;
        if line_index >= total_lines {
            lines.push(Line::from(""));
            continue;
        }
        let is_cursor_line = line_index == cursor.line;
        let mut spans: Vec<Span> = Vec::new();

        if gutter_width > 0 {
            let number = format!(
                "{:>width$} ",
                line_index + 1,
                width = gutter_width as usize - 1
            );
            spans.push(Span::styled(
                number,
                Style::default().fg(if is_cursor_line {
                    theme.line_number_active
                } else {
                    theme.line_number
                }),
            ));
        }

        // Marker column: breakpoint wins over diagnostics.
        let marker = if document.breakpoints.contains(&line_index) {
            Span::styled("●", Style::default().fg(theme.danger))
        } else if let Some(diagnostic) = document
            .diagnostics_on(line_index)
            .min_by_key(|d| d.severity)
        {
            Span::styled(
                diagnostic.severity.glyph(),
                Style::default().fg(theme.severity(diagnostic.severity)),
            )
        } else {
            Span::raw(" ")
        };
        spans.push(marker);

        spans.extend(render_line(
            document,
            line_index,
            document.h_scroll,
            text_width,
            theme,
        ));

        let mut line = Line::from(spans);
        if is_cursor_line && focused {
            line = line.style(Style::default().bg(theme.cursor_line));
        }
        lines.push(line);
    }

    frame.render_widget(Paragraph::new(lines), area);
    let _ = selection;

    if focused {
        let x = text_x + cursor.character.saturating_sub(document.h_scroll) as u16;
        let y = area.y + cursor.line.saturating_sub(first) as u16;
        if x < area.x + area.width && y < area.y + area.height {
            frame.set_cursor_position(Position::new(x, y));
        }
    }
}

/// Style one visible line: syntax, selection and search hits.
fn render_line<'a>(
    document: &'a Document,
    line_index: usize,
    h_scroll: usize,
    width: usize,
    theme: &Theme,
) -> Vec<Span<'a>> {
    let text = document.buffer.line(line_index);
    let chars: Vec<char> = text.chars().skip(h_scroll).take(width).collect();
    if chars.is_empty() {
        return vec![Span::raw("")];
    }

    // Per-character styles, then compressed into runs.
    let mut styles: Vec<Style> = vec![Style::default().fg(theme.text); chars.len()];

    if let Some(syntax) = &document.syntax {
        for span in syntax.line(line_index) {
            let start = span.start.saturating_sub(h_scroll);
            let end = span.end.saturating_sub(h_scroll).min(chars.len());
            for style in styles.iter_mut().take(end).skip(start) {
                *style = style.fg(theme.highlight(span.kind));
            }
        }
    }

    for hit in &document.search.matches {
        if hit.start.line != line_index {
            continue;
        }
        let start = hit.start.character.saturating_sub(h_scroll);
        let end = hit.end.character.saturating_sub(h_scroll).min(chars.len());
        let current = document.search.current_match() == Some(*hit);
        let background = if current {
            theme.accent
        } else {
            theme.surface_alt
        };
        for style in styles.iter_mut().take(end).skip(start) {
            *style = style.bg(background).fg(if current {
                theme.background
            } else {
                theme.text_bright
            });
        }
    }

    if let Some(selection) = document.buffer.selection() {
        if line_index >= selection.start.line && line_index <= selection.end.line {
            let start = if line_index == selection.start.line {
                selection.start.character
            } else {
                0
            };
            let end = if line_index == selection.end.line {
                selection.end.character
            } else {
                chars.len() + h_scroll
            };
            let start = start.saturating_sub(h_scroll);
            let end = end.saturating_sub(h_scroll).min(chars.len());
            for style in styles.iter_mut().take(end).skip(start) {
                *style = style.bg(theme.selection);
            }
        }
    }

    let mut spans = Vec::new();
    let mut run = String::new();
    let mut run_style = styles[0];
    for (index, ch) in chars.iter().enumerate() {
        if styles[index] != run_style {
            spans.push(Span::styled(std::mem::take(&mut run), run_style));
            run_style = styles[index];
        }
        run.push(*ch);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, run_style));
    }
    spans
}

fn draw_find_bar(frame: &mut Frame, area: Rect, document: &Document, theme: &Theme) {
    let spans = vec![
        Span::styled(
            " find ",
            Style::default().fg(theme.background).bg(theme.accent),
        ),
        Span::raw(" "),
        Span::styled(
            document.search.query.clone(),
            Style::default().fg(theme.text_bright),
        ),
        Span::raw("  "),
        Span::styled(document.search.counter(), theme.dim()),
        Span::styled(
            "   Enter next · Shift+Enter previous · Esc close",
            theme.dim(),
        ),
    ];
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Unified diff view, opened from the Git panel.
fn draw_diff(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    diff: &crate::domain::git::FileDiff,
    theme: &Theme,
) {
    let title = format!(
        " diff {}{} ",
        diff.path.display(),
        if diff.staged { " (staged)" } else { "" }
    );
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(
            Line::from(Span::styled(
                " Esc close · s stage · u unstage ",
                theme.dim(),
            ))
            .right_aligned(),
        );
    let inner = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };
    frame.render_widget(block, area);

    if diff.binary {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("binary file", theme.dim()))),
            inner,
        );
        return;
    }
    if diff.lines.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("no changes", theme.dim()))),
            inner,
        );
        return;
    }

    let height = inner.height as usize;
    let offset = state.diff_scroll.min(diff.lines.len().saturating_sub(1));
    let lines: Vec<Line> = diff
        .lines
        .iter()
        .skip(offset)
        .take(height)
        .map(|line| {
            let (prefix, style) = match line {
                DiffLine::Added(_) => ("+", Style::default().fg(theme.success)),
                DiffLine::Removed(_) => ("-", Style::default().fg(theme.danger)),
                DiffLine::Hunk(_) => ("", Style::default().fg(theme.accent_alt)),
                DiffLine::Meta(_) => ("", theme.dim()),
                DiffLine::Context(_) => (" ", Style::default().fg(theme.text)),
            };
            let content = truncate(line.text(), inner.width as usize);
            Line::from(vec![Span::styled(format!("{prefix}{content}"), style)])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}
