/**
 * Frame a message for the raw-PTY fallback path: bracketed-paste markers so a
 * multi-line body's own newlines stay newlines (the CR after the end marker is
 * the one real submit), mirroring claudemon's send_message_now and the TUI's
 * `bracketed_paste`.
 *
 * The body is neutralized first. A bracketed paste is only inert if its own
 * content cannot forge the end marker: an embedded ESC[201~ would close paste
 * mode early, and the bytes after it — with a trailing CR — land as live
 * keystrokes against the session's PTY. We replace every ESC (\x1b) with its
 * visible glyph U+241B before wrapping, exactly as xterm's bracketTextForPaste
 * does for its own paste path, so the frame we emit is the only paste boundary
 * and no injected control sequence survives. A trailing newline run is trimmed
 * so the appended CR is the sole submit.
 */
export function bracketedPasteSubmit(text: string): string {
  // eslint-disable-next-line no-control-regex
  const body = text.replace(/[\r\n]+$/, '').replace(/\x1b/g, '␛');
  return `\x1b[200~${body}\x1b[201~\r`;
}
