//! Input routing.
//!
//! Routing order matters and is deliberately explicit:
//!
//! 1. a modal dialog owns everything,
//! 2. then the command palette,
//! 3. then the command prefix (`Ctrl+Space`),
//! 4. then a focused terminal — which gets *everything* else, so child
//!    programs such as Claude Code, vim or a shell keep their keys,
//! 5. then the focused panel's own bindings and navigation.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::config::keys::Scope;
use crate::domain::diagnostics::Position;

use super::actions;
use super::focus::FocusTarget;
use super::services::Services;
use super::state::PromptPurpose;
use super::state::{AgentDetailTab, AppState, Modal, PaletteItem, PaletteMode};

/// Handle one key event.
pub fn handle_key(state: &mut AppState, services: &mut Services, key: KeyEvent) {
    // 1. Modal dialogs.
    if state.modal.is_some() {
        handle_modal_key(state, services, key);
        return;
    }

    // 2. Palette.
    if state.palette.is_some() {
        handle_palette_key(state, services, key);
        return;
    }

    // 3. Prefix mode.
    if state.prefix_active {
        state.prefix_active = false;
        if key.code == KeyCode::Esc {
            return;
        }
        if let Some(command) = state
            .keymap
            .resolve(Scope::Prefix, &key)
            .map(str::to_string)
        {
            actions::execute(state, services, &command);
        } else {
            state.info(format!("no command bound to {}", key_label(&key)));
        }
        return;
    }
    if state.keymap.prefix.matches(&key) {
        state.prefix_active = true;
        return;
    }

    // The help overlay swallows the next key.
    if state.help_visible {
        state.help_visible = false;
        if matches!(key.code, KeyCode::Esc | KeyCode::F(1)) {
            return;
        }
    }

    // The extensions overlay behaves the same way as a panel.
    if state.layout.extensions && handle_extensions_key(state, key) {
        return;
    }

    // 4. A focused terminal owns the keyboard.
    if let FocusTarget::Terminal(id) = state.focus {
        // Only the global scope (prefix, quick open, quit, F-keys) is stolen.
        if let Some(command) = state
            .keymap
            .resolve(Scope::Global, &key)
            .map(str::to_string)
        {
            actions::execute(state, services, &command);
            return;
        }
        forward_to_terminal(state, id, key);
        return;
    }

    // 5. Focus-specific handling.
    let handled = match state.focus {
        FocusTarget::Editor(_) => handle_editor_key(state, services, key),
        FocusTarget::Explorer => handle_explorer_key(state, services, key),
        FocusTarget::Outline => handle_outline_key(state, key),
        FocusTarget::Git => handle_git_key(state, services, key),
        FocusTarget::AgentList => handle_agent_key(state, services, key),
        FocusTarget::AgentDetail => handle_agent_detail_key(state, services, key),
        FocusTarget::Problems => handle_problems_key(state, key),
        FocusTarget::Debug => handle_debug_key(state, services, key),
        _ => false,
    };
    if handled {
        return;
    }

    // 6. Scoped and global bindings.
    let scope = if state.focus.is_editor() {
        Scope::Editor
    } else {
        Scope::Panel
    };
    if let Some(command) = state.keymap.resolve(scope, &key).map(str::to_string) {
        actions::execute(state, services, &command);
        return;
    }
    if let Some(command) = state
        .keymap
        .resolve(Scope::Global, &key)
        .map(str::to_string)
    {
        actions::execute(state, services, &command);
    }
}

fn key_label(key: &KeyEvent) -> String {
    crate::config::KeyChord::new(key.code, key.modifiers).to_string()
}

// ── modal ─────────────────────────────────────────────────────────────────

