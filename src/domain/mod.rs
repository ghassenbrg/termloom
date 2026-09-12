//! Pure workbench data types. No I/O, no third-party protocol types.

pub mod agent;
pub mod debug;
pub mod diagnostics;
pub mod extensions;
pub mod git;
pub mod ids;
pub mod terminal;

pub use ids::{AgentId, EditorTabId, ModalId, TerminalId};
