//! User configuration (TOML) and the standard file locations.
//!
//! Two layers are merged, later wins:
//!
//! 1. `<config_dir>/termloom/config.toml` — user-global
//! 2. `<workspace>/.termloom.toml` — project-specific
//!
//! Everything is optional: TermLoom runs with no config file at all.

pub mod keys;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub use keys::{KeyChord, Keymap};

/// Name of the per-project override file.
pub const PROJECT_CONFIG_FILE: &str = ".termloom.toml";

/// Top-level configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub ui: UiConfig,
    pub editor: EditorConfig,
    pub terminal: TerminalConfig,
    pub herdr: HerdrConfig,
    pub workspace: WorkspaceConfig,
    /// Agent launch presets, keyed by preset name (`claude`, `codex`, ...).
    pub agents: BTreeMap<String, AgentPreset>,
    /// Language server definitions, keyed by a server name (`rust-analyzer`).
    pub lsp: BTreeMap<String, LspServerConfig>,
    /// Debug adapter definitions, keyed by adapter name (`codelldb`).
    pub dap: BTreeMap<String, DapAdapterConfig>,
    /// Key chord overrides: command id -> chord string.
    pub keys: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub theme: String,
    pub show_explorer: bool,
    pub show_agents: bool,
    pub show_git: bool,
    pub show_outline: bool,
    pub show_terminals: bool,
    pub mouse: bool,
    /// Use Nerd Font glyphs when the terminal is known to support them.
    pub nerd_fonts: bool,
    /// Percentage of the height taken by the bottom terminal strip.
    pub terminal_height_pct: u16,
    /// Width of the left sidebar in columns.
    pub sidebar_width: u16,
    /// Width of the right agents panel in columns.
    pub agents_width: u16,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "termloom-dark".into(),
            show_explorer: true,
            show_agents: true,
            show_git: true,
            show_outline: true,
            show_terminals: true,
            mouse: true,
            nerd_fonts: false,
            terminal_height_pct: 30,
            sidebar_width: 30,
            agents_width: 42,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct EditorConfig {
    pub tab_width: usize,
    pub insert_spaces: bool,
    pub line_numbers: bool,
    /// Trim trailing whitespace when saving.
    pub trim_trailing_whitespace: bool,
    /// Ensure the file ends with a newline when saving.
    pub insert_final_newline: bool,
    /// Refuse to open files larger than this (bytes).
    pub max_file_size: u64,
    pub syntax_highlighting: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            tab_width: 4,
            insert_spaces: true,
            line_numbers: true,
            trim_trailing_whitespace: false,
            insert_final_newline: false,
            max_file_size: 8 * 1024 * 1024,
            syntax_highlighting: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalConfig {
    /// Shell program; defaults to `$SHELL` then `/bin/sh`.
    pub shell: Option<String>,
    /// Extra arguments passed to the shell.
    pub shell_args: Vec<String>,
    /// Lines of scrollback kept per session.
    pub scrollback: usize,
    /// Seconds of silence after which a live agent is considered idle.
    pub idle_after_secs: u64,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            shell: None,
            shell_args: Vec::new(),
            scrollback: 5_000,
            idle_after_secs: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceConfig {
    /// Directories never shown in the explorer or scanned for quick open.
    pub ignore: Vec<String>,
    /// Respect `.gitignore` when listing files.
    pub respect_gitignore: bool,
    /// Show dotfiles in the explorer.
    pub show_hidden: bool,
    /// Restore open editors/layout from the previous session.
    pub restore_session: bool,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            ignore: [
                ".git",
                "node_modules",
                "target",
                "build",
                "dist",
                ".dart_tool",
                ".gradle",
                "__pycache__",
                ".venv",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            respect_gitignore: true,
            show_hidden: false,
            restore_session: true,
        }
    }
}

/// How TermLoom should treat an installed Herdr.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HerdrMode {
    /// Use Herdr when it is installed and reachable.
    Auto,
    /// Always try Herdr; report an error when unavailable.
    Enabled,
    /// Never talk to Herdr.
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct HerdrConfig {
    pub mode: HerdrMode,
    /// Path to the `herdr` binary; resolved on `$PATH` when absent.
    pub command: Option<String>,
}

impl Default for HerdrConfig {
    fn default() -> Self {
        Self {
            mode: HerdrMode::Auto,
            command: None,
        }
    }
}

/// A launchable agent preset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AgentPreset {
    /// Display name in the palette / dashboard.
    pub label: Option<String>,
    /// Program to run.
    pub command: String,
    pub args: Vec<String>,
    /// Extra environment variables for the child process.
    pub env: BTreeMap<String, String>,
}

