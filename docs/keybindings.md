# Keybindings

TermLoom uses a multiplexer-style **command prefix** so that ordinary keys keep
reaching whatever runs inside a terminal pane — a shell, Vim, Claude Code or
Codex. Only the bindings in the *Global* table below are intercepted while a
terminal has focus; everything else is encoded and forwarded to the child
process.

Every binding resolves to a stable command id, so a rebind, the command
palette and the `F1` help overlay always agree.

## Global

Active everywhere, including while a terminal has focus.

| Keys | Command | Action |
|---|---|---|
| `Ctrl+Space` | `workbench.prefix` | Start a prefix chord |
| `Ctrl+P` | `palette.quick_open` | Quick open a file |
| `Ctrl+Q` | `app.quit` | Quit (asks about unsaved changes) |
| `F1` | `help.toggle` | Keyboard shortcuts overlay |
| `F5` | `debug.continue` | Continue |
| `F9` | `debug.toggle_breakpoint` | Toggle breakpoint |
| `F10` | `debug.step_over` | Step over |
| `F11` | `debug.step_in` | Step into |
| `F12` | `debug.step_out` | Step out |

## Prefix

Press `Ctrl+Space`, release, then the key. `Esc` cancels.

| Keys | Command | Action |
|---|---|---|
| `Ctrl+Space E` | `view.focus.explorer` | Focus Explorer |
| `Ctrl+Space O` | `view.focus.outline` | Focus Outline |
| `Ctrl+Space G` | `view.focus.git` | Focus Git |
| `Ctrl+Space A` | `view.focus.agents` | Focus Agents |
| `Ctrl+Space D` | `view.focus.editor` | Focus Editor |
| `Ctrl+Space T` | `terminal.new` | New terminal |
| `Ctrl+Space C` | `agent.new.claude` | New Claude session |
| `Ctrl+Space X` | `agent.new.codex` | New Codex session |
| `Ctrl+Space P` | `palette.commands` | Command palette |
| `Ctrl+Space W` | `editor.close_tab` | Close tab |
| `Ctrl+Space S` | `file.save` | Save |
| `Ctrl+Space Shift+S` | `file.save_all` | Save all |
| `Ctrl+Space B` | `view.toggle.explorer` | Toggle Explorer |
| `Ctrl+Space Shift+A` | `view.toggle.agents` | Toggle Agents panel |
| `Ctrl+Space Shift+T` | `view.toggle.terminals` | Toggle terminal strip |
| `Ctrl+Space M` | `view.toggle.problems` | Toggle Problems panel |
| `Ctrl+Space V` | `view.toggle.debug` | Toggle Debug panel |
| `Ctrl+Space K` | `terminal.kill` | Kill focused terminal |
| `Ctrl+Space R` | `terminal.restart` | Restart focused terminal |
| `Ctrl+Space N` | `terminal.next` | Next terminal |
| `Ctrl+Space H` or `Ctrl+Space Left` | `focus.left` | Focus panel to the left |
| `Ctrl+Space L` or `Ctrl+Space Right` | `focus.right` | Focus panel to the right |
| `Ctrl+Space Up` | `focus.up` | Focus panel above |
| `Ctrl+Space Down` | `focus.down` | Focus the terminal strip |
| `Ctrl+Space ?` | `help.toggle` | Keyboard shortcuts overlay |

## Editor

Active when an editor tab has focus.

| Keys | Command | Action |
|---|---|---|
| `Ctrl+S` | `file.save` | Save |
| `Ctrl+W` | `editor.close_tab` | Close tab |
| `Ctrl+F` | `editor.find` | Find in file |
| `Ctrl+Z` | `editor.undo` | Undo |
| `Ctrl+Y` | `editor.redo` | Redo |
| `Ctrl+G` | `editor.goto_line` | Go to line |
| `Ctrl+D` | `lsp.definition` | Go to definition |
| `Ctrl+R` | `lsp.references` | Find references |
| `Ctrl+K` | `lsp.hover` | Show hover |
| `Ctrl+N` | `lsp.completion` | Completion (server or installed snippets) |

Ordinary editing keys behave as expected: arrows, `Home`/`End`,
`PageUp`/`PageDown`, `Ctrl+Left`/`Ctrl+Right` for word motion, `Shift` with any
motion to extend the selection, `Tab`/`Shift+Tab` to indent, and `Esc` to clear
a selection, close the find bar or leave the editor.

## Panel keys

List panels share `j`/`k` or the arrow keys to move and `Enter` to act.

| Panel | Keys |
|---|---|
| Explorer | `Enter` open/expand, `Right`/`Left` expand/collapse, `a` new file, `A` new folder, `r` rename, `d` delete, `R` refresh |
| Git | `Enter` diff, `o` open file, `s` stage, `u` unstage, `x` discard, `R` refresh |
| Agents | `Enter` focus terminal, `i` send text, `s` stop, `r` restart, `n` rename, `x` remove, `c` new Claude, `t` add task, `Tab` detail tabs |
| Agent detail | `Tab`/`Right` next tab, `Left`/`Esc` back to the list; in Tasks: `t` add, `j`/`k` select, `Space` tick |
| Problems / References | `Enter` jump to the location, `Esc` close |
| Debug | `j`/`k` select a stack frame, `Enter` open its source, `Esc` close |
| Terminal | every key goes to the child; `Shift+PageUp`/`Shift+PageDown` scroll back |

## Rebinding

Override any command by its id in `config.toml`:

```toml
[keys]
"palette.quick_open" = "ctrl+o"
"file.save_all" = "ctrl+alt+s"
"agent.new.claude" = "f2"
```

An override replaces the command's direct binding when it has one, otherwise
its prefix binding; a command with no default binding gains a global one.
Chords are written as `ctrl+`, `alt+`, `shift+` and `super+` prefixes plus a
key name (`space`, `enter`, `esc`, `tab`, `f1`–`f24`, `left`, `pgup`, a single
character, …). `shift+s` and `S` mean the same chord and are distinct from `s`.
