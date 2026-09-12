//! Header, status bar and toast overlay.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::state::AppState;
use crate::domain::agent::AgentState;

use super::{truncate, LayoutMode, Theme};

/// Top row: product, workspace path, branch, and the palette hint.
pub fn draw_header(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let compact = area.width < 60;
    let brand = if compact {
        crate::PRODUCT_NAME_SHORT
    } else {
        crate::PRODUCT_NAME
    };

    let mut spans = vec![
        Span::styled(
            format!(" {brand} "),
            Style::default()
                .fg(theme.background)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            state.workspace.display_path(),
            Style::default().fg(theme.text_bright),
        ),
    ];

    if state.git_root().is_some() {
        let dirty = if state.git.is_dirty() { "*" } else { "" };
        spans.push(Span::styled("  ⎇ ", Style::default().fg(theme.accent)));
        spans.push(Span::styled(
            format!("{}{dirty}", state.git.head_label()),
            Style::default().fg(theme.text),
        ));
    }

    let hint = format!(
        "{} commands  ·  {} files ",
        state
            .keymap
            .hint_for("palette.commands")
            .unwrap_or_else(|| "—".into()),
        state
            .keymap
            .hint_for("palette.quick_open")
            .unwrap_or_else(|| "—".into())
    );

    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = (area.width as usize).saturating_sub(used + hint.chars().count());
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(hint, theme.dim()));

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.surface)),
        area,
    );
}

/// Bottom row: mode, focus, problems, git, agents, language service, cursor.
pub fn draw_status(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    mode: LayoutMode,
) {
    let mode_label = if state.prefix_active {
        "PREFIX"
    } else if state.modal.is_some() {
        "DIALOG"
    } else if state.palette.is_some() {
        "PALETTE"
    } else {
        "NORMAL"
    };
    let mode_style = Style::default()
        .fg(theme.background)
        .bg(if state.prefix_active {
            theme.warning
        } else {
            theme.accent
        })
        .add_modifier(Modifier::BOLD);

    let mut left = vec![
        Span::styled(format!(" {mode_label} "), mode_style),
        Span::styled(
            format!(" {} ", state.focus.label()),
            Style::default().fg(theme.text_dim),
        ),
    ];

    let (errors, warnings) = state.problem_counts();
    left.push(Span::styled(
        format!("✕{errors} "),
        Style::default().fg(if errors > 0 {
            theme.danger
        } else {
            theme.text_dim
        }),
    ));
    left.push(Span::styled(
        format!("!{warnings} "),
        Style::default().fg(if warnings > 0 {
            theme.warning
        } else {
            theme.text_dim
        }),
    ));

    if state.git_root().is_some() {
        left.push(Span::styled(
            format!("│ {} ", state.git.head_label()),
            Style::default().fg(theme.text_dim),
        ));
    }

    // Agent summary, with attention states called out.
    if !state.agents.is_empty() {
        let working = state.agents_working();
        let waiting = state.agents_needing_attention();
        let failed = state
            .agents
            .iter()
            .filter(|a| a.state == AgentState::Failed)
            .count();
        left.push(Span::styled("│ ", Style::default().fg(theme.border)));
        left.push(Span::styled(
            format!("●{working} "),
            Style::default().fg(theme.success),
        ));
        if waiting > 0 {
            left.push(Span::styled(
                format!("!{waiting} "),
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        if failed > 0 {
            left.push(Span::styled(
                format!("×{failed} "),
                Style::default().fg(theme.danger),
            ));
        }
    }

    let mut right: Vec<Span> = Vec::new();
    if let Some(document) = state.active_document() {
        let cursor = document.buffer.cursor();
        right.push(Span::styled(
            format!("Ln {}, Col {} ", cursor.line + 1, cursor.character + 1),
            Style::default().fg(theme.text_dim),
        ));
        right.push(Span::styled(
            format!("{} ", document.language.display_name()),
            Style::default().fg(theme.text),
        ));
        if document.read_only {
            right.push(Span::styled(
                "read-only ",
                Style::default().fg(theme.warning),
            ));
        }
    }
    right.push(Span::styled(
        format!("{} ", mode.label()),
        Style::default().fg(theme.text_dim),
    ));

    let used: usize = left
        .iter()
        .chain(right.iter())
        .map(|s| s.content.chars().count())
        .sum();
    let pad = (area.width as usize).saturating_sub(used);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(pad)));
    spans.extend(right);

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.surface)),
        area,
    );
}

/// Transient messages, stacked above the status bar on the right.
pub fn draw_toasts(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    if state.toasts.is_empty() {
        return;
    }
    let width = area.width.saturating_sub(4).min(60);
    if width < 12 {
        return;
    }
    let height = state.toasts.len() as u16 + 2;
    if area.height < height + 2 {
        return;
    }
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width + 2),
        y: area.y + area.height.saturating_sub(height + 1),
        width,
        height,
    };

    let lines: Vec<Line> = state
        .toasts
        .iter()
        .map(|toast| {
            Line::from(vec![
                Span::styled("● ", Style::default().fg(theme.notice(toast.level))),
                Span::styled(
                    truncate(&toast.text, width as usize - 4),
                    Style::default().fg(theme.text),
                ),
            ])
        })
        .collect();

    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.border))
                    .style(Style::default().bg(theme.surface)),
            )
            .style(Style::default().bg(theme.surface)),
        rect,
    );
}
