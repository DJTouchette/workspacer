//! The "terminal path": render an agent's raw PTY the way the Electron app's
//! terminal pane does, instead of the parsed transcript. claudemon streams the
//! PTY bytes (`/sessions/:id/stream`); we feed them through a `vt100` emulator
//! and render the resulting screen, and forward keystrokes back to
//! `/sessions/:id/input`. Because it's the real terminal, interaction is just
//! typing — number keys for pickers, `y`/`n`, Esc, arrows, etc.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_term::vt100;

/// Holds the emulated screen for one agent's terminal. Keyed by session id in
/// the app's `terms` map, so it doesn't carry the id itself.
pub struct Term {
    parser: vt100::Parser,
    pub rows: u16,
    pub cols: u16,
}

impl Default for Term {
    fn default() -> Self {
        Self::new()
    }
}

impl Term {
    pub fn new() -> Self {
        // Seeded to the daemon's default PTY size; resized to the pane on first
        // render (which also tells claudemon to reflow).
        let (rows, cols) = (32, 120);
        Term {
            parser: vt100::Parser::new(rows, cols, 0),
            rows,
            cols,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// Resize the emulator to match the pane. Returns true if the size actually
    /// changed (so the caller knows to tell claudemon to reflow the PTY).
    pub fn resize(&mut self, rows: u16, cols: u16) -> bool {
        if rows == 0 || cols == 0 || (rows == self.rows && cols == self.cols) {
            return false;
        }
        self.parser.set_size(rows, cols);
        self.rows = rows;
        self.cols = cols;
        true
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }
}

/// The detach chord: Ctrl-]. In a normal terminal crossterm delivers the
/// control byte `0x1D` as `Ctrl+'5'` (it maps `0x1C..=0x1F` → `'4'..='7'`);
/// with the kitty keyboard protocol it arrives as `Ctrl+']'`. Accept both so
/// pressing Ctrl-] always detaches regardless of terminal.
pub fn is_detach(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(']') | KeyCode::Char('5'))
}

/// Translate a key press into the bytes a terminal would send, so we can
/// forward it to the PTY. Returns `None` for keys we don't map.
pub fn encode_key(key: &KeyEvent) -> Option<Vec<u8>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let bytes = match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                // Ctrl-A..Ctrl-_ map to 0x01..0x1f.
                let upper = c.to_ascii_uppercase() as u8;
                if (0x40..=0x5f).contains(&upper) {
                    vec![upper & 0x1f]
                } else if c == ' ' {
                    vec![0] // Ctrl-Space → NUL
                } else {
                    return None;
                }
            } else {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => vec![0x1b, b'[', b'Z'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Left => vec![0x1b, b'[', b'D'],
        KeyCode::Right => vec![0x1b, b'[', b'C'],
        KeyCode::Up => vec![0x1b, b'[', b'A'],
        KeyCode::Down => vec![0x1b, b'[', b'B'],
        KeyCode::Home => vec![0x1b, b'[', b'H'],
        KeyCode::End => vec![0x1b, b'[', b'F'],
        KeyCode::PageUp => vec![0x1b, b'[', b'5', b'~'],
        KeyCode::PageDown => vec![0x1b, b'[', b'6', b'~'],
        KeyCode::Delete => vec![0x1b, b'[', b'3', b'~'],
        KeyCode::Insert => vec![0x1b, b'[', b'2', b'~'],
        _ => return None,
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn detect_detach_both_terminal_modes() {
        // Legacy: Ctrl-] arrives as Ctrl+'5'.
        assert!(is_detach(&key(KeyCode::Char('5'), KeyModifiers::CONTROL)));
        // Kitty protocol: Ctrl-] arrives as Ctrl+']'.
        assert!(is_detach(&key(KeyCode::Char(']'), KeyModifiers::CONTROL)));
        // Not a detach.
        assert!(!is_detach(&key(KeyCode::Char('5'), KeyModifiers::NONE)));
        assert!(!is_detach(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn encodes_common_keys() {
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(vec![b'\r'])
        );
        assert_eq!(
            encode_key(&key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(vec![b'a'])
        );
        // Ctrl-C → 0x03 (interrupt), forwarded to Claude.
        assert_eq!(
            encode_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(vec![0x03])
        );
        assert_eq!(
            encode_key(&key(KeyCode::Up, KeyModifiers::NONE)),
            Some(vec![0x1b, b'[', b'A'])
        );
    }
}
