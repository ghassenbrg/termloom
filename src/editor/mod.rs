//! Editor core: buffers, documents and in-file search.

pub mod buffer;
pub mod document;
pub mod search;

pub use buffer::TextBuffer;
pub use document::{Document, ExternalChange};
pub use search::SearchState;
