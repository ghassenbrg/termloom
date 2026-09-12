//! Git integration built on libgit2, with a Git CLI fallback for the few
//! operations where the CLI is more reliable.
//!
//! A `git2::Repository` is neither cheap to share across threads nor `Sync`,
//! so every function opens the repository itself. Refreshes therefore run on a
//! background thread without holding any handle in the app state.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use git2::{DiffFormat, DiffOptions, Repository, Status, StatusOptions};

use crate::domain::git::{DiffLine, FileDiff, FileStatus, GitFileChange, GitSnapshot};

/// Read-only repository probe: is this path inside a Git repository?
pub fn is_repository(root: &Path) -> bool {
    Repository::discover(root).is_ok()
}

/// Collect branch, upstream distance and per-file status.
pub fn snapshot(root: &Path) -> Result<GitSnapshot> {
    let repo = Repository::discover(root).context("opening git repository")?;
    let workdir = repo
        .workdir()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.to_path_buf());

    let mut snapshot = GitSnapshot {
        root: Some(workdir),
        ..Default::default()
    };

    match repo.head() {
        Ok(head) => {
            if head.is_branch() {
                snapshot.branch = head.shorthand().map(str::to_string);
            } else {
                snapshot.head_sha = head
                    .target()
                    .map(|oid| oid.to_string().chars().take(7).collect());
            }
            if let Some(branch_name) = snapshot.branch.clone() {
                snapshot.ahead_behind = upstream_distance(&repo, &branch_name);
                snapshot.has_upstream = snapshot.ahead_behind.is_some();
            }
        }
        Err(_) => {
            // Unborn HEAD (fresh repository with no commits).
            snapshot.branch = head_name_from_cli(root);
        }
    }

    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true)
        .include_ignored(false);

    let statuses = repo.statuses(Some(&mut options))?;
    for entry in statuses.iter() {
        let Some(path) = entry.path() else { continue };
        let flags = entry.status();
        let change = GitFileChange {
            path: PathBuf::from(path),
            index: index_status(flags),
            worktree: worktree_status(flags),
        };
        if change.index.is_some() || change.worktree.is_some() {
            snapshot.changes.push(change);
        }
    }
    snapshot.changes.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(snapshot)
}

fn index_status(flags: Status) -> Option<FileStatus> {
    if flags.contains(Status::INDEX_NEW) {
        Some(FileStatus::Added)
    } else if flags.contains(Status::INDEX_MODIFIED) {
        Some(FileStatus::Modified)
    } else if flags.contains(Status::INDEX_DELETED) {
        Some(FileStatus::Deleted)
    } else if flags.contains(Status::INDEX_RENAMED) {
        Some(FileStatus::Renamed)
    } else if flags.contains(Status::INDEX_TYPECHANGE) {
        Some(FileStatus::Modified)
    } else {
        None
    }
}

fn worktree_status(flags: Status) -> Option<FileStatus> {
    if flags.contains(Status::CONFLICTED) {
        Some(FileStatus::Conflicted)
    } else if flags.contains(Status::WT_NEW) {
        Some(FileStatus::Untracked)
    } else if flags.contains(Status::WT_MODIFIED) {
        Some(FileStatus::Modified)
    } else if flags.contains(Status::WT_DELETED) {
        Some(FileStatus::Deleted)
    } else if flags.contains(Status::WT_RENAMED) {
        Some(FileStatus::Renamed)
    } else if flags.contains(Status::WT_TYPECHANGE) {
        Some(FileStatus::Modified)
    } else {
        None
    }
}

fn upstream_distance(repo: &Repository, branch_name: &str) -> Option<(usize, usize)> {
    let branch = repo
        .find_branch(branch_name, git2::BranchType::Local)
        .ok()?;
    let upstream = branch.upstream().ok()?;
    let local_oid = branch.get().target()?;
    let upstream_oid = upstream.get().target()?;
    repo.graph_ahead_behind(local_oid, upstream_oid).ok()
}

