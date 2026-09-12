//! VS Code configuration compatibility.
//!
//! TermLoom reads the small, unambiguous parts of a repository's `.vscode`
//! directory: editor settings it can honour, launch configurations for the
//! debug adapter, and tasks the user can run explicitly. Nothing here ever
//! executes anything by itself, and unknown settings are reported rather than
//! reinterpreted.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use crate::config::Config;

/// Parse JSON with comments and trailing commas (the `.vscode` dialect).
pub fn parse_jsonc(text: &str) -> Result<Value> {
    Ok(serde_json::from_str(&strip_jsonc(text))?)
}

/// Remove comments and trailing commas, preserving string contents.
pub fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' => match chars.peek() {
                Some('/') => {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    chars.next();
                    let mut previous = '\0';
                    for c in chars.by_ref() {
                        if previous == '*' && c == '/' {
                            break;
                        }
                        previous = c;
                    }
                }
                _ => out.push(c),
            },
            _ => out.push(c),
        }
    }

    remove_trailing_commas(&out)
}

fn remove_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    let chars: Vec<char> = text.chars().collect();

    for (index, c) in chars.iter().enumerate() {
        if in_string {
            out.push(*c);
            if escaped {
                escaped = false;
            } else if *c == '\\' {
                escaped = true;
            } else if *c == '"' {
                in_string = false;
            }
            continue;
        }
        if *c == '"' {
            in_string = true;
            out.push(*c);
            continue;
        }
        if *c == ',' {
            // Look ahead: a comma before `}` or `]` is a trailing comma.
            let next = chars[index + 1..]
                .iter()
                .find(|c| !c.is_whitespace())
                .copied();
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        out.push(*c);
    }
    out
}

/// Editor settings TermLoom can honour, plus what it had to ignore.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VsCodeSettings {
    pub tab_size: Option<usize>,
    pub insert_spaces: Option<bool>,
    pub trim_trailing_whitespace: Option<bool>,
    pub insert_final_newline: Option<bool>,
    /// Glob-ish entries from `files.exclude` whose value is `true`.
    pub files_exclude: Vec<String>,
    /// Settings that were present but are not applicable in a terminal IDE.
    pub ignored: Vec<String>,
}

/// Read `.vscode/settings.json`.
pub fn read_settings(root: &Path) -> VsCodeSettings {
    let path = root.join(".vscode").join("settings.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return VsCodeSettings::default();
    };
    match parse_jsonc(&text) {
        Ok(value) => settings_from_value(&value),
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "could not parse settings.json");
            VsCodeSettings::default()
        }
    }
}

/// Apply the unambiguous editor/workspace settings TermLoom understands.
/// Explicit non-default TermLoom values win over VS Code imports.
pub fn apply_settings(config: &mut Config, settings: &VsCodeSettings) {
    let defaults = Config::default();
    if config.editor.tab_width == defaults.editor.tab_width {
        if let Some(value) = settings.tab_size {
            config.editor.tab_width = value.max(1);
        }
    }
    if config.editor.insert_spaces == defaults.editor.insert_spaces {
        if let Some(value) = settings.insert_spaces {
            config.editor.insert_spaces = value;
        }
    }
    if config.editor.trim_trailing_whitespace == defaults.editor.trim_trailing_whitespace {
        if let Some(value) = settings.trim_trailing_whitespace {
            config.editor.trim_trailing_whitespace = value;
        }
    }
    if config.editor.insert_final_newline == defaults.editor.insert_final_newline {
        if let Some(value) = settings.insert_final_newline {
            config.editor.insert_final_newline = value;
        }
    }
    for pattern in &settings.files_exclude {
        // The workspace scanner accepts directory names. Import common VS Code
        // forms such as `**/coverage` without pretending to implement its full
        // glob language.
        let name = pattern
            .strip_prefix("**/")
            .unwrap_or(pattern)
            .trim_end_matches("/**");
        if !name.is_empty()
            && !name.contains(['*', '?', '[', ']'])
            && !config.workspace.ignore.iter().any(|entry| entry == name)
        {
            config.workspace.ignore.push(name.to_string());
        }
    }
}

/// Settings keys TermLoom knows how to apply.
const SUPPORTED_SETTINGS: &[&str] = &[
    "editor.tabSize",
    "editor.insertSpaces",
    "files.trimTrailingWhitespace",
    "files.insertFinalNewline",
    "files.exclude",
];

pub fn settings_from_value(value: &Value) -> VsCodeSettings {
    let mut settings = VsCodeSettings {
        tab_size: value["editor.tabSize"].as_u64().map(|v| v as usize),
        insert_spaces: value["editor.insertSpaces"].as_bool(),
        trim_trailing_whitespace: value["files.trimTrailingWhitespace"].as_bool(),
        insert_final_newline: value["files.insertFinalNewline"].as_bool(),
        files_exclude: value["files.exclude"]
            .as_object()
            .map(|excludes| {
                excludes
                    .iter()
                    .filter(|(_, enabled)| enabled.as_bool().unwrap_or(false))
                    .map(|(pattern, _)| pattern.clone())
                    .collect()
            })
            .unwrap_or_default(),
        ignored: Vec::new(),
    };

    if let Some(map) = value.as_object() {
        settings.ignored = map
            .keys()
            .filter(|key| !SUPPORTED_SETTINGS.contains(&key.as_str()))
            .cloned()
            .collect();
        settings.ignored.sort();
    }
    settings
}

