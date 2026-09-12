//! Workspace state persistence.
//!
//! Only lightweight UI facts are stored: which files were open, where the
//! cursor was, which panels were visible and which terminals existed. Terminal
//! output, environment variables and anything secret-shaped are never written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Bumped when the on-disk shape changes incompatibly.
pub const STATE_VERSION: u32 = 1;

/// Saved layout flags.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutRecord {
    pub explorer: bool,
    pub outline: bool,
    pub git: bool,
    pub agents: bool,
    pub terminals: bool,
    pub problems: bool,
    pub debug: bool,
    pub sidebar_width: u16,
    pub agents_width: u16,
    pub terminal_height_pct: u16,
}

impl Default for LayoutRecord {
    fn default() -> Self {
        LayoutRecord {
            explorer: true,
            outline: true,
            git: true,
            agents: true,
            terminals: true,
            problems: false,
            debug: false,
            sidebar_width: 30,
            agents_width: 42,
            terminal_height_pct: 30,
        }
    }
}

/// A terminal that can be recreated on restore. Commands are recorded, output
/// is not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalRecord {
    pub label: String,
    pub command: Vec<String>,
    pub cwd: PathBuf,
    /// `shell`, `agent` or `task`.
    pub kind: String,
    /// Agent provider label when this terminal belonged to an agent.
    pub agent_kind: Option<String>,
}

/// Everything remembered about one workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceState {
    pub version: u32,
    pub root: PathBuf,
    pub open_files: Vec<PathBuf>,
    pub active_file: Option<PathBuf>,
    /// path -> (line, character)
    pub cursors: BTreeMap<String, (usize, usize)>,
    pub expanded_dirs: Vec<PathBuf>,
    pub layout: LayoutRecord,
    pub terminals: Vec<TerminalRecord>,
    pub selected_agent: usize,
    pub theme: Option<String>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        WorkspaceState {
            version: STATE_VERSION,
            root: PathBuf::new(),
            open_files: Vec::new(),
            active_file: None,
            cursors: BTreeMap::new(),
            expanded_dirs: Vec::new(),
            layout: LayoutRecord::default(),
            terminals: Vec::new(),
            selected_agent: 0,
            theme: None,
        }
    }
}

/// Stable, filesystem-safe key for a workspace path.
pub fn workspace_key(root: &Path) -> String {
    // FNV-1a over the path bytes: short, stable and dependency-free.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in root.to_string_lossy().as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Where a workspace's state file lives.
pub fn state_path(root: &Path) -> Option<PathBuf> {
    crate::config::state_dir().map(|dir| {
        dir.join("workspaces")
            .join(format!("{}.json", workspace_key(root)))
    })
}

/// Persist state for a workspace.
pub fn save(state: &WorkspaceState) -> Result<PathBuf> {
    let path = state_path(&state.root).context("no state directory available")?;
    save_to(state, &path)?;
    Ok(path)
}

/// Persist to an explicit path (used by tests).
pub fn save_to(state: &WorkspaceState, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Load state for a workspace. Missing or unreadable state is not an error:
/// the workbench simply starts fresh.
pub fn load(root: &Path) -> Option<WorkspaceState> {
    let path = state_path(root)?;
    load_from(&path)
}

/// Load from an explicit path.
pub fn load_from(path: &Path) -> Option<WorkspaceState> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<WorkspaceState>(&text) {
        Ok(state) if state.version == STATE_VERSION => Some(state),
        Ok(state) => {
            tracing::info!(
                found = state.version,
                expected = STATE_VERSION,
                "ignoring workspace state from an older version"
            );
            None
        }
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "unreadable workspace state");
            None
        }
    }
}

/// Recently opened workspaces, newest first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecentWorkspaces {
    pub paths: Vec<PathBuf>,
}

impl RecentWorkspaces {
    const MAX: usize = 20;

    pub fn load() -> RecentWorkspaces {
        let Some(path) = Self::path() else {
            return RecentWorkspaces::default();
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn record(root: &Path) -> Result<()> {
        let mut recent = RecentWorkspaces::load();
        recent.paths.retain(|p| p != root);
        recent.paths.insert(0, root.to_path_buf());
        recent.paths.truncate(Self::MAX);
        let Some(path) = Self::path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&recent)?)?;
        Ok(())
    }

    fn path() -> Option<PathBuf> {
        crate::config::state_dir().map(|dir| dir.join("recent.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(root: &Path) -> WorkspaceState {
        WorkspaceState {
            root: root.to_path_buf(),
            open_files: vec![root.join("src/main.rs")],
            active_file: Some(root.join("src/main.rs")),
            cursors: BTreeMap::from([("src/main.rs".to_string(), (12, 4))]),
            expanded_dirs: vec![root.join("src")],
            terminals: vec![TerminalRecord {
                label: "Claude".into(),
                command: vec!["claude".into()],
                cwd: root.to_path_buf(),
                kind: "agent".into(),
                agent_kind: Some("Claude".into()),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let state = sample(dir.path());
        save_to(&state, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, state);
    }

    #[test]
    fn workspace_keys_are_stable_and_distinct() {
        let a = workspace_key(Path::new("/home/dev/project"));
        let b = workspace_key(Path::new("/home/dev/other"));
        assert_eq!(a, workspace_key(Path::new("/home/dev/project")));
        assert_ne!(a, b);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn missing_state_is_not_an_error() {
        assert!(load_from(Path::new("/nope/does-not-exist.json")).is_none());
    }

    #[test]
    fn corrupt_state_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(load_from(&path).is_none());
    }

    #[test]
    fn state_from_a_future_version_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = sample(dir.path());
        state.version = STATE_VERSION + 7;
        save_to(&state, &path).unwrap();
        assert!(load_from(&path).is_none());
    }

    #[test]
    fn persisted_state_contains_no_terminal_output_or_environment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        save_to(&sample(dir.path()), &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        for forbidden in ["env", "output", "scrollback", "token", "secret"] {
            assert!(
                !text.contains(forbidden),
                "workspace state must not contain `{forbidden}`: {text}"
            );
        }
    }

    #[test]
    fn partial_state_files_use_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, r#"{"version":1,"root":"/tmp/x"}"#).unwrap();
        let loaded = load_from(&path).unwrap();
        assert!(loaded.open_files.is_empty());
        assert!(loaded.layout.explorer);
    }
}
