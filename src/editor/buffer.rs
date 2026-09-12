//! The text buffer behind every editor tab.
//!
//! Text is stored as a `Vec<String>` of lines; positions are `(line,
//! character)` with *character* (not byte) columns so the model lines up with
//! LSP positions and with what the user sees. Every mutation funnels through
//! [`TextBuffer::replace_range`], which is also what makes undo/redo exact.

use crate::domain::diagnostics::{Position, Range};

/// A reversible edit.
#[derive(Debug, Clone)]
struct EditRecord {
    /// Where the replacement started.
    start: Position,
    /// Text that was removed (empty for a pure insert).
    removed: String,
    /// Text that was inserted (empty for a pure delete).
    inserted: String,
    cursor_before: Position,
    cursor_after: Position,
}

/// Multi-line text with cursor, selection and undo history.
#[derive(Debug, Clone)]
pub struct TextBuffer {
    lines: Vec<String>,
    cursor: Position,
    /// Selection anchor; the selection spans anchor..cursor when set.
    anchor: Option<Position>,
    undo_stack: Vec<EditRecord>,
    redo_stack: Vec<EditRecord>,
    /// Monotonic version, incremented on every change (used by LSP sync).
    version: i64,
    /// Version at the last save; the buffer is dirty when they differ.
    saved_version: i64,
    /// Column the cursor tries to keep while moving vertically.
    goal_column: Option<usize>,
    /// Whether the original file used CRLF line endings.
    pub crlf: bool,
    /// Whether the original file ended with a trailing newline.
    pub trailing_newline: bool,
}

impl Default for TextBuffer {
    fn default() -> Self {
        TextBuffer::from_text("")
    }
}

