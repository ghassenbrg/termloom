//! Diagnostics, symbols and positions shared by the editor, Problems panel,
//! Outline and the LSP layer. These types are protocol-independent: the LSP
//! service converts `lsp` JSON into them, and Tree-sitter fills in symbols
//! when no language server is available.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Zero-based line/character position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct Position {
    pub line: usize,
    pub character: usize,
}

impl Position {
    pub fn new(line: usize, character: usize) -> Self {
        Self { line, character }
    }
}

/// Half-open range between two positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }

    pub fn single_line(line: usize, start: usize, end: usize) -> Self {
        Self {
            start: Position::new(line, start),
            end: Position::new(line, end),
        }
    }

    pub fn contains(&self, pos: Position) -> bool {
        pos >= self.start && pos < self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

impl Severity {
    pub fn glyph(self) -> &'static str {
        match self {
            Severity::Error => "×",
            Severity::Warning => "!",
            Severity::Information => "i",
            Severity::Hint => "·",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Information => "info",
            Severity::Hint => "hint",
        }
    }

    /// Maps LSP severity numbers (1..=4).
    pub fn from_lsp(value: Option<i64>) -> Severity {
        match value {
            Some(1) => Severity::Error,
            Some(2) => Severity::Warning,
            Some(3) => Severity::Information,
            Some(4) => Severity::Hint,
            _ => Severity::Information,
        }
    }
}

/// One problem reported for a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub path: PathBuf,
    pub range: Range,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
}

/// Symbol kinds we render in the Outline. Mirrors the useful subset of
/// `SymbolKind` from LSP; Tree-sitter extraction maps onto the same set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    File,
    Module,
    Namespace,
    Class,
    Method,
    Property,
    Field,
    Constructor,
    Enum,
    Interface,
    Function,
    Variable,
    Constant,
    Struct,
    TypeAlias,
    Heading,
    Key,
    Other,
}

impl SymbolKind {
    pub fn label(self) -> &'static str {
        match self {
            SymbolKind::File => "file",
            SymbolKind::Module => "module",
            SymbolKind::Namespace => "namespace",
            SymbolKind::Class => "class",
            SymbolKind::Method => "method",
            SymbolKind::Property => "property",
            SymbolKind::Field => "field",
            SymbolKind::Constructor => "constructor",
            SymbolKind::Enum => "enum",
            SymbolKind::Interface => "interface",
            SymbolKind::Function => "function",
            SymbolKind::Variable => "variable",
            SymbolKind::Constant => "const",
            SymbolKind::Struct => "struct",
            SymbolKind::TypeAlias => "type",
            SymbolKind::Heading => "heading",
            SymbolKind::Key => "key",
            SymbolKind::Other => "symbol",
        }
    }

    /// Maps LSP `SymbolKind` integers.
    pub fn from_lsp(value: i64) -> SymbolKind {
        match value {
            1 => SymbolKind::File,
            2 => SymbolKind::Module,
            3 => SymbolKind::Namespace,
            5 => SymbolKind::Class,
            6 => SymbolKind::Method,
            7 => SymbolKind::Property,
            8 => SymbolKind::Field,
            9 => SymbolKind::Constructor,
            10 => SymbolKind::Enum,
            11 => SymbolKind::Interface,
            12 => SymbolKind::Function,
            13 => SymbolKind::Variable,
            14 => SymbolKind::Constant,
            23 => SymbolKind::Struct,
            26 => SymbolKind::TypeAlias,
            _ => SymbolKind::Other,
        }
    }
}

/// A node in the Outline tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub range: Range,
    /// Nesting depth, pre-flattened for cheap rendering.
    pub depth: usize,
    /// Optional container name (`MyClass` for a method).
    pub container: Option<String>,
}

/// A location in the workspace, used by go-to-definition and references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub range: Range,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_contains_is_half_open() {
        let r = Range::single_line(3, 2, 6);
        assert!(r.contains(Position::new(3, 2)));
        assert!(r.contains(Position::new(3, 5)));
        assert!(!r.contains(Position::new(3, 6)));
        assert!(!r.contains(Position::new(2, 4)));
    }

    #[test]
    fn lsp_severity_mapping_defaults_to_info() {
        assert_eq!(Severity::from_lsp(Some(1)), Severity::Error);
        assert_eq!(Severity::from_lsp(Some(2)), Severity::Warning);
        assert_eq!(Severity::from_lsp(None), Severity::Information);
    }
}
