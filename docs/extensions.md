# VS Code extension compatibility

TermLoom is capability-compatible, not an Extension Host emulator:

```bash
termloom extension inspect ./package.vsix [--json]
termloom extension install ./package.vsix
termloom extension list [--json]
termloom extension remove publisher.extension
```

The inspector reads `extension/package.json`, validates every ZIP path and
referenced asset, and reports `Full`, `Partial`, `Unsupported`, or `Broken`.

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
