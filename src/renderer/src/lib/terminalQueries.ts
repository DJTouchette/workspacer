/**
 * Detect the escape sequences xterm emits in *reply* to terminal queries it
 * sees in the output stream — Device Attributes, cursor-position / device
 * status reports, OSC colour answers, and DCS answers.
 *
 * Workspacer's xterm panes are passive mirrors of a claudemon PTY, not the
 * session's real terminal — claudemon itself answers these queries (once,
 * authoritatively). If a mirror also forwarded its own auto-replies, they'd be
 * injected into the session as synthetic input (the OSC/DCS String Terminator
 * `ESC \` is what shows up as a stray "\" in the prompt). Real user keystrokes,
 * mouse events (CSI < … M/m) and bracketed-paste markers never match these
 * patterns, so they pass through untouched.
 */
export function isTerminalQueryReply(data: string): boolean {
  // Device Attributes (primary/secondary/tertiary): ESC [ ?|>|= … c
  if (/^\x1b\[[?>=][0-9;]*c$/.test(data)) return true;
  // Cursor position report (incl. DECXCPR): ESC [ ?? rows ; cols R
  if (/^\x1b\[\??[0-9]+;[0-9]+R$/.test(data)) return true;
  // Device status report: ESC [ <n> n
  if (/^\x1b\[[0-9]+n$/.test(data)) return true;
  // OSC answer (e.g. colour query 10/11/12): ESC ] … BEL | ST(ESC \)
  if (/^\x1b\][0-9].*(\x07|\x1b\\)$/.test(data)) return true;
  // DCS answer (XTGETTCAP, DECRQSS, …): ESC P … ST(ESC \)
  if (/^\x1bP.*\x1b\\$/.test(data)) return true;
  return false;
}
