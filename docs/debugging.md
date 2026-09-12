# Debugging

Configure an adapter command and the VS Code `type` values it serves:

```toml
[dap.lldb]
command = "lldb-dap"
args = []
types = ["lldb"]
```

Launch descriptions come from this configuration and
`.vscode/launch.json`. Nothing in a repository is launched automatically.
Starting debug shows the exact adapter command/cwd for confirmation. Supported
workspace variables are expanded; unknown variables remain visible.

Use `F9` to toggle a source breakpoint, `F5` to continue, and
`F10/F11/F12` to step over/in/out. The compact Debug panel shows lifecycle,
threads, stack, scopes/variables and adapter output. Selecting a frame opens
its source. Adapter failures are contained and surfaced in the workbench. The
real integration test uses `lldb-dap` when available.