impl TextBuffer {
    /// Build a buffer from file contents, remembering the line-ending style.
    pub fn from_text(text: &str) -> TextBuffer {
        let crlf = text.contains("\r\n");
        let normalised = text.replace("\r\n", "\n");
        let trailing_newline = normalised.ends_with('\n');
        let mut lines: Vec<String> = normalised.split('\n').map(|s| s.to_string()).collect();
        // `split` leaves a trailing empty element for text ending in a newline.
        if trailing_newline {
            lines.pop();
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        TextBuffer {
            lines,
            cursor: Position::default(),
            anchor: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            version: 0,
            saved_version: 0,
            goal_column: None,
            crlf,
            trailing_newline,
        }
    }

    /// Serialise back to text using the original line-ending style.
    pub fn to_text(&self) -> String {
        let mut out = self.lines.join("\n");
        if self.trailing_newline {
            out.push('\n');
        }
        if self.crlf {
            out = out.replace('\n', "\r\n");
        }
        out
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn line(&self, index: usize) -> &str {
        self.lines.get(index).map(String::as_str).unwrap_or("")
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn cursor(&self) -> Position {
        self.cursor
    }

    pub fn version(&self) -> i64 {
        self.version
    }

    pub fn is_dirty(&self) -> bool {
        self.version != self.saved_version
    }

    /// Mark the current contents as persisted.
    pub fn mark_saved(&mut self) {
        self.saved_version = self.version;
    }

    /// Number of characters in a line.
    pub fn line_len(&self, line: usize) -> usize {
        self.line(line).chars().count()
    }

    /// Clamp a position into the buffer.
    pub fn clamp(&self, pos: Position) -> Position {
        let line = pos.line.min(self.lines.len().saturating_sub(1));
        let character = pos.character.min(self.line_len(line));
        Position { line, character }
    }

    // ── selection ─────────────────────────────────────────────────────────

    pub fn selection(&self) -> Option<Range> {
        let anchor = self.anchor?;
        if anchor == self.cursor {
            return None;
        }
        let (start, end) = if anchor <= self.cursor {
            (anchor, self.cursor)
        } else {
            (self.cursor, anchor)
        };
        Some(Range { start, end })
    }

    pub fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    /// Start (or keep) a selection anchored at the current cursor.
    pub fn begin_selection(&mut self) {
        if self.anchor.is_none() {
            self.anchor = Some(self.cursor);
        }
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(Position::new(0, 0));
        let last = self.lines.len() - 1;
        self.cursor = Position::new(last, self.line_len(last));
    }

    /// Select the whole current line (used by line-oriented commands).
    pub fn select_line(&mut self) {
        let line = self.cursor.line;
        self.anchor = Some(Position::new(line, 0));
        self.cursor = Position::new(line, self.line_len(line));
    }

    /// Text inside `range`.
    pub fn text_in(&self, range: Range) -> String {
        let range = self.normalise(range);
        if range.start.line == range.end.line {
            return substring(
                self.line(range.start.line),
                range.start.character,
                range.end.character,
            );
        }
        let mut out = String::new();
        out.push_str(&substring(
            self.line(range.start.line),
            range.start.character,
            usize::MAX,
        ));
        for line in range.start.line + 1..range.end.line {
            out.push('\n');
            out.push_str(self.line(line));
        }
        out.push('\n');
        out.push_str(&substring(
            self.line(range.end.line),
            0,
            range.end.character,
        ));
        out
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|r| self.text_in(r))
    }

    // ── cursor movement ───────────────────────────────────────────────────

    /// Move the cursor, optionally extending the selection.
    pub fn move_to(&mut self, pos: Position, extend: bool) {
        if extend {
            self.begin_selection();
        } else {
            self.clear_selection();
        }
        self.cursor = self.clamp(pos);
        self.goal_column = None;
    }

    pub fn move_left(&mut self, extend: bool) {
        let pos = if self.cursor.character > 0 {
            Position::new(self.cursor.line, self.cursor.character - 1)
        } else if self.cursor.line > 0 {
            let line = self.cursor.line - 1;
            Position::new(line, self.line_len(line))
        } else {
            self.cursor
        };
        self.move_to(pos, extend);
    }

    pub fn move_right(&mut self, extend: bool) {
        let pos = if self.cursor.character < self.line_len(self.cursor.line) {
            Position::new(self.cursor.line, self.cursor.character + 1)
        } else if self.cursor.line + 1 < self.lines.len() {
            Position::new(self.cursor.line + 1, 0)
        } else {
            self.cursor
        };
        self.move_to(pos, extend);
    }

    /// Vertical movement keeps the "goal column" like a real editor.
    pub fn move_vertical(&mut self, delta: isize, extend: bool) {
        let goal = self.goal_column.unwrap_or(self.cursor.character);
        let target_line = (self.cursor.line as isize + delta)
            .clamp(0, self.lines.len().saturating_sub(1) as isize)
            as usize;
        let target = Position::new(target_line, goal.min(self.line_len(target_line)));
        if extend {
            self.begin_selection();
        } else {
            self.clear_selection();
        }
        self.cursor = target;
        self.goal_column = Some(goal);
    }

    pub fn move_line_start(&mut self, extend: bool) {
        // First press goes to the first non-space character, second to col 0.
        let line = self.line(self.cursor.line);
        let indent = line.chars().take_while(|c| c.is_whitespace()).count();
        let target = if self.cursor.character == indent {
            0
        } else {
            indent
        };
        self.move_to(Position::new(self.cursor.line, target), extend);
    }

    pub fn move_line_end(&mut self, extend: bool) {
        let end = self.line_len(self.cursor.line);
        self.move_to(Position::new(self.cursor.line, end), extend);
    }

    pub fn move_document_start(&mut self, extend: bool) {
        self.move_to(Position::new(0, 0), extend);
    }

    pub fn move_document_end(&mut self, extend: bool) {
        let last = self.lines.len() - 1;
        self.move_to(Position::new(last, self.line_len(last)), extend);
    }

    /// Move by one word, the way Ctrl+Left/Right behaves.
    pub fn move_word(&mut self, forward: bool, extend: bool) {
        let chars: Vec<char> = self.line(self.cursor.line).chars().collect();
        let mut col = self.cursor.character;
        if forward {
            if col >= chars.len() {
                if self.cursor.line + 1 < self.lines.len() {
                    self.move_to(Position::new(self.cursor.line + 1, 0), extend);
                }
                return;
            }
            while col < chars.len() && is_word_char(chars[col]) {
                col += 1;
            }
            while col < chars.len() && !is_word_char(chars[col]) {
                col += 1;
            }
        } else {
            if col == 0 {
                if self.cursor.line > 0 {
                    let line = self.cursor.line - 1;
                    self.move_to(Position::new(line, self.line_len(line)), extend);
                }
                return;
            }
            col -= 1;
            while col > 0 && !is_word_char(chars[col]) {
                col -= 1;
            }
            while col > 0 && is_word_char(chars[col - 1]) {
                col -= 1;
            }
        }
        self.move_to(Position::new(self.cursor.line, col), extend);
    }

    /// Word under the cursor, used for hover/definition fallbacks and search.
    pub fn word_at(&self, pos: Position) -> Option<String> {
        let chars: Vec<char> = self.line(pos.line).chars().collect();
        if chars.is_empty() {
            return None;
        }
        let mut start = pos.character.min(chars.len());
        let mut end = start;
        if start > 0 && !chars.get(start).is_some_and(|c| is_word_char(*c)) {
            start -= 1;
            end = start;
        }
        if !chars.get(start).is_some_and(|c| is_word_char(*c)) {
            return None;
        }
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        while end < chars.len() && is_word_char(chars[end]) {
            end += 1;
        }
        Some(chars[start..end].iter().collect())
    }

    // ── editing ───────────────────────────────────────────────────────────

    /// Replace `range` with `text`, recording an undo entry.
    pub fn replace_range(&mut self, range: Range, text: &str) -> String {
        let range = self.normalise(range);
        let removed = self.text_in(range);
        let cursor_before = self.cursor;
        self.apply_replace(range, text);
        let cursor_after = self.cursor;
        self.push_undo(EditRecord {
            start: range.start,
            removed: removed.clone(),
            inserted: text.to_string(),
            cursor_before,
            cursor_after,
        });
        removed
    }

    /// Raw replace without undo bookkeeping.
    fn apply_replace(&mut self, range: Range, text: &str) {
        let range = self.normalise(range);
        let prefix = substring(self.line(range.start.line), 0, range.start.character);
        let suffix = substring(self.line(range.end.line), range.end.character, usize::MAX);

        let inserted: Vec<&str> = text.split('\n').collect();
        let mut new_lines: Vec<String> = Vec::with_capacity(inserted.len());
        if inserted.len() == 1 {
            new_lines.push(format!("{prefix}{}{suffix}", inserted[0]));
        } else {
            new_lines.push(format!("{prefix}{}", inserted[0]));
            for piece in &inserted[1..inserted.len() - 1] {
                new_lines.push((*piece).to_string());
            }
            new_lines.push(format!("{}{suffix}", inserted[inserted.len() - 1]));
        }

        let end_line = self.lines.len().saturating_sub(1).min(range.end.line);
        self.lines.splice(range.start.line..=end_line, new_lines);
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }

        self.cursor = if inserted.len() == 1 {
            Position::new(
                range.start.line,
                range.start.character + inserted[0].chars().count(),
            )
        } else {
            Position::new(
                range.start.line + inserted.len() - 1,
                inserted[inserted.len() - 1].chars().count(),
            )
        };
        self.anchor = None;
        self.goal_column = None;
        self.version += 1;
    }

    fn push_undo(&mut self, record: EditRecord) {
        self.redo_stack.clear();
        // Coalesce plain typing so undo removes a word, not one letter.
        if let Some(last) = self.undo_stack.last_mut() {
            let simple_typing = record.removed.is_empty()
                && !record.inserted.contains('\n')
                && record.inserted.chars().count() == 1
                && last.removed.is_empty()
                && !last.inserted.contains('\n')
                && last.cursor_after == record.cursor_before
                && !record.inserted.starts_with(char::is_whitespace);
            if simple_typing {
                last.inserted.push_str(&record.inserted);
                last.cursor_after = record.cursor_after;
                return;
            }
        }
        self.undo_stack.push(record);
        const MAX_UNDO: usize = 4096;
        if self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
    }

    /// Insert text at the cursor, replacing the selection when there is one.
    pub fn insert(&mut self, text: &str) {
        let range = self
            .selection()
            .unwrap_or(Range::new(self.cursor, self.cursor));
        self.replace_range(range, text);
    }

    pub fn insert_char(&mut self, c: char) {
        let mut buf = [0u8; 4];
        self.insert(c.encode_utf8(&mut buf));
    }

    /// Insert a newline, copying the current line's indentation.
    pub fn insert_newline(&mut self, auto_indent: bool) {
        let indent: String = if auto_indent {
            self.line(self.cursor.line)
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect()
        } else {
            String::new()
        };
        let indent = if self.cursor.character < indent.chars().count() {
            String::new()
        } else {
            indent
        };
        self.insert(&format!("\n{indent}"));
    }

    /// Backspace.
    pub fn delete_backward(&mut self) {
        if self.has_selection() {
            self.delete_selection();
            return;
        }
        if self.cursor == Position::new(0, 0) {
            return;
        }
        let end = self.cursor;
        let start = if self.cursor.character > 0 {
            Position::new(self.cursor.line, self.cursor.character - 1)
        } else {
            let line = self.cursor.line - 1;
            Position::new(line, self.line_len(line))
        };
        self.replace_range(Range::new(start, end), "");
    }

    /// Delete key.
    pub fn delete_forward(&mut self) {
        if self.has_selection() {
            self.delete_selection();
            return;
        }
        let start = self.cursor;
        let end = if self.cursor.character < self.line_len(self.cursor.line) {
            Position::new(self.cursor.line, self.cursor.character + 1)
        } else if self.cursor.line + 1 < self.lines.len() {
            Position::new(self.cursor.line + 1, 0)
        } else {
            return;
        };
        self.replace_range(Range::new(start, end), "");
    }

    pub fn delete_selection(&mut self) -> Option<String> {
        let range = self.selection()?;
        Some(self.replace_range(range, ""))
    }

    /// Delete from the cursor to the end of the line (`Ctrl+K` style).
    pub fn delete_to_line_end(&mut self) {
        let end = Position::new(self.cursor.line, self.line_len(self.cursor.line));
        if end == self.cursor {
            self.delete_forward();
        } else {
            self.replace_range(Range::new(self.cursor, end), "");
        }
    }

    /// Indent the selected lines (or the current line) by one unit.
    pub fn indent(&mut self, tab_width: usize, spaces: bool) {
        let unit = if spaces {
            " ".repeat(tab_width)
        } else {
            "\t".into()
        };
        let Some(range) = self.selection() else {
            // No selection: insert the unit at the cursor.
            self.insert(&unit);
            return;
        };
        let (first, last) = (range.start.line, range.end.line);
        for line in first..=last {
            let start = Position::new(line, 0);
            self.replace_range(Range::new(start, start), &unit);
        }
        self.anchor = Some(Position::new(first, 0));
        self.cursor = Position::new(last, self.line_len(last));
    }

    /// Remove one indentation unit from the selected lines.
    pub fn dedent(&mut self, tab_width: usize) {
        let (first, last) = match self.selection() {
            Some(range) => (range.start.line, range.end.line),
            None => (self.cursor.line, self.cursor.line),
        };
        for line in first..=last {
            let text = self.line(line).to_string();
            let removable = if text.starts_with('\t') {
                1
            } else {
                text.chars()
                    .take(tab_width)
                    .take_while(|c| *c == ' ')
                    .count()
            };
            if removable > 0 {
                self.replace_range(
                    Range::new(Position::new(line, 0), Position::new(line, removable)),
                    "",
                );
            }
        }
        if first != last {
            self.anchor = Some(Position::new(first, 0));
            self.cursor = Position::new(last, self.line_len(last));
        }
    }

    // ── undo/redo ─────────────────────────────────────────────────────────

    pub fn undo(&mut self) -> bool {
        let Some(record) = self.undo_stack.pop() else {
            return false;
        };
        let end = advance(record.start, &record.inserted);
        self.apply_replace(Range::new(record.start, end), &record.removed);
        self.cursor = self.clamp(record.cursor_before);
        self.redo_stack.push(record);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(record) = self.redo_stack.pop() else {
            return false;
        };
        let end = advance(record.start, &record.removed);
        self.apply_replace(Range::new(record.start, end), &record.inserted);
        self.cursor = self.clamp(record.cursor_after);
        self.undo_stack.push(record);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    fn normalise(&self, range: Range) -> Range {
        let (mut start, mut end) = (self.clamp(range.start), self.clamp(range.end));
        if end < start {
            std::mem::swap(&mut start, &mut end);
        }
        Range { start, end }
    }
}

/// Position reached after inserting `text` at `start`.
fn advance(start: Position, text: &str) -> Position {
    let newlines = text.matches('\n').count();
    if newlines == 0 {
        Position::new(start.line, start.character + text.chars().count())
    } else {
        let last = text.rsplit('\n').next().unwrap_or("");
        Position::new(start.line + newlines, last.chars().count())
    }
}

/// Character-indexed substring (`end` may be `usize::MAX` for "to the end").
fn substring(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

/// Word characters for word-motion and word-under-cursor.
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_line_endings_and_trailing_newline() {
        let buf = TextBuffer::from_text("a\r\nb\r\n");
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.to_text(), "a\r\nb\r\n");

        let buf = TextBuffer::from_text("a\nb");
        assert_eq!(buf.to_text(), "a\nb");
    }

    #[test]
    fn empty_buffer_has_one_line() {
        let buf = TextBuffer::from_text("");
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.to_text(), "");
    }

