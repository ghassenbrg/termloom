//! Workspace discovery and the lazily-expanded project tree.
//!
//! Directory children are read only when a folder is expanded, so opening a
//! repository with 10k+ files does not walk the whole tree on the UI thread.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::WorkspaceConfig;

/// The opened project.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Directory passed on the command line (canonicalised).
    pub root: PathBuf,
    /// Git repository root, when the workspace is inside one.
    pub git_root: Option<PathBuf>,
    /// Display name (last path component).
    pub name: String,
}

impl Workspace {
    /// Resolve a user-supplied path into a workspace.
    ///
    /// Files are accepted: the parent directory becomes the workspace root and
    /// the file is reported so the caller can open it in an editor tab.
    pub fn discover(path: &Path) -> Result<(Workspace, Option<PathBuf>)> {
        let resolved = std::fs::canonicalize(path)
            .with_context(|| format!("cannot open {}", path.display()))?;

        let (root, initial_file) = if resolved.is_dir() {
            (resolved, None)
        } else {
            let parent = resolved
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("/"));
            (parent, Some(resolved))
        };

        if !root.is_dir() {
            bail!("{} is not a directory", root.display());
        }

        let name = root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("/")
            .to_string();

        Ok((
            Workspace {
                git_root: find_git_root(&root),
                root,
                name,
            },
            initial_file,
        ))
    }

    /// Path shown in the header, with `$HOME` shortened to `~`.
    pub fn display_path(&self) -> String {
        shorten_home(&self.root)
    }

    /// Path relative to the workspace root, or the full path when outside.
    pub fn relative<'a>(&self, path: &'a Path) -> &'a Path {
        path.strip_prefix(&self.root).unwrap_or(path)
    }
}

/// Walk up from `start` looking for a `.git` directory or file (worktrees).
pub fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

/// Replace the home prefix with `~`.
pub fn shorten_home(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            if rest.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

/// One entry in the project tree.
#[derive(Debug, Clone)]
pub struct TreeNode {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub expanded: bool,
    /// Depth below the root (root itself is 0).
    pub depth: usize,
    /// Children, loaded on first expansion.
    pub children: Vec<TreeNode>,
    pub loaded: bool,
}

impl TreeNode {
    fn new(path: PathBuf, is_dir: bool, depth: usize) -> Self {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        Self {
            path,
            name,
            is_dir,
            expanded: false,
            depth,
            children: Vec::new(),
            loaded: false,
        }
    }
}

/// A row of the flattened, currently visible tree.
#[derive(Debug, Clone)]
pub struct TreeRow {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub expanded: bool,
    pub depth: usize,
}

/// Lazily expanded directory tree rooted at the workspace.
#[derive(Debug, Clone)]
pub struct FileTree {
    pub root: PathBuf,
    nodes: Vec<TreeNode>,
    options: ScanOptions,
}

/// Filtering rules shared by the tree and the quick-open index.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub ignore_names: Vec<String>,
    pub respect_gitignore: bool,
    pub show_hidden: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        let cfg = WorkspaceConfig::default();
        Self {
            ignore_names: cfg.ignore,
            respect_gitignore: cfg.respect_gitignore,
            show_hidden: cfg.show_hidden,
        }
    }
}

impl ScanOptions {
    pub fn from_config(cfg: &WorkspaceConfig) -> Self {
        Self {
            ignore_names: cfg.ignore.clone(),
            respect_gitignore: cfg.respect_gitignore,
            show_hidden: cfg.show_hidden,
        }
    }

    /// Whether a directory entry should be hidden from the explorer.
    pub fn is_excluded(&self, name: &str) -> bool {
        if self.ignore_names.iter().any(|i| i == name) {
            return true;
        }
        if !self.show_hidden && name.starts_with('.') && name != ".." {
            return true;
        }
        false
    }
}

impl FileTree {
    /// Build a tree and load the root's children.
    pub fn new(root: PathBuf, options: ScanOptions) -> FileTree {
        let mut tree = FileTree {
            root: root.clone(),
            nodes: Vec::new(),
            options,
        };
        tree.nodes = read_dir(&root, 1, &tree.options);
        tree
    }

    /// Re-read every directory that is currently expanded.
    pub fn refresh(&mut self) {
        let expanded = self.expanded_paths();
        self.nodes = read_dir(&self.root, 1, &self.options);
        for path in expanded {
            self.expand(&path);
        }
    }

    /// Visible rows, in display order.
    pub fn rows(&self) -> Vec<TreeRow> {
        let mut out = Vec::new();
        collect_rows(&self.nodes, &mut out);
        out
    }

