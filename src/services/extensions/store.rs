//! Isolated installation and removal of declarative VSIX contents.

use std::fs::{self, File};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use zip::ZipArchive;

use crate::domain::extensions::CompatibilityReport;

use super::package::{inspect, inspect_directory, validate_archive};

/// Install into the platform data directory.
pub fn install(path: &Path) -> Result<CompatibilityReport> {
    let root = crate::config::extensions_dir()
        .ok_or_else(|| anyhow!("could not determine the TermLoom data directory"))?;
    install_into(path, &root)
}

/// Install into an explicit root (also used by tests).
pub fn install_into(path: &Path, root: &Path) -> Result<CompatibilityReport> {
    let inspected = inspect(path)?;
    fs::create_dir_all(root)
        .with_context(|| format!("creating extension directory {}", root.display()))?;
    let directory_name = safe_directory_name(&inspected.id);
    let destination = root.join(directory_name);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staging = root.join(format!(".install-{}-{stamp}", std::process::id()));
    let backup = root.join(format!(".replace-{}-{stamp}", std::process::id()));
    fs::create_dir(&staging)
        .with_context(|| format!("creating staging directory {}", staging.display()))?;

    let result = (|| -> Result<CompatibilityReport> {
        let file = File::open(path)?;
        let mut archive = ZipArchive::new(file)?;
        validate_archive(&mut archive)?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let relative = entry
                .enclosed_name()
                .ok_or_else(|| anyhow!("unsafe archive path: {}", entry.name()))?
                .to_path_buf();
            let output = staging.join(relative);
            if entry.is_dir() {
                fs::create_dir_all(&output)?;
                continue;
            }
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output_file = File::create(&output)?;
            io::copy(&mut entry, &mut output_file)?;
        }
        let staged_report = inspect_directory(&staging)?;
        if staged_report.class == crate::domain::extensions::CompatibilityClass::Broken {
            return Err(anyhow!(
                "{} contains missing or invalid declarative assets",
                staged_report.id
            ));
        }
        if destination.exists() {
            fs::rename(&destination, &backup)
                .with_context(|| format!("preparing to replace {}", destination.display()))?;
        }
        if let Err(error) = fs::rename(&staging, &destination) {
            if backup.exists() {
                fs::rename(&backup, &destination).with_context(|| {
                    format!(
                        "installing into {} failed and restoring the previous version also failed",
                        destination.display()
                    )
                })?;
            }
            return Err(error)
                .with_context(|| format!("installing into {}", destination.display()));
        }
        if backup.exists() {
            if let Err(error) = fs::remove_dir_all(&backup) {
                tracing::warn!(path = %backup.display(), %error, "could not remove replaced extension backup");
            }
        }
        inspect_directory(&destination)
    })();

    if staging.exists() {
        fs::remove_dir_all(&staging).ok();
    }
    result
}

pub fn list() -> Result<Vec<CompatibilityReport>> {
    let Some(root) = crate::config::extensions_dir() else {
        return Ok(Vec::new());
    };
    list_in(&root)
}

pub fn list_in(root: &Path) -> Result<Vec<CompatibilityReport>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut reports = Vec::new();
    for entry in fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !entry.file_type()?.is_dir()
            || name.starts_with(".install-")
            || name.starts_with(".replace-")
        {
            continue;
        }
        match inspect_directory(&entry.path()) {
            Ok(report) => reports.push(report),
            Err(error) => {
                tracing::warn!(path = %entry.path().display(), %error, "ignoring broken extension directory")
            }
        }
    }
    reports.sort_by_key(|report| report.id.to_ascii_lowercase());
    Ok(reports)
}

pub fn remove(id: &str) -> Result<bool> {
    let root = crate::config::extensions_dir()
        .ok_or_else(|| anyhow!("could not determine the TermLoom data directory"))?;
    remove_from(id, &root)
}

pub fn remove_from(id: &str, root: &Path) -> Result<bool> {
    let reports = list_in(root)?;
    let Some(report) = reports.into_iter().find(|report| report.id == id) else {
        return Ok(false);
    };
    let path = report
        .install_path
        .ok_or_else(|| anyhow!("installed extension has no path"))?;
    let canonical_root = root.canonicalize()?;
    let canonical_path = path.canonicalize()?;
    if canonical_path.parent() != Some(canonical_root.as_path()) {
        return Err(anyhow!(
            "refusing to remove a path outside the extension directory"
        ));
    }
    fs::remove_dir_all(&canonical_path)
        .with_context(|| format!("removing {}", canonical_path.display()))?;
    Ok(true)
}

fn safe_directory_name(id: &str) -> String {
    let readable: String = id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    // Sanitising alone can collide (`a/b` and `a_b`). Include the canonical
    // manifest id in a stable process-independent hash so one package can
    // never overwrite another package with a lookalike id.
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    format!("{readable}-{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    use super::*;
    use crate::services::extensions::package::MANIFEST_PATH;

    fn package(path: &Path) {
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(MANIFEST_PATH, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"name":"sample","publisher":"termloom","version":"1","contributes":{"languages":[{"id":"loom","extensions":[".loom"]}]}}"#)
            .unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn install_list_remove_round_trip() {
        let dir = tempdir().unwrap();
        let vsix = dir.path().join("sample.vsix");
        let root = dir.path().join("extensions");
        package(&vsix);

        let installed = install_into(&vsix, &root).unwrap();
        assert_eq!(installed.id, "termloom.sample");
        assert!(installed.install_path.unwrap().starts_with(&root));
        assert_eq!(list_in(&root).unwrap().len(), 1);
        assert!(remove_from("termloom.sample", &root).unwrap());
        assert!(list_in(&root).unwrap().is_empty());
        assert!(!remove_from("termloom.sample", &root).unwrap());
    }

    #[test]
    fn removal_uses_manifest_identity_not_user_paths() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("extensions");
        fs::create_dir_all(&root).unwrap();
        assert!(!remove_from("../../outside", &root).unwrap());
    }
}
