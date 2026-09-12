//! Colour tokens.
//!
//! The workbench never hardcodes colours at the call site: widgets ask the
//! theme for a semantic token. That keeps the dark, dense look consistent and
//! leaves room for VS Code theme imports to override the palette later.

use ratatui::style::{Color, Modifier, Style};
use std::collections::HashSet;

use crate::app::events::NoticeLevel;
use crate::domain::agent::AgentState;
use crate::domain::diagnostics::Severity;
use crate::domain::extensions::{CompatibilityReport, ExtensionAsset};
use crate::domain::git::FileStatus;
use crate::services::syntax::HighlightKind;
use crate::services::terminal::CellColor;

/// A complete colour set.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,

    pub background: Color,
    pub surface: Color,
    /// Slightly raised surface for headers and active rows.
    pub surface_alt: Color,
    pub border: Color,
    pub border_focus: Color,

    pub text: Color,
    pub text_dim: Color,
    pub text_bright: Color,

    pub accent: Color,
    pub accent_alt: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub info: Color,

    pub selection: Color,
    pub cursor_line: Color,
    pub line_number: Color,
    pub line_number_active: Color,

    // syntax
    pub syn_keyword: Color,
    pub syn_type: Color,
    pub syn_function: Color,
    pub syn_string: Color,
    pub syn_number: Color,
    pub syn_comment: Color,
    pub syn_constant: Color,
    pub syn_property: Color,
    pub syn_operator: Color,
    pub syn_punctuation: Color,
    pub syn_attribute: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::termloom_dark()
    }
}

impl Theme {
    /// The default dark theme: very dark ground, cool grey panels, cyan focus.
    pub fn termloom_dark() -> Theme {
        Theme {
            name: "termloom-dark".into(),
            background: Color::Rgb(13, 17, 23),
            surface: Color::Rgb(16, 21, 28),
            surface_alt: Color::Rgb(24, 31, 41),
            border: Color::Rgb(38, 46, 58),
            border_focus: Color::Rgb(45, 212, 191),

            text: Color::Rgb(201, 209, 217),
            text_dim: Color::Rgb(110, 121, 136),
            text_bright: Color::Rgb(236, 242, 248),

            accent: Color::Rgb(45, 212, 191),
            accent_alt: Color::Rgb(88, 166, 255),
            success: Color::Rgb(63, 185, 80),
            warning: Color::Rgb(210, 153, 34),
            danger: Color::Rgb(248, 81, 73),
            info: Color::Rgb(139, 148, 158),

            selection: Color::Rgb(33, 60, 74),
            cursor_line: Color::Rgb(22, 28, 37),
            line_number: Color::Rgb(78, 88, 102),
            line_number_active: Color::Rgb(160, 175, 190),

            syn_keyword: Color::Rgb(198, 120, 221),
            syn_type: Color::Rgb(229, 192, 123),
            syn_function: Color::Rgb(97, 175, 239),
            syn_string: Color::Rgb(152, 195, 121),
            syn_number: Color::Rgb(209, 154, 102),
            syn_comment: Color::Rgb(106, 115, 125),
            syn_constant: Color::Rgb(224, 108, 117),
            syn_property: Color::Rgb(86, 182, 194),
            syn_operator: Color::Rgb(171, 178, 191),
            syn_punctuation: Color::Rgb(130, 139, 150),
            syn_attribute: Color::Rgb(229, 192, 123),
        }
    }

    /// A high-contrast, palette-only theme for terminals without truecolor.
    pub fn ansi_fallback() -> Theme {
        Theme {
            name: "ansi".into(),
            background: Color::Reset,
            surface: Color::Reset,
            surface_alt: Color::DarkGray,
            border: Color::DarkGray,
            border_focus: Color::Cyan,

            text: Color::Gray,
            text_dim: Color::DarkGray,
            text_bright: Color::White,

            accent: Color::Cyan,
            accent_alt: Color::Blue,
            success: Color::Green,
            warning: Color::Yellow,
            danger: Color::Red,
            info: Color::Gray,

            selection: Color::Blue,
            cursor_line: Color::Reset,
            line_number: Color::DarkGray,
            line_number_active: Color::White,

            syn_keyword: Color::Magenta,
            syn_type: Color::Yellow,
            syn_function: Color::Blue,
            syn_string: Color::Green,
            syn_number: Color::Yellow,
            syn_comment: Color::DarkGray,
            syn_constant: Color::Red,
            syn_operator: Color::Gray,
            syn_property: Color::Cyan,
            syn_punctuation: Color::DarkGray,
            syn_attribute: Color::Yellow,
        }
    }

