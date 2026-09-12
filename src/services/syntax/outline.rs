//! Tree-sitter symbol extraction for the Outline panel.
//!
//! The Outline prefers LSP `documentSymbol` results when a language server is
//! connected; this extractor is the always-available fallback. Rules are
//! declarative per language so adding a grammar does not mean new logic.

use std::collections::HashMap;

use tree_sitter::{Language, Node, Parser};

use crate::domain::diagnostics::{Position, Range, Symbol, SymbolKind};

use super::languages::LanguageId;

/// Node kind → symbol kind rules for one language.
struct Rules {
    language: Language,
    kinds: &'static [(&'static str, SymbolKind)],
}

const RUST: &[(&str, SymbolKind)] = &[
    ("mod_item", SymbolKind::Module),
    ("struct_item", SymbolKind::Struct),
    ("enum_item", SymbolKind::Enum),
    ("trait_item", SymbolKind::Interface),
    ("impl_item", SymbolKind::Class),
    ("function_item", SymbolKind::Function),
    ("const_item", SymbolKind::Constant),
    ("static_item", SymbolKind::Constant),
    ("type_item", SymbolKind::TypeAlias),
    ("macro_definition", SymbolKind::Function),
];

const DART: &[(&str, SymbolKind)] = &[
    ("class_definition", SymbolKind::Class),
    ("mixin_declaration", SymbolKind::Class),
    ("extension_declaration", SymbolKind::Class),
    ("enum_declaration", SymbolKind::Enum),
    ("function_signature", SymbolKind::Function),
    ("method_signature", SymbolKind::Method),
    ("constructor_signature", SymbolKind::Constructor),
    ("getter_signature", SymbolKind::Property),
    ("setter_signature", SymbolKind::Property),
];

const JAVA: &[(&str, SymbolKind)] = &[
    ("class_declaration", SymbolKind::Class),
    ("interface_declaration", SymbolKind::Interface),
    ("enum_declaration", SymbolKind::Enum),
    ("record_declaration", SymbolKind::Struct),
    ("constructor_declaration", SymbolKind::Constructor),
    ("method_declaration", SymbolKind::Method),
];

const JS: &[(&str, SymbolKind)] = &[
    ("class_declaration", SymbolKind::Class),
    ("function_declaration", SymbolKind::Function),
    ("generator_function_declaration", SymbolKind::Function),
    ("method_definition", SymbolKind::Method),
];

const TS: &[(&str, SymbolKind)] = &[
    ("class_declaration", SymbolKind::Class),
    ("abstract_class_declaration", SymbolKind::Class),
    ("interface_declaration", SymbolKind::Interface),
    ("enum_declaration", SymbolKind::Enum),
    ("type_alias_declaration", SymbolKind::TypeAlias),
    ("function_declaration", SymbolKind::Function),
    ("method_definition", SymbolKind::Method),
    ("module", SymbolKind::Namespace),
];

const JSON: &[(&str, SymbolKind)] = &[("pair", SymbolKind::Key)];
const YAML: &[(&str, SymbolKind)] = &[("block_mapping_pair", SymbolKind::Key)];
const TOML: &[(&str, SymbolKind)] = &[
    ("table", SymbolKind::Namespace),
    ("table_array_element", SymbolKind::Namespace),
    ("pair", SymbolKind::Key),
];
const MARKDOWN: &[(&str, SymbolKind)] = &[
    ("atx_heading", SymbolKind::Heading),
    ("setext_heading", SymbolKind::Heading),
];
const BASH: &[(&str, SymbolKind)] = &[("function_definition", SymbolKind::Function)];

