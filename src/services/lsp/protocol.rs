//! LSP payload construction and conversion into TermLoom's own types.
//!
//! Nothing above this module sees `serde_json::Value`: requests go in as
//! domain values and results come out as [`crate::domain::diagnostics`] types.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::domain::diagnostics::{
    Diagnostic, Location, Position, Range, Severity, Symbol, SymbolKind,
};

/// What a server said it can do. Commands are only offered when the matching
/// flag is set, so the UI never advertises a capability the server lacks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServerCapabilities {
    pub hover: bool,
    pub completion: bool,
    pub definition: bool,
    pub references: bool,
    pub document_symbols: bool,
    pub workspace_symbols: bool,
    pub rename: bool,
    pub formatting: bool,
    /// 0 = none, 1 = full, 2 = incremental.
    pub text_document_sync: u8,
    pub signature_help: bool,
}

impl ServerCapabilities {
    /// Parse the `capabilities` object from an `initialize` result.
    pub fn from_initialize(result: &Value) -> ServerCapabilities {
        let capabilities = &result["capabilities"];
        let flag = |key: &str| -> bool {
            match &capabilities[key] {
                Value::Bool(value) => *value,
                Value::Object(_) => true,
                _ => false,
            }
        };
        let sync = match &capabilities["textDocumentSync"] {
            Value::Number(n) => n.as_u64().unwrap_or(0) as u8,
            Value::Object(object) => {
                object.get("change").and_then(Value::as_u64).unwrap_or(0) as u8
            }
            _ => 0,
        };
        ServerCapabilities {
            hover: flag("hoverProvider"),
            completion: flag("completionProvider"),
            definition: flag("definitionProvider"),
            references: flag("referencesProvider"),
            document_symbols: flag("documentSymbolProvider"),
            workspace_symbols: flag("workspaceSymbolProvider"),
            rename: flag("renameProvider"),
            formatting: flag("documentFormattingProvider"),
            text_document_sync: sync,
            signature_help: flag("signatureHelpProvider"),
        }
    }
}

/// `file:///…` URI for a path.
pub fn path_to_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Inverse of [`path_to_uri`]. Returns `None` for non-file URIs.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut decoded = Vec::new();
    let bytes = rest.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Some(PathBuf::from(String::from_utf8(decoded).ok()?))
}

/// Client capabilities advertised by TermLoom: only what it really implements.
pub fn client_capabilities() -> Value {
    json!({
        "workspace": {
            "workspaceFolders": true,
            "configuration": true,
            "didChangeConfiguration": { "dynamicRegistration": false },
            "symbol": { "dynamicRegistration": false }
        },
        "textDocument": {
            "synchronization": {
                "dynamicRegistration": false,
                "didSave": true,
                "willSave": false,
                "willSaveWaitUntil": false
            },
            "hover": { "contentFormat": ["plaintext", "markdown"] },
            "completion": {
                "completionItem": {
                    "snippetSupport": false,
                    "documentationFormat": ["plaintext"]
                },
                "contextSupport": false
            },
            "definition": { "linkSupport": false },
            "references": {},
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
            "rename": { "prepareSupport": false },
            "formatting": {},
            "publishDiagnostics": { "relatedInformation": false }
        },
        "general": { "positionEncodings": ["utf-16", "utf-8"] }
    })
}

/// `initialize` parameters.
pub fn initialize_params(root: &Path, initialization_options: Option<Value>) -> Value {
    let uri = path_to_uri(root);
    json!({
        "processId": std::process::id(),
        "clientInfo": { "name": crate::PRODUCT_NAME, "version": crate::VERSION },
        "rootUri": uri,
        "rootPath": root.to_string_lossy(),
        "workspaceFolders": [{
            "uri": uri,
            "name": root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
        }],
        "capabilities": client_capabilities(),
        "initializationOptions": initialization_options.unwrap_or(Value::Null)
    })
}

pub fn did_open(path: &Path, language_id: &str, version: i64, text: &str) -> Value {
    json!({
        "textDocument": {
            "uri": path_to_uri(path),
            "languageId": language_id,
            "version": version,
            "text": text
        }
    })
}

/// Full-document sync: one change event carrying the whole text.
pub fn did_change_full(path: &Path, version: i64, text: &str) -> Value {
    json!({
        "textDocument": { "uri": path_to_uri(path), "version": version },
        "contentChanges": [{ "text": text }]
    })
}

pub fn did_save(path: &Path, text: Option<&str>) -> Value {
    match text {
        Some(text) => json!({
            "textDocument": { "uri": path_to_uri(path) },
            "text": text
        }),
        None => json!({ "textDocument": { "uri": path_to_uri(path) } }),
    }
}

