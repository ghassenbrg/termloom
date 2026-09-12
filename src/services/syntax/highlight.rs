//! Tree-sitter syntax highlighting.
//!
//! Grammars are compiled in for the languages listed in the V1 requirements.
//! Anything else renders as plain text — highlighting is a nicety, never a
//! precondition for opening a file, and a parser failure is swallowed.

use std::collections::HashMap;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

use super::languages::LanguageId;

/// Semantic token classes the theme maps to colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HighlightKind {
    Keyword,
    Type,
    Function,
    Method,
    Property,
    Variable,
    Parameter,
    String,
    Number,
    Boolean,
    Comment,
    Constant,
    Operator,
    Punctuation,
    Attribute,
    Tag,
    Escape,
    Text,
}

impl HighlightKind {
    /// Map a Tree-sitter capture name (`function.method`) onto a token class.
    /// Matching is prefix-based so grammar-specific suffixes still land
    /// somewhere sensible.
    pub fn from_capture(name: &str) -> Option<HighlightKind> {
        let base = name.split('.').next().unwrap_or(name);
        let kind = match (name, base) {
            (n, _) if n.starts_with("function.method") => HighlightKind::Method,
            (n, _) if n.starts_with("keyword") => HighlightKind::Keyword,
            (n, _) if n.starts_with("constant.builtin") => HighlightKind::Boolean,
            (n, _) if n.starts_with("string.escape") || n.starts_with("escape") => {
                HighlightKind::Escape
            }
            (_, "function") => HighlightKind::Function,
            (_, "method") => HighlightKind::Method,
            (_, "type") => HighlightKind::Type,
            (_, "constructor") => HighlightKind::Type,
            (_, "property" | "field") => HighlightKind::Property,
            (_, "variable") => HighlightKind::Variable,
            (_, "parameter") => HighlightKind::Parameter,
            (_, "string") => HighlightKind::String,
            (_, "number" | "float" | "integer") => HighlightKind::Number,
            (_, "boolean") => HighlightKind::Boolean,
            (_, "comment") => HighlightKind::Comment,
            (_, "constant") => HighlightKind::Constant,
            (_, "operator") => HighlightKind::Operator,
            (_, "punctuation") => HighlightKind::Punctuation,
            (_, "attribute" | "annotation") => HighlightKind::Attribute,
            (_, "tag") => HighlightKind::Tag,
            (_, "label") => HighlightKind::Attribute,
            (_, "text" | "none") => HighlightKind::Text,
            _ => return None,
        };
        Some(kind)
    }
}

/// A highlighted character range inside one line (character columns).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub kind: HighlightKind,
}

/// Highlight spans for a whole document, indexed by line.
#[derive(Debug, Clone, Default)]
pub struct SyntaxIndex {
    lines: Vec<Vec<HighlightSpan>>,
}

impl SyntaxIndex {
    /// Spans for one line, already sorted and non-overlapping.
    pub fn line(&self, index: usize) -> &[HighlightSpan] {
        self.lines.get(index).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(Vec::is_empty)
    }
}

/// Static grammar table.
fn grammar_for(language: &LanguageId) -> Option<(Language, &'static str)> {
    let (lang_fn, query) = match language.as_str() {
        "rust" => (
            tree_sitter_rust::LANGUAGE,
            tree_sitter_rust::HIGHLIGHTS_QUERY,
        ),
        "dart" => (
            tree_sitter_dart::LANGUAGE,
            tree_sitter_dart::HIGHLIGHTS_QUERY,
        ),
        "java" => (
            tree_sitter_java::LANGUAGE,
            tree_sitter_java::HIGHLIGHTS_QUERY,
        ),
        "javascript" | "javascriptreact" => (
            tree_sitter_javascript::LANGUAGE,
            tree_sitter_javascript::HIGHLIGHT_QUERY,
        ),
        "typescript" => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ),
        "typescriptreact" => (
            tree_sitter_typescript::LANGUAGE_TSX,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ),
        "json" | "jsonc" => (
            tree_sitter_json::LANGUAGE,
            tree_sitter_json::HIGHLIGHTS_QUERY,
        ),
        "yaml" => (
            tree_sitter_yaml::LANGUAGE,
            tree_sitter_yaml::HIGHLIGHTS_QUERY,
        ),
        "toml" => (
            tree_sitter_toml_ng::LANGUAGE,
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        ),
        "markdown" => (
            tree_sitter_md::LANGUAGE,
            tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
        ),
        "shellscript" => (
            tree_sitter_bash::LANGUAGE,
            tree_sitter_bash::HIGHLIGHT_QUERY,
        ),
        _ => return None,
    };
    Some((Language::new(lang_fn), query))
}

