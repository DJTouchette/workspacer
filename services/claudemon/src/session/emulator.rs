//! Per-session headless terminal emulator.
//!
//! claudemon hands Claude a raw PTY, but the xterm panes in Workspacer are only
//! *mirrors* of the byte stream — there is no real terminal on Claude's end to
//! hold screen state or answer the queries Claude emits (cursor position,
//! device attributes, OSC colour). Historically each mirror answered those
//! queries itself, which injected synthetic input into the session (the stray
//! `\` from an OSC String Terminator) and made re-attach a blank screen until
//! the next repaint.
//!
//! This module makes the daemon the single authoritative terminal: every byte
//! of PTY output is fed through a `vt100` parser, so we always know the current
//! screen; the daemon answers the common queries itself, once and consistently;
//! and it can hand a freshly-attached viewer a self-contained redraw of the
//! current screen instead of a replay of raw history.

use vt100::Parser;

/// xterm's classic primary Device Attributes answer (VT100 with Advanced Video).
const DA1_RESPONSE: &[u8] = b"\x1b[?1;2c";
/// Secondary DA: "VT220, firmware 276, no cartridge". The version is cosmetic.
const DA2_RESPONSE: &[u8] = b"\x1b[>0;276;0c";
/// Device status report — terminal OK.
const DSR_OK_RESPONSE: &[u8] = b"\x1b[0n";

/// Longest query request we recognise (`ESC ] 11 ; ?` = 6 bytes). Used to size
/// the cross-chunk scan carry so a request split across two PTY writes is still
/// detected.
const MAX_QUERY_LEN: usize = 6;

/// A colour with four hex digits per channel, ready to splice into an OSC reply
/// (`rgb:RRRR/GGGG/BBBB`) the way real terminals report colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb16 {
    pub r: u16,
    pub g: u16,
    pub b: u16,
}

impl Rgb16 {
    /// Build from 8-bit-per-channel components, scaling each channel to 16 bits
    /// by replicating the byte (`0xAB` → `0xABAB`), matching xterm's reporting.
    pub const fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: ((r as u16) << 8) | r as u16,
            g: ((g as u16) << 8) | g as u16,
            b: ((b as u16) << 8) | b as u16,
        }
    }

    /// Sensible defaults so a session always has a consistent answer to colour
    /// queries even when the caller doesn't supply a theme. Dark background /
    /// light foreground matches Workspacer's default look.
    pub const DEFAULT_BG: Rgb16 = Rgb16::rgb8(0x1e, 0x1e, 0x1e);
    pub const DEFAULT_FG: Rgb16 = Rgb16::rgb8(0xd4, 0xd4, 0xd4);

    /// Parse an `#rrggbb` (or `rrggbb`) string. Returns `None` on malformed
    /// input so the caller can fall back to a default.
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Self::rgb8(r, g, b))
    }
}

/// Single authoritative terminal for one session.
pub struct TermEmulator {
    parser: Parser,
    /// Trailing bytes of the previous chunk kept *only* for split-query
    /// detection. Never re-fed to `parser` — it already consumed them.
    scan_carry: Vec<u8>,
    fg: Rgb16,
    bg: Rgb16,
}

impl TermEmulator {
    pub fn new(cols: u16, rows: u16, fg: Rgb16, bg: Rgb16) -> Self {
        // No scrollback: Claude runs as a full-screen (alternate-screen) app, so
        // a redraw only ever needs the visible grid. Keeps memory to one screen.
        Self {
            parser: Parser::new(rows, cols, 0),
            scan_carry: Vec::new(),
            fg,
            bg,
        }
    }

    pub fn set_size(&mut self, cols: u16, rows: u16) {
        self.parser.set_size(rows, cols);
    }

    /// Feed a chunk of PTY output into the screen model. Returns the bytes (if
    /// any) that must be written back to the child as replies to terminal
    /// queries it emitted in this chunk.
    pub fn process(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.parser.process(chunk);
        // Cursor after the whole chunk. A cursor-position request is virtually
        // always the last thing in a render (or a "move to 999;999 then ask"
        // size probe, which clamps to the real edge), so the post-chunk cursor
        // is the right answer. A mid-chunk CPR followed by further cursor motion
        // in the same write would report the post-chunk position — harmless, the
        // app re-queries and the screen self-heals.
        let (row, col) = self.parser.screen().cursor_position();
        self.scan_queries(chunk, row, col)
    }

