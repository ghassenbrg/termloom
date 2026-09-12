//! Key chord parsing and the default keymap.
//!
//! Keys are never wired directly to UI functions: a chord resolves to a stable
//! command id which the command registry executes. That keeps `[keys]`
//! overrides, the command palette and the help overlay in sync.

use std::collections::BTreeMap;
use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A single key press: a code plus modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl KeyChord {
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        Self {
            code,
            mods: normalise_mods(code, mods),
        }
    }

    /// Parse chords such as `ctrl+space`, `alt+shift+f`, `f5`, `esc`.
    pub fn parse(text: &str) -> Option<KeyChord> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let mut mods = KeyModifiers::NONE;
        let parts: Vec<&str> = text.split('+').collect();
        let (last, prefix) = parts.split_last()?;
        for p in prefix {
            match p.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "c" => mods |= KeyModifiers::CONTROL,
                "alt" | "meta" | "option" | "a" | "m" => mods |= KeyModifiers::ALT,
                "shift" | "s" => mods |= KeyModifiers::SHIFT,
                "super" | "cmd" => mods |= KeyModifiers::SUPER,
                _ => return None,
            }
        }
        let code = parse_code(last.trim())?;
        Some(KeyChord::new(code, mods))
    }

    /// True when a terminal key event activates this chord.
    pub fn matches(&self, event: &KeyEvent) -> bool {
        let incoming = KeyChord::new(event.code, event.modifiers);
        incoming == *self
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.contains(KeyModifiers::CONTROL) {
            write!(f, "Ctrl+")?;
        }
        if self.mods.contains(KeyModifiers::ALT) {
            write!(f, "Alt+")?;
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            write!(f, "Shift+")?;
        }
        if self.mods.contains(KeyModifiers::SUPER) {
            write!(f, "Super+")?;
        }
        match self.code {
            KeyCode::Char(' ') => write!(f, "Space"),
            KeyCode::Char(c) => write!(f, "{}", c.to_ascii_uppercase()),
            KeyCode::F(n) => write!(f, "F{n}"),
            KeyCode::Enter => write!(f, "Enter"),
            KeyCode::Esc => write!(f, "Esc"),
            KeyCode::Tab => write!(f, "Tab"),
            KeyCode::BackTab => write!(f, "Shift+Tab"),
            KeyCode::Backspace => write!(f, "Backspace"),
            KeyCode::Delete => write!(f, "Delete"),
            KeyCode::Left => write!(f, "Left"),
            KeyCode::Right => write!(f, "Right"),
            KeyCode::Up => write!(f, "Up"),
            KeyCode::Down => write!(f, "Down"),
            KeyCode::Home => write!(f, "Home"),
            KeyCode::End => write!(f, "End"),
            KeyCode::PageUp => write!(f, "PgUp"),
            KeyCode::PageDown => write!(f, "PgDn"),
            other => write!(f, "{other:?}"),
        }
    }
}

/// Shift is implicit in the character a terminal reports (`A` rather than
/// `shift+a`), so we drop it for character keys to avoid double counting.
fn normalise_mods(code: KeyCode, mods: KeyModifiers) -> KeyModifiers {
    let mut m = mods & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
    if !matches!(code, KeyCode::Char(_)) && mods.contains(KeyModifiers::SHIFT) {
        m |= KeyModifiers::SHIFT;
    }
    m
}

fn parse_code(name: &str) -> Option<KeyCode> {
    let lower = name.to_ascii_lowercase();
    let code = match lower.as_str() {
        "space" => KeyCode::Char(' '),
        "enter" | "return" | "cr" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" | "shift+tab" => KeyCode::BackTab,
        "backspace" | "bs" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        other => {
            if let Some(rest) = other.strip_prefix('f') {
                if let Ok(n) = rest.parse::<u8>() {
                    if (1..=24).contains(&n) {
                        return Some(KeyCode::F(n));
                    }
                }
            }
            let mut chars = name.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            KeyCode::Char(c.to_ascii_lowercase())
        }
    };
    Some(code)
}

/// Where a binding applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Active everywhere, including while a terminal has focus. Kept tiny so
    /// child programs (Claude, vim, shells) keep their keys.
    Global,
    /// Active after the command prefix (`Ctrl+Space`) was pressed.
    Prefix,
    /// Active when the editor has focus.
    Editor,
    /// Active when a list-like panel has focus (explorer, git, agents, ...).
    Panel,
}