/// Language ids with a compiled-in grammar.
pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "rust",
    "dart",
    "java",
    "javascript",
    "javascriptreact",
    "typescript",
    "typescriptreact",
    "json",
    "jsonc",
    "yaml",
    "toml",
    "markdown",
    "shellscript",
];

/// Compiles grammars/queries lazily and caches them per language.
#[derive(Default)]
pub struct Highlighter {
    cache: HashMap<String, Option<CompiledGrammar>>,
    /// Documents above this many bytes are not highlighted, to keep editing
    /// responsive on generated files.
    pub max_bytes: usize,
}

#[derive(Debug)]
struct CompiledGrammar {
    language: Language,
    query: Query,
}

impl Highlighter {
    pub fn new() -> Highlighter {
        Highlighter {
            cache: HashMap::new(),
            max_bytes: 2 * 1024 * 1024,
        }
    }

    /// Whether a grammar exists for this language.
    pub fn supports(&self, language: &LanguageId) -> bool {
        SUPPORTED_LANGUAGES.contains(&language.as_str())
    }

    /// Highlight a whole document. Returns `None` when unsupported, too large
    /// or when the grammar/query failed to load.
    pub fn highlight(&mut self, language: &LanguageId, text: &str) -> Option<SyntaxIndex> {
        if text.len() > self.max_bytes {
            return None;
        }
        let grammar = self.compiled(language)?;
        let mut parser = Parser::new();
        parser.set_language(&grammar.language).ok()?;
        let tree = parser.parse(text, None)?;

        let lines: Vec<&str> = text.split('\n').collect();
        // One slot per character; the first capture to claim a slot wins,
        // which reproduces Tree-sitter's "earlier pattern has priority" rule.
        let mut slots: Vec<Vec<Option<HighlightKind>>> = lines
            .iter()
            .map(|line| vec![None; line.chars().count()])
            .collect();

        let capture_names = grammar.query.capture_names();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&grammar.query, tree.root_node(), text.as_bytes());
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let name = capture_names[capture.index as usize];
                let Some(kind) = HighlightKind::from_capture(name) else {
                    continue;
                };
                let start = capture.node.start_position();
                let end = capture.node.end_position();
                if end.row >= slots.len() {
                    continue;
                }
                for row in start.row..=end.row {
                    let line = lines[row];
                    let from_byte = if row == start.row { start.column } else { 0 };
                    let to_byte = if row == end.row {
                        end.column
                    } else {
                        line.len()
                    };
                    let from = byte_to_char(line, from_byte);
                    let to = byte_to_char(line, to_byte);
                    for slot in slots[row].iter_mut().take(to).skip(from) {
                        if slot.is_none() {
                            *slot = Some(kind);
                        }
                    }
                }
            }
        }

        Some(SyntaxIndex {
            lines: slots.into_iter().map(compress).collect(),
        })
    }

    fn compiled(&mut self, language: &LanguageId) -> Option<&CompiledGrammar> {
        let key = language.as_str().to_string();
        self.cache.entry(key).or_insert_with(|| {
            let (lang, query_src) = grammar_for(language)?;
            match Query::new(&lang, query_src) {
                Ok(query) => Some(CompiledGrammar {
                    language: lang,
                    query,
                }),
                Err(err) => {
                    tracing::warn!(language = %language, error = %err, "highlight query failed to compile");
                    None
                }
            }
        });
        self.cache.get(language.as_str()).and_then(|v| v.as_ref())
    }
}

