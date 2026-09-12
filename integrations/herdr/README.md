# TermLoom for Herdr

This is a thin Herdr entrypoint for the standalone `termloom` binary. It does
not embed or fork TermLoom.

Install TermLoom first, then link this development plugin:

```bash
herdr plugin link ./integrations/herdr
herdr plugin pane open --plugin termloom.workbench --entrypoint workbench --placement zoomed
```

The pane inherits Herdr's `HERDR_*` context. With TermLoom's default
`herdr.mode = "auto"`, that context enables the optional Herdr agent backend.
Set `TERMLOOM_BIN` to an explicit binary path or `TERMLOOM_WORKSPACE` to choose
a workspace other than the pane's current directory.