fn rules_for(language: &LanguageId) -> Option<Rules> {
    let (lang_fn, kinds) = match language.as_str() {
        "rust" => (tree_sitter_rust::LANGUAGE, RUST),
        "dart" => (tree_sitter_dart::LANGUAGE, DART),
        "java" => (tree_sitter_java::LANGUAGE, JAVA),
        "javascript" | "javascriptreact" => (tree_sitter_javascript::LANGUAGE, JS),
        "typescript" => (tree_sitter_typescript::LANGUAGE_TYPESCRIPT, TS),
        "typescriptreact" => (tree_sitter_typescript::LANGUAGE_TSX, TS),
        "json" | "jsonc" => (tree_sitter_json::LANGUAGE, JSON),
        "yaml" => (tree_sitter_yaml::LANGUAGE, YAML),
        "toml" => (tree_sitter_toml_ng::LANGUAGE, TOML),
        "markdown" => (tree_sitter_md::LANGUAGE, MARKDOWN),
        "shellscript" => (tree_sitter_bash::LANGUAGE, BASH),
        _ => return None,
    };
    Some(Rules {
        language: Language::new(lang_fn),
        kinds,
    })
}

/// Whether outline extraction is available for a language.
pub fn supports(language: &LanguageId) -> bool {
    rules_for(language).is_some()
}

/// Extracts a flat, depth-annotated symbol list from a document.
#[derive(Default)]
pub struct OutlineExtractor {
    supported: HashMap<String, bool>,
}

impl OutlineExtractor {
    pub fn new() -> OutlineExtractor {
        OutlineExtractor::default()
    }

    /// True when a grammar with outline rules exists.
    pub fn supports(&mut self, language: &LanguageId) -> bool {
        *self
            .supported
            .entry(language.as_str().to_string())
            .or_insert_with(|| rules_for(language).is_some())
    }

    /// Extract symbols; `None` means "no outline support for this language",
    /// which the UI renders as an explicit empty state.
    pub fn extract(&mut self, language: &LanguageId, text: &str) -> Option<Vec<Symbol>> {
        let rules = rules_for(language)?;
        let mut parser = Parser::new();
        parser.set_language(&rules.language).ok()?;
        let tree = parser.parse(text, None)?;

        let mut symbols = Vec::new();
        let mut cursor = tree.root_node().walk();
        walk(
            tree.root_node(),
            &mut cursor,
            text,
            rules.kinds,
            0,
            None,
            &mut symbols,
        );
        Some(symbols)
    }
}

fn walk<'a>(
    node: Node<'a>,
    cursor: &mut tree_sitter::TreeCursor<'a>,
    source: &str,
    rules: &[(&str, SymbolKind)],
    depth: usize,
    container: Option<String>,
    out: &mut Vec<Symbol>,
) {
    let mut child_depth = depth;
    let mut child_container = container.clone();

    if let Some((_, kind)) = rules.iter().find(|(k, _)| *k == node.kind()) {
        if let Some(name) = symbol_name(node, source) {
            out.push(Symbol {
                name: name.clone(),
                kind: *kind,
                range: node_range(node),
                depth,
                container,
            });
            child_depth = depth + 1;
            child_container = Some(name);
        }
    }

    let children: Vec<Node<'a>> = node.children(cursor).collect();
    for child in children {
        let mut child_cursor = child.walk();
        walk(
            child,
            &mut child_cursor,
            source,
            rules,
            child_depth,
            child_container.clone(),
            out,
        );
    }
}

/// Best-effort name for a symbol node.
fn symbol_name(node: Node<'_>, source: &str) -> Option<String> {
    // Rust impl blocks have no name field; show them the way they are written
    // so `Foo` the struct and `impl Foo` are not two identical rows.
    if node.kind() == "impl_item" {
        let type_name = node
            .child_by_field_name("type")
            .and_then(|child| node_text(child, source))
            .map(clean_name)?;
        return Some(
            match node
                .child_by_field_name("trait")
                .and_then(|child| node_text(child, source))
            {
                Some(trait_name) => {
                    clean_name(format!("impl {} for {type_name}", trait_name.trim()))
                }
                None => format!("impl {type_name}"),
            },
        );
    }
    // Most grammars expose a `name` field.
    for field in ["name", "key", "path"] {
        if let Some(child) = node.child_by_field_name(field) {
            return node_text(child, source).map(clean_name);
        }
    }
    // Markdown headings: use the inline content of the heading line.
    if node.kind().contains("heading") {
        let text = node_text(node, source)?;
        return Some(clean_name(text.trim_start_matches('#').trim().to_string()));
    }
    // Rust `impl` blocks have a `type` field instead of a name.
    if let Some(child) = node.child_by_field_name("type") {
        return node_text(child, source).map(clean_name);
    }
    // Fall back to the first identifier-ish child.
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
    for child in children {
        if child.kind().contains("identifier") || child.kind().contains("string") {
            return node_text(child, source).map(clean_name);
        }
    }
    None
}

