//! Read-only VSIX and installed-package inspection.

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use zip::ZipArchive;

use crate::domain::extensions::{CompatibilityClass, CompatibilityReport, ExtensionAsset};

use super::classifier::{classify, safe_relative};
use super::manifest::ExtensionManifest;

pub const MANIFEST_PATH: &str = "extension/package.json";
const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_FILE_SIZE: u64 = 64 * 1024 * 1024;
const MAX_UNCOMPRESSED_SIZE: u64 = 512 * 1024 * 1024;

/// Inspect a VSIX without extracting or executing it.
pub fn inspect(path: &Path) -> Result<CompatibilityReport> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = ZipArchive::new(file)
        .with_context(|| format!("{} is not a readable VSIX/ZIP archive", path.display()))?;
    let entries = validate_archive(&mut archive)?;
    let mut manifest_text = String::new();
    archive
        .by_name(MANIFEST_PATH)
        .with_context(|| format!("VSIX has no {MANIFEST_PATH}"))?
        .read_to_string(&mut manifest_text)
        .context("reading extension/package.json as UTF-8")?;
    let manifest = ExtensionManifest::parse(&manifest_text)?;
    let mut report = classify(&manifest, None, |asset| {
        archive_asset_path(asset)
            .map(|path| entries.contains(&path))
            .unwrap_or(false)
    });
    validate_archive_assets(&mut archive, &mut report);
    Ok(report)
}

/// Inspect an already-extracted package directory.
pub fn inspect_directory(package_root: &Path) -> Result<CompatibilityReport> {
    let extension_root = package_root.join("extension");
    let manifest_path = extension_root.join("package.json");
    let text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest = ExtensionManifest::parse(&text)?;
    let canonical_root = extension_root
        .canonicalize()
        .with_context(|| format!("resolving {}", extension_root.display()))?;
    let mut report = classify(&manifest, Some(package_root.to_path_buf()), |asset| {
        if !safe_relative(asset) {
            return false;
        }
        extension_root
            .join(asset)
            .canonicalize()
            .is_ok_and(|path| path.is_file() && path.starts_with(&canonical_root))
    });
    validate_directory_assets(&extension_root, &mut report);
    Ok(report)
}

fn validate_archive_assets<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    report: &mut CompatibilityReport,
) {
    let mut valid = Vec::new();
    for asset in std::mem::take(&mut report.assets) {
        let result = match &asset {
            ExtensionAsset::Snippets { path, .. } => read_archive_json(archive, path).map(|_| ()),
            ExtensionAsset::Theme { path, .. } => {
                validate_archive_theme(archive, path, &mut HashSet::new(), 0)
            }
            ExtensionAsset::Language {
                configuration: Some(path),
                ..
            } => read_archive_json(archive, path).map(|_| ()),
            _ => Ok(()),
        };
        match result {
            Ok(()) => valid.push(asset),
            Err(error) => mark_asset_broken(report, &asset, &error),
        }
    }
    report.assets = valid;
}

fn validate_directory_assets(extension_root: &Path, report: &mut CompatibilityReport) {
    let canonical_root = match extension_root.canonicalize() {
        Ok(root) => root,
        Err(error) => {
            report.class = CompatibilityClass::Broken;
            tracing::warn!(path = %extension_root.display(), %error, "extension root disappeared during inspection");
            return;
        }
    };
    let mut valid = Vec::new();
    for asset in std::mem::take(&mut report.assets) {
        let result = match &asset {
            ExtensionAsset::Snippets { path, .. } => {
                read_directory_json(&canonical_root, path).map(|_| ())
            }
            ExtensionAsset::Theme { path, .. } => {
                validate_directory_theme(&canonical_root, path, &mut HashSet::new(), 0)
            }
            ExtensionAsset::Language {
                configuration: Some(path),
                ..
            } => read_directory_json(&canonical_root, path).map(|_| ()),
            _ => Ok(()),
        };
        match result {
            Ok(()) => valid.push(asset),
            Err(error) => mark_asset_broken(report, &asset, &error),
        }
    }
    report.assets = valid;
}