fn handle_modal_key(state: &mut AppState, services: &mut Services, key: KeyEvent) {
    let Some(modal) = state.modal.clone() else {
        return;
    };
    match modal {
        Modal::Confirm { action, .. } => match key.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                state.close_modal();
                actions::apply_confirm(state, services, action);
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => state.close_modal(),
            _ => {}
        },
        Modal::Prompt { purpose, value, .. } => match key.code {
            KeyCode::Enter => {
                state.close_modal();
                actions::apply_prompt(state, services, purpose, value);
            }
            KeyCode::Esc => state.close_modal(),
            KeyCode::Backspace => {
                if let Some(Modal::Prompt { value, .. }) = state.modal.as_mut() {
                    value.pop();
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(Modal::Prompt { value, .. }) = state.modal.as_mut() {
                    value.push(c);
                }
            }
            _ => {}
        },
        Modal::Message { .. } => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                state.close_modal();
            }
        }
        Modal::Select {
            options, purpose, ..
        } => match key.code {
            KeyCode::Esc => state.close_modal(),
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(Modal::Select { selected, .. }) = state.modal.as_mut() {
                    *selected = selected.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(Modal::Select { selected, .. }) = state.modal.as_mut() {
                    *selected = (*selected + 1).min(options.len().saturating_sub(1));
                }
            }
            KeyCode::Enter => {
                let index = match state.modal.as_ref() {
                    Some(Modal::Select { selected, .. }) => *selected,
                    _ => 0,
                };
                state.close_modal();
                actions::apply_selection(state, services, purpose, index);
            }
            _ => {}
        },
    }
}

// ── palette ───────────────────────────────────────────────────────────────

fn handle_palette_key(state: &mut AppState, services: &mut Services, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => state.close_palette(),
        KeyCode::Up => {
            if let Some(palette) = state.palette.as_mut() {
                palette.selected = palette.selected.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if let Some(palette) = state.palette.as_mut() {
                let max = palette.items.len().saturating_sub(1);
                palette.selected = (palette.selected + 1).min(max);
            }
        }
        KeyCode::Tab => {
            // Toggle between file and command modes.
            if let Some(palette) = state.palette.as_mut() {
                palette.mode = match palette.mode {
                    PaletteMode::Commands => PaletteMode::Files,
                    PaletteMode::Files => PaletteMode::Commands,
                };
                palette.selected = 0;
            }
            refresh_palette(state);
        }
        KeyCode::Enter => {
            let selection = state.palette_selection();
            state.close_palette();
            match selection {
                Some(PaletteItem::Command(command)) => {
                    actions::execute(state, services, command.id)
                }
                Some(PaletteItem::File(relative)) => {
                    let path = state.workspace.root.join(relative);
                    if let Err(err) = state.open_file(&path) {
                        state.error(format!("{err:#}"));
                    }
                }
                None => {}
            }
        }
        KeyCode::Backspace => {
            if let Some(palette) = state.palette.as_mut() {
                palette.query.pop();
            }
            refresh_palette(state);
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(palette) = state.palette.as_mut() {
                palette.query.push(c);
                palette.selected = 0;
            }
            refresh_palette(state);
        }
        _ => {}
    }
}

fn refresh_palette(state: &mut AppState) {
    let Some(mut palette) = state.palette.take() else {
        return;
    };
    state.refresh_palette_items(&mut palette);
    state.palette = Some(palette);
}

// ── terminal ──────────────────────────────────────────────────────────────

fn forward_to_terminal(state: &mut AppState, id: crate::domain::ids::TerminalId, key: KeyEvent) {
    // Scrollback navigation is ours; everything else is the child's.
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    if shift && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
        let delta = if key.code == KeyCode::PageUp { 10 } else { -10 };
        if let Ok(mut terminals) = state.terminals.lock() {
            if let Some(session) = terminals.get_mut(id) {
                session.scroll(delta);
            }
        }
        return;
    }

    let Some(bytes) = crate::services::terminal::input::encode_key(&key) else {
        return;
    };
    let mut error = None;
    if let Ok(mut terminals) = state.terminals.lock() {
        match terminals.get_mut(id) {
            Some(session) if session.is_running() => {
                if let Err(err) = session.write(&bytes) {
                    error = Some(format!("{err:#}"));
                }
            }
            Some(_) => {
                error = Some("this terminal has exited — restart it to keep using it".to_string())
            }
            None => {}
        }
    }
    if let Some(error) = error {
        state.warn(error);
    }
}

