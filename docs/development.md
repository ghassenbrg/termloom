# Development

TermLoom builds with Rust 1.90 or newer. CI pins 1.90 — the minimum supported
version — for every step, so validate against that toolchain rather than
whatever stable happens to be installed; clippy's lint set differs between
releases and a newer toolchain will happily pass code that CI rejects.

```bash
rustup toolchain install 1.90 --component rustfmt --component clippy
cargo +1.90 fmt --all -- --check
cargo +1.90 clippy --all-targets --all-features -- -D warnings
cargo +1.90 test --all-targets --locked
cargo +1.90 build --release --locked
```

Day to day, `cargo test --lib` on any recent toolchain is enough; run the
pinned gate before pushing.

```bash
cargo run -- doctor   # what this machine can integrate with
cargo run -- .        # the workbench, on this repository
```

Integration tests that need an external program (`rust-analyzer`, `lldb-dap`,
`herdr`) skip themselves with a printed reason when it is missing, so the
suite is green on a bare machine and meaningful on a full one. Run them with
`-- --nocapture` to see which ones skipped.

Manual release checks cover large/medium/small terminals, repeated resize, Git
and non-Git directories, missing/present agent CLIs, four concurrent PTYs, a
full-screen child TUI, LSP/DAP crash recovery, supported/unsupported VSIX files,
and Herdr present/absent. Do not test against real agent accounts in CI.