    fn scan_queries(&mut self, chunk: &[u8], cursor_row: u16, cursor_col: u16) -> Vec<u8> {
        // Scan the carry-over tail plus this chunk so a request whose prefix
        // landed at the end of the previous write is still caught.
        let mut buf = Vec::with_capacity(self.scan_carry.len() + chunk.len());
        buf.extend_from_slice(&self.scan_carry);
        buf.extend_from_slice(chunk);

        let mut out = Vec::new();
        let mut i = 0usize;
        let mut last_match_end = 0usize;
        while i < buf.len() {
            if buf[i] != 0x1b {
                i += 1;
                continue;
            }
            if let Some((len, reply)) = self.match_query(&buf[i..], cursor_row, cursor_col) {
                out.extend_from_slice(&reply);
                i += len;
                last_match_end = i;
            } else {
                i += 1;
            }
        }

        // Keep a short tail for next time so a request split across writes still
        // completes. Never keep bytes that were part of a completed match, and
        // cap the carry at MAX_QUERY_LEN-1 so it can't grow unbounded.
        let keep_from = buf.len().saturating_sub(MAX_QUERY_LEN - 1).max(last_match_end);
        self.scan_carry = buf[keep_from..].to_vec();

        out
    }

    /// If `rest` begins with a recognised query request, return its byte length
    /// and the reply to send. Only matches when the full request is present, so
    /// a partial request at a buffer boundary is left for the scan carry.
    fn match_query(&self, rest: &[u8], cursor_row: u16, cursor_col: u16) -> Option<(usize, Vec<u8>)> {
        // Cursor position report (DSR 6): ESC [ 6 n  → ESC [ row ; col R (1-based)
        if rest.starts_with(b"\x1b[6n") {
            let reply = format!("\x1b[{};{}R", cursor_row + 1, cursor_col + 1).into_bytes();
            return Some((4, reply));
        }
        // Device status report (DSR 5): ESC [ 5 n  → OK
        if rest.starts_with(b"\x1b[5n") {
            return Some((4, DSR_OK_RESPONSE.to_vec()));
        }
        // Primary Device Attributes: ESC [ 0 c / ESC [ c
        if rest.starts_with(b"\x1b[0c") {
            return Some((4, DA1_RESPONSE.to_vec()));
        }
        if rest.starts_with(b"\x1b[c") {
            return Some((3, DA1_RESPONSE.to_vec()));
        }
        // Secondary Device Attributes: ESC [ > 0 c / ESC [ > c
        if rest.starts_with(b"\x1b[>0c") {
            return Some((5, DA2_RESPONSE.to_vec()));
        }
        if rest.starts_with(b"\x1b[>c") {
            return Some((4, DA2_RESPONSE.to_vec()));
        }
        // OSC foreground colour query: ESC ] 10 ; ?
        if rest.starts_with(b"\x1b]10;?") {
            return Some((6, osc_color_reply(10, self.fg)));
        }
        // OSC background colour query: ESC ] 11 ; ?
        if rest.starts_with(b"\x1b]11;?") {
            return Some((6, osc_color_reply(11, self.bg)));
        }
        None
    }

    /// A self-contained redraw of the current screen for a freshly-attached
    /// viewer — no dependence on prior terminal state. Enters the alternate
    /// screen first when Claude is in it, so the viewer's modes match the
    /// session and a later `?1049l` from Claude is interpreted correctly.
    pub fn snapshot(&self) -> Vec<u8> {
        let screen = self.parser.screen();
        let mut out = Vec::new();
        if screen.alternate_screen() {
            out.extend_from_slice(b"\x1b[?1049h");
        }
        out.extend_from_slice(&screen.state_formatted());
        out
    }
}