/// A resolved binding.
#[derive(Debug, Clone)]
pub struct Binding {
    pub chord: KeyChord,
    pub command: String,
    pub scope: Scope,
}

/// Default bindings: `(chord, command id, scope)`.
pub const DEFAULT_BINDINGS: &[(&str, &str, Scope)] = &[
    // ── global ────────────────────────────────────────────────────────────
    ("ctrl+space", "workbench.prefix", Scope::Global),
    ("ctrl+p", "palette.quick_open", Scope::Global),
    ("ctrl+q", "app.quit", Scope::Global),
    ("f1", "help.toggle", Scope::Global),
    ("f5", "debug.continue", Scope::Global),
    ("f9", "debug.toggle_breakpoint", Scope::Global),
    ("f10", "debug.step_over", Scope::Global),
    ("f11", "debug.step_in", Scope::Global),
    ("f12", "debug.step_out", Scope::Global),
    // ── prefix (Ctrl+Space, then…) ────────────────────────────────────────
    ("e", "view.focus.explorer", Scope::Prefix),
    ("o", "view.focus.outline", Scope::Prefix),
    ("g", "view.focus.git", Scope::Prefix),
    ("a", "view.focus.agents", Scope::Prefix),
    ("d", "view.focus.editor", Scope::Prefix),
    ("t", "terminal.new", Scope::Prefix),
    ("c", "agent.new.claude", Scope::Prefix),
    ("x", "agent.new.codex", Scope::Prefix),
    ("p", "palette.commands", Scope::Prefix),
    ("w", "editor.close_tab", Scope::Prefix),
    ("s", "file.save", Scope::Prefix),
    ("shift+s", "file.save_all", Scope::Prefix),
    ("b", "view.toggle.explorer", Scope::Prefix),
    ("shift+a", "view.toggle.agents", Scope::Prefix),
    ("shift+t", "view.toggle.terminals", Scope::Prefix),
    ("m", "view.toggle.problems", Scope::Prefix),
    ("v", "view.toggle.debug", Scope::Prefix),
    ("k", "terminal.kill", Scope::Prefix),
    ("r", "terminal.restart", Scope::Prefix),
    ("n", "terminal.next", Scope::Prefix),
    ("left", "focus.left", Scope::Prefix),
    ("right", "focus.right", Scope::Prefix),
    ("up", "focus.up", Scope::Prefix),
    ("down", "focus.down", Scope::Prefix),
    ("h", "focus.left", Scope::Prefix),
    ("l", "focus.right", Scope::Prefix),
    ("?", "help.toggle", Scope::Prefix),
    // ── editor ────────────────────────────────────────────────────────────
    ("ctrl+s", "file.save", Scope::Editor),
    ("ctrl+w", "editor.close_tab", Scope::Editor),
    ("ctrl+f", "editor.find", Scope::Editor),
    ("ctrl+z", "editor.undo", Scope::Editor),
    ("ctrl+y", "editor.redo", Scope::Editor),
    ("ctrl+g", "editor.goto_line", Scope::Editor),
    ("ctrl+d", "lsp.definition", Scope::Editor),
    ("ctrl+r", "lsp.references", Scope::Editor),
    ("ctrl+k", "lsp.hover", Scope::Editor),
    ("ctrl+n", "lsp.completion", Scope::Editor),
];

/// Chord → command lookup, split by scope.
#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: Vec<Binding>,
    /// Chord that enters prefix mode.
    pub prefix: KeyChord,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap::from_overrides(&BTreeMap::new())
    }
}

impl Keymap {
    /// Build the default keymap, applying `command id -> chord` overrides.
    pub fn from_overrides(overrides: &BTreeMap<String, String>) -> Keymap {
        let mut bindings: Vec<Binding> = DEFAULT_BINDINGS
            .iter()
            .filter_map(|(chord, command, scope)| {
                KeyChord::parse(chord).map(|chord| Binding {
                    chord,
                    command: (*command).to_string(),
                    scope: *scope,
                })
            })
            .collect();

        for (command, chord_text) in overrides {
            let Some(chord) = KeyChord::parse(chord_text) else {
                continue;
            };
            // Replace the command's primary (non-prefix) binding when it has
            // one, otherwise its prefix binding, otherwise add a global one.
            let target = bindings
                .iter()
                .position(|b| &b.command == command && b.scope != Scope::Prefix)
                .or_else(|| bindings.iter().position(|b| &b.command == command));
            if let Some(index) = target {
                bindings[index].chord = chord;
            } else {
                bindings.push(Binding {
                    chord,
                    command: command.clone(),
                    scope: Scope::Global,
                });
            }
        }

        let prefix = bindings
            .iter()
            .find(|b| b.command == "workbench.prefix")
            .map(|b| b.chord)
            .unwrap_or_else(|| KeyChord::new(KeyCode::Char(' '), KeyModifiers::CONTROL));

        Keymap { bindings, prefix }
    }