// ── editor ────────────────────────────────────────────────────────────────

fn handle_editor_key(state: &mut AppState, services: &mut Services, key: KeyEvent) -> bool {
    // A completion popup grabs navigation keys while it is open.
    if state.completion.is_some() && handle_completion_key(state, key) {
        return true;
    }
    // Any key other than Esc dismisses hover; Esc closes it explicitly.
    if state.hover.is_some() {
        state.hover = None;
        if key.code == KeyCode::Esc {
            return true;
        }
    }
    // A diff view takes over the editor area.
    if state.diff.is_some() {
        match key.code {
            KeyCode::Esc => {
                state.diff = None;
                return true;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.diff_scroll = state.diff_scroll.saturating_sub(1);
                return true;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.diff_scroll += 1;
                return true;
            }
            KeyCode::PageUp => {
                state.diff_scroll = state.diff_scroll.saturating_sub(20);
                return true;
            }
            KeyCode::PageDown => {
                state.diff_scroll += 20;
                return true;
            }
            _ => return false,
        }
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let tab_width = state.config.editor.tab_width;
    let insert_spaces = state.config.editor.insert_spaces;

    let Some(document) = state.active_document_mut() else {
        return false;
    };

    match key.code {
        KeyCode::Char(c) if !ctrl && !alt => {
            document.buffer.insert_char(c);
            true
        }
        KeyCode::Enter if !ctrl => {
            document.buffer.insert_newline(true);
            true
        }
        KeyCode::Backspace => {
            document.buffer.delete_backward();
            true
        }
        KeyCode::Delete => {
            document.buffer.delete_forward();
            true
        }
        KeyCode::Tab => {
            document.buffer.indent(tab_width, insert_spaces);
            true
        }
        KeyCode::BackTab => {
            document.buffer.dedent(tab_width);
            true
        }
        KeyCode::Left if ctrl => {
            document.buffer.move_word(false, shift);
            true
        }
        KeyCode::Right if ctrl => {
            document.buffer.move_word(true, shift);
            true
        }
        KeyCode::Left => {
            document.buffer.move_left(shift);
            true
        }
        KeyCode::Right => {
            document.buffer.move_right(shift);
            true
        }
        KeyCode::Up => {
            document.buffer.move_vertical(-1, shift);
            true
        }
        KeyCode::Down => {
            document.buffer.move_vertical(1, shift);
            true
        }
        KeyCode::Home if ctrl => {
            document.buffer.move_document_start(shift);
            true
        }
        KeyCode::End if ctrl => {
            document.buffer.move_document_end(shift);
            true
        }
        KeyCode::Home => {
            document.buffer.move_line_start(shift);
            true
        }
        KeyCode::End => {
            document.buffer.move_line_end(shift);
            true
        }
        KeyCode::PageUp => {
            document.buffer.move_vertical(-20, shift);
            true
        }
        KeyCode::PageDown => {
            document.buffer.move_vertical(20, shift);
            true
        }
        KeyCode::Esc => {
            if document.buffer.has_selection() {
                document.buffer.clear_selection();
            } else if document.search.is_active() {
                document.search.clear();
            } else {
                state.focus = FocusTarget::Explorer;
            }
            true
        }
        KeyCode::F(3) => {
            actions::execute(state, services, "editor.find_next");
            true
        }
        _ => false,
    }
}

/// Keys handled while the completion popup is open.
fn handle_completion_key(state: &mut AppState, key: KeyEvent) -> bool {
    let Some(completion) = state.completion.as_mut() else {
        return false;
    };
    let count = completion.filtered().len();
    match key.code {
        KeyCode::Up => {
            completion.selected = completion.selected.saturating_sub(1);
            true
        }
        KeyCode::Down => {
            if count > 0 {
                completion.selected = (completion.selected + 1).min(count - 1);
            }
            true
        }
        KeyCode::Esc => {
            state.completion = None;
            true
        }
        KeyCode::Enter | KeyCode::Tab => {
            accept_completion(state);
            true
        }
        KeyCode::Backspace => {
            // Let the editor delete, then keep the popup in sync.
            if let Some(document) = state.active_document_mut() {
                document.buffer.delete_backward();
            }
            update_completion_prefix(state);
            true
        }
        KeyCode::Char(c)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && (c.is_alphanumeric() || c == '_') =>
        {
            if let Some(document) = state.active_document_mut() {
                document.buffer.insert_char(c);
            }
            update_completion_prefix(state);
            true
        }
        _ => {
            // Anything else (space, punctuation, movement) closes the popup
            // and is handled normally by the editor.
            state.completion = None;
            false
        }
    }
}

/// Re-read the word under the cursor after typing or deleting.
fn update_completion_prefix(state: &mut AppState) {
    let prefix = state
        .active_document()
        .map(|document| {
            let cursor = document.buffer.cursor();
            document.buffer.word_at(cursor).unwrap_or_default()
        })
        .unwrap_or_default();
    let close = match state.completion.as_mut() {
        Some(completion) => {
            completion.prefix = prefix;
            completion.selected = 0;
            completion.filtered().is_empty()
        }
        None => false,
    };
    if close {
        state.completion = None;
    }
}

/// Replace the typed prefix with the selected item.
fn accept_completion(state: &mut AppState) {
    let Some(completion) = state.completion.take() else {
        return;
    };
    let Some(item) = completion.selection() else {
        return;
    };
    let Some(document) = state.active_document_mut() else {
        return;
    };
    let cursor = document.buffer.cursor();
    let prefix_len = completion.prefix.chars().count();
    let start = crate::domain::diagnostics::Position::new(
        cursor.line,
        cursor.character.saturating_sub(prefix_len),
    );
    document.buffer.replace_range(
        crate::domain::diagnostics::Range::new(start, cursor),
        &item.insert_text,
    );
}

// ── explorer ──────────────────────────────────────────────────────────────

fn handle_explorer_key(state: &mut AppState, services: &mut Services, key: KeyEvent) -> bool {
    let rows = state.tree.rows();
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.explorer.move_by(-1, rows.len());
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.explorer.move_by(1, rows.len());
            true
        }
        KeyCode::PageUp => {
            state.explorer.move_by(-10, rows.len());
            true
        }
        KeyCode::PageDown => {
            state.explorer.move_by(10, rows.len());
            true
        }
        KeyCode::Home => {
            state.explorer.selected = 0;
            true
        }
        KeyCode::End => {
            state.explorer.selected = rows.len().saturating_sub(1);
            true
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            let Some(row) = rows.get(state.explorer.selected).cloned() else {
                return true;
            };
            if row.is_dir {
                state.tree.toggle(&row.path);
            } else if let Err(err) = state.open_file(&row.path) {
                state.error(format!("{err:#}"));
            }
            true
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if let Some(row) = rows.get(state.explorer.selected) {
                if row.is_dir && !row.expanded {
                    let path = row.path.clone();
                    state.tree.expand(&path);
                } else {
                    state.explorer.move_by(1, rows.len());
                }
            }
            true
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if let Some(row) = rows.get(state.explorer.selected).cloned() {
                if row.is_dir && row.expanded {
                    state.tree.collapse(&row.path);
                } else if let Some(parent) = row.path.parent() {
                    // Jump to the containing folder.
                    if let Some(index) = rows.iter().position(|r| r.path == parent) {
                        state.explorer.selected = index;
                    }
                }
            }
            true
        }
        KeyCode::Char('a') => {
            actions::execute(state, services, "file.new");
            true
        }
        KeyCode::Char('A') => {
            actions::execute(state, services, "file.new_folder");
            true
        }
        KeyCode::Char('r') => {
            actions::execute(state, services, "file.rename");
            true
        }
        KeyCode::Char('d') => {
            actions::execute(state, services, "file.delete");
            true
        }
        KeyCode::Char('R') => {
            state.refresh_tree();
            true
        }
        _ => false,
    }
}