pub fn did_close(path: &Path) -> Value {
    json!({ "textDocument": { "uri": path_to_uri(path) } })
}

/// `{ textDocument, position }` — the shape most requests share.
pub fn text_document_position(path: &Path, position: Position) -> Value {
    json!({
        "textDocument": { "uri": path_to_uri(path) },
        "position": { "line": position.line, "character": position.character }
    })
}

pub fn references_params(path: &Path, position: Position, include_declaration: bool) -> Value {
    let mut params = text_document_position(path, position);
    params["context"] = json!({ "includeDeclaration": include_declaration });
    params
}

pub fn rename_params(path: &Path, position: Position, new_name: &str) -> Value {
    let mut params = text_document_position(path, position);
    params["newName"] = json!(new_name);
    params
}

pub fn formatting_params(path: &Path, tab_size: usize, insert_spaces: bool) -> Value {
    json!({
        "textDocument": { "uri": path_to_uri(path) },
        "options": {
            "tabSize": tab_size,
            "insertSpaces": insert_spaces,
            "trimTrailingWhitespace": false,
            "insertFinalNewline": false
        }
    })
}

// ── result conversion ─────────────────────────────────────────────────────

fn parse_position(value: &Value) -> Position {
    Position::new(
        value["line"].as_u64().unwrap_or(0) as usize,
        value["character"].as_u64().unwrap_or(0) as usize,
    )
}

pub fn parse_range(value: &Value) -> Range {
    Range::new(
        parse_position(&value["start"]),
        parse_position(&value["end"]),
    )
}

/// Convert a `textDocument/publishDiagnostics` notification.
pub fn parse_diagnostics(params: &Value) -> Option<(PathBuf, Vec<Diagnostic>)> {
    let path = uri_to_path(params["uri"].as_str()?)?;
    let diagnostics = params["diagnostics"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| Diagnostic {
                    path: path.clone(),
                    range: parse_range(&item["range"]),
                    severity: Severity::from_lsp(item["severity"].as_i64()),
                    message: item["message"].as_str().unwrap_or_default().to_string(),
                    source: item["source"].as_str().map(str::to_string),
                    code: match &item["code"] {
                        Value::String(code) => Some(code.clone()),
                        Value::Number(code) => Some(code.to_string()),
                        _ => None,
                    },
                })
                .collect()
        })
        .unwrap_or_default();
    Some((path, diagnostics))
}

/// Convert `Location`, `Location[]`, or `LocationLink[]` results.
pub fn parse_locations(result: &Value) -> Vec<Location> {
    fn single(value: &Value) -> Option<Location> {
        // LocationLink uses `targetUri`/`targetSelectionRange`.
        let uri = value["uri"]
            .as_str()
            .or_else(|| value["targetUri"].as_str())?;
        let range = if value.get("range").is_some() {
            parse_range(&value["range"])
        } else if value.get("targetSelectionRange").is_some() {
            parse_range(&value["targetSelectionRange"])
        } else {
            parse_range(&value["targetRange"])
        };
        Some(Location {
            path: uri_to_path(uri)?,
            range,
        })
    }

    match result {
        Value::Array(items) => items.iter().filter_map(single).collect(),
        Value::Object(_) => single(result).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// Convert hover contents (string, MarkupContent or the legacy array form).
pub fn parse_hover(result: &Value) -> Vec<String> {
    fn text_of(value: &Value) -> Option<String> {
        match value {
            Value::String(text) => Some(text.clone()),
            Value::Object(object) => object
                .get("value")
                .and_then(Value::as_str)
                .map(str::to_string),
            _ => None,
        }
    }
    let contents = &result["contents"];
    let raw = match contents {
        Value::Array(items) => items
            .iter()
            .filter_map(text_of)
            .collect::<Vec<_>>()
            .join("\n"),
        other => text_of(other).unwrap_or_default(),
    };
    raw.lines()
        .map(|line| line.trim_end().to_string())
        .filter(|line| !line.starts_with("```"))
        .collect()
}

/// One completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub detail: Option<String>,
    /// Text actually inserted (falls back to the label).
    pub insert_text: String,
    pub kind: Option<String>,
}

const COMPLETION_KINDS: &[&str] = &[
    "",
    "text",
    "method",
    "function",
    "constructor",
    "field",
    "variable",
    "class",
    "interface",
    "module",
    "property",
    "unit",
    "value",
    "enum",
    "keyword",
    "snippet",
    "color",
    "file",
    "reference",
    "folder",
    "enum member",
    "constant",
    "struct",
    "event",
    "operator",
    "type parameter",
];

pub fn parse_completion(result: &Value) -> Vec<CompletionItem> {
    let items = match result {
        Value::Array(items) => items.clone(),
        Value::Object(object) => object
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            let label = item["label"].as_str()?.to_string();
            let insert_text = item["insertText"]
                .as_str()
                .or_else(|| item["textEdit"]["newText"].as_str())
                .unwrap_or(&label)
                .to_string();
            let kind = item["kind"]
                .as_u64()
                .and_then(|k| COMPLETION_KINDS.get(k as usize))
                .filter(|k| !k.is_empty())
                .map(|k| (*k).to_string());
            Some(CompletionItem {
                label,
                detail: item["detail"].as_str().map(str::to_string),
                insert_text,
                kind,
            })
        })
        .collect()
}

