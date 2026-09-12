# Changelog

All notable changes to TermLoom are recorded here.

## 0.1.0 — 2026-09-13

First release: a terminal-native, agent-first workbench.

### Workbench

- Responsive three-column layout (explorer/sidebar, editor, agents) over a
  terminal strip and a status bar, with wide, compact and minimal modes.
- Lazily expanded project explorer with Git decorations, create/rename/delete
  behind confirmations, and reveal-active-file.
- Editor tabs with dirty state, UTF-8 editing, selection, clipboard,
  undo/redo with typing coalescence, in-file search, go-to-line, indent and
  comment toggling, and detection of external edits before a save overwrites
  them.
- Tree-sitter highlighting and outlines for Rust, Dart, Java, JavaScript,
  TypeScript, JSON, YAML, TOML, Markdown and Shell, with a plain-text
  fallback for everything else.
- Command palette and quick open over a fuzzy index, a command registry with
  stable ids, configurable keybindings, and an F1 shortcut overlay.
- Git branch, status, diff, stage, unstage and confirmed discard.
- Workspace persistence for open files, cursors, layout and shells.

### Terminals and agents

- Real interactive pseudo terminals parsed with vt100: ANSI colours, cursor,
  resize, scrollback, exit status and restart, several sessions at once.
- Terminal-first input: a focused pane receives every key except the global
  chords and the `Ctrl+Space` prefix.
- Agent sessions for Claude Code, Codex, a shell or any command, normalised
  into one lifecycle with the provenance and confidence of each reading, and
  no invented task summaries or change attribution.
- Agent dashboard and detail panel (status, files, tasks, output) with focus,
  input, stop, restart, rename and remove.

### Language services and debugging

- Generic stdio LSP client: capability negotiation, document sync,
  diagnostics, completion, hover, definition, references, document symbols,
  rename, formatting, crash detection and restart. Validated against
  rust-analyzer.
- Generic stdio DAP client: launch/attach, breakpoints, stepping, threads,
  stack frames, scopes, variables and evaluation, in a compact debug panel.
  Validated against lldb-dap.
- `.vscode/settings.json`, `launch.json` and `tasks.json` are read
  conservatively; unknown settings are reported rather than reinterpreted and
  nothing from a repository runs on its own.

### Extensions

- `termloom extension inspect|install|list|remove` for local VSIX packages.
- Capability-based compatibility reports (Full, Partial, Unsupported,
  Broken) with a reason for every contribution.
- Language associations, snippets and colour themes load natively; TextMate
  grammars are installed and reported as needing an adapter.
- Archive traversal, symlinks and oversized packages are refused, extraction
  is staged and swapped atomically, and extension code is never executed.

### Integration and tooling

- Optional Herdr backend behind the same agent interface, used only when
  TermLoom runs inside Herdr or the user enables it, plus a thin launcher
  plugin that does not contain the application.
- `termloom doctor`, `termloom config`, file logging, panic-safe terminal
  restoration and CI for formatting, lints and tests.