// ── outline ───────────────────────────────────────────────────────────────

fn handle_outline_key(state: &mut AppState, key: KeyEvent) -> bool {
    let len = state
        .active_document()
        .map(|d| d.symbols.len())
        .unwrap_or(0);
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.outline_selection.move_by(-1, len);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.outline_selection.move_by(1, len);
            true
        }
        KeyCode::Enter => {
            let index = state.outline_selection.selected;
            let position = state
                .active_document()
                .and_then(|d| d.symbols.get(index))
                .map(|symbol| symbol.range.start);
            if let (Some(position), Some(id)) = (position, state.active_tab) {
                if let Some(document) = state.document_mut(id) {
                    document.goto(position);
                }
                state.focus = FocusTarget::Editor(id);
            }
            true
        }
        _ => false,
    }
}

// ── git ───────────────────────────────────────────────────────────────────

fn handle_git_key(state: &mut AppState, services: &mut Services, key: KeyEvent) -> bool {
    let len = state.git.changes.len();
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.git_selection.move_by(-1, len);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.git_selection.move_by(1, len);
            true
        }
        KeyCode::Enter => {
            actions::execute(state, services, "git.open_diff");
            true
        }
        KeyCode::Char('o') => {
            actions::execute(state, services, "git.open_file");
            true
        }
        KeyCode::Char('s') => {
            actions::execute(state, services, "git.stage");
            true
        }
        KeyCode::Char('u') => {
            actions::execute(state, services, "git.unstage");
            true
        }
        KeyCode::Char('x') => {
            actions::execute(state, services, "git.discard");
            true
        }
        KeyCode::Char('R') => {
            actions::execute(state, services, "git.refresh");
            true
        }
        _ => false,
    }
}

