//! Safe, capability-based VS Code extension compatibility.
//!
//! Packages are treated as archives containing declarative data.  Nothing in
//! this module executes an extension's `main` or `browser` entrypoint.

mod assets;
mod classifier;
pub mod manifest;
mod package;
mod store;

pub use assets::load_snippets;
pub use classifier::classify;
pub use package::{inspect, inspect_directory};
pub use store::{install, install_into, list, list_in, remove, remove_from};
