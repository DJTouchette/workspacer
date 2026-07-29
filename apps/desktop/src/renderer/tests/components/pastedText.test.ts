import { describe, it, expect } from 'vitest';
import {
  shouldCollapsePaste,
  pastePlaceholder,
  expandPastedText,
  pruneBlocks,
  referencedBlockIds,
  spliceAtSelection,
  PASTE_COLLAPSE_LINES,
  PASTE_COLLAPSE_CHARS,
} from '../../src/components/claude/pastedText';

/**
 * Long-paste collapsing. The contract that matters: what the user sees is a
 * short marker, what the agent receives is the full text — so expansion has to
 * be exact (no trimming, no re-wrapping) and has to survive the marker being
 * moved, duplicated, or typed around.
 */

const lines = (n: number) => Array.from({ length: n }, (_, i) => `line ${i}`).join('\n');

describe('shouldCollapsePaste', () => {
  it('leaves a short paste alone', () => {
    expect(shouldCollapsePaste('a quick note')).toBe(false);
    expect(shouldCollapsePaste(lines(PASTE_COLLAPSE_LINES - 1))).toBe(false);
  });

  it('collapses at the line threshold', () => {
    expect(shouldCollapsePaste(lines(PASTE_COLLAPSE_LINES))).toBe(true);
  });

  it('collapses a single enormous line (a minified blob has no newlines)', () => {
    expect(shouldCollapsePaste('x'.repeat(PASTE_COLLAPSE_CHARS))).toBe(true);
  });
});

describe('pastePlaceholder', () => {
  it('measures in lines for a multi-line paste', () => {
    expect(pastePlaceholder(1, lines(42))).toBe('[Pasted text #1 +42 lines]');
  });

  it('measures in characters when "+1 lines" would say nothing', () => {
    expect(pastePlaceholder(3, 'x'.repeat(2500))).toBe('[Pasted text #3 +2,500 chars]');
  });
});

describe('expandPastedText', () => {
  const blocks = () => new Map([[1, lines(9)]]);

  it('restores the held text byte for byte, in place', () => {
    const out = expandPastedText('before [Pasted text #1 +9 lines] after', blocks());
    expect(out).toBe(`before ${lines(9)} after`);
  });

  it('expands a marker the user moved or duplicated', () => {
    const out = expandPastedText('[Pasted text #1 +9 lines]\n[Pasted text #1 +9 lines]', blocks());
    expect(out).toBe(`${lines(9)}\n${lines(9)}`);
  });

  it('trusts the id, not the measure the user may have edited', () => {
    expect(expandPastedText('[Pasted text #1 +2 lines]', blocks())).toBe(lines(9));
  });

  it('leaves a marker-shaped string with no block alone rather than inventing one', () => {
    const typed = '[Pasted text #7 +5 lines]';
    expect(expandPastedText(typed, blocks())).toBe(typed);
  });

  it('is a no-op on ordinary text', () => {
    expect(expandPastedText('just a message', blocks())).toBe('just a message');
  });
});

describe('pruneBlocks', () => {
  it('forgets blocks whose marker left the composer, keeps the rest', () => {
    const held = new Map([
      [1, 'one'],
      [2, 'two'],
    ]);
    pruneBlocks('still here: [Pasted text #2 +3 lines]', held);
    expect([...held.keys()]).toEqual([2]);
  });

  it('clears everything when the composer is emptied (a send)', () => {
    const held = new Map([[1, 'one']]);
    pruneBlocks('', held);
    expect(held.size).toBe(0);
  });

  it('referencedBlockIds finds every marker', () => {
    expect(referencedBlockIds('[Pasted text #1 +2 lines] x [Pasted text #12 +9 lines]')).toEqual(
      new Set([1, 12]),
    );
  });
});

describe('spliceAtSelection', () => {
  it('inserts at the caret and reports where the caret lands', () => {
    expect(spliceAtSelection('hello world', 6, 6, 'brave ')).toEqual({
      value: 'hello brave world',
      caret: 12,
    });
  });

  it('replaces the selected range', () => {
    expect(spliceAtSelection('hello world', 6, 11, 'there')).toEqual({
      value: 'hello there',
      caret: 11,
    });
  });
});
