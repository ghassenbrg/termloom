//! Terminal infrastructure: pseudo terminals, VT parsing and key encoding.

pub mod input;
pub mod manager;
pub mod screen;
pub mod session;

pub use manager::TerminalManager;
pub use screen::{Cell, CellColor, ScreenSnapshot};
pub use session::{EventSink, SpawnSpec, TerminalEvent, TerminalSession};
