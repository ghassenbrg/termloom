# Language services

TermLoom's generic LSP client launches configured servers over stdio, performs
initialize/initialized and full document sync, negotiates capabilities,
correlates responses, reports diagnostics, and shuts down cleanly. A crashed
server is marked failed and can be restarted from the command palette.

```toml
[lsp.rust-analyzer]
command = "rust-analyzer"
args = []
languages = ["rust"]
root_markers = ["Cargo.toml"]
auto_start = true
```

When advertised, completion, hover, definition, references, document symbols,
rename, and formatting are enabled. Diagnostics appear on lines and in the
Problems panel. Outline falls back to Tree-sitter. The implementation is
validated against a real `rust-analyzer`, but no protocol code is Rust-specific.

Relevant `.vscode/settings.json` data is imported conservatively; unknown
settings are listed/ignored rather than reinterpreted.
