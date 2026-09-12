//! Syntax services: language identification, highlighting and outlines.

pub mod highlight;
pub mod languages;
pub mod outline;

pub use highlight::{HighlightKind, HighlightSpan, Highlighter, SyntaxIndex};
pub use languages::{detect, LanguageId, LanguageRegistry};
pub use outline::OutlineExtractor;