/// Convert `documentSymbol` results (hierarchical or flat).
pub fn parse_symbols(result: &Value) -> Vec<Symbol> {
    let mut out = Vec::new();
    let Some(items) = result.as_array() else {
        return out;
    };
    for item in items {
        if item.get("location").is_some() {
            // SymbolInformation (flat).
            out.push(Symbol {
                name: item["name"].as_str().unwrap_or_default().to_string(),
                kind: SymbolKind::from_lsp(item["kind"].as_i64().unwrap_or(0)),
                range: parse_range(&item["location"]["range"]),
                depth: 0,
                container: item["containerName"].as_str().map(str::to_string),
            });
        } else {
            push_document_symbol(item, 0, None, &mut out);
        }
    }
    out
}

fn push_document_symbol(
    item: &Value,
    depth: usize,
    container: Option<String>,
    out: &mut Vec<Symbol>,
) {
    let name = item["name"].as_str().unwrap_or_default().to_string();
    let range = if item.get("selectionRange").is_some() {
        parse_range(&item["selectionRange"])
    } else {
        parse_range(&item["range"])
    };
    out.push(Symbol {
        name: name.clone(),
        kind: SymbolKind::from_lsp(item["kind"].as_i64().unwrap_or(0)),
        range,
        depth,
        container,
    });
    if let Some(children) = item["children"].as_array() {
        for child in children {
            push_document_symbol(child, depth + 1, Some(name.clone()), out);
        }
    }
}

/// A text edit returned by formatting or rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
}

