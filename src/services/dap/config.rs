//! Debug launch configurations.
//!
//! Configurations come from two places: `[dap.<name>]` in TermLoom's own
//! config, which says how to *run an adapter*, and launch configurations —
//! either `[dap.<name>]` defaults or a VS Code `.vscode/launch.json` — which
//! say *what to debug*. Repository files never run anything on their own: the
//! user picks a configuration explicitly.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::config::{Config, DapAdapterConfig};

/// A fully resolved configuration: which adapter to run and what to debug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugLaunchConfig {
    /// Display name (`Debug executable`).
    pub name: String,
    /// VS Code style `type` (`lldb`, `debugpy`, ...).
    pub type_name: String,
    /// True for `attach`, false for `launch`.
    pub attach: bool,
    pub adapter_command: String,
    pub adapter_args: Vec<String>,
    pub adapter_env: Vec<(String, String)>,
    pub cwd: PathBuf,
    /// Arguments passed straight through to the adapter's launch request.
    pub arguments: Value,
    /// Breakpoints to install before `configurationDone`.
    pub breakpoints: HashMap<PathBuf, Vec<usize>>,
}

impl DebugLaunchConfig {
    /// The `launch`/`attach` request body.
    pub fn request_arguments(&self) -> Value {
        let mut arguments = self.arguments.clone();
        if let Value::Object(map) = &mut arguments {
            map.entry("cwd")
                .or_insert_with(|| json!(self.cwd.to_string_lossy()));
            map.entry("name").or_insert_with(|| json!(self.name));
        }
        arguments
    }

