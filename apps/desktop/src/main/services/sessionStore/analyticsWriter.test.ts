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

  it('records a single per-model slice keyed by statusLine.modelDisplay when usage.models is absent', () => {
    recordModelsMock.mockClear();
    writeHistory(mkManagedSession(), 'active');

    expect(recordModelsMock).toHaveBeenCalledTimes(1);
    expect(recordModelsMock).toHaveBeenCalledWith('codex-1', {
      'gpt-5-codex': { inputTokens: 12_000, outputTokens: 3_400, costUSD: 0.4 },
    });
  });

  it('does not call recordModels when there is no model to key the slice by', () => {
    recordModelsMock.mockClear();
    const session = mkManagedSession();
    session.statusLine!.modelDisplay = undefined;
    writeHistory(session, 'active');

    expect(recordModelsMock).not.toHaveBeenCalled();
  });

  it('still prefers usage.models over the statusLine fallback for Claude sessions', () => {
    recordModelsMock.mockClear();
    const session = mkManagedSession();
    session.provider = 'claude';
    session.usage = {
      model: 'claude-opus-4-1',
      contextTokens: 0,
      contextLimit: 0,
      totalInputTokens: 0,
      totalOutputTokens: 0,
      costUSD: 0,
      models: { 'claude-opus-4-1': { inputTokens: 500, outputTokens: 100, costUSD: 0.02 } },
    } as never;
    writeHistory(session, 'active');

    expect(recordModelsMock).toHaveBeenCalledTimes(1);
    expect(recordModelsMock).toHaveBeenCalledWith('codex-1', {
      'claude-opus-4-1': { inputTokens: 500, outputTokens: 100, costUSD: 0.02 },
    });
  });
});