/// `ESC ] <which> ; rgb:RRRR/GGGG/BBBB ST` — terminated with ST (`ESC \`), the
/// form Claude's own query uses.
fn osc_color_reply(which: u8, c: Rgb16) -> Vec<u8> {
    format!(
        "\x1b]{};rgb:{:04x}/{:04x}/{:04x}\x1b\\",
        which, c.r, c.g, c.b
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emu() -> TermEmulator {
        TermEmulator::new(80, 24, Rgb16::DEFAULT_FG, Rgb16::DEFAULT_BG)
    }

    #[test]
    fn cpr_reports_one_based_cursor() {
        let mut e = emu();
        // Move to row 5, col 10 (1-based), then request cursor position.
        let reply = e.process(b"\x1b[5;10H\x1b[6n");
        assert_eq!(reply, b"\x1b[5;10R");
    }

    #[test]
    fn size_probe_clamps_to_screen_edge() {
        let mut e = emu(); // 24 rows x 80 cols
        // Classic width probe: jump way off-screen, then ask where we landed.
        let reply = e.process(b"\x1b[999;999H\x1b[6n");
        assert_eq!(reply, b"\x1b[24;80R");
    }

    #[test]
    fn device_attributes_and_status() {
        let mut e = emu();
        assert_eq!(e.process(b"\x1b[c"), b"\x1b[?1;2c");
        assert_eq!(e.process(b"\x1b[0c"), b"\x1b[?1;2c");
        assert_eq!(e.process(b"\x1b[>c"), b"\x1b[>0;276;0c");
        assert_eq!(e.process(b"\x1b[>0c"), b"\x1b[>0;276;0c");
        assert_eq!(e.process(b"\x1b[5n"), b"\x1b[0n");
    }

    #[test]
    fn osc_background_query_answered_with_st() {
        let mut e = TermEmulator::new(80, 24, Rgb16::DEFAULT_FG, Rgb16::rgb8(0x1e, 0x1e, 0x1e));
        let reply = e.process(b"\x1b]11;?\x1b\\");
        assert_eq!(reply, b"\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\");
    }

    #[test]
    fn ordinary_output_yields_no_reply() {
        let mut e = emu();
        // SGR colour change + text must not be mistaken for a query.
        assert!(e.process(b"hello \x1b[31mRED\x1b[m world").is_empty());
    }

    #[test]
    fn query_split_across_two_chunks_is_detected() {
        let mut e = emu();
        assert!(e.process(b"\x1b[").is_empty());
        let reply = e.process(b"6n");
        assert_eq!(reply, b"\x1b[1;1R");
    }

    #[test]
    fn osc_query_split_across_chunks_is_detected() {
        let mut e = TermEmulator::new(80, 24, Rgb16::DEFAULT_FG, Rgb16::rgb8(0x00, 0x00, 0x00));
        assert!(e.process(b"\x1b]11").is_empty());
        let reply = e.process(b";?\x1b\\");
        assert_eq!(reply, b"\x1b]11;rgb:0000/0000/0000\x1b\\");
    }

    #[test]
    fn carry_does_not_double_count_completed_query() {
        let mut e = emu();
        // A complete CPR with trailing text in the same chunk: the reply uses
        // the post-chunk cursor (after "abc", col 4, 1-based), and the trailing
        // bytes carried for split-detection must NOT make the next chunk re-emit
        // a reply.
        let first = e.process(b"\x1b[6nabc");
        assert_eq!(first, b"\x1b[1;4R");
        let second = e.process(b"xyz");
        assert!(second.is_empty());
    }

    #[test]
    fn snapshot_reproduces_visible_text() {
        let mut e = emu();
        e.process(b"hello world");
        let snap = e.snapshot();
        // Feed the snapshot into a fresh parser; it should reconstruct the text.
        let mut fresh = Parser::new(24, 80, 0);
        fresh.process(&snap);
        assert!(fresh.screen().contents().contains("hello world"));
    }

    #[test]
    fn snapshot_enters_alternate_screen_when_active() {
        let mut e = emu();
        e.process(b"\x1b[?1049h");
        let snap = e.snapshot();
        assert!(snap.windows(8).any(|w| w == b"\x1b[?1049h"));
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(Rgb16::from_hex("#1e1e1e"), Some(Rgb16::rgb8(0x1e, 0x1e, 0x1e)));
        assert_eq!(Rgb16::from_hex("ffffff"), Some(Rgb16::rgb8(0xff, 0xff, 0xff)));
        assert_eq!(Rgb16::from_hex("nope"), None);
        assert_eq!(Rgb16::from_hex("#12345"), None);
    }
}
