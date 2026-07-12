import { describe, expect, it } from 'vitest';
import { compactClaudeSnapshotForBackground } from './compactClaudeSnapshot';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';

function snapshot(overrides: Partial<ClaudeSessionSnapshot> = {}): ClaudeSessionSnapshot {
  return {
    sessionId: 's1',
    cwd: '/work',
    ptyId: 's1',
    status: 'active',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    pendingApproval: null,
    pendingQuestions: null,
    subagents: [],
    workflows: [],
    ambientState: 'idle',
    lastActivity: 1,
    totalToolCalls: 0,
    usage: null,
    ...overrides,
  };
}

describe('compactClaudeSnapshotForBackground', () => {
  it('keeps only recent conversation turns and truncates large text', () => {
    const longText = 'x'.repeat(6000);
    const compact = compactClaudeSnapshotForBackground(
      snapshot({
        conversation: Array.from({ length: 20 }, (_, i) => ({
          role: i % 2 === 0 ? 'user' : 'assistant',
          content: i === 19 ? longText : `turn ${i}`,
          timestamp: i,
        })),
      }),
    );

    expect(compact.conversation).toHaveLength(12);
    expect(compact.conversation[0].content).toBe('turn 8');
    expect(compact.conversation.at(-1)?.content.length).toBeLessThan(longText.length);
    expect(compact.conversation.at(-1)?.content).toContain('[truncated ');
  });

  it('bounds tool/file payloads without mutating the original snapshot', () => {
    const big = { content: 'a'.repeat(5000) };
    const original = snapshot({
      completedToolCalls: Array.from({ length: 25 }, (_, i) => ({
        id: `t${i}`,
        name: 'Write',
        input: i === 24 ? big : { ok: true },
        response: i === 24 ? big : undefined,
        status: 'complete',
        startedAt: i,
      })),
      fileChanges: Array.from({ length: 100 }, (_, i) => ({
        path: `file-${i}.ts`,
        toolName: 'Write',
        input: i === 99 ? big : { ok: true },
        timestamp: i,
      })),
      pendingApproval: {
        toolName: 'Write',
        toolInput: big,
        timestamp: 123,
      },
    });

    const compact = compactClaudeSnapshotForBackground(original);

    expect(compact.completedToolCalls).toHaveLength(20);
    expect(compact.completedToolCalls[0].id).toBe('t5');
    expect(compact.fileChanges).toHaveLength(80);
    expect(compact.fileChanges[0].path).toBe('file-20.ts');
    expect(compact.pendingApproval?.toolInput).toMatchObject({ __workspacerTruncated: true });
    expect(compact.completedToolCalls.at(-1)?.input).toMatchObject({
      __workspacerTruncated: true,
    });
    expect(original.completedToolCalls.at(-1)?.input).toBe(big);
    expect(original.fileChanges.at(-1)?.input).toBe(big);
  });
});