// ── agents ────────────────────────────────────────────────────────────────

fn handle_agent_key(state: &mut AppState, services: &mut Services, key: KeyEvent) -> bool {
    let len = state.agents.len();
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.agent_selection.move_by(-1, len);
            actions::refresh_agent_files(state);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.agent_selection.move_by(1, len);
            actions::refresh_agent_files(state);
            true
        }
        KeyCode::Enter => {
            actions::execute(state, services, "agent.focus_terminal");
            true
        }
        KeyCode::Tab => {
            state.focus = FocusTarget::AgentDetail;
            true
        }
        KeyCode::Char('i') => {
            actions::execute(state, services, "agent.send_input");
            true
        }
        KeyCode::Char('s') => {
            actions::execute(state, services, "agent.stop");
            true
        }
        KeyCode::Char('r') => {
            actions::execute(state, services, "agent.restart");
            true
        }
        KeyCode::Char('n') => {
            actions::execute(state, services, "agent.rename");
            true
        }
        KeyCode::Char('x') => {
            actions::execute(state, services, "agent.remove");
            true
        }
        KeyCode::Char('c') => {
            actions::execute(state, services, "agent.new.claude");
            true
        }
        KeyCode::Char('t') => {
            actions::execute(state, services, "agent.task.add");
            true
        }
        _ => false,
    }
}

fn handle_agent_detail_key(state: &mut AppState, services: &mut Services, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Tab | KeyCode::Right => {
            state.agent_tab = state.agent_tab.next();
            true
        }
        KeyCode::Esc | KeyCode::Left => {
            state.focus = FocusTarget::AgentList;
            true
        }
        KeyCode::Char('t') if state.agent_tab == AgentDetailTab::Tasks => {
            actions::execute(state, services, "agent.task.add");
            true
        }
        KeyCode::Char(' ') if state.agent_tab == AgentDetailTab::Tasks => {
            // Toggle the first unfinished task; the tasks list is short.
            if let Some(agent) = state.agents.get_mut(state.agent_selection.selected) {
                if let Some(task) = agent.tasks.iter_mut().find(|t| !t.done) {
                    task.done = true;
                } else if let Some(task) = agent.tasks.last_mut() {
                    task.done = false;
                }
                state.agent_tasks.insert(agent.id, agent.tasks.clone());
            }
            true
        }
        KeyCode::Enter => {
            actions::execute(state, services, "agent.focus_terminal");
            true
        }
        _ => false,
    }
}

// ── problems / debug ──────────────────────────────────────────────────────