impl Default for AgentPreset {
    fn default() -> Self {
        Self {
            label: None,
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
        }
    }
}

/// A language server definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LspServerConfig {
    /// Executable launched over stdio.
    pub command: String,
    pub args: Vec<String>,
    /// Language ids this server handles (`rust`, `typescript`, ...).
    pub languages: Vec<String>,
    /// Files/directories whose presence marks the project root.
    pub root_markers: Vec<String>,
    /// Arbitrary JSON passed as `initializationOptions`.
    pub initialization_options: Option<toml::Value>,
    /// Arbitrary JSON passed as workspace `settings`.
    pub settings: Option<toml::Value>,
    /// Start automatically when a matching file is opened.
    pub auto_start: bool,
    pub env: BTreeMap<String, String>,
}

impl Default for LspServerConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            languages: Vec::new(),
            root_markers: Vec::new(),
            initialization_options: None,
            settings: None,
            auto_start: true,
            env: BTreeMap::new(),
        }
    }
}

/// A debug adapter definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct DapAdapterConfig {
    /// Executable launched over stdio.
    pub command: String,
    pub args: Vec<String>,
    /// `launch.json` `type` values this adapter serves.
    pub types: Vec<String>,
    pub env: BTreeMap<String, String>,
}

impl Default for DapAdapterConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            types: Vec::new(),
            env: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Built-in defaults, including the Claude/Codex/Shell presets and a
    /// `rust-analyzer` mapping so a fresh install is useful immediately.
    pub fn with_builtin_defaults() -> Config {
        let mut cfg = Config::default();
        cfg.agents.insert(
            "claude".into(),
            AgentPreset {
                label: Some("Claude".into()),
                command: "claude".into(),
                ..Default::default()
            },
        );
        cfg.agents.insert(
            "codex".into(),
            AgentPreset {
                label: Some("Codex".into()),
                command: "codex".into(),
                ..Default::default()
            },
        );
        cfg.lsp.insert(
            "rust-analyzer".into(),
            LspServerConfig {
                command: "rust-analyzer".into(),
                languages: vec!["rust".into()],
                root_markers: vec!["Cargo.toml".into()],
                ..Default::default()
            },
        );
        cfg
    }

    /// Load global config, then project config, on top of the builtin defaults.
    pub fn load(workspace_root: Option<&Path>) -> (Config, Vec<ConfigDiagnostic>) {
        let mut diagnostics = Vec::new();
        let mut config = Config::with_builtin_defaults();

        if let Some(path) = global_config_path() {
            match Self::load_file(&path) {
                Ok(Some(other)) => config.merge(other),
                Ok(None) => {}
                Err(err) => diagnostics.push(ConfigDiagnostic {
                    path: path.clone(),
                    message: format!("{err:#}"),
                }),
            }
        }

        if let Some(root) = workspace_root {
            let path = root.join(PROJECT_CONFIG_FILE);
            match Self::load_file(&path) {
                Ok(Some(other)) => config.merge(other),
                Ok(None) => {}
                Err(err) => diagnostics.push(ConfigDiagnostic {
                    path,
                    message: format!("{err:#}"),
                }),
            }
        }

        (config, diagnostics)
    }

    /// Parse a single config file. `Ok(None)` when the file does not exist.
    pub fn load_file(path: &Path) -> Result<Option<Config>> {
        if !path.exists() {
            return Ok(None);
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let parsed: Config =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Some(parsed))
    }

    /// Overlay `other` on top of `self`. Scalar values are replaced when they
    /// differ from the default; maps are merged per key.
    pub fn merge(&mut self, other: Config) {
        let defaults = Config::default();
        if other.ui != defaults.ui {
            self.ui = merge_ui(self.ui.clone(), other.ui, &defaults.ui);
        }
        if other.editor != defaults.editor {
            self.editor = other.editor;
        }
        if other.terminal != defaults.terminal {
            self.terminal = other.terminal;
        }
        if other.herdr != defaults.herdr {
            self.herdr = other.herdr;
        }
        if other.workspace != defaults.workspace {
            self.workspace = other.workspace;
        }
        self.agents.extend(other.agents);
        self.lsp.extend(other.lsp);
        self.dap.extend(other.dap);
        self.keys.extend(other.keys);
    }

    /// Shell to launch for terminal sessions.
    pub fn shell_command(&self) -> (String, Vec<String>) {
        let shell = self
            .terminal
            .shell
            .clone()
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/sh".to_string());
        (shell, self.terminal.shell_args.clone())
    }

    /// Resolved keymap (defaults plus `[keys]` overrides).
    pub fn keymap(&self) -> Keymap {
        Keymap::from_overrides(&self.keys)
    }

    /// Serialise to TOML (used by `termloom config --write-default`).
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
}

