# Development

Use stable Rust 1.90 or newer:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo run -- doctor
```

Manual release checks cover large/medium/small terminals, repeated resize, Git
and non-Git directories, missing/present agent CLIs, four concurrent PTYs, a
full-screen child TUI, LSP/DAP crash recovery, supported/unsupported VSIX files,
and Herdr present/absent. Do not test against real agent accounts in CI.