fn handle_problems_key(state: &mut AppState, key: KeyEvent) -> bool {
    if state.references.is_some() {
        return handle_references_key(state, key);
    }
    let len = state.problems.len();
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.problems_selection.move_by(-1, len);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.problems_selection.move_by(1, len);
            true
        }
        KeyCode::Enter => {
            let Some(diagnostic) = state
                .problems
                .get(state.problems_selection.selected)
                .cloned()
            else {
                return true;
            };
            match state.open_file(&diagnostic.path) {
                Ok(id) => {
                    if let Some(document) = state.document_mut(id) {
                        document.goto(Position::new(
                            diagnostic.range.start.line,
                            diagnostic.range.start.character,
                        ));
                    }
                }
                Err(err) => state.error(format!("{err:#}")),
            }
            true
        }
        KeyCode::Esc => {
            state.layout.problems = false;
            state.focus = match state.active_tab {
                Some(id) => FocusTarget::Editor(id),
                None => FocusTarget::Explorer,
            };
            true
        }
        _ => false,
    }
}

fn handle_references_key(state: &mut AppState, key: KeyEvent) -> bool {
    let len = state
        .references
        .as_ref()
        .map(|r| r.locations.len())
        .unwrap_or(0);
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(references) = state.references.as_mut() {
                references.selected = references.selected.saturating_sub(1);
            }
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(references) = state.references.as_mut() {
                if len > 0 {
                    references.selected = (references.selected + 1).min(len - 1);
                }
            }
            true
        }
        KeyCode::Enter => {
            let location = state
                .references
                .as_ref()
                .and_then(|r| r.locations.get(r.selected).cloned());
            if let Some(location) = location {
                match state.open_file(&location.path) {
                    Ok(id) => {
                        if let Some(document) = state.document_mut(id) {
                            document.goto(location.range.start);
                        }
                    }
                    Err(err) => state.error(format!("{err:#}")),
                }
            }
            true
        }
        KeyCode::Esc => {
            state.references = None;
            true
        }
        _ => false,
    }
}

fn handle_debug_key(state: &mut AppState, services: &mut Services, key: KeyEvent) -> bool {
    let len = state.debug.frames.len();
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.debug.selected_frame = state.debug.selected_frame.saturating_sub(1);
            select_debug_frame(state, services);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if len > 0 {
                state.debug.selected_frame = (state.debug.selected_frame + 1).min(len - 1);
            }
            select_debug_frame(state, services);
            true
        }
        KeyCode::Enter => {
            select_debug_frame(state, services);
            true
        }
        KeyCode::Char('e') => {
            actions::execute(state, services, "debug.evaluate");
            true
        }
        KeyCode::Esc => {
            state.layout.debug = false;
            state.focus = match state.active_tab {
                Some(id) => FocusTarget::Editor(id),
                None => FocusTarget::Explorer,
            };
            true
        }
        _ => false,
    }
}

/// Ask the adapter for the selected frame's variables and show its source.
fn select_debug_frame(state: &mut AppState, services: &mut Services) {
    let Some(frame) = state.debug.frames.get(state.debug.selected_frame).cloned() else {
        return;
    };
    if let Some(session) = services.debug.as_ref() {
        let _ = session.send(crate::services::dap::DebugCommand::SelectFrame(frame.id));
    }
    let Some(path) = frame.path.clone() else {
        return;
    };
    if let Ok(id) = state.open_file(&path) {
        let line = frame.line.saturating_sub(1);
        if let Some(document) = state.document_mut(id) {
            document.goto(crate::domain::diagnostics::Position::new(line, 0));
        }
    }
}

fn handle_extensions_key(state: &mut AppState, key: KeyEvent) -> bool {
    let len = state.extensions.installed.len();
    match key.code {
        KeyCode::Esc => {
            state.layout.extensions = false;
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.extensions.selected = state.extensions.selected.saturating_sub(1);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if len > 0 {
                state.extensions.selected = (state.extensions.selected + 1).min(len - 1);
            }
            true
        }
        KeyCode::Char('i') => {
            state.prompt("Install VSIX", "", PromptPurpose::InstallVsix);
            true
        }
        KeyCode::Char('x') => {
            if let Some(id) = state
                .extensions
                .installed
                .get(state.extensions.selected)
                .map(|report| report.id.clone())
            {
                state.confirm(
                    "Remove extension",
                    format!("Remove {id} and its installed declarative assets?"),
                    crate::app::state::ConfirmAction::RemoveExtension(id),
                );
            }
            true
        }
        _ => false,
    }
}

