//! Keyboard → PTY byte encoding.
//!
//! When a terminal pane has focus the child program owns the keyboard, so key
//! events are translated into the escape sequences a terminal would normally
//! send. Getting this right is what makes Claude Code, vim, less and shell
//! line editing usable inside a pane.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Encode a key event for the child process. Returns `None` for keys with no
/// terminal representation (pure modifier presses, unmapped media keys).
pub fn encode_key(event: &KeyEvent) -> Option<Vec<u8>> {
    let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    let alt = event.modifiers.contains(KeyModifiers::ALT);
    let shift = event.modifiers.contains(KeyModifiers::SHIFT);

    let mut bytes: Vec<u8> = match event.code {
        KeyCode::Char(c) => {
            if ctrl {
                // Control characters: Ctrl+A..Ctrl+Z plus the usual symbols.
                let upper = c.to_ascii_uppercase();
                match upper {
                    'A'..='Z' => vec![(upper as u8) - b'A' + 1],
                    '@' | ' ' => vec![0],
                    '[' => vec![27],
                    '\\' => vec![28],
                    ']' => vec![29],
                    '^' => vec![30],
                    '_' | '/' | '?' => vec![31],
                    _ => c.to_string().into_bytes(),
                }
            } else {
                c.to_string().into_bytes()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Left => arrow(b'D', event.modifiers),
        KeyCode::Right => arrow(b'C', event.modifiers),
        KeyCode::Up => arrow(b'A', event.modifiers),
        KeyCode::Down => arrow(b'B', event.modifiers),
        KeyCode::Home => arrow(b'H', event.modifiers),
        KeyCode::End => arrow(b'F', event.modifiers),
        KeyCode::PageUp => tilde(5, event.modifiers),
        KeyCode::PageDown => tilde(6, event.modifiers),
        KeyCode::Insert => tilde(2, event.modifiers),
        KeyCode::Delete => tilde(3, event.modifiers),
        KeyCode::F(n) => function_key(n, event.modifiers)?,
        KeyCode::Null => vec![0],
        _ => return None,
    };

    // Alt is transmitted as an ESC prefix, except for sequences that already
    // encode their modifiers in a CSI parameter.
    let already_csi = bytes.starts_with(b"\x1b[") || bytes.starts_with(b"\x1bO");
    if alt && !already_csi {
        let mut out = vec![0x1b];
        out.append(&mut bytes);
        return Some(out);
    }
    let _ = shift;
    Some(bytes)
}

/// Encode a chunk of text (paste, or programmatic input sent to an agent).
pub fn encode_text(text: &str) -> Vec<u8> {
    // Terminals expect CR for line breaks on input.
    text.replace('\n', "\r").into_bytes()
}

/// xterm modifier parameter: 1 + bitmask(shift=1, alt=2, ctrl=4).
fn modifier_param(mods: KeyModifiers) -> Option<u8> {
    let mut value = 0;
    if mods.contains(KeyModifiers::SHIFT) {
        value |= 1;
    }
    if mods.contains(KeyModifiers::ALT) {
        value |= 2;
    }
    if mods.contains(KeyModifiers::CONTROL) {
        value |= 4;
    }
    (value != 0).then_some(value + 1)
}

fn arrow(final_byte: u8, mods: KeyModifiers) -> Vec<u8> {
    match modifier_param(mods) {
        Some(param) => format!("\x1b[1;{}{}", param, final_byte as char).into_bytes(),
        None => vec![0x1b, b'[', final_byte],
    }
}

fn tilde(number: u8, mods: KeyModifiers) -> Vec<u8> {
    match modifier_param(mods) {
        Some(param) => format!("\x1b[{number};{param}~").into_bytes(),
        None => format!("\x1b[{number}~").into_bytes(),
    }
}

fn function_key(n: u8, mods: KeyModifiers) -> Option<Vec<u8>> {
    // F1-F4 use SS3 when unmodified; the rest use CSI ~ with a code.
    let unmodified = modifier_param(mods).is_none();
    let bytes = match (n, unmodified) {
        (1, true) => b"\x1bOP".to_vec(),
        (2, true) => b"\x1bOQ".to_vec(),
        (3, true) => b"\x1bOR".to_vec(),
        (4, true) => b"\x1bOS".to_vec(),
        _ => {
            let code = match n {
                1 => 11,
                2 => 12,
                3 => 13,
                4 => 14,
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return None,
            };
            match modifier_param(mods) {
                Some(param) => format!("\x1b[{code};{param}~").into_bytes(),
                None => format!("\x1b[{code}~").into_bytes(),
            }
        }
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> Vec<u8> {
        encode_key(&KeyEvent::new(code, mods)).expect("encodable key")
    }

    #[test]
    fn plain_characters_pass_through() {
        assert_eq!(key(KeyCode::Char('a'), KeyModifiers::NONE), b"a");
        assert_eq!(key(KeyCode::Char('é'), KeyModifiers::NONE), "é".as_bytes());
    }

    #[test]
    fn control_characters_follow_the_ascii_rule() {
        assert_eq!(key(KeyCode::Char('c'), KeyModifiers::CONTROL), vec![3]);
        assert_eq!(key(KeyCode::Char('d'), KeyModifiers::CONTROL), vec![4]);
        assert_eq!(key(KeyCode::Char('z'), KeyModifiers::CONTROL), vec![26]);
        assert_eq!(key(KeyCode::Char(' '), KeyModifiers::CONTROL), vec![0]);
    }

    #[test]
    fn enter_backspace_and_tab_use_terminal_conventions() {
        assert_eq!(key(KeyCode::Enter, KeyModifiers::NONE), b"\r");
        assert_eq!(key(KeyCode::Backspace, KeyModifiers::NONE), vec![0x7f]);
        assert_eq!(key(KeyCode::Tab, KeyModifiers::NONE), b"\t");
        assert_eq!(key(KeyCode::BackTab, KeyModifiers::NONE), b"\x1b[Z");
    }

    #[test]
    fn arrows_use_csi_and_encode_modifiers() {
        assert_eq!(key(KeyCode::Up, KeyModifiers::NONE), b"\x1b[A");
        assert_eq!(key(KeyCode::Left, KeyModifiers::CONTROL), b"\x1b[1;5D");
        assert_eq!(key(KeyCode::Right, KeyModifiers::SHIFT), b"\x1b[1;2C");
    }

    #[test]
    fn alt_prefixes_with_escape_but_not_twice() {
        assert_eq!(key(KeyCode::Char('b'), KeyModifiers::ALT), b"\x1bb");
        // Arrow keys carry the modifier in the CSI parameter instead.
        assert_eq!(key(KeyCode::Up, KeyModifiers::ALT), b"\x1b[1;3A");
    }

    #[test]
    fn navigation_keys_use_tilde_sequences() {
        assert_eq!(key(KeyCode::Delete, KeyModifiers::NONE), b"\x1b[3~");
        assert_eq!(key(KeyCode::PageUp, KeyModifiers::NONE), b"\x1b[5~");
        assert_eq!(key(KeyCode::Home, KeyModifiers::NONE), b"\x1b[H");
    }

    #[test]
    fn function_keys_switch_between_ss3_and_csi() {
        assert_eq!(key(KeyCode::F(1), KeyModifiers::NONE), b"\x1bOP");
        assert_eq!(key(KeyCode::F(5), KeyModifiers::NONE), b"\x1b[15~");
        assert_eq!(key(KeyCode::F(1), KeyModifiers::CONTROL), b"\x1b[11;5~");
        assert!(encode_key(&KeyEvent::new(KeyCode::F(20), KeyModifiers::NONE)).is_none());
    }

    #[test]
    fn text_input_converts_newlines_to_carriage_returns() {
        assert_eq!(encode_text("a\nb"), b"a\rb");
    }
}