    /// Look up a theme by name, falling back to the default.
    pub fn by_name(name: &str) -> Theme {
        match name {
            "ansi" | "ansi-fallback" => Theme::ansi_fallback(),
            _ => Theme::termloom_dark(),
        }
    }

    /// Resolve a built-in theme or an installed VSIX theme. Extension themes
    /// can be selected by their label or unambiguously as `<extension-id>:<label>`.
    pub fn by_name_or_extensions(name: &str, reports: &[CompatibilityReport]) -> Theme {
        if matches!(name, "termloom-dark" | "ansi" | "ansi-fallback") {
            return Theme::by_name(name);
        }
        for report in reports {
            let Some(root) = report.install_path.as_ref() else {
                continue;
            };
            for asset in &report.assets {
                let ExtensionAsset::Theme { label, path, .. } = asset else {
                    continue;
                };
                if name != label && name != format!("{}:{label}", report.id) {
                    continue;
                }
                let extension_root = root.join("extension");
                let path = extension_root.join(path);
                match Theme::from_vscode_file(label, &path, &extension_root) {
                    Ok(theme) => return theme,
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error, "could not load extension theme")
                    }
                }
            }
        }
        Theme::by_name(name)
    }

    fn from_vscode_file(
        name: &str,
        path: &std::path::Path,
        extension_root: &std::path::Path,
    ) -> anyhow::Result<Theme> {
        let canonical_root = extension_root.canonicalize()?;
        let canonical_path = path.canonicalize()?;
        anyhow::ensure!(
            canonical_path.starts_with(&canonical_root),
            "VS Code theme escapes the extension directory"
        );
        let text = std::fs::read_to_string(&canonical_path)?;
        let value = crate::services::vscode::parse_jsonc(&text)?;
        let mut theme = Theme::termloom_dark();
        theme.name = name.to_string();
        let mut visited = HashSet::new();
        apply_vscode_theme(
            &mut theme,
            &canonical_path,
            &canonical_root,
            &value,
            &mut visited,
            0,
        )?;
        Ok(theme)
    }

    // ── semantic helpers ──────────────────────────────────────────────────

    /// Border style for a panel, highlighted when focused.
    pub fn panel_border(&self, focused: bool) -> Style {
        if focused {
            Style::default().fg(self.border_focus)
        } else {
            Style::default().fg(self.border)
        }
    }

    /// Panel title style.
    pub fn panel_title(&self, focused: bool) -> Style {
        let style = Style::default().add_modifier(Modifier::BOLD);
        if focused {
            style.fg(self.accent)
        } else {
            style.fg(self.text_dim)
        }
    }

    pub fn base(&self) -> Style {
        Style::default().fg(self.text).bg(self.background)
    }

    pub fn dim(&self) -> Style {
        Style::default().fg(self.text_dim)
    }

    /// Selected row in a list; `focused` distinguishes the active panel.
    pub fn selected_row(&self, focused: bool) -> Style {
        if focused {
            Style::default()
                .bg(self.selection)
                .fg(self.text_bright)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(self.surface_alt).fg(self.text)
        }
    }

    pub fn agent_state(&self, state: AgentState) -> Color {
        match state {
            AgentState::Working => self.success,
            AgentState::WaitingForInput => self.warning,
            AgentState::Failed => self.danger,
            AgentState::Done => self.accent_alt,
            AgentState::Starting => self.accent,
            AgentState::Idle | AgentState::Exited | AgentState::Unknown => self.text_dim,
        }
    }

    pub fn git_status(&self, status: FileStatus) -> Color {
        match status {
            FileStatus::Modified => self.warning,
            FileStatus::Added => self.success,
            FileStatus::Deleted => self.danger,
            FileStatus::Renamed => self.accent_alt,
            FileStatus::Untracked => self.info,
            FileStatus::Conflicted => self.danger,
            FileStatus::Ignored => self.text_dim,
        }
    }

    pub fn severity(&self, severity: Severity) -> Color {
        match severity {
            Severity::Error => self.danger,
            Severity::Warning => self.warning,
            Severity::Information => self.accent_alt,
            Severity::Hint => self.text_dim,
        }
    }

    pub fn notice(&self, level: NoticeLevel) -> Color {
        match level {
            NoticeLevel::Info => self.accent_alt,
            NoticeLevel::Warning => self.warning,
            NoticeLevel::Error => self.danger,
        }
    }

    pub fn highlight(&self, kind: HighlightKind) -> Color {
        match kind {
            HighlightKind::Keyword => self.syn_keyword,
            HighlightKind::Type => self.syn_type,
            HighlightKind::Function | HighlightKind::Method => self.syn_function,
            HighlightKind::String | HighlightKind::Escape => self.syn_string,
            HighlightKind::Number | HighlightKind::Boolean => self.syn_number,
            HighlightKind::Comment => self.syn_comment,
            HighlightKind::Constant => self.syn_constant,
            HighlightKind::Property | HighlightKind::Parameter => self.syn_property,
            HighlightKind::Variable => self.text,
            HighlightKind::Operator => self.syn_operator,
            HighlightKind::Punctuation => self.syn_punctuation,
            HighlightKind::Attribute | HighlightKind::Tag => self.syn_attribute,
            HighlightKind::Text => self.text,
        }
    }

    /// Convert a colour reported by a child terminal program.
    pub fn terminal_color(&self, color: CellColor, foreground: bool) -> Color {
        match color {
            CellColor::Default => {
                if foreground {
                    self.text
                } else {
                    self.background
                }
            }
            CellColor::Indexed(i) => Color::Indexed(i),
            CellColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        }
    }
}

