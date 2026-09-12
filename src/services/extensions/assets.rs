//! Load the declarative extension assets TermLoom can consume directly.

use std::collections::HashMap;

use crate::domain::extensions::{CompatibilityReport, ExtensionAsset};
use crate::services::lsp::CompletionItem;

/// Load VS Code snippet JSON into native completion candidates.
///
/// Invalid files are skipped and logged; one damaged extension cannot prevent
/// the workbench from starting.
pub fn load_snippets(reports: &[CompatibilityReport]) -> HashMap<String, Vec<CompletionItem>> {
    let mut snippets: HashMap<String, Vec<CompletionItem>> = HashMap::new();
    for report in reports {
        let Some(root) = report.install_path.as_ref() else {
            continue;
        };
        for asset in &report.assets {
            let ExtensionAsset::Snippets { language, path } = asset else {
                continue;
            };
            let path = root.join("extension").join(path);
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "could not read extension snippets");
                    continue;
                }
            };
            let value = match crate::services::vscode::parse_jsonc(&text) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "could not parse extension snippets");
                    continue;
                }
            };
            let Some(definitions) = value.as_object() else {
                continue;
            };
            for (name, definition) in definitions {
                let prefixes: Vec<&str> = match &definition["prefix"] {
                    serde_json::Value::String(prefix) => vec![prefix],
                    serde_json::Value::Array(prefixes) => {
                        prefixes.iter().filter_map(|value| value.as_str()).collect()
                    }
                    _ => Vec::new(),
                };
                let body = match &definition["body"] {
                    serde_json::Value::String(body) => body.clone(),
                    serde_json::Value::Array(lines) => lines
                        .iter()
                        .filter_map(|value| value.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => continue,
                };
                let insert_text = expand_placeholders(&body);
                let detail = definition["description"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| Some(format!("{name} · {}", report.display_name)));
                for prefix in prefixes {
                    snippets
                        .entry(language.clone())
                        .or_default()
                        .push(CompletionItem {
                            label: prefix.to_string(),
                            detail: detail.clone(),
                            insert_text: insert_text.clone(),
                            kind: Some("snippet".into()),
                        });
                }
            }
        }
    }
    for items in snippets.values_mut() {
        items.sort_by(|a, b| a.label.cmp(&b.label));
        items.dedup_by(|a, b| a.label == b.label && a.insert_text == b.insert_text);
    }
    snippets
}

/// Convert the common VS Code tabstop forms to deterministic plain text. V1
/// inserts the first/default value and does not implement linked tabstops.
fn expand_placeholders(body: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    let mut output = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\\' && chars.get(index + 1) == Some(&'$') {
            output.push('$');
            index += 2;
            continue;
        }
        if chars[index] != '$' {
            output.push(chars[index]);
            index += 1;
            continue;
        }
        if chars.get(index + 1) == Some(&'{') {
            let Some(relative_end) = chars[index + 2..].iter().position(|ch| *ch == '}') else {
                output.push('$');
                index += 1;
                continue;
            };
            let end = index + 2 + relative_end;
            let inside: String = chars[index + 2..end].iter().collect();
            if let Some((_, default)) = inside.split_once(':') {
                output.push_str(default);
            } else if let Some((_, choices)) = inside.split_once('|') {
                output.push_str(
                    choices
                        .trim_end_matches('|')
                        .split(',')
                        .next()
                        .unwrap_or(""),
                );
            } else if !inside.chars().all(|ch| ch.is_ascii_digit()) {
                // Environment-style variables are not evaluated.
                output.push_str(&format!("${{{inside}}}"));
            }
            index = end + 1;
            continue;
        }
        let mut end = index + 1;
        while chars.get(end).is_some_and(|ch| ch.is_ascii_digit()) {
            end += 1;
        }
        if end == index + 1 {
            output.push('$');
            index += 1;
        } else {
            index = end;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tabstops_without_evaluating_variables() {
        assert_eq!(
            expand_placeholders("fn ${1:name}(${2|a,b|}) { $0 } ${TM_FILE}"),
            "fn name(a) {  } ${TM_FILE}"
        );
        assert_eq!(expand_placeholders(r"\$1 = $1"), "$1 = ");
    }
}
