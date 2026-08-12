/**
 * Tests for mergeConversationWindow — the client-side half of pushing bounded
 * snapshots on the bus.
 *
 * The host stopped publishing every session's whole transcript on every flush
 * and now publishes the newest N turns anchored by `conversationOffset`. This
 * function is what lets a pane holding full history splice that window in. Get
 * it wrong and the actively-watched transcript renders incorrectly, so the
 * cases below are the failure modes, not the happy path.
 */

import { describe, it, expect } from 'vitest';
import { mergeConversationWindow } from './mergeConversationWindow';

/** Turns as bare labels — the merge is index arithmetic, not content. */
const turns = (...labels: string[]) => labels;

describe('mergeConversationWindow', () => {
  it('adopts the window when nothing is retained yet', () => {
    const out = mergeConversationWindow(null, {
      conversation: turns('c', 'd'),
      conversationOffset: 2,
    });
    expect(out).toEqual({ kind: 'adopt', conversation: ['c', 'd'], conversationOffset: 2 });
  });

  // The ordinary streaming case: history of 5, window covers the last 2 and
  // adds a 6th.
  it('splices an overlapping window onto retained history', () => {
    const out = mergeConversationWindow(
      { conversation: turns('a', 'b', 'c', 'd', 'e'), conversationOffset: 0 },
      { conversation: turns('d', 'e', 'f'), conversationOffset: 3 },
    );
    expect(out).toEqual({
      kind: 'merged',
      conversation: ['a', 'b', 'c', 'd', 'e', 'f'],
      conversationOffset: 0,
    });
  });

  // The window's copies win on the overlap: a turn's content grows while it
  // streams, so the newer push is the more complete one.
  it('prefers the window copy of overlapping turns', () => {
    const out = mergeConversationWindow(
      { conversation: turns('a', 'partial'), conversationOffset: 0 },
      { conversation: turns('complete', 'next'), conversationOffset: 1 },
    );
    expect(out).toMatchObject({ conversation: ['a', 'complete', 'next'] });
  });

  it('handles a window that exactly adjoins the retained tail', () => {
    const out = mergeConversationWindow(
      { conversation: turns('a', 'b'), conversationOffset: 0 },
      { conversation: turns('c'), conversationOffset: 2 },
    );
    expect(out).toEqual({ kind: 'merged', conversation: ['a', 'b', 'c'], conversationOffset: 0 });
  });

  // Turns went missing between flushes. Concatenating would splice 'a','b'
  // straight onto 'f' and render a transcript that never happened.
  it('reports a gap instead of concatenating across missing turns', () => {
    const out = mergeConversationWindow(
      { conversation: turns('a', 'b'), conversationOffset: 0 },
      { conversation: turns('f', 'g'), conversationOffset: 5 },
    );
    expect(out).toEqual({ kind: 'gap' });
  });

  // The host rebuilt the transcript (resetConversationOffsetIfRebuilt): offset
  // returns to 0 and the replay is authoritative.
  it('adopts a rebuilt transcript that replays from the top', () => {
    const out = mergeConversationWindow(
      { conversation: turns('a', 'b', 'c'), conversationOffset: 4 },
      { conversation: turns('x', 'y', 'z', 'w'), conversationOffset: 0 },
    );
    expect(out).toEqual({
      kind: 'adopt',
      conversation: ['x', 'y', 'z', 'w'],
      conversationOffset: 0,
    });
  });

  // An out-of-order push must not delete turns the user can already see.
  it('ignores a stale window that ends before our history does', () => {
    const out = mergeConversationWindow(
      { conversation: turns('a', 'b', 'c', 'd'), conversationOffset: 0 },
      { conversation: turns('b', 'c'), conversationOffset: 1 },
    );
    expect(out).toEqual({ kind: 'stale' });
  });

  it('ignores a late push that adds nothing new', () => {
    const out = mergeConversationWindow(
      { conversation: turns('a', 'b', 'c'), conversationOffset: 0 },
      { conversation: turns('c'), conversationOffset: 2 },
    );
    expect(out).toEqual({ kind: 'stale' });
  });

  // A trimmed host history (capConversationInPlace banked 2000 turns) still
  // anchors correctly — the arithmetic is absolute, never array-relative.
  it('merges correctly when both sides are far from index 0', () => {
    const out = mergeConversationWindow(
      { conversation: turns('p', 'q', 'r'), conversationOffset: 2000 },
      { conversation: turns('r', 's'), conversationOffset: 2002 },
    );
    expect(out).toEqual({
      kind: 'merged',
      conversation: ['p', 'q', 'r', 's'],
      conversationOffset: 2000,
    });
  });

  it('treats a missing offset as absolute zero on both sides', () => {
    const out = mergeConversationWindow(
      { conversation: turns('a') },
      { conversation: turns('a', 'b') },
    );
    expect(out).toEqual({ kind: 'adopt', conversation: ['a', 'b'], conversationOffset: 0 });
  });

  it('adopts an empty retained conversation rather than reporting a gap', () => {
    const out = mergeConversationWindow(
      { conversation: [], conversationOffset: 0 },
      { conversation: turns('a'), conversationOffset: 0 },
    );
    expect(out).toMatchObject({ conversation: ['a'] });
  });
});
