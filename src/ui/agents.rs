//! Agent dashboard and the agent detail panel.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::focus::FocusTarget;
use crate::app::state::{AgentDetailTab, AppState};
use crate::domain::agent::{format_duration, AgentSession};

use super::widgets::{field, panel, panel_with_hint, window};
use super::{truncate, Theme, WorkbenchLayout};

/// Draw the agents column: list on top, detail below.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    layout: &mut WorkbenchLayout,
) {
    let show_detail = area.height >= 18;
    let rows = if show_detail {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1)])
            .split(area)
    };

    draw_list(frame, rows[0], state, theme);
    layout.agents = Some(rows[0]);

    if show_detail {
        draw_detail(frame, rows[1], state, theme);
        layout.agent_detail = Some(rows[1]);
    }
}

fn draw_list(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let focused = state.focus == FocusTarget::AgentList;
    let hint = state
        .keymap
        .hint_for("agent.new.claude")
        .map(|chord| format!("{chord} new claude"))
        .unwrap_or_default();
    let block = panel_with_hint("AGENTS", hint, focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    if state.agents.is_empty() {
        let lines = vec![
            Line::from(Span::styled("no agent sessions", theme.dim())),
            Line::from(""),
            Line::from(Span::styled(
                format!(
                    "{}  start Claude",
                    state
                        .keymap
                        .hint_for("agent.new.claude")
                        .unwrap_or_else(|| "—".into())
                ),
                Style::default().fg(theme.text_dim),
            )),
            Line::from(Span::styled(
                format!(
                    "{}  start Codex",
                    state
                        .keymap
                        .hint_for("agent.new.codex")
                        .unwrap_or_else(|| "—".into())
                ),
                Style::default().fg(theme.text_dim),
            )),
        ];
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }

    // Each agent takes two rows: the status row and a summary row.
    let per_agent = 2usize;
    let visible = (inner.height as usize / per_agent).max(1);
    let offset = window(
        state.agent_selection.selected,
        state.agents.len(),
        visible,
        state.agent_selection.offset,
    );

    let mut lines: Vec<Line> = Vec::new();
    for (index, agent) in state.agents.iter().enumerate().skip(offset).take(visible) {
        let selected = index == state.agent_selection.selected;
        lines.push(agent_row(
            agent,
            inner.width as usize,
            selected,
            focused,
            theme,
        ));
        lines.push(agent_summary_row(agent, inner.width as usize, theme));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// `● Claude        Working      12m`
fn agent_row<'a>(
    agent: &'a AgentSession,
    width: usize,
    selected: bool,
    focused: bool,
    theme: &Theme,
) -> Line<'a> {
    let colour = theme.agent_state(agent.state);
    let duration = format_duration(agent.duration());
    let state_label = agent.state.label();

    let name_budget = width.saturating_sub(state_label.len() + duration.len() + 6);
    let name = truncate(&agent.label, name_budget.max(4));
    let pad = name_budget.saturating_sub(name.chars().count());

    let mut spans = vec![
        Span::styled(
            format!("{} ", agent.state.glyph()),
            Style::default().fg(colour),
        ),
        Span::styled(
            name,
            Style::default()
                .fg(theme.text_bright)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" ".repeat(pad + 1)),
        Span::styled(state_label, Style::default().fg(colour)),
        Span::raw(" "),
        Span::styled(duration, theme.dim()),
    ];
    if selected {
        spans = spans
            .into_iter()
            .map(|span| Span::styled(span.content, span.style.patch(theme.selected_row(focused))))
            .collect();
    }
    Line::from(spans)
}

/// Second row: a task summary when one is reliably known, else the command.
fn agent_summary_row<'a>(agent: &'a AgentSession, width: usize, theme: &Theme) -> Line<'a> {
    let text = match &agent.task_summary {
        Some(summary) => summary.clone(),
        None => agent.command.join(" "),
    };
    Line::from(Span::styled(
        format!("  {}", truncate(&text, width.saturating_sub(2))),
        theme.dim(),
    ))
}

