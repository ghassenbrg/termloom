//! VS Code extension manifests (`package.json`).
//!
//! Parsing is deliberately tolerant: an unknown or malformed contribution
//! must not make the whole package unreadable, because the inspector's job is
//! to report what is there, including the parts TermLoom cannot use.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde_json::Value;

/// The parts of a manifest TermLoom cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionManifest {
    pub name: String,
    pub publisher: String,
    pub display_name: String,
    pub version: String,
    pub description: Option<String>,
    /// `engines.vscode`, e.g. `^1.75.0`.
    pub engine: Option<String>,
    /// Node entrypoint. Recorded so we can report that it will not be run.
    pub main: Option<String>,
    /// Web entrypoint, same treatment.
    pub browser: Option<String>,
    pub activation_events: Vec<String>,
    pub categories: Vec<String>,
    /// Raw `contributes` object, inspected by the classifier.
    pub contributes: Value,
}

impl ExtensionManifest {
    /// `publisher.name`, the canonical extension id.
    pub fn id(&self) -> String {
        if self.publisher.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.publisher, self.name)
        }
    }

    /// Parse a `package.json`.
    pub fn parse(text: &str) -> Result<ExtensionManifest> {
        let value: Value = serde_json::from_str(text)
            .map_err(|err| anyhow!("package.json is not valid JSON: {err}"))?;
        let name = value["name"]
            .as_str()
            .ok_or_else(|| anyhow!("package.json has no `name`"))?
            .to_string();
        let version = value["version"]
            .as_str()
            .ok_or_else(|| anyhow!("package.json has no `version`"))?
            .to_string();

        Ok(ExtensionManifest {
            display_name: value["displayName"].as_str().unwrap_or(&name).to_string(),
            publisher: value["publisher"].as_str().unwrap_or_default().to_string(),
            name,
            version,
            description: value["description"].as_str().map(str::to_string),
            engine: value["engines"]["vscode"].as_str().map(str::to_string),
            main: value["main"].as_str().map(str::to_string),
            browser: value["browser"].as_str().map(str::to_string),
            activation_events: string_array(&value["activationEvents"]),
            categories: string_array(&value["categories"]),
            contributes: value
                .get("contributes")
                .cloned()
                .unwrap_or(Value::Object(Default::default())),
        })
    }

    /// Contribution keys present in the manifest, sorted.
    pub fn contribution_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .contributes
            .as_object()
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort();
        keys
    }

    /// True when the package ships executable extension code.
    pub fn has_entrypoint(&self) -> bool {
        self.main.is_some() || self.browser.is_some()
    }

    /// Language contributions, if any.
    pub fn languages(&self) -> Vec<LanguageContribution> {
        self.contributes["languages"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(LanguageContribution {
                            id: item["id"].as_str()?.to_string(),
                            aliases: string_array(&item["aliases"]),
                            extensions: string_array(&item["extensions"]),
                            filenames: string_array(&item["filenames"]),
                            configuration: item["configuration"].as_str().map(PathBuf::from),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn grammars(&self) -> Vec<GrammarContribution> {
        self.contributes["grammars"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(GrammarContribution {
                            language: item["language"].as_str().map(str::to_string),
                            scope_name: item["scopeName"].as_str()?.to_string(),
                            path: PathBuf::from(item["path"].as_str()?),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn snippets(&self) -> Vec<SnippetContribution> {
        self.contributes["snippets"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(SnippetContribution {
                            language: item["language"].as_str()?.to_string(),
                            path: PathBuf::from(item["path"].as_str()?),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn themes(&self) -> Vec<ThemeContribution> {
        self.contributes["themes"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(ThemeContribution {
                            label: item["label"].as_str().unwrap_or("theme").to_string(),
                            dark: item["uiTheme"].as_str().unwrap_or("vs-dark") != "vs",
                            path: PathBuf::from(item["path"].as_str()?),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn debuggers(&self) -> Vec<DebuggerContribution> {
        self.contributes["debuggers"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(DebuggerContribution {
                            type_name: item["type"].as_str()?.to_string(),
                            label: item["label"].as_str().unwrap_or_default().to_string(),
                            program: item["program"].as_str().map(PathBuf::from),
                            runtime: item["runtime"].as_str().map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Language server metadata is not a standard contribution point, but
    /// many extensions declare one in `configuration` or in their README.
    /// Anything we cannot resolve safely is reported as needing adaptation.
    pub fn declares_language_server(&self) -> bool {
        self.categories
            .iter()
            .any(|category| category.eq_ignore_ascii_case("programming languages"))
            && self.has_entrypoint()
    }
}

fn string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageContribution {
    pub id: String,
    pub aliases: Vec<String>,
    pub extensions: Vec<String>,
    pub filenames: Vec<String>,
    pub configuration: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarContribution {
    pub language: Option<String>,
    pub scope_name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetContribution {
    pub language: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeContribution {
    pub label: String,
    pub dark: bool,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebuggerContribution {
    pub type_name: String,
    pub label: String,
    pub program: Option<PathBuf>,
    pub runtime: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const DART_LIKE: &str = r#"{
      "name": "dart-code",
      "publisher": "Dart-Code",
      "displayName": "Dart",
      "version": "3.96.0",
      "description": "Dart language support",
      "engines": { "vscode": "^1.75.0" },
      "main": "./out/extension",
      "categories": ["Programming Languages", "Debuggers"],
      "activationEvents": ["onLanguage:dart"],
      "contributes": {
        "languages": [{
          "id": "dart",
          "aliases": ["Dart"],
          "extensions": [".dart"],
          "configuration": "./syntaxes/dart-language-configuration.json"
        }],
        "grammars": [{
          "language": "dart",
          "scopeName": "source.dart",
          "path": "./syntaxes/dart.json"
        }],
        "snippets": [{ "language": "dart", "path": "./snippets/dart.json" }],
        "themes": [{ "label": "Dart Dark", "uiTheme": "vs-dark", "path": "./themes/dark.json" }],
        "debuggers": [{ "type": "dart", "label": "Dart", "program": "./out/dap.js", "runtime": "node" }],
        "commands": [{ "command": "dart.restart", "title": "Restart" }],
        "views": { "debug": [] }
      }
    }"#;

    #[test]
    fn parses_identity_and_contributions() {
        let manifest = ExtensionManifest::parse(DART_LIKE).unwrap();
        assert_eq!(manifest.id(), "Dart-Code.dart-code");
        assert_eq!(manifest.display_name, "Dart");
        assert_eq!(manifest.version, "3.96.0");
        assert_eq!(manifest.engine.as_deref(), Some("^1.75.0"));
        assert!(manifest.has_entrypoint());
        assert_eq!(
            manifest.contribution_keys(),
            vec![
                "commands",
                "debuggers",
                "grammars",
                "languages",
                "snippets",
                "themes",
                "views"
            ]
        );
    }

    #[test]
    fn extracts_declarative_assets() {
        let manifest = ExtensionManifest::parse(DART_LIKE).unwrap();
        let languages = manifest.languages();
        assert_eq!(languages[0].id, "dart");
        assert_eq!(languages[0].extensions, vec![".dart"]);
        assert_eq!(manifest.grammars()[0].scope_name, "source.dart");
        assert_eq!(manifest.snippets()[0].language, "dart");
        assert!(manifest.themes()[0].dark);
        let debuggers = manifest.debuggers();
        assert_eq!(debuggers[0].type_name, "dart");
        assert_eq!(debuggers[0].runtime.as_deref(), Some("node"));
    }

    #[test]
    fn a_manifest_without_contributions_is_still_valid() {
        let manifest = ExtensionManifest::parse(r#"{"name":"x","version":"1.0.0"}"#).unwrap();
        assert_eq!(manifest.id(), "x");
        assert!(manifest.contribution_keys().is_empty());
        assert!(!manifest.has_entrypoint());
    }

    #[test]
    fn missing_required_fields_are_errors() {
        assert!(ExtensionManifest::parse(r#"{"version":"1"}"#).is_err());
        assert!(ExtensionManifest::parse("not json").is_err());
    }

    #[test]
    fn malformed_contributions_are_skipped_not_fatal() {
        let manifest = ExtensionManifest::parse(
            r#"{"name":"x","version":"1","contributes":{"grammars":[{"scopeName":"a"},{"noPath":true}]}}"#,
        )
        .unwrap();
        // The entry without a `path` is dropped; the rest survives.
        assert!(manifest.grammars().is_empty());
    }
}