    /// Expand a directory (loading its children when needed).
    pub fn expand(&mut self, path: &Path) -> bool {
        let options = self.options.clone();
        match find_node_mut(&mut self.nodes, path) {
            Some(node) if node.is_dir => {
                if !node.loaded {
                    node.children = read_dir(&node.path, node.depth + 1, &options);
                    node.loaded = true;
                }
                node.expanded = true;
                true
            }
            _ => false,
        }
    }

    /// Collapse a directory, keeping its loaded children cached.
    pub fn collapse(&mut self, path: &Path) -> bool {
        match find_node_mut(&mut self.nodes, path) {
            Some(node) if node.is_dir => {
                node.expanded = false;
                true
            }
            _ => false,
        }
    }

    /// Toggle expansion; returns the new expanded state.
    pub fn toggle(&mut self, path: &Path) -> bool {
        let expanded = self
            .node(path)
            .map(|n| n.is_dir && n.expanded)
            .unwrap_or(false);
        if expanded {
            self.collapse(path);
            false
        } else {
            self.expand(path)
        }
    }

    /// Expand every ancestor so `path` becomes visible.
    pub fn reveal(&mut self, path: &Path) {
        let Ok(rel) = path.strip_prefix(&self.root) else {
            return;
        };
        let mut current = self.root.clone();
        let components: Vec<_> = rel.components().collect();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            current = current.join(component);
            if !self.expand(&current) {
                // Directory not in the tree (filtered or removed): stop here.
                return;
            }
        }
    }

    pub fn node(&self, path: &Path) -> Option<&TreeNode> {
        find_node(&self.nodes, path)
    }

    fn expanded_paths(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        fn walk(nodes: &[TreeNode], out: &mut Vec<PathBuf>) {
            for node in nodes {
                if node.is_dir && node.expanded {
                    out.push(node.path.clone());
                    walk(&node.children, out);
                }
            }
        }
        walk(&self.nodes, &mut out);
        out
    }
}

fn collect_rows(nodes: &[TreeNode], out: &mut Vec<TreeRow>) {
    for node in nodes {
        out.push(TreeRow {
            path: node.path.clone(),
            name: node.name.clone(),
            is_dir: node.is_dir,
            expanded: node.expanded,
            depth: node.depth,
        });
        if node.is_dir && node.expanded {
            collect_rows(&node.children, out);
        }
    }
}

fn find_node<'a>(nodes: &'a [TreeNode], path: &Path) -> Option<&'a TreeNode> {
    for node in nodes {
        if node.path == path {
            return Some(node);
        }
        if node.is_dir && path.starts_with(&node.path) {
            if let Some(found) = find_node(&node.children, path) {
                return Some(found);
            }
        }
    }
    None
}

fn find_node_mut<'a>(nodes: &'a mut [TreeNode], path: &Path) -> Option<&'a mut TreeNode> {
    for node in nodes {
        if node.path == path {
            return Some(node);
        }
        if node.is_dir && path.starts_with(&node.path) {
            if let Some(found) = find_node_mut(&mut node.children, path) {
                return Some(found);
            }
        }
    }
    None
}

/// Read one directory level, applying the scan filters. Directories first,
/// then files, each alphabetically (case-insensitive).
fn read_dir(dir: &Path, depth: usize, options: &ScanOptions) -> Vec<TreeNode> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut nodes: Vec<TreeNode> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if options.is_excluded(&name) {
                return None;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some(TreeNode::new(entry.path(), is_dir, depth))
        })
        .collect();

    nodes.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    nodes
}

/// Flat list of workspace files used by quick open. Built off the UI thread.
#[derive(Debug, Clone, Default)]
pub struct FileIndex {
    /// Paths relative to the workspace root.
    pub files: Vec<PathBuf>,
    /// True when the scan hit [`FileIndex::MAX_FILES`] and stopped early.
    pub truncated: bool,
}

impl FileIndex {
    /// Upper bound so a pathological tree cannot exhaust memory.
    pub const MAX_FILES: usize = 200_000;

    /// Walk the workspace honouring ignore rules. Blocking; run in a thread.
    pub fn scan(root: &Path, options: &ScanOptions) -> FileIndex {
        let mut builder = ignore::WalkBuilder::new(root);
        builder
            .hidden(!options.show_hidden)
            .git_ignore(options.respect_gitignore)
            .git_global(options.respect_gitignore)
            .git_exclude(options.respect_gitignore)
            .parents(options.respect_gitignore)
            .follow_links(false);

        let ignore_names = options.ignore_names.clone();
        builder.filter_entry(move |entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            !ignore_names.contains(&name)
        });

