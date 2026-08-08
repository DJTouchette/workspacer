import { describe, it, expect } from 'vitest';
import { bracketedPasteSubmit } from './bracketedPaste';

describe('bracketedPasteSubmit — the raw-PTY fallback frame', () => {
  it('wraps a plain body in one paste frame with a trailing submit CR', () => {
    expect(bracketedPasteSubmit('hi there')).toBe('\x1b[200~hi there\x1b[201~\r');
  });

  it('trims a trailing newline run so the appended CR is the sole submit', () => {
    expect(bracketedPasteSubmit('line1\nline2\n\n')).toBe('\x1b[200~line1\nline2\x1b[201~\r');
  });

  it('neutralizes an embedded end marker so the body cannot break out of paste mode', () => {
    // Attacker-authored content forging its own ESC[201~ + CR + command.
    const out = bracketedPasteSubmit('hello\x1b[201~\rrm -rf ~');
    const inner = out.slice('\x1b[200~'.length, out.length - '\x1b[201~\r'.length);
    // No raw ESC survives in the body, so no forged paste marker can exist there.
    expect(inner.includes('\x1b')).toBe(false);
    // The ESC is neutralized to its visible glyph rather than dropped.
    expect(inner.includes('␛')).toBe(true);
    // Exactly one opening + one closing marker: the frame's own.
    expect(out.match(/\x1b\[20[01]~/g)).toEqual(['\x1b[200~', '\x1b[201~']);
  });
});
