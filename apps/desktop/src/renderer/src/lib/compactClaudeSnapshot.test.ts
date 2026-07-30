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

  describe('memoization', () => {
    // This runs on every session IPC tick — up to ~60/s per streaming session —
    // so re-deriving an entry that was final hundreds of ticks ago is the bulk
    // of the cost. Snapshots arrive over IPC with fresh object identities each
    // tick, so the memo keys off the entries' own immutable identity.
    const settledTool = (id: string) => ({
      id,
      name: 'Read',
      input: { path: 'a.ts' },
      response: 'contents',
      status: 'complete' as const,
      startedAt: 1,
      completedAt: 2,
    });

    it('reuses the compacted form of a settled tool call across ticks', () => {
      const first = compactClaudeSnapshotForBackground(
        snapshot({ completedToolCalls: [settledTool('memo-1')] }),
      );
      // Structurally identical, freshly-allocated — as an IPC clone would be.
      const second = compactClaudeSnapshotForBackground(
        snapshot({ completedToolCalls: [settledTool('memo-1')] }),
      );
      expect(second.completedToolCalls[0]).toBe(first.completedToolCalls[0]);
    });

    it('recomputes when a tool call settles into a new state', () => {
      const running = compactClaudeSnapshotForBackground(
        snapshot({
          activeToolCalls: [
            { ...settledTool('memo-2'), status: 'running', completedAt: undefined },
          ],
        }),
      );
      const done = compactClaudeSnapshotForBackground(
        snapshot({ completedToolCalls: [settledTool('memo-2')] }),
      );
      expect(done.completedToolCalls[0]).not.toBe(running.activeToolCalls[0]);
      expect(done.completedToolCalls[0].status).toBe('complete');
    });

    it('never memoizes a running tool call, whose response is still filling in', () => {
      const key = { ...settledTool('memo-3'), status: 'running' as const, completedAt: undefined };
      const early = compactClaudeSnapshotForBackground(
        snapshot({ activeToolCalls: [{ ...key, response: undefined }] }),
      );
      const later = compactClaudeSnapshotForBackground(
        snapshot({ activeToolCalls: [{ ...key, response: 'partial output' }] }),
      );
      expect(early.activeToolCalls[0].response).toBeUndefined();
      expect(later.activeToolCalls[0].response).toBe('partial output');
    });

    it('reuses the compacted form of an already-recorded file change', () => {
      const change = { path: 'x.ts', toolName: 'Write', input: { ok: true }, timestamp: 7 };
      const first = compactClaudeSnapshotForBackground(snapshot({ fileChanges: [{ ...change }] }));
      const second = compactClaudeSnapshotForBackground(snapshot({ fileChanges: [{ ...change }] }));
      expect(second.fileChanges[0]).toBe(first.fileChanges[0]);
    });

    it('distinguishes file changes that differ only by path', () => {
      const base = { toolName: 'Write', input: { ok: true }, timestamp: 9 };
      const a = compactClaudeSnapshotForBackground(
        snapshot({ fileChanges: [{ ...base, path: 'a.ts' }] }),
      );
      const b = compactClaudeSnapshotForBackground(
        snapshot({ fileChanges: [{ ...base, path: 'b.ts' }] }),
      );
      expect(b.fileChanges[0].path).toBe('b.ts');
      expect(a.fileChanges[0].path).toBe('a.ts');
    });
  });
});
