/**
 * Shared terminal utilities used by ClaudePane, TerminalPane, useClaudeSpawn,
 * and usePTY. Extracted verbatim to avoid duplication.
 */

import type { FitAddon } from '@xterm/addon-fit';
import type { Terminal } from '@xterm/xterm';

/** Ensure each CSS font-family name with spaces is quoted */
export function quoteFontFamily(ff: string): string {
  return ff
    .split(',')
    .map((f) => {
      f = f.trim();
      if (!f) return f;
      if (/^["']/.test(f) || /^(monospace|sans-serif|serif|cursive|fantasy|system-ui)$/i.test(f))
        return f;
      if (f.includes(' ')) return `"${f}"`;
      return f;
    })
    .join(', ');
}

/**
 * True when the element is actually laid out and on-screen. A container under a
 * `display:none` ancestor (e.g. a non-active agent's workspace) reports a 0×0
 * box, so this also rejects hidden panes. We use this to avoid fitting a
 * terminal while it's hidden — fitting a zero-size container collapses the grid
 * to its minimum and pushes that bogus size to the PTY, which then reflows
 * (garbles) when the pane is shown again.
 */
export function isTermVisible(el: HTMLElement | null): boolean {
  return !!el && el.clientWidth > 0 && el.clientHeight > 0;
}

/**
 * Fit the terminal addon, retrying via rAF and timeouts so the container
 * reaches its final size before the first fit call. When a container is given,
 * each attempt is skipped while that container is hidden.
 */
export function fitWithRetry(fitAddon: FitAddon, container?: HTMLElement | null): void {
  const fit = () => {
    if (container !== undefined && !isTermVisible(container)) return;
    try {
      fitAddon.fit();
    } catch {}
  };
  requestAnimationFrame(fit);
  setTimeout(fit, 100);
  setTimeout(fit, 300);
}

/**
 * Re-fit and force a full repaint after a pane becomes visible again. Switching
 * agents toggles a workspace between `display:none` and `block` without
 * unmounting its terminals, and xterm leaves a hidden terminal's rendered cells
 * untouched — so stale/garbled glyphs can linger until every row is rewritten.
 * Refitting (only while visible) then `refresh()`-ing every row clears that.
 */
export function refitAndRepaint(
  fitAddon: FitAddon | null,
  term: Terminal | null,
  container: HTMLElement | null,
): void {
  if (!fitAddon || !term || !isTermVisible(container)) return;
  try {
    fitAddon.fit();
    term.refresh(0, term.rows - 1);
  } catch {}
}

/**
 * Convert a binary string (as returned by the IPC output callback) to a
 * Uint8Array suitable for xterm's term.write().
 */
export function binaryStringToUint8Array(data: string): Uint8Array {
  const bytes = new Uint8Array(data.length);
  for (let i = 0; i < data.length; i++) {
    bytes[i] = data.charCodeAt(i);
  }
  return bytes;
}