    #[test]
    fn typing_and_newlines_update_the_cursor() {
        let mut buf = TextBuffer::from_text("");
        buf.insert("hello");
        assert_eq!(buf.cursor(), Position::new(0, 5));
        buf.insert_newline(false);
        buf.insert("world");
        assert_eq!(buf.to_text(), "hello\nworld");
        assert_eq!(buf.cursor(), Position::new(1, 5));
    }

    #[test]
    fn auto_indent_copies_leading_whitespace() {
        let mut buf = TextBuffer::from_text("    let x = 1;");
        buf.move_document_end(false);
        buf.insert_newline(true);
        assert_eq!(buf.line(1), "    ");
        assert_eq!(buf.cursor(), Position::new(1, 4));
    }

    #[test]
    fn dirty_flag_tracks_saves() {
        let mut buf = TextBuffer::from_text("x");
        assert!(!buf.is_dirty());
        buf.insert("y");
        assert!(buf.is_dirty());
        buf.mark_saved();
        assert!(!buf.is_dirty());
        buf.undo();
        assert!(buf.is_dirty(), "undoing past the save point is still dirty");
    }

    #[test]
    fn undo_and_redo_restore_exact_text() {
        let mut buf = TextBuffer::from_text("one\ntwo\n");
        buf.move_document_end(false);
        buf.insert("three");
        let after = buf.to_text();
        assert!(buf.undo());
        assert_eq!(buf.to_text(), "one\ntwo\n");
        assert!(buf.redo());
        assert_eq!(buf.to_text(), after);
    }

