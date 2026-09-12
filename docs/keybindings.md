# Keybindings

`Ctrl+Space` is the global prefix so ordinary keys remain available to shells,
Vim, Claude and Codex. After the prefix: `E/O/G/A/D` focus
Explorer/Outline/Git/Agents/Editor, `T` creates a terminal, `C` launches Claude,
`X` launches Codex, `P` opens commands, arrows or `H/J/K/L` move focus, and
`?` opens help.

Direct global keys are `Ctrl+P` quick open, `Ctrl+Q` quit, `F1` help, `F5`
continue, `F9` breakpoint, and `F10/F11/F12` step over/in/out. Editor keys
include `Ctrl+S`, `Ctrl+W`, `Ctrl+F`, `Ctrl+Z`, `Ctrl+Y`, `Ctrl+G`, `Ctrl+D`
definition, `Ctrl+R` references, `Ctrl+K` hover, and `Ctrl+N` completion or
installed snippets.

Override a command by stable ID:

```toml
[keys]
"palette.quick_open" = "ctrl+o"
"file.save" = "ctrl+s"
```

When a terminal has focus only global bindings and the workbench prefix are
intercepted; all other key sequences are encoded for the PTY.
