import { describe, it, expect, vi } from 'vitest';

// Capture what writeHistory persists without touching the real SQLite store.
const recordMock = vi.fn();
const recordModelsMock = vi.fn();
vi.mock('../sessionHistory', () => ({
  sessionHistory: {
    record: (...a: unknown[]) => recordMock(...a),
    recordModels: (...a: unknown[]) => recordModelsMock(...a),
  },
}));

import { writeHistory } from './analyticsWriter';
import type { ClaudeSessionState } from '../claudeSessionStore';

/** A managed-provider (Codex) session: usage is never populated for these —
 *  their cost/tokens/model live only on statusLine. */
function mkManagedSession(): ClaudeSessionState {
  return {
    sessionId: 'codex-1',
    cwd: '',
    provider: 'codex',
    usage: null,
    statusLine: {
      modelDisplay: 'gpt-5-codex',
      costUSD: 0.4,
      totalInputTokens: 12_000,
      totalOutputTokens: 3_400,
    },
    startedAt: Date.now() - 1000,
    peakContext: 0,
    totalToolCalls: 0,
    conversation: [],
    subagents: [],
    workflows: [],
  } as unknown as ClaudeSessionState;
}

describe('writeHistory — managed provider (Codex) falls back to statusLine', () => {
  it('records statusLine cost/tokens/model when session.usage is null', () => {
    recordMock.mockClear();
    writeHistory(mkManagedSession(), 'active');

    expect(recordMock).toHaveBeenCalledTimes(1);
    const rec = recordMock.mock.calls[0][0] as {
      costUSD: number;
      inputTokens: number;
      outputTokens: number;
      model: string;
    };
    expect(rec.costUSD).toBe(0.4);
    expect(rec.inputTokens).toBe(12_000);
    expect(rec.outputTokens).toBe(3_400);
    expect(rec.model).toBe('gpt-5-codex');
  });
});
