/**
 * Shared terminal utilities used by ClaudePane, TerminalPane, useClaudeSpawn,
 * and usePTY. Extracted verbatim to avoid duplication.
 */

import type { FitAddon } from '@xterm/addon-fit';

/** Ensure each CSS font-family name with spaces is quoted */
export function quoteFontFamily(ff: string): string {
  return ff.split(',').map(f => {
    f = f.trim();
    if (!f) return f;
    if (/^["']/.test(f) || /^(monospace|sans-serif|serif|cursive|fantasy|system-ui)$/i.test(f)) return f;
    if (f.includes(' ')) return `"${f}"`;
    return f;
  }).join(', ');
}

/**
 * Fit the terminal addon, retrying via rAF and timeouts so the container
 * reaches its final size before the first fit call.
 */
export function fitWithRetry(fitAddon: FitAddon): void {
  const fit = () => { try { fitAddon.fit(); } catch {} };
  requestAnimationFrame(fit);
  setTimeout(fit, 100);
  setTimeout(fit, 300);
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