fn mark_asset_broken(
    report: &mut CompatibilityReport,
    asset: &ExtensionAsset,
    error: &anyhow::Error,
) {
    report.class = CompatibilityClass::Broken;
    let label = asset.label();
    if let Some(capability) = report
        .capabilities
        .iter_mut()
        .find(|capability| capability.name == label)
    {
        capability.support = crate::domain::extensions::CapabilitySupport::Unsupported;
        capability.detail = format!("invalid declarative asset: {error:#}");
    }
}

fn read_archive_json<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    relative: &Path,
) -> Result<serde_json::Value> {
    let path = archive_asset_path(relative).ok_or_else(|| anyhow!("unsafe asset path"))?;
    let name = path.to_string_lossy();
    let mut text = String::new();
    archive
        .by_name(&name)
        .with_context(|| format!("reading {name}"))?
        .read_to_string(&mut text)
        .with_context(|| format!("reading {name} as UTF-8"))?;
    let value =
        crate::services::vscode::parse_jsonc(&text).with_context(|| format!("parsing {name}"))?;
    if !value.is_object() {
        return Err(anyhow!("{name} must contain a JSON object"));
    }
    Ok(value)
}

fn validate_archive_theme<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    relative: &Path,
    visited: &mut HashSet<PathBuf>,
    depth: usize,
) -> Result<()> {
    if depth >= 16 {
        return Err(anyhow!("theme include nesting is too deep"));
    }
    if !visited.insert(relative.to_path_buf()) {
        return Err(anyhow!("theme contains an include cycle"));
    }
    let value = read_archive_json(archive, relative)?;
    if let Some(include) = value["include"].as_str() {
        let include = resolve_include(relative, include)
            .ok_or_else(|| anyhow!("theme include escapes the extension directory"))?;
        validate_archive_theme(archive, &include, visited, depth + 1)?;
    }
    visited.remove(relative);
    Ok(())
}

fn read_directory_json(root: &Path, relative: &Path) -> Result<serde_json::Value> {
    let path = root.join(relative).canonicalize()?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(anyhow!("asset escapes the extension directory"));
    }
    let text = std::fs::read_to_string(&path)?;
    let value = crate::services::vscode::parse_jsonc(&text)?;
    if !value.is_object() {
        return Err(anyhow!("{} must contain a JSON object", path.display()));
    }
    Ok(value)
}

fn validate_directory_theme(
    root: &Path,
    relative: &Path,
    visited: &mut HashSet<PathBuf>,
    depth: usize,
) -> Result<()> {
    if depth >= 16 {
        return Err(anyhow!("theme include nesting is too deep"));
    }
    if !visited.insert(relative.to_path_buf()) {
        return Err(anyhow!("theme contains an include cycle"));
    }
    let value = read_directory_json(root, relative)?;
    if let Some(include) = value["include"].as_str() {
        let include = resolve_include(relative, include)
            .ok_or_else(|| anyhow!("theme include escapes the extension directory"))?;
        validate_directory_theme(root, &include, visited, depth + 1)?;
    }
    visited.remove(relative);
    Ok(())
}

