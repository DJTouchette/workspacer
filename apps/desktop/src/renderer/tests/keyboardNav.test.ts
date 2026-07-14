import { describe, it, expect } from 'vitest';
import { comboMatcher } from '../src/hooks/useKeyboardNav';

// jsdom is not macOS, so the `mod` token resolves to Ctrl here.
const ev = (over: Partial<KeyboardEvent>): KeyboardEvent =>
  ({
    key: 'a',
    code: '',
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    ...over,
  }) as KeyboardEvent;

describe('comboMatcher — mod token resolution', () => {
  it('a `mod+shift+n` binding fires on Ctrl+Shift+N (not the bare Shift+N)', () => {
    const match = comboMatcher('mod+shift+n');
    // The real combo the UI advertises.
    expect(match(ev({ key: 'N', ctrlKey: true, shiftKey: true }))).toBe(true);
    // Regression: it must NOT fire on Shift+N with no Ctrl — the pre-fix behaviour.
    expect(match(ev({ key: 'N', ctrlKey: false, shiftKey: true }))).toBe(false);
  });

  it('resolves `mod+p` to Ctrl+P and rejects a stray modifier', () => {
    const match = comboMatcher('mod+p');
    expect(match(ev({ key: 'p', ctrlKey: true }))).toBe(true);
    expect(match(ev({ key: 'p', ctrlKey: true, shiftKey: true }))).toBe(false); // exact modifiers
    expect(match(ev({ key: 'p', metaKey: true }))).toBe(false); // Cmd is not Ctrl off-mac
  });

  it('leaves literal combos and chord steps untouched', () => {
    expect(comboMatcher('ctrl+tab')(ev({ key: 'Tab', ctrlKey: true }))).toBe(true);
    // A bare chord step (single key, no modifiers).
    expect(comboMatcher('t')(ev({ key: 't' }))).toBe(true);
    expect(comboMatcher('t')(ev({ key: 't', ctrlKey: true }))).toBe(false);
  });
});
