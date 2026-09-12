//! A terminal screen snapshot decoupled from the VT parser.
//!
//! `vt100` owns ANSI parsing; this module converts its cell grid into plain
//! data the UI layer can render without knowing the parser exists.

/// A colour as reported by the child program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellColor {
    /// Use the theme's default foreground/background.
    Default,
    /// ANSI palette index (0-255).
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl From<vt100::Color> for CellColor {
    fn from(value: vt100::Color) -> Self {
        match value {
            vt100::Color::Default => CellColor::Default,
            vt100::Color::Idx(i) => CellColor::Indexed(i),
            vt100::Color::Rgb(r, g, b) => CellColor::Rgb(r, g, b),
        }
    }
}

/// One rendered character cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Cell contents; may be empty (blank) or a multi-codepoint grapheme.
    pub text: String,
    pub fg: CellColor,
    pub bg: CellColor,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            text: String::new(),
            fg: CellColor::Default,
            bg: CellColor::Default,
            bold: false,
            italic: false,
            underline: false,
            inverse: false,
        }
    }
}

/// An immutable view of the child's screen at one moment.
#[derive(Debug, Clone, Default)]
pub struct ScreenSnapshot {
    pub rows: Vec<Vec<Cell>>,
    pub cursor: (u16, u16),
    pub cursor_visible: bool,
    /// Window title set by the child through OSC sequences, if any.
    pub title: Option<String>,
    /// How far back in the scrollback the view currently is.
    pub scrollback: usize,
    /// True while the child is using the alternate screen (vim, less, ...).
    pub alternate_screen: bool,
}

impl ScreenSnapshot {
    /// Build a snapshot from a parser screen.
    pub fn capture(screen: &vt100::Screen) -> ScreenSnapshot {
        let (rows, cols) = screen.size();
        let mut grid = Vec::with_capacity(rows as usize);
        for row in 0..rows {
            let mut line = Vec::with_capacity(cols as usize);
            for col in 0..cols {
                line.push(match screen.cell(row, col) {
                    Some(cell) => Cell {
                        text: cell.contents(),
                        fg: cell.fgcolor().into(),
                        bg: cell.bgcolor().into(),
                        bold: cell.bold(),
                        italic: cell.italic(),
                        underline: cell.underline(),
                        inverse: cell.inverse(),
                    },
                    None => Cell::default(),
                });
            }
            grid.push(line);
        }

        let title = {
            let t = screen.title();
            (!t.is_empty()).then(|| t.to_string())
        };

        ScreenSnapshot {
            rows: grid,
            cursor: screen.cursor_position(),
            cursor_visible: !screen.hide_cursor(),
            title,
            scrollback: screen.scrollback(),
            alternate_screen: screen.alternate_screen(),
        }
    }

    /// Plain text of the whole screen, used by the agent detail "Output" tab
    /// and by local state heuristics.
    pub fn to_text(&self) -> String {
        self.rows
            .iter()
            .map(|row| {
                let line: String = row
                    .iter()
                    .map(|c| {
                        if c.text.is_empty() {
                            " ".to_string()
                        } else {
                            c.text.clone()
                        }
                    })
                    .collect();
                line.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The last `n` non-empty lines, newest last.
    pub fn tail(&self, n: usize) -> Vec<String> {
        let text = self.to_text();
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(n);
        lines[start..].iter().map(|s| s.to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> ScreenSnapshot {
        let mut parser = vt100::Parser::new(4, 20, 100);
        parser.process(bytes);
        ScreenSnapshot::capture(parser.screen())
    }

    #[test]
    fn plain_text_is_captured() {
        let snap = parse(b"hello");
        assert_eq!(snap.rows.len(), 4);
        assert_eq!(snap.rows[0][0].text, "h");
        assert_eq!(snap.to_text().lines().next().unwrap(), "hello");
    }

    #[test]
    fn ansi_colours_and_attributes_are_decoded() {
        // bold red on default, then reset.
        let snap = parse(b"\x1b[1;31mred\x1b[0m plain");
        let cell = &snap.rows[0][0];
        assert!(cell.bold);
        assert_eq!(cell.fg, CellColor::Indexed(1));
        let plain = &snap.rows[0][4];
        assert!(!plain.bold);
        assert_eq!(plain.fg, CellColor::Default);
    }

    #[test]
    fn rgb_colours_survive() {
        let snap = parse(b"\x1b[38;2;10;20;30mx");
        assert_eq!(snap.rows[0][0].fg, CellColor::Rgb(10, 20, 30));
    }

    #[test]
    fn cursor_position_and_visibility_are_tracked() {
        let snap = parse(b"ab");
        assert_eq!(snap.cursor, (0, 2));
        assert!(snap.cursor_visible);
        let hidden = parse(b"\x1b[?25l");
        assert!(!hidden.cursor_visible);
    }

    #[test]
    fn cursor_movement_sequences_are_honoured() {
        let snap = parse(b"line1\r\nline2\x1b[H");
        assert_eq!(snap.cursor, (0, 0));
        assert_eq!(snap.to_text().lines().nth(1).unwrap(), "line2");
    }

    #[test]
    fn tail_returns_the_last_lines() {
        let snap = parse(b"a\r\nb\r\nc");
        assert_eq!(snap.tail(2), vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn alternate_screen_is_reported() {
        let snap = parse(b"\x1b[?1049h");
        assert!(snap.alternate_screen);
    }
}
