//! Filesystem watching.
//!
//! Changes are debounced before they reach the event loop: a build or an agent
//! can touch hundreds of files in a burst, and each burst should cost one tree
//! refresh, not hundreds.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer};

/// Keeps the watcher alive; dropping it stops watching.
pub struct WorkspaceWatcher {
    _debouncer: Debouncer<notify::RecommendedWatcher, notify_debouncer_full::RecommendedCache>,
}

/// Default debounce window.
pub const DEBOUNCE: Duration = Duration::from_millis(400);

impl WorkspaceWatcher {
    /// Watch `root` recursively, calling `on_change` with the changed paths.
    ///
    /// Paths inside ignored directories are filtered out before the callback
    /// runs, so `.git/` churn and build output do not wake the UI.
    pub fn start(
        root: &Path,
        ignore: Vec<String>,
        on_change: impl Fn(Vec<PathBuf>) + Send + 'static,
    ) -> Result<WorkspaceWatcher> {
        let ignore_for_filter = ignore.clone();
        let mut debouncer =
            new_debouncer(
                DEBOUNCE,
                None,
                move |result: DebounceEventResult| match result {
                    Ok(events) => {
                        let mut paths: Vec<PathBuf> = events
                            .into_iter()
                            .flat_map(|event| event.paths.clone())
                            .filter(|path| !is_ignored(path, &ignore_for_filter))
                            .collect();
                        paths.sort();
                        paths.dedup();
                        if !paths.is_empty() {
                            on_change(paths);
                        }
                    }
                    Err(errors) => {
                        for error in errors {
                            tracing::debug!(error = %error, "filesystem watch error");
                        }
                    }
                },
            )
            .context("starting the filesystem watcher")?;

        debouncer
            .watch(root, RecursiveMode::Recursive)
            .with_context(|| format!("watching {}", root.display()))?;

        Ok(WorkspaceWatcher {
            _debouncer: debouncer,
        })
    }
}

/// True when any path component is an ignored directory name.
pub fn is_ignored(path: &Path, ignore: &[String]) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        ignore.iter().any(|entry| *entry == name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn ignores_build_and_git_directories() {
        let ignore = vec![".git".to_string(), "target".to_string()];
        assert!(is_ignored(Path::new("/repo/.git/index"), &ignore));
        assert!(is_ignored(Path::new("/repo/target/debug/app"), &ignore));
        assert!(!is_ignored(Path::new("/repo/src/main.rs"), &ignore));
    }

    #[test]
    fn reports_changes_to_watched_files() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = WorkspaceWatcher::start(dir.path(), vec![".git".into()], move |paths| {
            let _ = tx.send(paths);
        })
        .unwrap();

        std::fs::write(dir.path().join("new.txt"), "hello").unwrap();

        let paths = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("a change notification");
        assert!(
            paths.iter().any(|p| p.ends_with("new.txt")),
            "unexpected paths: {paths:?}"
        );
    }

    #[test]
    fn ignored_paths_do_not_wake_the_loop() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = WorkspaceWatcher::start(dir.path(), vec!["target".into()], move |paths| {
            let _ = tx.send(paths);
        })
        .unwrap();

        std::fs::write(dir.path().join("target/artifact"), "x").unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(1500)).is_err(),
            "changes under an ignored directory must be filtered out"
        );
    }
}
