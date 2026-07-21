/**
 * conversationOffset must record how many turns background compaction dropped
 * from the FRONT, accumulating across repeated compaction. ClaudePane keys and
 * anchors turns by GLOBAL index (offset + array index); if the offset ever
 * disagreed with the true drop count, every conversation key would renumber on
 * the compact↔full swap an agent switch performs, remounting the whole
 * transcript (the stream-mode switch flicker).
 */
import { describe, it, expect } from 'vitest';
import { compactClaudeSnapshotForBackground } from '../src/lib/compactClaudeSnapshot';
import type { ClaudeSessionSnapshot, ConversationTurn } from '../src/types/claudeSession';

const turn = (i: number): ConversationTurn => ({
  role: i % 2 === 0 ? 'user' : 'assistant',
  content: `turn ${i}`,
  timestamp: 1000 + i,
});

const snap = (turns: number, offset?: number): ClaudeSessionSnapshot =>
  ({
    sessionId: 'S1',
    cwd: '/x',
    ptyId: 'p',
    status: 'active',
    conversation: Array.from({ length: turns }, (_, i) => turn(i)),
    conversationOffset: offset,
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
  }) as unknown as ClaudeSessionSnapshot;

describe('compactClaudeSnapshotForBackground — conversationOffset', () => {
  it('records the dropped-turn count so global indices stay stable', () => {
    const out = compactClaudeSnapshotForBackground(snap(50));
    expect(out.conversation.length).toBe(12);
    expect(out.conversationOffset).toBe(38);
    // Global index of the first kept turn: offset + 0 must point at turn 38.
    expect(out.conversation[0].content).toBe('turn 38');
  });

  it('keeps offset 0 when nothing is dropped', () => {
    const out = compactClaudeSnapshotForBackground(snap(5));
    expect(out.conversation.length).toBe(5);
    expect(out.conversationOffset).toBe(0);
  });

  it('accumulates across repeated compaction instead of resetting', () => {
    // A hidden pane compacts its own already-compact state (deactivation
    // compacts prev, then compact update flushes land on top).
    const once = compactClaudeSnapshotForBackground(snap(50));
    const twice = compactClaudeSnapshotForBackground(once);
    expect(twice.conversationOffset).toBe(38);
    // And compacting a compact snapshot that grew a tail keeps global indices
    // aligned: 12 kept + new turns, re-trimmed to 12.
    const grown = {
      ...once,
      conversation: [...once.conversation, turn(50), turn(51)],
    } as ClaudeSessionSnapshot;
    const rec = compactClaudeSnapshotForBackground(grown);
    expect(rec.conversation.length).toBe(12);
    expect(rec.conversationOffset).toBe(40);
    expect(rec.conversation[rec.conversation.length - 1].content).toBe('turn 51');
  });
});