    #[test]
    fn typing_is_coalesced_into_one_undo_step() {
        let mut buf = TextBuffer::from_text("");
        for c in "hello".chars() {
            buf.insert_char(c);
        }
        assert!(buf.undo());
        assert_eq!(buf.to_text(), "", "one undo removes the whole word");
    }

    #[test]
    fn multiline_selection_delete_and_undo() {
        let mut buf = TextBuffer::from_text("aaa\nbbb\nccc\n");
        buf.move_to(Position::new(0, 1), false);
        buf.move_to(Position::new(2, 2), true);
        assert_eq!(buf.selected_text().unwrap(), "aa\nbbb\ncc");
        buf.delete_selection();
        assert_eq!(buf.to_text(), "ac\n");
        buf.undo();
        assert_eq!(buf.to_text(), "aaa\nbbb\nccc\n");
    }

    #[test]
    fn utf8_columns_are_character_based() {
        let mut buf = TextBuffer::from_text("héllo → wörld");
        buf.move_to(Position::new(0, 6), false);
        buf.insert("X");
        assert_eq!(buf.line(0), "héllo X→ wörld");
        assert_eq!(buf.line_len(0), 14);
    }

    #[test]
    fn backspace_joins_lines() {
        let mut buf = TextBuffer::from_text("ab\ncd");
        buf.move_to(Position::new(1, 0), false);
        buf.delete_backward();
        assert_eq!(buf.to_text(), "abcd");
        assert_eq!(buf.cursor(), Position::new(0, 2));
    }