/// Per-field merge for the UI section so a project file can flip one flag.
fn merge_ui(mut base: UiConfig, other: UiConfig, defaults: &UiConfig) -> UiConfig {
    if other.theme != defaults.theme {
        base.theme = other.theme;
    }
    if other.show_explorer != defaults.show_explorer {
        base.show_explorer = other.show_explorer;
    }
    if other.show_agents != defaults.show_agents {
        base.show_agents = other.show_agents;
    }
    if other.show_git != defaults.show_git {
        base.show_git = other.show_git;
    }
    if other.show_outline != defaults.show_outline {
        base.show_outline = other.show_outline;
    }
    if other.show_terminals != defaults.show_terminals {
        base.show_terminals = other.show_terminals;
    }
    if other.mouse != defaults.mouse {
        base.mouse = other.mouse;
    }
    if other.nerd_fonts != defaults.nerd_fonts {
        base.nerd_fonts = other.nerd_fonts;
    }
    if other.terminal_height_pct != defaults.terminal_height_pct {
        base.terminal_height_pct = other.terminal_height_pct;
    }
    if other.sidebar_width != defaults.sidebar_width {
        base.sidebar_width = other.sidebar_width;
    }
    if other.agents_width != defaults.agents_width {
        base.agents_width = other.agents_width;
    }
    base
}

/// A configuration file that could not be read or parsed. Surfaced in the UI
/// instead of being silently ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDiagnostic {
    pub path: PathBuf,
    pub message: String,
}

/// `<config>/termloom/config.toml`
pub fn global_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("termloom").join("config.toml"))
}

/// `<state|data>/termloom/` — workspace state and logs.
pub fn state_dir() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|d| d.join("termloom"))
}

/// `<data>/termloom/extensions/` — installed VSIX assets.
pub fn extensions_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("termloom").join("extensions"))
}

/// `<state>/termloom/logs/termloom.log`
pub fn log_path() -> Option<PathBuf> {
    state_dir().map(|d| d.join("logs").join("termloom.log"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_include_agent_presets() {
        let cfg = Config::with_builtin_defaults();
        assert_eq!(cfg.agents["claude"].command, "claude");
        assert_eq!(cfg.agents["codex"].command, "codex");
        assert!(cfg.lsp.contains_key("rust-analyzer"));
    }

    #[test]
    fn parses_documented_example() {
        let text = r#"
[ui]
theme = "tokyo-night"
show_git = false

[editor]
tab_width = 2
insert_spaces = true

[agents.claude]
command = "claude"
args = ["--dangerously-skip-permissions"]

[herdr]
mode = "disabled"

[keys]
"workspace.reload" = "ctrl+r"
"#;
        let parsed: Config = toml::from_str(text).expect("valid config");
        assert_eq!(parsed.ui.theme, "tokyo-night");
        assert!(!parsed.ui.show_git);
        assert_eq!(parsed.editor.tab_width, 2);
        assert_eq!(parsed.herdr.mode, HerdrMode::Disabled);
        assert_eq!(parsed.keys["workspace.reload"], "ctrl+r");
    }

    #[test]
    fn unknown_keys_are_reported_not_ignored() {
        let err = toml::from_str::<Config>("[ui]\nnope = 1\n").unwrap_err();
        assert!(err.to_string().contains("nope"), "{err}");
    }

    #[test]
    fn project_config_overrides_only_what_it_sets() {
        let mut cfg = Config::with_builtin_defaults();
        cfg.ui.theme = "custom".into();
        let project: Config = toml::from_str("[ui]\nshow_agents = false\n").unwrap();
        cfg.merge(project);
        assert_eq!(cfg.ui.theme, "custom", "theme must survive the merge");
        assert!(!cfg.ui.show_agents);
        // presets are preserved
        assert!(cfg.agents.contains_key("claude"));
    }

    #[test]
    fn project_config_can_add_an_agent_preset() {
        let mut cfg = Config::with_builtin_defaults();
        let project: Config =
            toml::from_str("[agents.tests]\ncommand = \"cargo\"\nargs = [\"test\"]\n").unwrap();
        cfg.merge(project);
        assert_eq!(cfg.agents["tests"].args, vec!["test".to_string()]);
        assert_eq!(cfg.agents.len(), 3);
    }

    #[test]
    fn roundtrips_through_toml() {
        let cfg = Config::with_builtin_defaults();
        let text = cfg.to_toml().unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let path = PathBuf::from("/definitely/not/here/config.toml");
        assert!(Config::load_file(&path).unwrap().is_none());
    }
}