    /// The command line that will run, for the consent prompt.
    pub fn command_line(&self) -> String {
        std::iter::once(self.adapter_command.clone())
            .chain(self.adapter_args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A configuration the user can pick, before an adapter is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchEntry {
    pub name: String,
    pub type_name: String,
    pub attach: bool,
    pub arguments: Value,
    /// Where it came from, shown in the picker.
    pub source: LaunchSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchSource {
    /// `[dap.<name>]` in TermLoom configuration.
    TermLoom,
    /// `.vscode/launch.json`.
    VsCode,
}

impl LaunchSource {
    pub fn label(self) -> &'static str {
        match self {
            LaunchSource::TermLoom => "termloom",
            LaunchSource::VsCode => "launch.json",
        }
    }
}

/// Read `.vscode/launch.json`, tolerating comments and trailing commas.
pub fn read_vscode_launch(root: &Path) -> Vec<LaunchEntry> {
    let path = root.join(".vscode").join("launch.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(value) = crate::services::vscode::parse_jsonc(&text) else {
        tracing::warn!(path = %path.display(), "could not parse launch.json");
        return Vec::new();
    };
    value["configurations"]
        .as_array()
        .map(|configurations| {
            configurations
                .iter()
                .filter_map(|configuration| {
                    let name = configuration["name"].as_str()?.to_string();
                    let type_name = configuration["type"].as_str().unwrap_or("").to_string();
                    let attach = configuration["request"].as_str() == Some("attach");
                    Some(LaunchEntry {
                        name,
                        type_name,
                        attach,
                        arguments: configuration.clone(),
                        source: LaunchSource::VsCode,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Every configuration available in this workspace.
pub fn available(config: &Config, root: &Path) -> Vec<LaunchEntry> {
    let mut entries: Vec<LaunchEntry> = config
        .dap
        .iter()
        .flat_map(|(name, adapter)| {
            adapter.types.iter().map(move |type_name| LaunchEntry {
                name: format!("{name} ({type_name})"),
                type_name: type_name.clone(),
                attach: false,
                arguments: json!({}),
                source: LaunchSource::TermLoom,
            })
        })
        .collect();
    entries.extend(read_vscode_launch(root));
    entries
}

/// Find the adapter that serves a launch configuration `type`.
pub fn adapter_for<'a>(
    adapters: &'a BTreeMap<String, DapAdapterConfig>,
    type_name: &str,
) -> Option<(&'a String, &'a DapAdapterConfig)> {
    adapters
        .iter()
        .find(|(_, adapter)| adapter.types.iter().any(|t| t == type_name))
        .or_else(|| adapters.iter().find(|(name, _)| name.as_str() == type_name))
}

/// Resolve a chosen entry into something runnable.
pub fn resolve(
    config: &Config,
    root: &Path,
    entry: &LaunchEntry,
    breakpoints: HashMap<PathBuf, Vec<usize>>,
) -> Result<DebugLaunchConfig> {
    let (adapter_name, adapter) = adapter_for(&config.dap, &entry.type_name).ok_or_else(|| {
        anyhow!(
            "no debug adapter configured for type `{}` — add one under [dap] in config.toml",
            entry.type_name
        )
    })?;
    if adapter.command.is_empty() {
        return Err(anyhow!("[dap.{adapter_name}] has no command"));
    }

    let arguments = substitute_variables(entry.arguments.clone(), root);
    Ok(DebugLaunchConfig {
        name: entry.name.clone(),
        type_name: entry.type_name.clone(),
        attach: entry.attach,
        adapter_command: adapter.command.clone(),
        adapter_args: adapter.args.clone(),
        adapter_env: adapter
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        cwd: root.to_path_buf(),
        arguments,
        breakpoints,
    })
}

/// Expand the VS Code variables TermLoom understands.
///
/// Unknown `${…}` variables are left untouched and reported by the caller
/// rather than being silently reinterpreted.
pub fn substitute_variables(value: Value, root: &Path) -> Value {
    match value {
        Value::String(text) => Value::String(substitute_in_string(&text, root)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| substitute_variables(item, root))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, substitute_variables(value, root)))
                .collect(),
        ),
        other => other,
    }
}

fn substitute_in_string(text: &str, root: &Path) -> String {
    let root_text = root.to_string_lossy().to_string();
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    text.replace("${workspaceFolder}", &root_text)
        .replace("${workspaceRoot}", &root_text)
        .replace("${workspaceFolderBasename}", &name)
        .replace("${cwd}", &root_text)
        .replace(
            "${userHome}",
            &dirs::home_dir().unwrap_or_default().to_string_lossy(),
        )
}

/// `${…}` variables left unresolved in a configuration.
pub fn unresolved_variables(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_unresolved(value, &mut out);
    out.sort();
    out.dedup();
    out
}

fn collect_unresolved(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            let mut rest = text.as_str();
            while let Some(start) = rest.find("${") {
                let Some(end) = rest[start..].find('}') else {
                    break;
                };
                out.push(rest[start..start + end + 1].to_string());
                rest = &rest[start + end + 1..];
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect_unresolved(item, out)),
        Value::Object(map) => map
            .values()
            .for_each(|value| collect_unresolved(value, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DapAdapterConfig;

    fn config_with_adapter() -> Config {
        let mut config = Config::default();
        config.dap.insert(
            "lldb".into(),
            DapAdapterConfig {
                command: "lldb-dap".into(),
                args: vec![],
                types: vec!["lldb".into()],
                env: Default::default(),
            },
        );
        config
    }

    #[test]
    fn reads_vscode_launch_configurations_with_comments() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vscode")).unwrap();
        std::fs::write(
            dir.path().join(".vscode/launch.json"),
            r#"{
  // a comment
  "version": "0.2.0",
  "configurations": [
    {
      "name": "Debug binary",
      "type": "lldb",
      "request": "launch",
      "program": "${workspaceFolder}/target/debug/app",
    },
  ]
}"#,
        )
        .unwrap();
        let entries = read_vscode_launch(dir.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Debug binary");
        assert_eq!(entries[0].type_name, "lldb");
        assert!(!entries[0].attach);
        assert_eq!(entries[0].source, LaunchSource::VsCode);
    }

    #[test]
    fn missing_launch_json_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_vscode_launch(dir.path()).is_empty());
    }

    #[test]
    fn variables_are_expanded_against_the_workspace() {
        let value = json!({"program": "${workspaceFolder}/target/debug/app"});
        let expanded = substitute_variables(value, Path::new("/repo"));
        assert_eq!(expanded["program"], "/repo/target/debug/app");
    }

    #[test]
    fn unknown_variables_are_reported_not_guessed() {
        let value = json!({"program": "${command:pickProcess}", "args": ["${env:FOO}"]});
        let expanded = substitute_variables(value, Path::new("/repo"));
        let unresolved = unresolved_variables(&expanded);
        assert_eq!(
            unresolved,
            vec![
                "${command:pickProcess}".to_string(),
                "${env:FOO}".to_string()
            ]
        );
    }

    #[test]
    fn resolving_needs_a_configured_adapter() {
        let entry = LaunchEntry {
            name: "Debug".into(),
            type_name: "lldb".into(),
            attach: false,
            arguments: json!({"program": "/bin/ls"}),
            source: LaunchSource::VsCode,
        };
        let err = resolve(
            &Config::default(),
            Path::new("/repo"),
            &entry,
            HashMap::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("no debug adapter configured"), "{err}");

        let resolved = resolve(
            &config_with_adapter(),
            Path::new("/repo"),
            &entry,
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(resolved.adapter_command, "lldb-dap");
        assert_eq!(resolved.request_arguments()["program"], "/bin/ls");
        assert_eq!(resolved.request_arguments()["cwd"], "/repo");
    }

    #[test]
    fn available_lists_both_sources() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vscode")).unwrap();
        std::fs::write(
            dir.path().join(".vscode/launch.json"),
            r#"{"configurations":[{"name":"From VS Code","type":"lldb","request":"launch"}]}"#,
        )
        .unwrap();
        let entries = available(&config_with_adapter(), dir.path());
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|e| e.source == LaunchSource::TermLoom));
        assert!(entries.iter().any(|e| e.source == LaunchSource::VsCode));
    }
}