fn apply_vscode_theme(
    theme: &mut Theme,
    path: &std::path::Path,
    canonical_root: &std::path::Path,
    value: &serde_json::Value,
    visited: &mut HashSet<std::path::PathBuf>,
    depth: usize,
) -> anyhow::Result<()> {
    anyhow::ensure!(depth < 16, "VS Code theme include nesting is too deep");
    let canonical_path = path.canonicalize()?;
    anyhow::ensure!(
        canonical_path.starts_with(canonical_root),
        "VS Code theme include escapes the extension directory"
    );
    anyhow::ensure!(
        visited.insert(canonical_path.clone()),
        "VS Code theme contains an include cycle"
    );
    if let Some(include) = value["include"].as_str() {
        let include_path = canonical_path
            .parent()
            .unwrap_or(canonical_root)
            .join(include);
        let include_text = std::fs::read_to_string(&include_path)?;
        let include_value = crate::services::vscode::parse_jsonc(&include_text)?;
        apply_vscode_theme(
            theme,
            &include_path,
            canonical_root,
            &include_value,
            visited,
            depth + 1,
        )?;
    }
    let colors = &value["colors"];
    set_color(&mut theme.background, &colors["editor.background"]);
    set_color(&mut theme.text, &colors["editor.foreground"]);
    set_color(&mut theme.text_bright, &colors["editor.foreground"]);
    set_color(&mut theme.surface, &colors["sideBar.background"]);
    set_color(
        &mut theme.surface_alt,
        &colors["editorGroupHeader.tabsBackground"],
    );
    set_color(&mut theme.border, &colors["panel.border"]);
    set_color(&mut theme.border_focus, &colors["focusBorder"]);
    set_color(&mut theme.accent, &colors["activityBarBadge.background"]);
    set_color(&mut theme.selection, &colors["editor.selectionBackground"]);
    set_color(
        &mut theme.cursor_line,
        &colors["editor.lineHighlightBackground"],
    );
    set_color(
        &mut theme.line_number,
        &colors["editorLineNumber.foreground"],
    );
    set_color(
        &mut theme.line_number_active,
        &colors["editorLineNumber.activeForeground"],
    );
    if let Some(rules) = value["tokenColors"].as_array() {
        for rule in rules {
            let Some(color) = rule["settings"]["foreground"]
                .as_str()
                .and_then(parse_hex_color)
            else {
                continue;
            };
            let scopes: Vec<&str> = match &rule["scope"] {
                serde_json::Value::String(scope) => scope.split(',').map(str::trim).collect(),
                serde_json::Value::Array(scopes) => {
                    scopes.iter().filter_map(|scope| scope.as_str()).collect()
                }
                _ => Vec::new(),
            };
            for scope in scopes {
                apply_scope_color(theme, scope, color);
            }
        }
    }
    visited.remove(&canonical_path);
    Ok(())
}

