//! Renders the whole workbench into a test backend at several terminal sizes.
//!
//! These act as layout regression tests: they assert that each panel is
//! present and that the workbench stays usable when the terminal shrinks.

use std::sync::{Arc, Mutex};

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use termloom::app::state::AppState;
use termloom::config::Config;
use termloom::services::terminal::{EventSink, TerminalManager};
use termloom::services::workspace::Workspace;
use termloom::ui::{self, Theme};

/// A small repository with a couple of files.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src/app")).unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("src/app/mod.rs"), "pub struct App;\n").unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
    dir
}

fn state_for(dir: &std::path::Path) -> AppState {
    let (workspace, _) = Workspace::discover(dir).unwrap();
    let sink: EventSink = Arc::new(|_| {});
    let terminals = Arc::new(Mutex::new(TerminalManager::new(sink)));
    let mut state = AppState::new(
        workspace,
        Config::with_builtin_defaults(),
        Vec::new(),
        terminals,
    );
    let main = dir.join("src/main.rs");
    state.tree.expand(&dir.join("src"));
    state.open_file(&main).unwrap();
    state.refresh_syntax();
    state
}

fn render(state: &AppState, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = Theme::termloom_dark();
    terminal
        .draw(|frame| {
            ui::draw(frame, state, &theme);
        })
        .unwrap();
    buffer_to_string(terminal.backend().buffer())
}

fn buffer_to_string(buffer: &ratatui::buffer::Buffer) -> String {
    let area = buffer.area();
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn large_layout_shows_every_panel() {
    let dir = fixture();
    let state = state_for(dir.path());
    let output = render(&state, 160, 44);
    println!("{output}");

    for expected in [
        "TermLoom",
        "EXPLORER",
        "EDITOR",
        "AGENTS",
        "TERMINAL",
        "GIT",
        "OUTLINE",
        "main.rs",
        "NORMAL",
        "Ln 1, Col 1",
    ] {
        assert!(
            output.contains(expected),
            "missing `{expected}` in:\n{output}"
        );
    }
}

#[test]
fn medium_layout_drops_the_agents_column() {
    let dir = fixture();
    let state = state_for(dir.path());
    let output = render(&state, 100, 30);
    println!("{output}");
    assert!(output.contains("EXPLORER"));
    assert!(output.contains("EDITOR"));
    // The agents panel needs more width than this.
    assert!(!output.contains("AGENTS"), "{output}");
}

#[test]
fn small_layout_shows_one_surface() {
    let dir = fixture();
    let state = state_for(dir.path());
    let output = render(&state, 60, 16);
    println!("{output}");
    assert!(output.contains("TermLoom") || output.contains("TL"));
    assert!(output.contains("EDITOR"));
    assert!(!output.contains("EXPLORER"));
}

#[test]
fn tiny_terminals_do_not_panic() {
    let dir = fixture();
    let state = state_for(dir.path());
    for (width, height) in [(20, 5), (10, 3), (40, 8), (1, 1)] {
        let _ = render(&state, width, height);
    }
}

#[test]
fn editor_shows_line_numbers_and_content() {
    let dir = fixture();
    let state = state_for(dir.path());
    let output = render(&state, 160, 44);
    assert!(output.contains("fn main()"), "{output}");
    assert!(output.contains("println!"), "{output}");
}

#[test]
fn explorer_lists_the_project_tree() {
    let dir = fixture();
    let state = state_for(dir.path());
    let output = render(&state, 160, 44);
    assert!(output.contains("src"), "{output}");
    assert!(output.contains("Cargo.toml"), "{output}");
}
