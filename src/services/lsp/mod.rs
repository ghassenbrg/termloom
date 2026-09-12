//! Language Server Protocol client.
//!
//! Layering: [`crate::services::framing`] speaks the JSON-RPC base protocol,
//! [`protocol`]
//! builds payloads and converts results into TermLoom types, [`client`] owns
//! one server process, and [`manager`] routes documents and requests to the
//! right client.

pub mod client;
pub mod manager;
pub mod protocol;

pub use client::{
    ClientStatus, LspClient, LspEvent, LspResult, LspSink, RequestContext, RequestKind,
};
pub use manager::{LspManager, RequestExtra, ServerStatus};
pub use protocol::{CompletionItem, ServerCapabilities, TextEdit};
