import { describe, it, expect } from 'vitest';
import { KIND_PRIORITY, sortItems, agentAttentionScore } from '../src/lib/attentionRouter';
import type { AttentionItem } from '../src/types/attention';

function item(partial: Partial<AttentionItem>): AttentionItem {
  return {
    id: 'x',
    agentId: 'a',
    agentName: 'A',
    sessionId: 's',
    kind: 'done',
    priority: KIND_PRIORITY.done,
    createdAt: 0,
    status: 'open',
    title: 't',
    payload: { type: 'summary', summary: '' },
    signature: 'sig',
    ...partial,
  };
}

describe('attentionRouter', () => {
  it('orders by priority desc, then oldest first within a tier', () => {
    const approvalOld = item({
      kind: 'approval',
      priority: 100,
      createdAt: 100,
      signature: 'a-old',
    });
    const approvalNew = item({
      kind: 'approval',
      priority: 100,
      createdAt: 200,
      signature: 'a-new',
    });
    const done = item({ kind: 'done', priority: 20, createdAt: 50, signature: 'd' });
    const sorted = sortItems([done, approvalNew, approvalOld]);
    expect(sorted.map((i) => i.signature)).toEqual(['a-old', 'a-new', 'd']);
  });

  it('keeps blocking kinds above merely-happening kinds', () => {
    expect(KIND_PRIORITY.approval).toBeGreaterThan(KIND_PRIORITY.done);
    expect(KIND_PRIORITY.question).toBeGreaterThan(KIND_PRIORITY.bigdiff);
  });

  it('agentAttentionScore: an open item buoys above any bare ambient state', () => {
    const withItem = agentAttentionScore('idle', KIND_PRIORITY.approval);
    const workingNoItem = agentAttentionScore('streaming', 0);
    expect(withItem).toBeGreaterThan(workingNoItem);
  });

  it('agentAttentionScore: blocked > working > idle > stopped', () => {
    const blocked = agentAttentionScore('waiting_approval', 0);
    const working = agentAttentionScore('streaming', 0);
    const idle = agentAttentionScore('idle', 0);
    const stopped = agentAttentionScore(undefined, 0);
    expect(blocked).toBeGreaterThan(working);
    expect(working).toBeGreaterThan(idle);
    expect(idle).toBeGreaterThan(stopped);
  });
});
