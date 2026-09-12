//! Filesystem mutations made from the explorer.
//!
//! Every operation is validated against the workspace root so a crafted name
//! cannot touch files outside the project, and deletes are explicit.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// Reject paths that escape `root`.
pub fn ensure_inside(root: &Path, path: &Path) -> Result<PathBuf> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    // `canonicalize` needs the path to exist, so normalise manually instead.
    let normalised = normalise(&candidate);
    let root = normalise(root);
    if !normalised.starts_with(&root) {
        bail!("{} is outside the workspace", normalised.display());
    }
    Ok(normalised)
}

/// Lexical normalisation: resolve `.` and `..` without touching the disk.
pub fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Create an empty file, failing when it already exists.
pub fn create_file(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = ensure_inside(root, path)?;
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&path, b"").with_context(|| format!("creating {}", path.display()))?;
    Ok(path)
}

/// Create a directory (and any missing parents).
pub fn create_dir(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = ensure_inside(root, path)?;
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    std::fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
    Ok(path)
}

/// Rename a file or directory inside the workspace.
pub fn rename(root: &Path, from: &Path, to: &Path) -> Result<PathBuf> {
    let from = ensure_inside(root, from)?;
    let to = ensure_inside(root, to)?;
    if !from.exists() {
        bail!("{} does not exist", from.display());
    }
    if to.exists() {
        bail!("{} already exists", to.display());
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::rename(&from, &to)
        .with_context(|| format!("renaming {} to {}", from.display(), to.display()))?;
    Ok(to)
}

/// Delete a file or directory tree. Callers must confirm first.
pub fn delete(root: &Path, path: &Path) -> Result<()> {
    let path = ensure_inside(root, path)?;
    if path == normalise(root) {
        bail!("refusing to delete the workspace root");
    }
    if path.is_dir() {
        std::fs::remove_dir_all(&path).with_context(|| format!("deleting {}", path.display()))?;
    } else {
        std::fs::remove_file(&path).with_context(|| format!("deleting {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        dir
    }

    #[test]
    fn rejects_escapes_from_the_workspace() {
        let dir = workspace();
        let err = ensure_inside(dir.path(), Path::new("../outside.txt")).unwrap_err();
        assert!(err.to_string().contains("outside the workspace"), "{err}");
        assert!(ensure_inside(dir.path(), Path::new("/etc/passwd")).is_err());
        assert!(ensure_inside(dir.path(), Path::new("src/../src/main.rs")).is_ok());
    }

    #[test]
    fn creates_files_and_folders() {
        let dir = workspace();
        let file = create_file(dir.path(), Path::new("src/new.rs")).unwrap();
        assert!(file.exists());
        let folder = create_dir(dir.path(), Path::new("src/nested/deep")).unwrap();
        assert!(folder.is_dir());
    }

    #[test]
    fn refuses_to_clobber_existing_paths() {
        let dir = workspace();
        assert!(create_file(dir.path(), Path::new("src/main.rs")).is_err());
        assert!(create_dir(dir.path(), Path::new("src")).is_err());
    }

    #[test]
    fn renames_within_the_workspace() {
        let dir = workspace();
        let moved = rename(
            dir.path(),
            Path::new("src/main.rs"),
            Path::new("src/app.rs"),
        )
        .unwrap();
        assert!(moved.exists());
        assert!(!dir.path().join("src/main.rs").exists());
        assert!(rename(dir.path(), Path::new("src/app.rs"), Path::new("../x.rs")).is_err());
    }

    #[test]
    fn deletes_files_and_trees_but_never_the_root() {
        let dir = workspace();
        delete(dir.path(), Path::new("src/main.rs")).unwrap();
        assert!(!dir.path().join("src/main.rs").exists());
        delete(dir.path(), Path::new("src")).unwrap();
        assert!(!dir.path().join("src").exists());

        let err = delete(dir.path(), Path::new(".")).unwrap_err();
        assert!(err.to_string().contains("workspace root"), "{err}");
    }
}
