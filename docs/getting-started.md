# Getting started

Install with `cargo install --path .`, run `termloom doctor`, then open a
directory with `termloom .` or `termloom /path/to/repository`. A file path is
also accepted and opens its parent workspace with that file active.

Use arrows or `j`/`k` in list panels and Enter to open/select. `Ctrl+Space`
starts a workbench command chord; `F1` lists every shortcut. `Ctrl+P` is quick
open, while `Ctrl+Space P` searches commands. Create a shell with
`Ctrl+Space T`, Claude with `Ctrl+Space C`, or Codex with `Ctrl+Space X`.

The application works in non-Git folders and without language/debug servers or
agent CLIs. Missing optional tools are reported instead of preventing startup.
Workspace state is saved on shutdown without terminal contents or environment
variables.