/// A task from `.vscode/tasks.json`. Nothing runs until the user says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VsCodeTask {
    pub label: String,
    /// `shell` or `process`.
    pub kind: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
}

impl VsCodeTask {
    /// Full command line as it would be run.
    pub fn command_line(&self) -> String {
        std::iter::once(self.command.clone())
            .chain(self.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Read `.vscode/tasks.json`.
pub fn read_tasks(root: &Path) -> Vec<VsCodeTask> {
    let path = root.join(".vscode").join("tasks.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(value) = parse_jsonc(&text) else {
        tracing::warn!(path = %path.display(), "could not parse tasks.json");
        return Vec::new();
    };
    value["tasks"]
        .as_array()
        .map(|tasks| {
            tasks
                .iter()
                .filter_map(|task| {
                    let label = task["label"].as_str()?.to_string();
                    let command = task["command"].as_str()?.to_string();
                    Some(VsCodeTask {
                        label,
                        kind: task["type"].as_str().unwrap_or("shell").to_string(),
                        command,
                        args: task["args"]
                            .as_array()
                            .map(|args| {
                                args.iter()
                                    .filter_map(|arg| arg.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        cwd: task["options"]["cwd"].as_str().map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_comments_and_trailing_commas() {
        let text = r#"{
  // line comment
  "a": 1, /* block */
  "b": [1, 2,],
}"#;
        let value = parse_jsonc(text).unwrap();
        assert_eq!(value["a"], 1);
        assert_eq!(value["b"][1], 2);
    }

    #[test]
    fn keeps_comment_like_text_inside_strings() {
        let value = parse_jsonc(r#"{"url": "https://example.com/a", "glob": "**/*.rs"}"#).unwrap();
        assert_eq!(value["url"], "https://example.com/a");
        assert_eq!(value["glob"], "**/*.rs");
    }

    #[test]
    fn reads_supported_settings_and_lists_the_rest() {
        let value = parse_jsonc(
            r#"{
  "editor.tabSize": 2,
  "editor.insertSpaces": true,
  "files.exclude": { "**/target": true, "**/.DS_Store": false },
  "workbench.colorTheme": "Dracula",
  "editor.minimap.enabled": false
}"#,
        )
        .unwrap();
        let settings = settings_from_value(&value);
        assert_eq!(settings.tab_size, Some(2));
        assert_eq!(settings.insert_spaces, Some(true));
        assert_eq!(settings.files_exclude, vec!["**/target".to_string()]);
        assert_eq!(
            settings.ignored,
            vec![
                "editor.minimap.enabled".to_string(),
                "workbench.colorTheme".to_string()
            ],
            "unknown settings are reported, not applied"
        );
    }

    #[test]
    fn applies_supported_settings_without_overriding_explicit_values() {
        let mut config = Config::with_builtin_defaults();
        let settings = VsCodeSettings {
            tab_size: Some(2),
            insert_spaces: Some(false),
            trim_trailing_whitespace: Some(true),
            insert_final_newline: Some(true),
            files_exclude: vec!["**/coverage".into(), "**/*.generated".into()],
            ignored: Vec::new(),
        };
        apply_settings(&mut config, &settings);
        assert_eq!(config.editor.tab_width, 2);
        assert!(!config.editor.insert_spaces);
        assert!(config.editor.trim_trailing_whitespace);
        assert!(config.editor.insert_final_newline);
        assert!(config.workspace.ignore.contains(&"coverage".into()));
        assert!(!config.workspace.ignore.contains(&"*.generated".into()));

        config.editor.tab_width = 8;
        apply_settings(&mut config, &settings);
        assert_eq!(config.editor.tab_width, 8);
    }

    #[test]
    fn reads_tasks_without_running_them() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vscode")).unwrap();
        std::fs::write(
            dir.path().join(".vscode/tasks.json"),
            r#"{
  "version": "2.0.0",
  "tasks": [
    { "label": "test", "type": "shell", "command": "cargo", "args": ["test"] }
  ]
}"#,
        )
        .unwrap();
        let tasks = read_tasks(dir.path());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].label, "test");
        assert_eq!(tasks[0].command_line(), "cargo test");
    }

    #[test]
    fn missing_files_are_not_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_settings(dir.path()), VsCodeSettings::default());
        assert!(read_tasks(dir.path()).is_empty());
    }

    #[test]
    fn malformed_files_do_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vscode")).unwrap();
        std::fs::write(dir.path().join(".vscode/settings.json"), "{ not json").unwrap();
        assert_eq!(read_settings(dir.path()), VsCodeSettings::default());
    }
}
