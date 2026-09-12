//! Git data shown by the workbench. No libgit2 types appear here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Per-file working-tree status, already collapsed into what the UI needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
    Ignored,
}

impl FileStatus {
    /// Single-character decoration used in the explorer and Git panel.
    pub fn code(self) -> char {
        match self {
            FileStatus::Modified => 'M',
            FileStatus::Added => 'A',
            FileStatus::Deleted => 'D',
            FileStatus::Renamed => 'R',
            FileStatus::Untracked => '?',
            FileStatus::Conflicted => 'U',
            FileStatus::Ignored => '!',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FileStatus::Modified => "modified",
            FileStatus::Added => "added",
            FileStatus::Deleted => "deleted",
            FileStatus::Renamed => "renamed",
            FileStatus::Untracked => "untracked",
            FileStatus::Conflicted => "conflicted",
            FileStatus::Ignored => "ignored",
        }
    }
}

/// One changed path, with index and worktree state kept separate so the panel
/// can offer stage/unstage correctly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFileChange {
    /// Path relative to the repository root.
    pub path: PathBuf,
    /// Status of the change that is staged in the index, if any.
    pub index: Option<FileStatus>,
    /// Status of the change in the working tree, if any.
    pub worktree: Option<FileStatus>,
}

impl GitFileChange {
    /// Whether any part of this change is staged.
    pub fn is_staged(&self) -> bool {
        self.index.is_some()
    }

    /// Status to display as the primary decoration.
    pub fn display_status(&self) -> FileStatus {
        self.worktree.or(self.index).unwrap_or(FileStatus::Modified)
    }

    /// Two-column porcelain-style code, e.g. `M ` or ` M` or `MM`.
    pub fn code(&self) -> String {
        let idx = self.index.map(|s| s.code()).unwrap_or(' ');
        let wt = self.worktree.map(|s| s.code()).unwrap_or(' ');
        format!("{idx}{wt}")
    }
}

/// A snapshot of repository state, refreshed on a debounce.
#[derive(Debug, Clone, Default)]
pub struct GitSnapshot {
    pub root: Option<PathBuf>,
    pub branch: Option<String>,
    /// Detached HEAD short sha when there is no branch.
    pub head_sha: Option<String>,
    pub changes: Vec<GitFileChange>,
    /// Commits ahead / behind upstream, when an upstream is configured.
    pub ahead_behind: Option<(usize, usize)>,
    pub has_upstream: bool,
}

impl GitSnapshot {
    /// True when the working tree or index differ from HEAD.
    pub fn is_dirty(&self) -> bool {
        !self.changes.is_empty()
    }

    /// Status for an absolute path, if the file is changed.
    pub fn status_for(&self, root: &Path, path: &Path) -> Option<FileStatus> {
        let rel = path.strip_prefix(root).ok()?;
        self.changes
            .iter()
            .find(|c| c.path == rel)
            .map(|c| c.display_status())
    }

    /// Whether any changed file lives under `dir` (used to decorate folders).
    pub fn dir_has_changes(&self, root: &Path, dir: &Path) -> bool {
        let Ok(rel) = dir.strip_prefix(root) else {
            return false;
        };
        if rel.as_os_str().is_empty() {
            return !self.changes.is_empty();
        }
        self.changes.iter().any(|c| c.path.starts_with(rel))
    }

    /// Short branch description for the status bar.
    pub fn head_label(&self) -> String {
        match (&self.branch, &self.head_sha) {
            (Some(b), _) => b.clone(),
            (None, Some(sha)) => format!("detached@{sha}"),
            (None, None) => "no head".to_string(),
        }
    }
}

/// A unified diff for one file, already split into lines for rendering.
#[derive(Debug, Clone, Default)]
pub struct FileDiff {
    pub path: PathBuf,
    pub staged: bool,
    pub lines: Vec<DiffLine>,
    /// Set when the file is binary and no textual diff exists.
    pub binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    Hunk(String),
    Context(String),
    Added(String),
    Removed(String),
    Meta(String),
}

impl DiffLine {
    pub fn text(&self) -> &str {
        match self {
            DiffLine::Hunk(s)
            | DiffLine::Context(s)
            | DiffLine::Added(s)
            | DiffLine::Removed(s)
            | DiffLine::Meta(s) => s,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str, index: Option<FileStatus>, wt: Option<FileStatus>) -> GitFileChange {
        GitFileChange {
            path: PathBuf::from(path),
            index,
            worktree: wt,
        }
    }

    #[test]
    fn porcelain_codes_match_git_layout() {
        assert_eq!(change("a", Some(FileStatus::Added), None).code(), "A ");
        assert_eq!(change("a", None, Some(FileStatus::Modified)).code(), " M");
        assert_eq!(
            change("a", Some(FileStatus::Modified), Some(FileStatus::Modified)).code(),
            "MM"
        );
    }

    #[test]
    fn folder_decoration_follows_nested_changes() {
        let root = PathBuf::from("/repo");
        let snap = GitSnapshot {
            root: Some(root.clone()),
            changes: vec![change("src/app/main.rs", None, Some(FileStatus::Modified))],
            ..Default::default()
        };
        assert!(snap.dir_has_changes(&root, &root.join("src")));
        assert!(snap.dir_has_changes(&root, &root.join("src/app")));
        assert!(!snap.dir_has_changes(&root, &root.join("docs")));
        assert_eq!(
            snap.status_for(&root, &root.join("src/app/main.rs")),
            Some(FileStatus::Modified)
        );
    }

    #[test]
    fn head_label_handles_detached_head() {
        let snap = GitSnapshot {
            head_sha: Some("a1b2c3d".into()),
            ..Default::default()
        };
        assert_eq!(snap.head_label(), "detached@a1b2c3d");
    }
}
