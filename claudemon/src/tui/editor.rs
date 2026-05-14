//! Text editor for the chat input box.
//!
//! Holds the buffer, cursor (as a byte offset into the UTF-8 buffer), and
//! an input-history ring. All editing primitives are UTF-8 safe and
//! exhaustively unit-tested below — the TUI layer just forwards keys.

use std::collections::VecDeque;

const HISTORY_CAP: usize = 100;

#[derive(Debug, Default)]
pub struct Editor {
    text: String,
    /// Byte offset in `text`. Always lies on a char boundary.
    cursor: usize,
    /// Past messages the user has sent, oldest first.
    history: VecDeque<String>,
    /// Current position when browsing with Up/Down. `None` means we're
    /// composing fresh content (not yet pulled from history).
    history_idx: Option<usize>,
    /// Stashed buffer from before we started browsing, so Down past the
    /// newest entry restores what the user was typing.
    composing: String,
}

impl Editor {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- read-only accessors ----

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    #[allow(dead_code)]
    pub fn history(&self) -> &VecDeque<String> {
        &self.history
    }

    // ---- editing ----

    pub fn insert(&mut self, ch: char) {
        // Any user keystroke leaves history-browse mode but keeps the
        // current text so they can edit a recalled entry inline.
        self.history_idx = None;
        let mut tmp = [0u8; 4];
        let s = ch.encode_utf8(&mut tmp);
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    pub fn insert_newline(&mut self) {
        self.insert('\n');
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = prev_boundary(&self.text, self.cursor);
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
        self.history_idx = None;
    }

    pub fn delete_forward(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let next = next_boundary(&self.text, self.cursor);
        self.text.replace_range(self.cursor..next, "");
        self.history_idx = None;
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = prev_boundary(&self.text, self.cursor);
        }
    }
    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            self.cursor = next_boundary(&self.text, self.cursor);
        }
    }
    pub fn move_home(&mut self) {
        // Home → start of current line.
        let line_start = self.text[..self.cursor]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        self.cursor = line_start;
    }
    pub fn move_end(&mut self) {
        // End → end of current line.
        let rest = &self.text[self.cursor..];
        let offset = rest.find('\n').unwrap_or(rest.len());
        self.cursor += offset;
    }

    pub fn delete_word_back(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // Skip trailing whitespace, then skip the word.
        let bytes = self.text.as_bytes();
        let mut i = self.cursor;
        while i > 0 && bytes[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        while i > 0 && !bytes[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        // Walk to a char boundary if we landed mid-codepoint.
        while i > 0 && !self.text.is_char_boundary(i) {
            i -= 1;
        }
        self.text.replace_range(i..self.cursor, "");
        self.cursor = i;
        self.history_idx = None;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.history_idx = None;
    }

    // ---- history ----

    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        match self.history_idx {
            None => {
                self.composing = self.text.clone();
                self.history_idx = Some(self.history.len() - 1);
            }
            Some(0) => return, // already at oldest
            Some(i) => self.history_idx = Some(i - 1),
        }
        self.load_from_history();
    }

    pub fn history_next(&mut self) {
        let Some(i) = self.history_idx else { return };
        if i + 1 >= self.history.len() {
            // Moving past the newest entry returns to the composing buffer.
            self.history_idx = None;
            self.text = std::mem::take(&mut self.composing);
            self.cursor = self.text.len();
        } else {
            self.history_idx = Some(i + 1);
            self.load_from_history();
        }
    }

    fn load_from_history(&mut self) {
        if let Some(i) = self.history_idx {
            if let Some(entry) = self.history.get(i) {
                self.text = entry.clone();
                self.cursor = self.text.len();
            }
        }
    }

    /// Consume the current text, push it onto history, and reset state.
    /// Returns the text the caller should send. Empty input is treated
    /// as "nothing to send" — returns `None`.
    pub fn take_and_remember(&mut self) -> Option<String> {
        if self.text.is_empty() {
            return None;
        }
        let out = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.history_idx = None;
        self.composing.clear();
        // Skip pushing if it's identical to the most-recent entry — keeps
        // the history clean when users send the same prompt twice.
        if self.history.back().map(String::as_str) != Some(out.as_str()) {
            self.history.push_back(out.clone());
            while self.history.len() > HISTORY_CAP {
                self.history.pop_front();
            }
        }
        Some(out)
    }

    /// Restore the input on send failure so the user can edit + retry.
    pub fn restore(&mut self, text: String) {
        self.cursor = text.len();
        self.text = text;
    }

    // ---- visual cursor coordinates ----

    /// Compute the (col, row) of the cursor inside an input area of the
    /// given inner width. Hard `\n` and soft wraps both advance rows.
    /// Both values are 0-based.
    pub fn visual_cursor(&self, inner_width: u16) -> (u16, u16) {
        let width = inner_width.max(1) as usize;
        let mut col = 0usize;
        let mut row = 0usize;
        for (i, ch) in self.text.char_indices() {
            if i >= self.cursor {
                break;
            }
            if ch == '\n' {
                row += 1;
                col = 0;
                continue;
            }
            col += 1;
            if col >= width {
                row += 1;
                col = 0;
            }
        }
        (col.min(u16::MAX as usize) as u16, row.min(u16::MAX as usize) as u16)
    }

    /// Number of visual rows the buffer occupies, for sizing the input
    /// box. Always at least 1.
    pub fn visual_rows(&self, inner_width: u16) -> u16 {
        let width = inner_width.max(1) as usize;
        let mut rows: u16 = 1;
        let mut col = 0usize;
        for ch in self.text.chars() {
            if ch == '\n' {
                rows = rows.saturating_add(1);
                col = 0;
                continue;
            }
            col += 1;
            if col >= width {
                rows = rows.saturating_add(1);
                col = 0;
            }
        }
        rows
    }
}

