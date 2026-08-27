import { beforeEach, describe, expect, it, vi } from 'vitest';

const getSnapshot = vi.fn();
const getSubagentConversation = vi.fn();

vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: {
    getSnapshot: (...args: unknown[]) => getSnapshot(...args),
  },
}));

vi.mock('./claudemonSessionClient', () => ({
  claudemonSessionClient: {
    getSubagentConversation: (...args: unknown[]) => getSubagentConversation(...args),
  },
}));

const { readProviderSubagentConversation, readProviderSubagentTranscript } =
  await import('./providerSubagentConversation');

function codexSnapshot(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    sessionId: 'parent-1',
    cwd: '/repo',
    status: 'active',
    ambientState: 'background',
    startedAt: 1000,
    lastActivity: 2000,
    provider: 'codex',
    subagents: [{ id: 'child-1', type: 'codex', status: 'running', startedAt: 1000 }],
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('provider subagent conversation', () => {
  it('folds Codex child-thread items into renderer conversation turns', async () => {
    getSnapshot.mockReturnValue(codexSnapshot());
    getSubagentConversation.mockResolvedValue({
      seq: 4,
      items: [
        { kind: 'user_message', text: 'inspect this', timestamp: '2026-08-26T00:00:00Z' },
        {
          kind: 'tool_use',
          id: 'call-1',
          name: 'exec_command',
          input: { cmd: 'ls' },
          timestamp: '2026-08-26T00:00:01Z',
        },
        {
          kind: 'tool_result',
          tool_use_id: 'call-1',
          content: 'ok',
          is_error: false,
          timestamp: '2026-08-26T00:00:02Z',
        },
        { kind: 'assistant_text', text: 'done', timestamp: '2026-08-26T00:00:03Z' },
      ],
    });

    const turns = await readProviderSubagentConversation('parent-1', 'child-1');

    expect(getSubagentConversation).toHaveBeenCalledWith('parent-1', 'child-1');
    expect(turns).toHaveLength(3);
    expect(turns?.[0]).toMatchObject({ role: 'user', content: 'inspect this' });
    expect(turns?.[1].toolCalls?.[0]).toMatchObject({
      id: 'call-1',
      name: 'exec_command',
      response: 'ok',
      status: 'complete',
    });
    expect(turns?.[2]).toMatchObject({ role: 'assistant', content: 'done' });

    const raw = await readProviderSubagentTranscript('parent-1', 'child-1');
    expect(raw).toEqual([
      { role: 'user', text: 'inspect this' },
      { role: 'assistant', text: '⚙ exec_command\n↳ ok' },
      { role: 'assistant', text: 'done' },
    ]);
  });

  it('refuses non-Codex sessions and unknown child ids before asking the daemon', async () => {
    getSnapshot.mockReturnValue(codexSnapshot({ provider: 'claude' }));
    await expect(readProviderSubagentConversation('parent-1', 'child-1')).resolves.toBeNull();

    getSnapshot.mockReturnValue(codexSnapshot());
    await expect(readProviderSubagentConversation('parent-1', 'other-child')).resolves.toBeNull();

    expect(getSubagentConversation).not.toHaveBeenCalled();
  });
});
