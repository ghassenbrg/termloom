//! TermLoom — an agent-native development environment for your terminal.
//!
//! The crate is organised in layers:
//!
//! * [`domain`] — pure data types (ids, agent/terminal/git/diagnostic models).
//! * [`services`] — infrastructure adapters (filesystem, git, pty, lsp, dap, ...).
//!   Third-party types must not leak out of this layer.
//! * [`app`] — workbench state, the command registry, the event loop reducer.
//! * [`ui`] — Ratatui drawing code. Draws from `app` state, never mutates it.
//! * [`cli`] — argument parsing and non-TUI subcommands (`doctor`, `extension`).

pub mod app;
pub mod cli;
pub mod config;
pub mod domain;
pub mod editor;
pub mod services;
pub mod ui;

/// Product name as shown in the workbench header.
pub const PRODUCT_NAME: &str = "TermLoom";
/// Short fallback used when the header has almost no horizontal room.
pub const PRODUCT_NAME_SHORT: &str = "TL";
/// Product tagline.
pub const TAGLINE: &str = "Weave your development workspace";
/// Crate version, surfaced by `--version` and `doctor`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
