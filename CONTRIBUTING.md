# Contributing

TermLoom uses stable Rust (minimum 1.90). Keep domain types independent from
Ratatui, portable-pty, git2, transport, and Herdr implementation types. New
user actions belong in the stable command registry and should be reachable by
palette or keyboard.

Before submitting a change, run the same gate CI runs, on the same toolchain.
Clippy gains lints between releases, so a newer local toolchain can pass while
CI fails — pin it:

```bash
rustup toolchain install 1.90 --component rustfmt --component clippy
cargo +1.90 fmt --all -- --check
cargo +1.90 clippy --all-targets --all-features -- -D warnings
cargo +1.90 test --all-targets --locked
cargo +1.90 build --release --locked
```

Tests must not require Claude/Codex credentials. Use temporary repositories,
fake process fixtures, and Ratatui's test backend. Changes to terminal routing,
archive extraction, destructive Git/file operations, or adapter execution need
negative/security tests as well as the happy path.

See [docs/development.md](docs/development.md) for manual checks.

Releases are cut by pushing a `v*` tag; see [RELEASING.md](RELEASING.md).
