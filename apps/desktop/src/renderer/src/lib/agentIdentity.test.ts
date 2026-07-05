import { describe, it, expect } from 'vitest';
import { agentIdForSession, dedupeBySessionId } from './agentIdentity';
import type { AgentWorkspace } from '../types/pane';

const card = (id: string, sessionId?: string): AgentWorkspace => ({
  id,
  name: id,
  cwd: '/x',
  sessionId,
  tabs: [],
  activeTabId: '',
});

describe('agentIdForSession', () => {
  it('is deterministic per session', () => {
    expect(agentIdForSession('abc')).toBe('agent-abc');
    expect(agentIdForSession('abc')).toBe(agentIdForSession('abc'));
  });
});

describe('dedupeBySessionId', () => {
  it('keeps a single card per session, dropping duplicates', () => {
    const out = dedupeBySessionId([
      card('agent-S1', 'S1'),
      card('agent-1700-9', 'S1'), // a divergent id another client minted for S1
      card('agent-S2', 'S2'),
    ]);
    expect(out.map((a) => a.sessionId)).toEqual(['S1', 'S2']);
  });

  it('chooses the same survivor regardless of order (deterministic)', () => {
    const a = card('agent-bbb', 'S1');
    const b = card('agent-aaa', 'S1'); // lexicographically smaller
    expect(dedupeBySessionId([a, b])[0].id).toBe('agent-aaa');
    expect(dedupeBySessionId([b, a])[0].id).toBe('agent-aaa');
  });

  it('preserves first-occurrence position of the survivor', () => {
    const out = dedupeBySessionId([
      card('agent-S2', 'S2'),
      card('agent-zzz', 'S1'),
      card('agent-aaa', 'S1'),
    ]);
    expect(out.map((a) => a.sessionId)).toEqual(['S2', 'S1']);
    expect(out[1].id).toBe('agent-aaa'); // smaller id wins, placed where S1 first appeared
  });

  it('never collapses cards without a sessionId (stopped/local agents)', () => {
    const out = dedupeBySessionId([
      card('overview'), // global, no session
      card('stopped-a'), // stopped, no session
      card('stopped-b'),
    ]);
    expect(out).toHaveLength(3);
  });

  it('leaves a clean single-session list untouched', () => {
    const list = [card('overview'), card('agent-S1', 'S1')];
    expect(dedupeBySessionId(list)).toEqual(list);
  });
});
