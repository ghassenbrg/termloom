# VS Code extension compatibility

TermLoom is capability-compatible, not an Extension Host emulator:

```bash
termloom extension inspect ./package.vsix [--json]
termloom extension install ./package.vsix
termloom extension list [--json]
termloom extension remove publisher.extension
```

The inspector reads `extension/package.json`, validates every ZIP path and
referenced asset, and reports one of four classes. Installation succeeding
never implies support: the class comes from what TermLoom can actually do with
the package.

| Class | Meaning |
|---|---|
| `Full` | every capability the package contributes is usable as-is |
| `Partial` | some capabilities are usable, or usable after explicit configuration; the rest are listed with a reason |
| `Unsupported` | nothing portable can be reused safely |
| `Broken` | a capability that should have worked failed validation — usually a referenced asset that is missing or has an unsafe path |

| Contribution | V1 behavior |
|---|---|
| language definitions/file associations | loaded natively |
| snippets | loaded into completion; first/default tabstop values |
| color themes | selected by label or `id:label` in config |
| TextMate grammars | safely installed; partial because rendering uses Tree-sitter |
| debugger metadata | partial; explicit DAP config and consent required |
| configuration schemas | partial; only explicitly mapped settings apply |
| commands, menus, keybindings | unsupported unless TermLoom maps them |
| views, custom editors, webviews | unsupported |
| `main` / `browser` JavaScript | never executed |

Absolute/traversal entries and symlinks are rejected, installation is staged
under the platform TermLoom data directory, and removal uses manifest identity.
A missing referenced portable asset is `Broken`. Bundled servers/adapters are
not enabled merely by installation; configure and approve the exact executable.
Installed assets can be disabled for one workspace without deleting them:

```toml
[extensions]
disabled = ["publisher.extension"]
```

## Why the Extension Host is not emulated

LSP and DAP are deliberately editor-independent, and manifest contributions
such as languages, snippets and themes are plain data. The rest of the VS Code
extension API is not a portable standard: it assumes a Node.js host, a DOM,
webviews, and a UI model built around VS Code's own widgets. Emulating it would
mean shipping a JavaScript runtime, reimplementing an undocumented and moving
API surface, and running untrusted third-party code on the user's machine to
get features a terminal cannot display anyway.

TermLoom therefore reuses the portable half of the ecosystem and says plainly
what it cannot do. An extension's `main` or `browser` entrypoint is recorded in
the report and never executed — not at install time, not at workspace open, not
on any activation event.

## Trust model

Treat every `.vsix` as untrusted input:

- archive entries with absolute paths, `..` components or symlinks are refused
  before anything is written;
- entry count, per-file size and total uncompressed size are capped, so a zip
  bomb fails the install instead of filling the disk;
- extraction is staged in the TermLoom data directory and swapped into place
  atomically, with the previous version restored if the swap fails;
- a package whose declared assets are missing or unparseable is rejected rather
  than half-installed;
- themes and snippets are parsed as data, and a theme may only include files
  from inside its own extension directory;
- a bundled language server or debug adapter is never started because a package
  was installed. It becomes available only once you configure it under `[lsp]`
  or `[dap]`, and TermLoom shows the exact executable and arguments before the
  first run;
- nothing in a repository — `tasks.json`, `launch.json`, a workspace config —
  runs on its own.

Diagnostics about extensions go to the log file, never with environment values
attached.