    #[test]
    fn indent_and_dedent_operate_on_selected_lines() {
        let mut buf = TextBuffer::from_text("a\nb\nc\n");
        buf.move_to(Position::new(0, 0), false);
        buf.move_to(Position::new(1, 1), true);
        buf.indent(2, true);
        assert_eq!(buf.to_text(), "  a\n  b\nc\n");
        buf.dedent(2);
        assert_eq!(buf.to_text(), "a\nb\nc\n");
    }

    #[test]
    fn vertical_movement_keeps_the_goal_column() {
        let mut buf = TextBuffer::from_text("longer line\nx\nanother long line\n");
        buf.move_to(Position::new(0, 9), false);
        buf.move_vertical(1, false);
        assert_eq!(
            buf.cursor(),
            Position::new(1, 1),
            "clamped on the short line"
        );
        buf.move_vertical(1, false);
        assert_eq!(buf.cursor(), Position::new(2, 9), "goal column restored");
    }

    #[test]
    fn word_motion_and_word_at_cursor() {
        let mut buf = TextBuffer::from_text("let value = compute();");
        buf.move_to(Position::new(0, 0), false);
        buf.move_word(true, false);
        assert_eq!(buf.cursor(), Position::new(0, 4));
        assert_eq!(buf.word_at(Position::new(0, 5)).as_deref(), Some("value"));
        assert_eq!(buf.word_at(Position::new(0, 10)), None);
    }

    #[test]
    fn home_toggles_between_indent_and_column_zero() {
        let mut buf = TextBuffer::from_text("    indented");
        buf.move_document_end(false);
        buf.move_line_start(false);
        assert_eq!(buf.cursor(), Position::new(0, 4));
        buf.move_line_start(false);
        assert_eq!(buf.cursor(), Position::new(0, 0));
    }
}
