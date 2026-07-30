import { describe, it, expect } from 'vitest';
import {
  shouldCollapsePaste,
  pastePlaceholder,
  expandPastedText,
  holdBlock,
  releaseBlocks,
  referencedBlockIds,
  spliceAtSelection,
  PASTE_COLLAPSE_LINES,
  PASTE_COLLAPSE_CHARS,
  MAX_HELD_BLOCKS,
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

describe('holding and releasing blocks', () => {
  it('keeps a block whose marker is not currently in the text', () => {
    // The critical property: mid-edit the composer passes through states where
    // the marker does not parse (delete the ']', cut it to move it). Dropping
    // the block at that instant would destroy the paste — the map is the only
    // copy — and the repaired marker would then send as a literal string.
    const held = new Map<number, string>();
    holdBlock(held, 1, 'the payload');
    expect(expandPastedText('[Pasted text #1 +9 lines', held)).toBe('[Pasted text #1 +9 lines');
    // ...and once the marker is whole again it still expands.
    expect(expandPastedText('[Pasted text #1 +9 lines]', held)).toBe('the payload');
  });

  it('releases only the ids a delivered message carried', () => {
    const held = new Map<number, string>();
    holdBlock(held, 1, 'sent');
    holdBlock(held, 2, 'still drafting');
    releaseBlocks(held, referencedBlockIds('[Pasted text #1 +2 lines]'));
    expect([...held.keys()]).toEqual([2]);
  });

  it('evicts oldest-first past the block cap, never the newest', () => {
    const held = new Map<number, string>();
    for (let i = 1; i <= MAX_HELD_BLOCKS + 3; i++) holdBlock(held, i, `block ${i}`);
    expect(held.size).toBeLessThanOrEqual(MAX_HELD_BLOCKS);
    expect(held.has(MAX_HELD_BLOCKS + 3)).toBe(true); // the one in use
    expect(held.has(1)).toBe(false); // the oldest went first
  });

  it('holds a single oversized paste rather than evicting it to satisfy the budget', () => {
    const held = new Map<number, string>();
    holdBlock(held, 1, 'x'.repeat(9_000_000)); // over MAX_HELD_CHARS on its own
    expect(held.get(1)).toHaveLength(9_000_000);
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