    /// Resolve a key event in a scope.
    pub fn resolve(&self, scope: Scope, event: &KeyEvent) -> Option<&str> {
        self.bindings
            .iter()
            .find(|b| b.scope == scope && b.chord.matches(event))
            .map(|b| b.command.as_str())
    }

    /// The chord bound to a command, for display in menus and help. Direct
    /// chords win over prefix chords because they are shorter to show.
    pub fn chord_for(&self, command: &str) -> Option<KeyChord> {
        self.binding_for(command).map(|b| b.chord)
    }

    fn binding_for(&self, command: &str) -> Option<&Binding> {
        self.bindings
            .iter()
            .find(|b| b.command == command && b.scope != Scope::Prefix)
            .or_else(|| self.bindings.iter().find(|b| b.command == command))
    }

    /// Display string like `Ctrl+Space E` for prefix bindings.
    pub fn hint_for(&self, command: &str) -> Option<String> {
        let binding = self.binding_for(command)?;
        Some(match binding.scope {
            Scope::Prefix => format!("{} {}", self.prefix, binding.chord),
            _ => binding.chord.to_string(),
        })
    }

    pub fn all(&self) -> &[Binding] {
        &self.bindings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifier_chords() {
        let chord = KeyChord::parse("ctrl+space").unwrap();
        assert_eq!(chord.code, KeyCode::Char(' '));
        assert!(chord.mods.contains(KeyModifiers::CONTROL));
        assert_eq!(KeyChord::parse("f5").unwrap().code, KeyCode::F(5));
        assert_eq!(KeyChord::parse("esc").unwrap().code, KeyCode::Esc);
        assert!(KeyChord::parse("bogus+x").is_none());
        assert!(KeyChord::parse("").is_none());
    }

    #[test]
    fn char_chords_ignore_shift_modifier() {
        // Terminals report `Shift+a` as `A`; treating both the same avoids
        // bindings that can never fire.
        let a = KeyChord::new(KeyCode::Char('a'), KeyModifiers::SHIFT);
        let b = KeyChord::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(a, b);
    }

    #[test]
    fn resolves_default_bindings_per_scope() {
        let map = Keymap::default();
        let ctrl_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(map.resolve(Scope::Editor, &ctrl_s), Some("file.save"));
        assert_eq!(map.resolve(Scope::Global, &ctrl_s), None);

        let e = KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE);
        assert_eq!(map.resolve(Scope::Prefix, &e), Some("view.focus.explorer"));
    }

    #[test]
    fn overrides_replace_the_default_chord() {
        let mut overrides = BTreeMap::new();
        overrides.insert("file.save".to_string(), "ctrl+x".to_string());
        let map = Keymap::from_overrides(&overrides);
        let ctrl_x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(map.resolve(Scope::Editor, &ctrl_x), Some("file.save"));
        let ctrl_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(map.resolve(Scope::Editor, &ctrl_s), None);
    }

    #[test]
    fn unknown_command_override_adds_a_global_binding() {
        let mut overrides = BTreeMap::new();
        overrides.insert("workspace.reload".to_string(), "ctrl+alt+r".to_string());
        let map = Keymap::from_overrides(&overrides);
        let ev = KeyEvent::new(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert_eq!(map.resolve(Scope::Global, &ev), Some("workspace.reload"));
    }

    #[test]
    fn hints_include_the_prefix() {
        let map = Keymap::default();
        assert_eq!(
            map.hint_for("view.focus.explorer").as_deref(),
            Some("Ctrl+Space E")
        );
        assert_eq!(map.hint_for("file.save").as_deref(), Some("Ctrl+S"));
    }

    #[test]
    fn every_default_binding_parses() {
        for (chord, command, _) in DEFAULT_BINDINGS {
            assert!(
                KeyChord::parse(chord).is_some(),
                "chord {chord} for {command} does not parse"
            );
        }
    }
}