// ── mouse ─────────────────────────────────────────────────────────────────

/// Handle a mouse event against the last drawn layout.
pub fn handle_mouse(
    state: &mut AppState,
    services: &mut Services,
    event: MouseEvent,
    layout: &crate::ui::WorkbenchLayout,
) {
    let (x, y) = (event.column, event.row);
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let Some(target) = layout.hit_test(x, y) else {
                return;
            };
            match target {
                FocusTarget::Explorer => {
                    state.focus = FocusTarget::Explorer;
                    if let Some(rect) = layout.explorer {
                        let row = y.saturating_sub(rect.y + 1) as usize;
                        let index = state.explorer.offset + row;
                        if index < state.tree.rows().len() {
                            state.explorer.selected = index;
                        }
                    }
                }
                FocusTarget::Git => {
                    state.focus = FocusTarget::Git;
                    if let Some(rect) = layout.git {
                        // Row 0 inside the panel is the branch summary.
                        let row = y.saturating_sub(rect.y + 2) as usize;
                        let index = state.git_selection.offset + row;
                        if index < state.git.changes.len() {
                            state.git_selection.selected = index;
                        }
                    }
                }
                FocusTarget::Outline => state.focus = FocusTarget::Outline,
                FocusTarget::AgentList => {
                    state.focus = FocusTarget::AgentList;
                    if let Some(rect) = layout.agents {
                        // Agents take two rows each.
                        let row = y.saturating_sub(rect.y + 1) as usize / 2;
                        let index = state.agent_selection.offset + row;
                        if index < state.agents.len() {
                            state.agent_selection.selected = index;
                            actions::refresh_agent_files(state);
                        }
                    }
                }
                FocusTarget::AgentDetail => state.focus = FocusTarget::AgentDetail,
                FocusTarget::Terminal(id) => state.focus_terminal(id),
                FocusTarget::Editor(_) => {
                    if let Some(id) = state.active_tab {
                        state.focus = FocusTarget::Editor(id);
                        if let Some(rect) = layout.editor {
                            if let Some(document) = state.document_mut(id) {
                                let line = document.scroll + y.saturating_sub(rect.y) as usize;
                                let gutter = document.buffer.line_count().to_string().len() + 2;
                                let column = (x.saturating_sub(rect.x) as usize)
                                    .saturating_sub(gutter)
                                    + document.h_scroll;
                                document.goto(Position::new(line, column));
                            }
                        }
                    }
                }
                _ => {}
            }
            let _ = services;
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let delta: isize = if event.kind == MouseEventKind::ScrollUp {
                -3
            } else {
                3
            };
            scroll_at(state, x, y, delta, layout);
        }
        _ => {}
    }
}

fn scroll_at(
    state: &mut AppState,
    x: u16,
    y: u16,
    delta: isize,
    layout: &crate::ui::WorkbenchLayout,
) {
    match layout.hit_test(x, y) {
        Some(FocusTarget::Explorer) => {
            let len = state.tree.rows().len();
            state.explorer.move_by(delta, len);
        }
        Some(FocusTarget::Git) => {
            let len = state.git.changes.len();
            state.git_selection.move_by(delta, len);
        }
        Some(FocusTarget::AgentList) => {
            let len = state.agents.len();
            state.agent_selection.move_by(delta, len);
        }
        Some(FocusTarget::Terminal(id)) => {
            if let Ok(mut terminals) = state.terminals.lock() {
                if let Some(session) = terminals.get_mut(id) {
                    session.scroll(-delta);
                }
            }
        }
        Some(FocusTarget::Editor(_)) => {
            if let Some(document) = state.active_document_mut() {
                document.buffer.move_vertical(delta, false);
            }
        }
        _ => {}
    }
}