/// `git symbolic-ref` fallback for repositories without any commit yet.
fn head_name_from_cli(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// Unified diff for one path. `staged` selects index-vs-HEAD instead of
/// worktree-vs-index.
pub fn diff_file(root: &Path, relative: &Path, staged: bool) -> Result<FileDiff> {
    let repo = Repository::discover(root)?;
    let mut options = DiffOptions::new();
    options
        .pathspec(relative)
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .context_lines(3);

    let diff = if staged {
        let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
        repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut options))?
    } else {
        repo.diff_index_to_workdir(None, Some(&mut options))?
    };

    let mut out = FileDiff {
        path: relative.to_path_buf(),
        staged,
        ..Default::default()
    };

    diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
        let content = String::from_utf8_lossy(line.content())
            .trim_end_matches('\n')
            .to_string();
        let entry = match line.origin() {
            '+' => DiffLine::Added(content),
            '-' => DiffLine::Removed(content),
            ' ' => DiffLine::Context(content),
            'H' => DiffLine::Hunk(content),
            'F' => DiffLine::Meta(content),
            'B' => {
                out.binary = true;
                DiffLine::Meta(content)
            }
            _ => DiffLine::Meta(content),
        };
        out.lines.push(entry);
        true
    })?;

    Ok(out)
}

/// Stage a path (handles deletions).
pub fn stage(root: &Path, relative: &Path) -> Result<()> {
    let repo = Repository::discover(root)?;
    let mut index = repo.index()?;
    let workdir = repo.workdir().unwrap_or(root).to_path_buf();
    if workdir.join(relative).exists() {
        index.add_path(relative)?;
    } else {
        index.remove_path(relative)?;
    }
    index.write()?;
    Ok(())
}

/// Unstage a path, restoring the HEAD version in the index.
pub fn unstage(root: &Path, relative: &Path) -> Result<()> {
    let repo = Repository::discover(root)?;
    match repo.head().ok().and_then(|h| h.peel_to_commit().ok()) {
        Some(commit) => {
            repo.reset_default(Some(commit.as_object()), [relative])?;
        }
        None => {
            // No commits yet: simply drop the entry from the index.
            let mut index = repo.index()?;
            index.remove_path(relative)?;
            index.write()?;
        }
    }
    Ok(())
}

/// Discard working-tree changes for a path. Destructive: callers must confirm.
///
/// Untracked files are deleted; tracked files are restored from the index.
pub fn discard(root: &Path, relative: &Path) -> Result<()> {
    let repo = Repository::discover(root)?;
    let workdir = repo.workdir().unwrap_or(root).to_path_buf();
    let absolute = workdir.join(relative);

    let status = repo.status_file(relative).unwrap_or(Status::empty());
    if status.contains(Status::WT_NEW) && !status.contains(Status::INDEX_NEW) {
        if absolute.is_file() {
            std::fs::remove_file(&absolute)
                .with_context(|| format!("removing {}", absolute.display()))?;
        }
        return Ok(());
    }

    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.force().path(relative).remove_untracked(false);
    repo.checkout_index(None, Some(&mut checkout))
        .context("restoring file from the index")?;
    Ok(())
}

/// Stage every change (`git add -A`).
pub fn stage_all(root: &Path) -> Result<()> {
    let repo = Repository::discover(root)?;
    let mut index = repo.index()?;
    index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
    index.write()?;
    Ok(())
}