fn draw_detail(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let focused = state.focus == FocusTarget::AgentDetail;
    let block = panel("AGENT", focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 2 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    // Tab bar.
    let mut spans = Vec::new();
    for tab in AgentDetailTab::ALL {
        let active = tab == state.agent_tab;
        let style = if active {
            Style::default()
                .fg(theme.text_bright)
                .bg(theme.surface_alt)
                .add_modifier(Modifier::BOLD)
        } else {
            theme.dim()
        };
        spans.push(Span::styled(format!(" {} ", tab.label()), style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), rows[0]);

    let Some(agent) = state.selected_agent() else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("no agent selected", theme.dim()))),
            rows[1],
        );
        return;
    };

    let lines = match state.agent_tab {
        AgentDetailTab::Status => status_lines(agent, state, theme),
        AgentDetailTab::Files => files_lines(state, theme),
        AgentDetailTab::Tasks => task_lines(agent, theme),
        AgentDetailTab::Output => output_lines(state, rows[1].width as usize, theme),
    };
    frame.render_widget(Paragraph::new(lines), rows[1]);
}

fn status_lines<'a>(agent: &'a AgentSession, state: &AppState, theme: &Theme) -> Vec<Line<'a>> {
    let terminal = agent
        .terminal_id
        .map(|id| format!("#{}", id.raw()))
        .unwrap_or_else(|| "none".into());
    // Provenance is shown so a heuristic state is never mistaken for fact.
    let source = format!(
        "{:?} · confidence {:.0}%",
        agent.state_source,
        agent.confidence * 100.0
    );
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{} ", agent.state.glyph()),
                Style::default().fg(theme.agent_state(agent.state)),
            ),
            Span::styled(
                agent.state.label(),
                Style::default()
                    .fg(theme.agent_state(agent.state))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}", format_duration(agent.duration())),
                theme.dim(),
            ),
        ]),
        field("state from", source, theme),
        field("type", agent.kind.label().to_string(), theme),
        field("command", agent.command.join(" "), theme),
        field(
            "cwd",
            crate::services::workspace::shorten_home(&agent.cwd),
            theme,
        ),
        field("backend", agent.backend.label().to_string(), theme),
        field("terminal", terminal, theme),
        field(
            "last output",
            format!("{} ago", format_duration(agent.idle_for())),
            theme,
        ),
    ];
    if let Some(summary) = &agent.task_summary {
        lines.push(field("task", summary.clone(), theme));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "{} focus terminal · {} send text",
            state
                .keymap
                .hint_for("agent.focus_terminal")
                .unwrap_or_else(|| "Enter".into()),
            state
                .keymap
                .hint_for("agent.send_input")
                .unwrap_or_else(|| "i".into())
        ),
        theme.dim(),
    )));
    lines
}

fn files_lines<'a>(state: &'a AppState, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines = vec![Line::from(Span::styled(
        "repository changes since this session started",
        theme.dim(),
    ))];
    if state.agent_files.is_empty() {
        lines.push(Line::from(Span::styled("no new changes", theme.dim())));
    } else {
        for path in &state.agent_files {
            lines.push(Line::from(Span::styled(
                format!("  {}", path.display()),
                Style::default().fg(theme.text),
            )));
        }
    }
    lines.push(Line::from(""));
    // Attribution honesty: we know what changed, not who changed it.
    lines.push(Line::from(Span::styled(
        "changes are not attributed to a specific agent",
        theme.dim(),
    )));
    lines
}

fn task_lines<'a>(agent: &'a AgentSession, theme: &Theme) -> Vec<Line<'a>> {
    if agent.tasks.is_empty() {
        return vec![
            Line::from(Span::styled("no tasks yet", theme.dim())),
            Line::from(""),
            Line::from(Span::styled(
                "tasks are your own checklist for this session",
                theme.dim(),
            )),
        ];
    }
    agent
        .tasks
        .iter()
        .map(|task| {
            let (glyph, style) = if task.done {
                ("[x]", Style::default().fg(theme.success))
            } else {
                ("[ ]", Style::default().fg(theme.text))
            };
            Line::from(vec![
                Span::styled(format!("{glyph} "), style),
                Span::styled(task.text.clone(), Style::default().fg(theme.text)),
            ])
        })
        .collect()
}

fn output_lines<'a>(state: &'a AppState, width: usize, theme: &Theme) -> Vec<Line<'a>> {
    if state.agent_output.is_empty() {
        return vec![Line::from(Span::styled("no output captured", theme.dim()))];
    }
    state
        .agent_output
        .iter()
        .rev()
        .take(200)
        .rev()
        .map(|line| {
            Line::from(Span::styled(
                truncate(line, width),
                Style::default().fg(theme.text_dim),
            ))
        })
        .collect()
}
