# Configuration

TermLoom loads built-in defaults, the platform global `termloom/config.toml`,
then `<workspace>/.termloom.toml`. Project values override global values. Run
`termloom config` for the exact schema/defaults or
`termloom config --write-default` to create the global file safely.

Sections are `[ui]`, `[editor]`, `[terminal]`, `[workspace]`, `[herdr]`, `[extensions]`,
`[agents.<name>]`, `[lsp.<name>]`, `[dap.<name>]`, and `[keys]`. Unknown keys
produce a visible diagnostic.

```toml
[ui]
theme = "termloom-dark"
show_explorer = true
show_agents = true
mouse = true
terminal_height_pct = 30

[editor]
tab_width = 4
insert_spaces = true
line_numbers = true
trim_trailing_whitespace = false
insert_final_newline = false

[terminal]
shell = "/bin/zsh"
shell_args = ["-l"]
scrollback = 5000
idle_after_secs = 20

[workspace]
ignore = [".git", "node_modules", "target", "build"]
respect_gitignore = true
show_hidden = false
restore_session = true

[extensions]
# Packages remain installed globally but contribute nothing in this workspace.
disabled = ["publisher.extension"]

[agents.tests]
label = "Tests"
command = "cargo"
args = ["test"]

[keys]
"workspace.reload" = "ctrl+r"
```

TermLoom never stores a child environment in workspace persistence. Avoid
putting secrets in repository-owned `.termloom.toml` files.
