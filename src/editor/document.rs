//! An open editor tab: buffer plus everything the workbench tracks about it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{bail, Context, Result};

use crate::config::EditorConfig;
use crate::domain::diagnostics::{Diagnostic, Position, Symbol};
use crate::domain::ids::EditorTabId;
use crate::services::syntax::{LanguageId, SyntaxIndex};

use super::buffer::TextBuffer;
use super::search::SearchState;

/// File identity recorded when the document was last read or written, used to
/// notice edits made outside TermLoom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskState {
    pub modified: Option<SystemTime>,
    pub len: u64,
}

impl DiskState {
    fn read(path: &Path) -> Option<DiskState> {
        let meta = std::fs::metadata(path).ok()?;
        Some(DiskState {
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// What happened to a file on disk while it was open here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalChange {
    None,
    /// Contents changed on disk.
    Modified,
    /// The file no longer exists.
    Deleted,
}

/// One open document.
#[derive(Debug)]
pub struct Document {
    pub id: EditorTabId,
    /// `None` for scratch buffers that have never been saved.
    pub path: Option<PathBuf>,
    pub buffer: TextBuffer,
    pub language: LanguageId,
    /// First visible line.
    pub scroll: usize,
    /// First visible column (character based).
    pub h_scroll: usize,
    /// Cached highlight index and the buffer version it was built from.
    pub syntax: Option<SyntaxIndex>,
    pub syntax_version: i64,
    /// Cached outline and its buffer version.
    pub symbols: Vec<Symbol>,
    pub symbols_version: i64,
    /// Diagnostics reported for this file by a language server.
    pub diagnostics: Vec<Diagnostic>,
    /// Lines carrying a debug breakpoint (0-based).
    pub breakpoints: BTreeSet<usize>,
    pub search: SearchState,
    disk: Option<DiskState>,
    pub external_change: ExternalChange,
    pub read_only: bool,
}

impl Document {
    /// Open a file from disk.
    pub fn open(path: &Path, config: &EditorConfig, language: LanguageId) -> Result<Document> {
        let meta =
            std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
        if meta.is_dir() {
            bail!("{} is a directory", path.display());
        }
        if meta.len() > config.max_file_size {
            bail!(
                "{} is {} bytes, larger than the configured limit of {}",
                path.display(),
                meta.len(),
                config.max_file_size
            );
        }
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        if bytes.contains(&0) {
            bail!("{} looks like a binary file", path.display());
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let read_only = meta.permissions().readonly();

        Ok(Document {
            id: EditorTabId::next(),
            path: Some(path.to_path_buf()),
            buffer: TextBuffer::from_str(&text),
            language,
            scroll: 0,
            h_scroll: 0,
            syntax: None,
            syntax_version: -1,
            symbols: Vec::new(),
            symbols_version: -1,
            diagnostics: Vec::new(),
            breakpoints: BTreeSet::new(),
            search: SearchState::default(),
            disk: DiskState::read(path),
            external_change: ExternalChange::None,
            read_only,
        })
    }

    /// An unsaved scratch buffer.
    pub fn scratch(language: LanguageId) -> Document {
        Document {
            id: EditorTabId::next(),
            path: None,
            buffer: TextBuffer::from_str(""),
            language,
            scroll: 0,
            h_scroll: 0,
            syntax: None,
            syntax_version: -1,
            symbols: Vec::new(),
            symbols_version: -1,
            diagnostics: Vec::new(),
            breakpoints: BTreeSet::new(),
            search: SearchState::default(),
            disk: None,
            external_change: ExternalChange::None,
            read_only: false,
        }
    }

    /// Tab label (`main.rs`, or `untitled`).
    pub fn display_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("untitled")
            .to_string()
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.is_dirty()
    }

    /// Write the buffer to disk, applying the configured save-time fixups.
    pub fn save(&mut self, config: &EditorConfig) -> Result<PathBuf> {
        let Some(path) = self.path.clone() else {
            bail!("this buffer has no path; use save-as");
        };
        if self.read_only {
            bail!("{} is read-only", path.display());
        }
        self.apply_save_fixups(config);

        let text = self.buffer.to_text();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&path, text.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;

        self.buffer.mark_saved();
        self.disk = DiskState::read(&path);
        self.external_change = ExternalChange::None;
        Ok(path)
    }

    /// Save under a new path.
    pub fn save_as(&mut self, path: &Path, config: &EditorConfig) -> Result<PathBuf> {
        self.path = Some(path.to_path_buf());
        self.read_only = false;
        self.save(config)
    }

    fn apply_save_fixups(&mut self, config: &EditorConfig) {
        if config.trim_trailing_whitespace {
            let trimmed: String = self
                .buffer
                .lines()
                .iter()
                .map(|line| line.trim_end().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let cursor = self.buffer.cursor();
            let mut next = TextBuffer::from_str(&trimmed);
            next.crlf = self.buffer.crlf;
            next.trailing_newline = self.buffer.trailing_newline;
            // Preserve dirtiness and cursor; the fixup is part of this save.
            next.move_to(cursor, false);
            self.buffer = next;
        }
        if config.insert_final_newline {
            self.buffer.trailing_newline = true;
        }
    }

    /// Re-read the file, discarding in-memory changes.
    pub fn reload(&mut self) -> Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reloading {}", path.display()))?;
        let cursor = self.buffer.cursor();
        self.buffer = TextBuffer::from_str(&text);
        self.buffer.move_to(cursor, false);
        self.disk = DiskState::read(&path);
        self.external_change = ExternalChange::None;
        self.syntax_version = -1;
        self.symbols_version = -1;
        Ok(())
    }

    /// Compare the recorded file identity with the current one.
    ///
    /// The result is stored on the document so the UI can warn before a save
    /// overwrites someone else's edit; it never silently reloads.
    pub fn check_external_change(&mut self) -> ExternalChange {
        let Some(path) = self.path.clone() else {
            return ExternalChange::None;
        };
        let change = match (DiskState::read(&path), &self.disk) {
            (None, Some(_)) => ExternalChange::Deleted,
            (Some(current), Some(recorded)) if &current != recorded => ExternalChange::Modified,
            _ => ExternalChange::None,
        };
        self.external_change = change;
        change
    }

    /// True when saving would clobber an external edit.
    pub fn save_would_conflict(&self) -> bool {
        self.external_change == ExternalChange::Modified
    }

    /// Accept the on-disk state without reloading (used after a forced save).
    pub fn accept_disk_state(&mut self) {
        if let Some(path) = &self.path {
            self.disk = DiskState::read(path);
        }
        self.external_change = ExternalChange::None;
    }

    /// Toggle a breakpoint; returns the new state.
    pub fn toggle_breakpoint(&mut self, line: usize) -> bool {
        if self.breakpoints.contains(&line) {
            self.breakpoints.remove(&line);
            false
        } else {
            self.breakpoints.insert(line);
            true
        }
    }

    /// Keep the cursor inside the viewport, returning the new scroll offset.
    pub fn ensure_cursor_visible(&mut self, height: usize, width: usize) {
        let cursor = self.buffer.cursor();
        if height > 0 {
            if cursor.line < self.scroll {
                self.scroll = cursor.line;
            } else if cursor.line >= self.scroll + height {
                self.scroll = cursor.line + 1 - height;
            }
        }
        if width > 0 {
            if cursor.character < self.h_scroll {
                self.h_scroll = cursor.character;
            } else if cursor.character >= self.h_scroll + width {
                self.h_scroll = cursor.character + 1 - width;
            }
        }
    }

    /// Move the cursor to a position and reveal it.
    pub fn goto(&mut self, position: Position) {
        let clamped = self.buffer.clamp(position);
        self.buffer.move_to(clamped, false);
    }

    /// Diagnostics on a given line.
    pub fn diagnostics_on(&self, line: usize) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(move |d| d.range.start.line == line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::syntax::detect;

    fn config() -> EditorConfig {
        EditorConfig::default()
    }

    fn temp_file(name: &str, contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    #[test]
    fn opens_a_file_and_detects_its_language() {
        let (_dir, path) = temp_file("main.rs", "fn main() {}\n");
        let doc = Document::open(&path, &config(), detect(&path)).unwrap();
        assert_eq!(doc.display_name(), "main.rs");
        assert_eq!(doc.language.as_str(), "rust");
        assert!(!doc.is_dirty());
    }

    #[test]
    fn refuses_binary_and_oversized_files() {
        let (_dir, path) = temp_file("blob.bin", "abc\0def");
        let err = Document::open(&path, &config(), detect(&path)).unwrap_err();
        assert!(err.to_string().contains("binary"), "{err}");

        let (_dir2, big) = temp_file("big.txt", &"x".repeat(1024));
        let mut cfg = config();
        cfg.max_file_size = 10;
        let err = Document::open(&big, &cfg, detect(&big)).unwrap_err();
        assert!(err.to_string().contains("larger than"), "{err}");
    }

    #[test]
    fn editing_then_saving_writes_to_disk() {
        let (_dir, path) = temp_file("notes.txt", "one\n");
        let mut doc = Document::open(&path, &config(), detect(&path)).unwrap();
        // "one\n" is one line plus a trailing newline, so appending a line
        // means inserting a newline and the text.
        doc.buffer.move_document_end(false);
        doc.buffer.insert("\ntwo");
        assert!(doc.is_dirty());

        doc.save(&config()).unwrap();
        assert!(!doc.is_dirty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
    }

    #[test]
    fn save_fixups_are_opt_in() {
        let (_dir, path) = temp_file("f.txt", "trailing   \nno-newline");
        let mut cfg = config();
        cfg.trim_trailing_whitespace = true;
        cfg.insert_final_newline = true;
        let mut doc = Document::open(&path, &cfg, detect(&path)).unwrap();
        doc.save(&cfg).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "trailing\nno-newline\n"
        );
    }

    #[test]
    fn external_modification_is_detected_and_not_silently_overwritten() {
        let (_dir, path) = temp_file("shared.txt", "original\n");
        let mut doc = Document::open(&path, &config(), detect(&path)).unwrap();
        doc.buffer.insert("local edit ");

        // Somebody else writes the file (an agent, or another editor).
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&path, "changed by an agent\n").unwrap();

        assert_eq!(doc.check_external_change(), ExternalChange::Modified);
        assert!(doc.save_would_conflict());

        // Reloading picks up their version and drops the conflict.
        doc.reload().unwrap();
        assert_eq!(doc.buffer.to_text(), "changed by an agent\n");
        assert_eq!(doc.external_change, ExternalChange::None);
    }

    #[test]
    fn deletion_is_detected() {
        let (_dir, path) = temp_file("gone.txt", "x");
        let mut doc = Document::open(&path, &config(), detect(&path)).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(doc.check_external_change(), ExternalChange::Deleted);
    }

    #[test]
    fn saving_updates_the_recorded_disk_state() {
        let (_dir, path) = temp_file("f.txt", "a\n");
        let mut doc = Document::open(&path, &config(), detect(&path)).unwrap();
        doc.buffer.insert("b");
        doc.save(&config()).unwrap();
        assert_eq!(doc.check_external_change(), ExternalChange::None);
    }

    #[test]
    fn scratch_buffers_cannot_be_saved_without_a_path() {
        let mut doc = Document::scratch(crate::services::syntax::languages::plaintext());
        assert_eq!(doc.display_name(), "untitled");
        assert!(doc.save(&config()).is_err());
    }

    #[test]
    fn viewport_follows_the_cursor() {
        let text = (0..100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (_dir, path) = temp_file("long.txt", &text);
        let mut doc = Document::open(&path, &config(), detect(&path)).unwrap();
        doc.goto(Position::new(80, 0));
        doc.ensure_cursor_visible(20, 40);
        assert_eq!(doc.scroll, 61);
        doc.goto(Position::new(2, 0));
        doc.ensure_cursor_visible(20, 40);
        assert_eq!(doc.scroll, 2);
    }

    #[test]
    fn breakpoints_toggle() {
        let mut doc = Document::scratch(crate::services::syntax::languages::plaintext());
        assert!(doc.toggle_breakpoint(4));
        assert!(doc.breakpoints.contains(&4));
        assert!(!doc.toggle_breakpoint(4));
        assert!(doc.breakpoints.is_empty());
    }
}