fn resolve_include(from: &Path, include: &str) -> Option<PathBuf> {
    let include = Path::new(include);
    if include.is_absolute() {
        return None;
    }
    let mut output = from.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
    for component in include.components() {
        match component {
            Component::Normal(value) => output.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !output.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!output.as_os_str().is_empty()).then_some(output)
}

/// Validate every entry before either inspection or extraction.  `enclosed_name`
/// rejects absolute paths and `..`; symlinks are rejected because following one
/// after installation could escape the isolated extension directory.
pub(crate) fn validate_archive<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<HashSet<PathBuf>> {
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(anyhow!(
            "VSIX has too many entries ({}; maximum is {MAX_ARCHIVE_ENTRIES})",
            archive.len()
        ));
    }
    let mut entries = HashSet::new();
    let mut total_size = 0_u64;
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let enclosed = file
            .enclosed_name()
            .ok_or_else(|| anyhow!("unsafe archive path: {}", file.name()))?;
        if file
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(anyhow!("symbolic links are not allowed in VSIX packages"));
        }
        if file.size() > MAX_FILE_SIZE {
            return Err(anyhow!(
                "VSIX entry {} is too large (maximum is {} MiB)",
                file.name(),
                MAX_FILE_SIZE / 1024 / 1024
            ));
        }
        total_size = total_size
            .checked_add(file.size())
            .ok_or_else(|| anyhow!("VSIX uncompressed size overflow"))?;
        if total_size > MAX_UNCOMPRESSED_SIZE {
            return Err(anyhow!(
                "VSIX is too large when unpacked (maximum is {} MiB)",
                MAX_UNCOMPRESSED_SIZE / 1024 / 1024
            ));
        }
        entries.insert(enclosed.to_path_buf());
    }
    if !entries.contains(Path::new(MANIFEST_PATH)) {
        return Err(anyhow!("VSIX has no {MANIFEST_PATH}"));
    }
    Ok(entries)
}

pub(crate) fn archive_asset_path(asset: &Path) -> Option<PathBuf> {
    safe_relative(asset).then(|| Path::new("extension").join(asset))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn inspects_a_real_vsix_without_running_entrypoint() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sample.vsix");
        let file = File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(MANIFEST_PATH, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"name":"sample","publisher":"termloom","version":"1.0.0","main":"out/main.js","contributes":{"languages":[{"id":"loom","extensions":[".loom"]}],"snippets":[{"language":"loom","path":"snippets/loom.json"}]}}"#)
            .unwrap();
        zip.start_file("extension/snippets/loom.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"{}").unwrap();
        zip.finish().unwrap();

        let report = inspect(&path).unwrap();
        assert_eq!(report.id, "termloom.sample");
        assert_eq!(report.assets.len(), 2);
        assert!(report.capabilities.iter().any(|capability| {
            capability.name == "extension entrypoint"
                && capability.support == crate::domain::extensions::CapabilitySupport::Unsupported
        }));
    }

    #[test]
    fn rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.vsix");
        let file = File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(MANIFEST_PATH, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"name":"bad","version":"1"}"#).unwrap();
        zip.start_file("../escape", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"nope").unwrap();
        zip.finish().unwrap();
        assert!(inspect(&path).unwrap_err().to_string().contains("unsafe"));
    }

    #[test]
    fn invalid_supported_json_marks_the_package_broken() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad-snippets.vsix");
        let file = File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(MANIFEST_PATH, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"name":"bad","version":"1","contributes":{"snippets":[{"language":"x","path":"snippets/x.json"}]}}"#)
            .unwrap();
        zip.start_file("extension/snippets/x.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"not json").unwrap();
        zip.finish().unwrap();

        let report = inspect(&path).unwrap();
        assert_eq!(report.class, CompatibilityClass::Broken);
        assert!(report.assets.is_empty());
        assert!(report.capabilities[0].detail.contains("invalid"));
    }

    #[test]
    fn validates_theme_includes_inside_the_archive() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("theme.vsix");
        let file = File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(MANIFEST_PATH, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"name":"theme","version":"1","contributes":{"themes":[{"label":"Theme","path":"themes/dark.json"}]}}"#)
            .unwrap();
        zip.start_file("extension/themes/dark.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"include":"../base.json","colors":{}}"#)
            .unwrap();
        zip.start_file("extension/base.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br##"{"colors":{"editor.background":"#000000"}}"##)
            .unwrap();
        zip.finish().unwrap();

        assert_eq!(inspect(&path).unwrap().class, CompatibilityClass::Full);
    }
}