fn clean_name(raw: String) -> String {
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'');
    let single_line = trimmed.lines().next().unwrap_or(trimmed);
    let mut name = single_line.to_string();
    const MAX: usize = 60;
    if name.chars().count() > MAX {
        name = name.chars().take(MAX - 1).collect::<String>() + "…";
    }
    name
}

fn node_text(node: Node<'_>, source: &str) -> Option<String> {
    source
        .get(node.start_byte()..node.end_byte())
        .map(str::to_string)
}

fn node_range(node: Node<'_>) -> Range {
    let start = node.start_position();
    let end = node.end_position();
    Range::new(
        Position::new(start.row, start.column),
        Position::new(end.row, end.column),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impl_blocks_are_labelled_as_written() {
        let mut extractor = OutlineExtractor::new();
        let src = "struct Widget;\nimpl Widget { fn new() {} }\nimpl Clone for Widget { fn clone(&self) -> Self { Widget } }\n";
        let symbols = extractor.extract(&LanguageId::new("rust"), src).unwrap();
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Widget"), "{names:?}");
        assert!(names.contains(&"impl Widget"), "{names:?}");
        assert!(names.contains(&"impl Clone for Widget"), "{names:?}");
    }

    #[test]
    fn extracts_rust_structure_with_nesting() {
        let mut extractor = OutlineExtractor::new();
        let src = r#"
mod app {
    pub struct State { count: usize }

    impl State {
        pub fn new() -> Self { Self { count: 0 } }
    }
}
"#;
        let symbols = extractor.extract(&LanguageId::new("rust"), src).unwrap();
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"app"), "{names:?}");
        assert!(names.contains(&"State"), "{names:?}");
        assert!(names.contains(&"new"), "{names:?}");

        let new_fn = symbols.iter().find(|s| s.name == "new").unwrap();
        assert_eq!(new_fn.kind, SymbolKind::Function);
        assert!(new_fn.depth >= 2, "nested symbols keep their depth");
        assert_eq!(new_fn.container.as_deref(), Some("impl State"));
    }

    #[test]
    fn extracts_markdown_headings() {
        let mut extractor = OutlineExtractor::new();
        let symbols = extractor
            .extract(&LanguageId::new("markdown"), "# Title\n\n## Section\n")
            .unwrap();
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Title"), "{names:?}");
        assert!(names.contains(&"Section"), "{names:?}");
    }

    #[test]
    fn extracts_typescript_classes_and_methods() {
        let mut extractor = OutlineExtractor::new();
        let src = "export class Service {\n  run(): void {}\n}\ninterface Props { a: number }\n";
        let symbols = extractor
            .extract(&LanguageId::new("typescript"), src)
            .unwrap();
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Service"), "{names:?}");
        assert!(names.contains(&"run"), "{names:?}");
        assert!(names.contains(&"Props"), "{names:?}");
    }

    #[test]
    fn unsupported_language_returns_none() {
        let mut extractor = OutlineExtractor::new();
        assert!(extractor
            .extract(&LanguageId::new("plaintext"), "hello")
            .is_none());
        assert!(!extractor.supports(&LanguageId::new("plaintext")));
    }

    #[test]
    fn symbol_positions_point_at_the_declaration() {
        let mut extractor = OutlineExtractor::new();
        let symbols = extractor
            .extract(&LanguageId::new("rust"), "fn a() {}\nfn b() {}\n")
            .unwrap();
        assert_eq!(symbols[0].range.start.line, 0);
        assert_eq!(symbols[1].range.start.line, 1);
    }

    #[test]
    fn broken_source_does_not_panic() {
        let mut extractor = OutlineExtractor::new();
        let symbols = extractor
            .extract(&LanguageId::new("rust"), "fn broken( { { {")
            .unwrap();
        // Tree-sitter recovers; we only require that extraction is safe.
        let _ = symbols;
    }
}