/// Turn per-character kinds into runs.
fn compress(slots: Vec<Option<HighlightKind>>) -> Vec<HighlightSpan> {
    let mut spans: Vec<HighlightSpan> = Vec::new();
    let mut current: Option<(usize, HighlightKind)> = None;
    for (index, slot) in slots.iter().enumerate() {
        match (current, slot) {
            (Some((start, kind)), Some(next)) if kind == *next => {
                let _ = start;
            }
            (Some((start, kind)), _) => {
                spans.push(HighlightSpan {
                    start,
                    end: index,
                    kind,
                });
                current = slot.map(|k| (index, k));
            }
            (None, Some(next)) => current = Some((index, *next)),
            (None, None) => {}
        }
    }
    if let Some((start, kind)) = current {
        spans.push(HighlightSpan {
            start,
            end: slots.len(),
            kind,
        });
    }
    spans
}

/// Convert a byte column inside a line into a character column.
fn byte_to_char(line: &str, byte: usize) -> usize {
    if byte >= line.len() {
        return line.chars().count();
    }
    line[..byte].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds_on_line(index: &SyntaxIndex, line: usize) -> Vec<HighlightKind> {
        index.line(line).iter().map(|s| s.kind).collect()
    }

    #[test]
    fn highlights_rust_keywords_and_strings() {
        let mut hl = Highlighter::new();
        let index = hl
            .highlight(
                &LanguageId::new("rust"),
                "fn main() {\n    let s = \"hi\"; // note\n}\n",
            )
            .expect("rust grammar available");
        assert!(kinds_on_line(&index, 0).contains(&HighlightKind::Keyword));
        let line1 = kinds_on_line(&index, 1);
        assert!(line1.contains(&HighlightKind::String), "{line1:?}");
        assert!(line1.contains(&HighlightKind::Comment), "{line1:?}");
    }

    #[test]
    fn every_advertised_language_parses() {
        let mut hl = Highlighter::new();
        let samples = [
            ("rust", "fn a() {}"),
            ("dart", "void main() { print('x'); }"),
            ("java", "class A { void b() {} }"),
            ("javascript", "const a = 1;"),
            ("typescript", "const a: number = 1;"),
            ("json", "{\"a\": 1}"),
            ("yaml", "a: 1\n"),
            ("toml", "a = 1\n"),
            ("markdown", "# Title\n"),
            ("shellscript", "echo hi\n"),
        ];
        for (lang, src) in samples {
            let id = LanguageId::new(lang);
            assert!(hl.supports(&id), "{lang} should be supported");
            let index = hl.highlight(&id, src);
            assert!(index.is_some(), "{lang} produced no index");
        }
    }

    #[test]
    fn unsupported_languages_return_none() {
        let mut hl = Highlighter::new();
        assert!(hl
            .highlight(&LanguageId::new("plaintext"), "hello")
            .is_none());
    }

    #[test]
    fn oversized_documents_are_skipped() {
        let mut hl = Highlighter::new();
        hl.max_bytes = 8;
        assert!(hl
            .highlight(&LanguageId::new("rust"), "fn main() { let a = 1; }")
            .is_none());
    }

    #[test]
    fn spans_use_character_columns_on_unicode_lines() {
        let mut hl = Highlighter::new();
        let index = hl
            .highlight(&LanguageId::new("rust"), "// héllo → wörld\nfn a() {}")
            .unwrap();
        let comment = index.line(0)[0];
        assert_eq!(comment.kind, HighlightKind::Comment);
        assert_eq!(comment.start, 0);
        assert_eq!(comment.end, 16, "character count, not byte count");
    }

    #[test]
    fn capture_names_map_to_token_classes() {
        assert_eq!(
            HighlightKind::from_capture("keyword.control"),
            Some(HighlightKind::Keyword)
        );
        assert_eq!(
            HighlightKind::from_capture("function.method"),
            Some(HighlightKind::Method)
        );
        assert_eq!(HighlightKind::from_capture("unmapped.thing"), None);
    }
}