pub fn parse_text_edits(result: &Value) -> Vec<TextEdit> {
    result
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| TextEdit {
                    range: parse_range(&item["range"]),
                    new_text: item["newText"].as_str().unwrap_or_default().to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Convert a `WorkspaceEdit` into per-file edits.
pub fn parse_workspace_edit(result: &Value) -> Vec<(PathBuf, Vec<TextEdit>)> {
    let mut out = Vec::new();
    if let Some(changes) = result["changes"].as_object() {
        for (uri, edits) in changes {
            if let Some(path) = uri_to_path(uri) {
                out.push((path, parse_text_edits(edits)));
            }
        }
    }
    if let Some(document_changes) = result["documentChanges"].as_array() {
        for change in document_changes {
            let Some(uri) = change["textDocument"]["uri"].as_str() else {
                continue;
            };
            if let Some(path) = uri_to_path(uri) {
                out.push((path, parse_text_edits(&change["edits"])));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uris_round_trip_including_spaces() {
        let path = PathBuf::from("/tmp/my project/src/main.rs");
        let uri = path_to_uri(&path);
        assert!(uri.starts_with("file:///tmp/my%20project"), "{uri}");
        assert_eq!(uri_to_path(&uri), Some(path));
        assert_eq!(uri_to_path("http://example.com"), None);
    }

    #[test]
    fn capabilities_are_parsed_conservatively() {
        let result = json!({
            "capabilities": {
                "hoverProvider": true,
                "completionProvider": { "triggerCharacters": ["."] },
                "definitionProvider": true,
                "renameProvider": false,
                "textDocumentSync": { "openClose": true, "change": 2 }
            }
        });
        let capabilities = ServerCapabilities::from_initialize(&result);
        assert!(capabilities.hover);
        assert!(capabilities.completion);
        assert!(capabilities.definition);
        assert!(!capabilities.rename, "absent means unsupported");
        assert!(!capabilities.formatting);
        assert_eq!(capabilities.text_document_sync, 2);
    }

    #[test]
    fn diagnostics_convert_to_domain_values() {
        let params = json!({
            "uri": "file:///repo/src/main.rs",
            "diagnostics": [{
                "range": {"start":{"line":3,"character":4},"end":{"line":3,"character":9}},
                "severity": 1,
                "message": "cannot find value `x`",
                "source": "rustc",
                "code": "E0425"
            }]
        });
        let (path, diagnostics) = parse_diagnostics(&params).unwrap();
        assert_eq!(path, PathBuf::from("/repo/src/main.rs"));
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(diagnostics[0].range.start.line, 3);
        assert_eq!(diagnostics[0].source.as_deref(), Some("rustc"));
        assert_eq!(diagnostics[0].code.as_deref(), Some("E0425"));
    }

    #[test]
    fn locations_accept_all_three_result_shapes() {
        let single = json!({
            "uri": "file:///a.rs",
            "range": {"start":{"line":1,"character":0},"end":{"line":1,"character":5}}
        });
        assert_eq!(parse_locations(&single).len(), 1);

        let array = json!([single]);
        assert_eq!(parse_locations(&array).len(), 1);

        let links = json!([{
            "targetUri": "file:///b.rs",
            "targetRange": {"start":{"line":2,"character":0},"end":{"line":2,"character":1}},
            "targetSelectionRange": {"start":{"line":2,"character":4},"end":{"line":2,"character":8}}
        }]);
        let parsed = parse_locations(&links);
        assert_eq!(parsed[0].path, PathBuf::from("/b.rs"));
        assert_eq!(parsed[0].range.start.character, 4);

        assert!(parse_locations(&Value::Null).is_empty());
    }

    #[test]
    fn hover_handles_markup_and_legacy_shapes() {
        let markup = json!({"contents": {"kind":"markdown","value":"```rust\nfn a()\n```\ndocs"}});
        assert_eq!(parse_hover(&markup), vec!["fn a()", "docs"]);

        let legacy = json!({"contents": ["one", {"value": "two"}]});
        assert_eq!(parse_hover(&legacy), vec!["one", "two"]);
    }

    #[test]
    fn completion_handles_lists_and_insert_text() {
        let result = json!({
            "isIncomplete": false,
            "items": [
                {"label":"push","kind":2,"detail":"fn push(&mut self)"},
                {"label":"pop","insertText":"pop()"}
            ]
        });
        let items = parse_completion(&result);
        assert_eq!(items[0].label, "push");
        assert_eq!(items[0].kind.as_deref(), Some("method"));
        assert_eq!(items[0].insert_text, "push", "falls back to the label");
        assert_eq!(items[1].insert_text, "pop()");
    }

    #[test]
    fn hierarchical_symbols_keep_their_depth() {
        let result = json!([{
            "name": "App",
            "kind": 5,
            "range": {"start":{"line":0,"character":0},"end":{"line":10,"character":0}},
            "selectionRange": {"start":{"line":0,"character":6},"end":{"line":0,"character":9}},
            "children": [{
                "name": "run",
                "kind": 6,
                "range": {"start":{"line":2,"character":0},"end":{"line":4,"character":0}},
                "selectionRange": {"start":{"line":2,"character":4},"end":{"line":2,"character":7}}
            }]
        }]);
        let symbols = parse_symbols(&result);
        assert_eq!(symbols[0].name, "App");
        assert_eq!(symbols[0].kind, SymbolKind::Class);
        assert_eq!(symbols[1].name, "run");
        assert_eq!(symbols[1].depth, 1);
        assert_eq!(symbols[1].container.as_deref(), Some("App"));
    }

    #[test]
    fn flat_symbol_information_is_supported() {
        let result = json!([{
            "name": "main",
            "kind": 12,
            "location": {
                "uri": "file:///a.rs",
                "range": {"start":{"line":0,"character":0},"end":{"line":1,"character":0}}
            }
        }]);
        let symbols = parse_symbols(&result);
        assert_eq!(symbols[0].name, "main");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn workspace_edits_use_both_encodings() {
        let changes = json!({
            "changes": {
                "file:///a.rs": [{
                    "range": {"start":{"line":0,"character":0},"end":{"line":0,"character":3}},
                    "newText": "new"
                }]
            }
        });
        let edits = parse_workspace_edit(&changes);
        assert_eq!(edits[0].0, PathBuf::from("/a.rs"));
        assert_eq!(edits[0].1[0].new_text, "new");

        let document_changes = json!({
            "documentChanges": [{
                "textDocument": {"uri":"file:///b.rs","version":1},
                "edits": [{
                    "range": {"start":{"line":1,"character":0},"end":{"line":1,"character":1}},
                    "newText": "x"
                }]
            }]
        });
        assert_eq!(
            parse_workspace_edit(&document_changes)[0].0,
            PathBuf::from("/b.rs")
        );
    }

    #[test]
    fn initialize_params_declare_the_workspace_root() {
        let params = initialize_params(Path::new("/repo"), None);
        assert_eq!(params["rootUri"], "file:///repo");
        assert_eq!(params["workspaceFolders"][0]["name"], "repo");
        assert!(params["capabilities"]["textDocument"]["hover"].is_object());
    }
}
