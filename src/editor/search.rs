//! In-file search.

use crate::domain::diagnostics::{Position, Range};

use super::buffer::TextBuffer;

/// Live state of the find bar.
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub query: String,
    pub case_sensitive: bool,
    /// Match ranges in document order.
    pub matches: Vec<Range>,
    /// Index into `matches` of the highlighted hit.
    pub current: usize,
}

impl SearchState {
    /// Recompute matches for the current query.
    pub fn refresh(&mut self, buffer: &TextBuffer) {
        self.matches = find_all(buffer, &self.query, self.case_sensitive);
        if self.current >= self.matches.len() {
            self.current = 0;
        }
    }

    pub fn is_active(&self) -> bool {
        !self.query.is_empty()
    }

    /// Move to the next match after the cursor, wrapping around.
    pub fn next_from(&mut self, cursor: Position) -> Option<Range> {
        if self.matches.is_empty() {
            return None;
        }
        let index = self
            .matches
            .iter()
            .position(|m| m.start > cursor)
            .unwrap_or(0);
        self.current = index;
        Some(self.matches[index])
    }

    /// Move to the previous match before the cursor, wrapping around.
    pub fn prev_from(&mut self, cursor: Position) -> Option<Range> {
        if self.matches.is_empty() {
            return None;
        }
        let index = self
            .matches
            .iter()
            .rposition(|m| m.start < cursor)
            .unwrap_or(self.matches.len() - 1);
        self.current = index;
        Some(self.matches[index])
    }

    /// The currently highlighted match.
    pub fn current_match(&self) -> Option<Range> {
        self.matches.get(self.current).copied()
    }

    /// `3/12` style counter for the find bar.
    pub fn counter(&self) -> String {
        if self.matches.is_empty() {
            "no results".to_string()
        } else {
            format!("{}/{}", self.current + 1, self.matches.len())
        }
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.current = 0;
    }
}

/// Every occurrence of `needle`, as character ranges.
pub fn find_all(buffer: &TextBuffer, needle: &str, case_sensitive: bool) -> Vec<Range> {
    if needle.is_empty() {
        return Vec::new();
    }
    let needle_chars: Vec<char> = if case_sensitive {
        needle.chars().collect()
    } else {
        needle.to_lowercase().chars().collect()
    };

    let mut out = Vec::new();
    for (line_index, line) in buffer.lines().iter().enumerate() {
        let haystack: Vec<char> = if case_sensitive {
            line.chars().collect()
        } else {
            line.to_lowercase().chars().collect()
        };
        if haystack.len() < needle_chars.len() {
            continue;
        }
        let mut start = 0;
        while start + needle_chars.len() <= haystack.len() {
            if haystack[start..start + needle_chars.len()] == needle_chars[..] {
                out.push(Range::single_line(
                    line_index,
                    start,
                    start + needle_chars.len(),
                ));
                start += needle_chars.len();
            } else {
                start += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer() -> TextBuffer {
        TextBuffer::from_str("alpha beta\nBETA gamma\nbeta\n")
    }

    #[test]
    fn finds_all_case_insensitive_matches_by_default() {
        let matches = find_all(&buffer(), "beta", false);
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0], Range::single_line(0, 6, 10));
        assert_eq!(matches[1], Range::single_line(1, 0, 4));
    }

    #[test]
    fn case_sensitive_search_skips_different_casing() {
        let matches = find_all(&buffer(), "beta", true);
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn empty_query_matches_nothing() {
        assert!(find_all(&buffer(), "", false).is_empty());
    }

    #[test]
    fn overlapping_matches_advance_past_the_hit() {
        let buf = TextBuffer::from_str("aaaa");
        assert_eq!(find_all(&buf, "aa", true).len(), 2);
    }

    #[test]
    fn navigation_wraps_around() {
        let buf = buffer();
        let mut state = SearchState {
            query: "beta".into(),
            ..Default::default()
        };
        state.refresh(&buf);
        assert_eq!(state.counter(), "1/3");

        let hit = state.next_from(Position::new(0, 0)).unwrap();
        assert_eq!(hit.start.line, 0);
        let hit = state.next_from(hit.start).unwrap();
        assert_eq!(hit.start.line, 1);
        let hit = state.next_from(Position::new(2, 5)).unwrap();
        assert_eq!(hit.start.line, 0, "wrapped back to the first match");

        let hit = state.prev_from(Position::new(0, 0)).unwrap();
        assert_eq!(hit.start.line, 2, "wrapped back to the last match");
    }

    #[test]
    fn no_matches_reports_clearly() {
        let buf = buffer();
        let mut state = SearchState {
            query: "nothing".into(),
            ..Default::default()
        };
        state.refresh(&buf);
        assert_eq!(state.counter(), "no results");
        assert!(state.next_from(Position::new(0, 0)).is_none());
    }

    #[test]
    fn unicode_matches_use_character_columns() {
        let buf = TextBuffer::from_str("héllo wörld");
        let matches = find_all(&buf, "wörld", true);
        assert_eq!(matches[0], Range::single_line(0, 6, 11));
    }
}
