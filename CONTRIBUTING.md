# Contributing

TermLoom uses stable Rust (minimum 1.90). Keep domain types independent from
Ratatui, portable-pty, git2, transport, and Herdr implementation types. New
user actions belong in the stable command registry and should be reachable by
palette or keyboard.

Before submitting a change, run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Tests must not require Claude/Codex credentials. Use temporary repositories,
fake process fixtures, and Ratatui's test backend. Changes to terminal routing,
archive extraction, destructive Git/file operations, or adapter execution need
negative/security tests as well as the happy path.

See [docs/development.md](docs/development.md) for manual checks.