fn prev_boundary(s: &str, byte: usize) -> usize {
    if byte == 0 {
        return 0;
    }
    let mut i = byte - 1;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn next_boundary(s: &str, byte: usize) -> usize {
    if byte >= s.len() {
        return s.len();
    }
    let mut i = byte + 1;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(s: &str) -> Editor {
        let mut e = Editor::new();
        for ch in s.chars() {
            e.insert(ch);
        }
        e
    }

    #[test]
    fn basic_insert_and_cursor() {
        let mut e = Editor::new();
        e.insert('h');
        e.insert('i');
        assert_eq!(e.text(), "hi");
        assert_eq!(e.cursor(), 2);
    }

    #[test]
    fn backspace_removes_previous_char() {
        let mut e = ed("hello");
        e.backspace();
        assert_eq!(e.text(), "hell");
        assert_eq!(e.cursor(), 4);
    }

    #[test]
    fn move_left_right_respects_utf8() {
        let mut e = ed("héllo");
        e.move_home();
        assert_eq!(e.cursor(), 0);
        e.move_right();
        e.move_right(); // skip the multi-byte é
        assert!(e.text().is_char_boundary(e.cursor()));
        e.move_left();
        assert_eq!(e.cursor(), 1);
    }

    #[test]
    fn home_and_end_in_multiline() {
        let mut e = Editor::new();
        for ch in "alpha\nbeta\ngamma".chars() {
            e.insert(ch);
        }
        // cursor at end of "gamma"
        assert_eq!(e.cursor(), e.text().len());
        e.move_home();
        // Should be at start of "gamma"
        assert_eq!(&e.text()[e.cursor()..], "gamma");
        e.move_end();
        assert_eq!(e.cursor(), e.text().len());
    }

    #[test]
    fn delete_word_back() {
        let mut e = ed("the quick brown fox");
        e.delete_word_back(); // drops "fox"
        assert_eq!(e.text(), "the quick brown ");
        e.delete_word_back(); // drops trailing space + "brown"
        assert_eq!(e.text(), "the quick ");
    }

    #[test]
    fn newline_insertion() {
        let mut e = ed("line1");
        e.insert_newline();
        e.insert('a');
        assert_eq!(e.text(), "line1\na");
    }

    #[test]
    fn history_recall_and_back() {
        let mut e = ed("first");
        let _ = e.take_and_remember();
        let mut e = e;
        for ch in "second".chars() {
            e.insert(ch);
        }
        let _ = e.take_and_remember();
        // Composing a draft, then browse back.
        for ch in "draft".chars() {
            e.insert(ch);
        }
        e.history_prev(); // → "second"
        assert_eq!(e.text(), "second");
        e.history_prev(); // → "first"
        assert_eq!(e.text(), "first");
        e.history_prev(); // stays at oldest
        assert_eq!(e.text(), "first");
        e.history_next(); // → "second"
        assert_eq!(e.text(), "second");
        e.history_next(); // → back to draft
        assert_eq!(e.text(), "draft");
    }

    #[test]
    fn typing_during_history_browse_drops_back_to_compose_state() {
        let mut e = Editor::new();
        e.insert('a');
        let _ = e.take_and_remember();
        e.history_prev();
        assert_eq!(e.text(), "a");
        // Editing a recalled entry should NOT advance the index when
        // the user types — they're editing the recalled value inline.
        e.insert('!');
        assert_eq!(e.text(), "a!");
        // …but sending creates a new history entry.
        assert_eq!(e.take_and_remember(), Some("a!".to_string()));
    }

    #[test]
    fn duplicate_sends_dont_dupe_history() {
        let mut e = ed("ping");
        let _ = e.take_and_remember();
        for ch in "ping".chars() {
            e.insert(ch);
        }
        let _ = e.take_and_remember();
        assert_eq!(e.history().len(), 1);
    }

    #[test]
    fn visual_cursor_on_single_line() {
        let mut e = ed("hello");
        assert_eq!(e.visual_cursor(20), (5, 0));
        e.move_home();
        assert_eq!(e.visual_cursor(20), (0, 0));
    }

    #[test]
    fn visual_cursor_after_newline() {
        let mut e = Editor::new();
        for ch in "ab\ncd".chars() {
            e.insert(ch);
        }
        assert_eq!(e.visual_cursor(20), (2, 1));
    }

    #[test]
    fn visual_cursor_with_soft_wrap() {
        let mut e = ed("abcdef");
        // width 4: chars wrap after col 3 -> row 1
        // a(0,0) b(1,0) c(2,0) d wraps row 1 col 0, e(1,1), f(2,1)
        // cursor at end: (3,1)
        assert_eq!(e.visual_cursor(4), (2, 1));
    }

    #[test]
    fn visual_rows_grows_with_newlines() {
        let mut e = ed("line1");
        assert_eq!(e.visual_rows(20), 1);
        e.insert_newline();
        assert_eq!(e.visual_rows(20), 2);
        e.insert_newline();
        assert_eq!(e.visual_rows(20), 3);
    }

    #[test]
    fn take_returns_none_when_empty() {
        let mut e = Editor::new();
        assert_eq!(e.take_and_remember(), None);
    }

    #[test]
    fn clear_resets_state() {
        let mut e = ed("stuff");
        e.clear();
        assert!(e.is_empty());
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn restore_preserves_text_and_puts_cursor_at_end() {
        let mut e = Editor::new();
        e.restore("recovered".into());
        assert_eq!(e.text(), "recovered");
        assert_eq!(e.cursor(), 9);
    }
}