/// Unstage everything (`git reset`).
pub fn unstage_all(root: &Path) -> Result<()> {
    let repo = Repository::discover(root)?;
    let Some(commit) = repo.head().ok().and_then(|h| h.peel_to_commit().ok()) else {
        let mut index = repo.index()?;
        index.clear()?;
        index.write()?;
        return Ok(());
    };
    let paths: Vec<PathBuf> = snapshot(root)?
        .changes
        .into_iter()
        .filter(|c| c.is_staged())
        .map(|c| c.path)
        .collect();
    if !paths.is_empty() {
        repo.reset_default(Some(commit.as_object()), paths.iter())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A repository with one commit and a known dirty state.
    fn repo_fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let repo = Repository::init(&root).unwrap();
        {
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Test").unwrap();
            config.set_str("user.email", "test@example.com").unwrap();
        }
        fs::write(root.join("tracked.txt"), "one\ntwo\n").unwrap();
        fs::write(root.join("deleted.txt"), "gone\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.add_path(Path::new("deleted.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = repo.signature().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        drop(tree);

        fs::write(root.join("tracked.txt"), "one\nchanged\n").unwrap();
        fs::write(root.join("new.txt"), "fresh\n").unwrap();
        fs::remove_file(root.join("deleted.txt")).unwrap();
        (dir, root)
    }

    #[test]
    fn detects_repository() {
        let (_dir, root) = repo_fixture();
        assert!(is_repository(&root));
        assert!(!is_repository(Path::new("/")));
    }

    #[test]
    fn maps_worktree_statuses() {
        let (_dir, root) = repo_fixture();
        let snap = snapshot(&root).unwrap();
        assert!(snap.is_dirty());

        let by_path = |name: &str| {
            snap.changes
                .iter()
                .find(|c| c.path == Path::new(name))
                .cloned()
                .unwrap_or_else(|| panic!("{name} missing from {:?}", snap.changes))
        };
        assert_eq!(by_path("tracked.txt").worktree, Some(FileStatus::Modified));
        assert_eq!(by_path("new.txt").worktree, Some(FileStatus::Untracked));
        assert_eq!(by_path("deleted.txt").worktree, Some(FileStatus::Deleted));
        assert!(!by_path("tracked.txt").is_staged());
    }

    #[test]
    fn reports_the_branch_name() {
        let (_dir, root) = repo_fixture();
        let snap = snapshot(&root).unwrap();
        assert!(snap.branch.is_some(), "expected a branch name");
        assert!(!snap.has_upstream);
    }

    #[test]
    fn stage_then_unstage_roundtrip() {
        let (_dir, root) = repo_fixture();
        stage(&root, Path::new("tracked.txt")).unwrap();
        let staged = snapshot(&root).unwrap();
        let entry = staged
            .changes
            .iter()
            .find(|c| c.path == Path::new("tracked.txt"))
            .unwrap();
        assert_eq!(entry.index, Some(FileStatus::Modified));
        assert!(entry.is_staged());

        unstage(&root, Path::new("tracked.txt")).unwrap();
        let after = snapshot(&root).unwrap();
        let entry = after
            .changes
            .iter()
            .find(|c| c.path == Path::new("tracked.txt"))
            .unwrap();
        assert_eq!(entry.index, None);
        assert_eq!(entry.worktree, Some(FileStatus::Modified));
    }

    #[test]
    fn staging_a_deletion_works() {
        let (_dir, root) = repo_fixture();
        stage(&root, Path::new("deleted.txt")).unwrap();
        let snap = snapshot(&root).unwrap();
        let entry = snap
            .changes
            .iter()
            .find(|c| c.path == Path::new("deleted.txt"))
            .unwrap();
        assert_eq!(entry.index, Some(FileStatus::Deleted));
    }

    #[test]
    fn diff_contains_added_and_removed_lines() {
        let (_dir, root) = repo_fixture();
        let diff = diff_file(&root, Path::new("tracked.txt"), false).unwrap();
        let added: Vec<&str> = diff
            .lines
            .iter()
            .filter_map(|l| match l {
                DiffLine::Added(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        let removed: Vec<&str> = diff
            .lines
            .iter()
            .filter_map(|l| match l {
                DiffLine::Removed(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(added, vec!["changed"]);
        assert_eq!(removed, vec!["two"]);
        assert!(diff.lines.iter().any(|l| matches!(l, DiffLine::Hunk(_))));
    }

    #[test]
    fn staged_diff_uses_the_index() {
        let (_dir, root) = repo_fixture();
        stage(&root, Path::new("tracked.txt")).unwrap();
        let diff = diff_file(&root, Path::new("tracked.txt"), true).unwrap();
        assert!(diff.staged);
        assert!(diff
            .lines
            .iter()
            .any(|l| matches!(l, DiffLine::Added(s) if s == "changed")));
    }

    #[test]
    fn discard_restores_tracked_files() {
        let (_dir, root) = repo_fixture();
        discard(&root, Path::new("tracked.txt")).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "one\ntwo\n"
        );
    }

    #[test]
    fn discard_deletes_untracked_files() {
        let (_dir, root) = repo_fixture();
        discard(&root, Path::new("new.txt")).unwrap();
        assert!(!root.join("new.txt").exists());
    }

    #[test]
    fn stage_all_then_unstage_all() {
        let (_dir, root) = repo_fixture();
        stage_all(&root).unwrap();
        let staged = snapshot(&root).unwrap();
        assert!(staged.changes.iter().all(|c| c.is_staged()), "{staged:?}");

        unstage_all(&root).unwrap();
        let after = snapshot(&root).unwrap();
        assert!(
            after.changes.iter().all(|c| !c.is_staged()),
            "nothing should remain staged: {after:?}"
        );
    }

    #[test]
    fn snapshot_outside_a_repository_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(snapshot(dir.path()).is_err());
    }
}
