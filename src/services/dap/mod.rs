//! Debug Adapter Protocol client.
//!
//! [`client`] owns the adapter process, [`protocol`] builds and parses
//! payloads, [`session`] runs the handshake and turns protocol traffic into
//! events, and [`config`] resolves what to debug (including VS Code
//! `launch.json` entries).

pub mod client;
pub mod config;
pub mod protocol;
pub mod session;

pub use config::{available, resolve, DebugLaunchConfig, LaunchEntry, LaunchSource};
pub use session::{DebugCommand, DebugEvent, DebugSession, DebugSink};
