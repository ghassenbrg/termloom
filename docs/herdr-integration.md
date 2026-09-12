# Herdr integration

Herdr is optional. With `herdr.mode = "auto"`, TermLoom connects only when the
process inherited `HERDR_ENV=1`, so a standalone launch never attaches to an
unrelated Herdr session. `enabled` always attempts connection and reports a
failure; `disabled` never invokes Herdr. `herdr.command` can select an explicit
binary.

The adapter uses Herdr's versioned CLI/JSON surface, rate-limits refreshes, and
maps external agents into the same model as local PTYs. Backend failure is
logged and cannot stop the editor or local agents.

The launcher under `integrations/herdr/` is a thin Herdr plugin:

```bash
herdr plugin link ./integrations/herdr
herdr plugin pane open --plugin termloom.workbench --entrypoint workbench --placement zoomed
```

It executes the installed `termloom` binary in a managed pane. Override the
binary with `TERMLOOM_BIN` or the path with `TERMLOOM_WORKSPACE`. TermLoom is
not packaged inside the plugin.