        let mut files = Vec::new();
        let mut truncated = false;
        for entry in builder.build().flatten() {
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                if files.len() >= Self::MAX_FILES {
                    truncated = true;
                    break;
                }
                if let Ok(rel) = entry.path().strip_prefix(root) {
                    files.push(rel.to_path_buf());
                }
            }
        }
        files.sort();
        FileIndex { files, truncated }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src/app")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]").unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(root.join("src/app/mod.rs"), "").unwrap();
        fs::write(root.join("target/debug/binary"), "").unwrap();
        dir
    }

    #[test]
    fn discovers_git_root_from_a_subdirectory() {
        let dir = fixture();
        let (ws, initial) = Workspace::discover(&dir.path().join("src/app")).unwrap();
        assert!(initial.is_none());
        let git_root = ws.git_root.expect("git root found");
        assert_eq!(
            std::fs::canonicalize(&git_root).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
    }

    #[test]
    fn opening_a_file_uses_its_parent_as_root() {
        let dir = fixture();
        let (ws, initial) = Workspace::discover(&dir.path().join("src/main.rs")).unwrap();
        assert_eq!(initial.unwrap().file_name().unwrap(), "main.rs");
        assert_eq!(ws.root.file_name().unwrap(), "src");
    }

    #[test]
    fn missing_path_is_an_error() {
        assert!(Workspace::discover(Path::new("/no/such/place/at/all")).is_err());
    }

    #[test]
    fn tree_hides_ignored_directories() {
        let dir = fixture();
        let tree = FileTree::new(dir.path().to_path_buf(), ScanOptions::default());
        let names: Vec<String> = tree.rows().into_iter().map(|r| r.name).collect();
        assert!(names.contains(&"src".to_string()));
        assert!(names.contains(&"Cargo.toml".to_string()));
        assert!(!names.contains(&"target".to_string()), "{names:?}");
        assert!(!names.contains(&".git".to_string()));
    }

    #[test]
    fn directories_sort_before_files() {
        let dir = fixture();
        let tree = FileTree::new(dir.path().to_path_buf(), ScanOptions::default());
        let rows = tree.rows();
        assert!(rows[0].is_dir);
        assert_eq!(rows[0].name, "src");
    }

    #[test]
    fn children_load_only_on_expansion() {
        let dir = fixture();
        let mut tree = FileTree::new(dir.path().to_path_buf(), ScanOptions::default());
        assert_eq!(tree.rows().len(), 2, "root has src/ and Cargo.toml");

        tree.expand(&dir.path().join("src"));
        let names: Vec<String> = tree.rows().into_iter().map(|r| r.name).collect();
        assert!(names.contains(&"main.rs".to_string()));

        tree.collapse(&dir.path().join("src"));
        assert_eq!(tree.rows().len(), 2);
    }

    #[test]
    fn reveal_expands_every_ancestor() {
        let dir = fixture();
        let mut tree = FileTree::new(dir.path().to_path_buf(), ScanOptions::default());
        tree.reveal(&dir.path().join("src/app/mod.rs"));
        let rows: Vec<String> = tree.rows().into_iter().map(|r| r.name).collect();
        assert!(rows.contains(&"mod.rs".to_string()), "{rows:?}");
    }

    #[test]
    fn refresh_keeps_expansion_state() {
        let dir = fixture();
        let mut tree = FileTree::new(dir.path().to_path_buf(), ScanOptions::default());
        tree.expand(&dir.path().join("src"));
        fs::write(dir.path().join("src/added.rs"), "").unwrap();
        tree.refresh();
        let names: Vec<String> = tree.rows().into_iter().map(|r| r.name).collect();
        assert!(names.contains(&"added.rs".to_string()));
        assert!(names.contains(&"main.rs".to_string()));
    }

    #[test]
    fn file_index_skips_ignored_directories() {
        let dir = fixture();
        let index = FileIndex::scan(dir.path(), &ScanOptions::default());
        assert!(index.files.contains(&PathBuf::from("src/main.rs")));
        assert!(!index
            .files
            .iter()
            .any(|p| p.starts_with("target") || p.starts_with(".git")));
        assert!(!index.truncated);
    }

    #[test]
    fn home_paths_are_shortened() {
        if let Some(home) = dirs::home_dir() {
            assert_eq!(shorten_home(&home), "~");
            assert_eq!(shorten_home(&home.join("code")), "~/code");
        }
    }
}
