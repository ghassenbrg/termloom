# Troubleshooting

Run `termloom doctor` first. It reports terminal/shell/Git availability and
optional Claude, Codex, rust-analyzer and Herdr integrations. Logs are written
under the platform TermLoom state directory; override with `--log-file` and set
verbosity with `--log-level` or `TERMLOOM_LOG`.

- If the screen is garbled after a hard kill, run `reset` or `stty sane`.
- If an LSP/DAP command is missing, verify its executable is on `PATH` and the
  configured language/type matches the file or launch configuration.
- If Herdr agents do not appear in auto mode, launch inside a Herdr pane
  (`HERDR_ENV=1`) or deliberately set `herdr.mode = "enabled"`.
- If a VSIX is `Broken`, inspect its report for a missing/unsafe asset. Partial
  usually means it depends on VS Code APIs TermLoom will not execute.
- Outside Git, Git panels are empty by design; editing and PTYs continue.