fn set_color(target: &mut Color, value: &serde_json::Value) {
    if let Some(color) = value.as_str().and_then(parse_hex_color) {
        *target = color;
    }
}

fn parse_hex_color(value: &str) -> Option<Color> {
    let value = value.strip_prefix('#')?;
    if value.len() != 6 && value.len() != 8 {
        return None;
    }
    let red = u8::from_str_radix(&value[0..2], 16).ok()?;
    let green = u8::from_str_radix(&value[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&value[4..6], 16).ok()?;
    Some(Color::Rgb(red, green, blue))
}

fn apply_scope_color(theme: &mut Theme, scope: &str, color: Color) {
    if scope.contains("comment") {
        theme.syn_comment = color;
    } else if scope.contains("string") {
        theme.syn_string = color;
    } else if scope.contains("constant.numeric") {
        theme.syn_number = color;
    } else if scope.contains("entity.name.function") {
        theme.syn_function = color;
    } else if scope.contains("entity.name.type")
        || scope.contains("entity.name.class")
        || scope.contains("support.type")
    {
        theme.syn_type = color;
    } else if scope.contains("variable.other.property") {
        theme.syn_property = color;
    } else if scope.contains("constant") {
        theme.syn_constant = color;
    } else if scope.contains("keyword.operator") {
        theme.syn_operator = color;
    } else if scope.contains("keyword") || scope.contains("storage") {
        theme.syn_keyword = color;
    } else if scope.contains("punctuation") {
        theme.syn_punctuation = color;
    } else if scope.contains("attribute") {
        theme.syn_attribute = color;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_lookup_falls_back_to_the_default() {
        assert_eq!(Theme::by_name("nope").name, "termloom-dark");
        assert_eq!(Theme::by_name("ansi").name, "ansi");
    }

    #[test]
    fn focus_changes_the_border_colour() {
        let theme = Theme::termloom_dark();
        assert_ne!(
            theme.panel_border(true).fg,
            theme.panel_border(false).fg,
            "focus must be visible without colour alone elsewhere"
        );
    }

    #[test]
    fn every_agent_state_has_a_colour_and_a_glyph() {
        let theme = Theme::termloom_dark();
        for state in [
            AgentState::Starting,
            AgentState::Working,
            AgentState::WaitingForInput,
            AgentState::Idle,
            AgentState::Done,
            AgentState::Failed,
            AgentState::Exited,
            AgentState::Unknown,
        ] {
            let _ = theme.agent_state(state);
            assert!(!state.glyph().is_empty(), "{state:?} needs a glyph");
        }
    }

    #[test]
    fn terminal_default_colours_use_theme_tokens() {
        let theme = Theme::termloom_dark();
        assert_eq!(theme.terminal_color(CellColor::Default, true), theme.text);
        assert_eq!(
            theme.terminal_color(CellColor::Indexed(3), true),
            Color::Indexed(3)
        );
        assert_eq!(
            theme.terminal_color(CellColor::Rgb(1, 2, 3), false),
            Color::Rgb(1, 2, 3)
        );
    }

    #[test]
    fn parses_vscode_hex_colors() {
        assert_eq!(parse_hex_color("#112233"), Some(Color::Rgb(17, 34, 51)));
        assert_eq!(
            parse_hex_color("#aabbcc80"),
            Some(Color::Rgb(170, 187, 204))
        );
        assert_eq!(parse_hex_color("red"), None);
    }

    #[test]
    fn loads_vscode_theme_includes_before_local_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("extension");
        std::fs::create_dir_all(root.join("themes")).unwrap();
        std::fs::write(
            root.join("base.json"),
            r##"{"colors":{"editor.background":"#112233","editor.foreground":"#abcdef"},"tokenColors":[{"scope":"comment","settings":{"foreground":"#445566"}}]}"##,
        )
        .unwrap();
        std::fs::write(
            root.join("themes/dark.json"),
            r##"{"include":"../base.json","colors":{"editor.background":"#010203"}}"##,
        )
        .unwrap();

        let theme =
            Theme::from_vscode_file("Imported", &root.join("themes/dark.json"), &root).unwrap();
        assert_eq!(theme.name, "Imported");
        assert_eq!(theme.background, Color::Rgb(1, 2, 3));
        assert_eq!(theme.text, Color::Rgb(171, 205, 239));
        assert_eq!(theme.syn_comment, Color::Rgb(68, 85, 102));
    }
}
