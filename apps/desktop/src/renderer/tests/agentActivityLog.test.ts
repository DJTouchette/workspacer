import { describe, it, expect } from 'vitest';
import { collectRecentActivity } from '../src/lib/agentActivityLog';
import type { ClaudeSessionSnapshot, ToolCall } from '../src/types/claudeSession';

function tool(overrides: Partial<ToolCall>): ToolCall {
  return {
    id: 'tc-1',
    name: 'Bash',
    input: { command: 'ls' },
    status: 'complete',
    startedAt: 100,
    completedAt: 150,
    ...overrides,
  };
}

function snap(overrides: Partial<ClaudeSessionSnapshot>): ClaudeSessionSnapshot {
  return {
    sessionId: 's1',
    cwd: '/w',
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
    ambientState: 'streaming',
    lastActivity: 0,
    totalToolCalls: 0,
    usage: null,
    ...overrides,
  } as ClaudeSessionSnapshot;
}

describe('collectRecentActivity', () => {
  it('returns empty for a missing snapshot', () => {
    expect(collectRecentActivity(undefined)).toEqual([]);
  });

  it('merges tool calls and assistant messages in time order', () => {
    const s = snap({
      completedToolCalls: [tool({ id: 'a', startedAt: 10, completedAt: 20 })],
      activeToolCalls: [
        tool({
          id: 'b',
          name: 'Read',
          input: { file_path: '/x/y.ts' },
          status: 'running',
          startedAt: 40,
          completedAt: undefined,
        }),
      ],
      conversation: [{ role: 'assistant', content: 'Looking at the config first.', timestamp: 30 }],
    });
    const lines = collectRecentActivity(s, 3);
    expect(lines.map((l) => l.kind)).toEqual(['tool', 'message', 'tool-running']);
    expect(lines[1].text).toBe('Looking at the config first.');
    expect(lines[2].text).toContain('Read');
  });

  it('keeps only the last `max` entries, newest last', () => {
    const s = snap({
      conversation: [
        { role: 'assistant', content: 'one', timestamp: 1 },
        { role: 'assistant', content: 'two', timestamp: 2 },
        { role: 'assistant', content: 'three', timestamp: 3 },
        { role: 'assistant', content: 'four', timestamp: 4 },
      ],
    });
    expect(collectRecentActivity(s, 3).map((l) => l.text)).toEqual(['two', 'three', 'four']);
  });

  it('recovers tool calls from conversation turns after Stop cleared the hook lists', () => {
    // applyStopEvent empties active/completedToolCalls at turn end — the
    // transcript-derived copies on the turns are then the only tool history.
    const s = snap({
      ambientState: 'idle',
      conversation: [
        {
          role: 'assistant',
          content: '',
          timestamp: 50,
          toolCalls: [
            tool({
              id: 'kept',
              name: 'Grep',
              input: { pattern: 'foo' },
              startedAt: 45,
              completedAt: 50,
            }),
          ],
        },
        { role: 'assistant', content: 'Done — found it.', timestamp: 60 },
      ],
    });
    const lines = collectRecentActivity(s, 3);
    expect(lines.map((l) => l.kind)).toEqual(['tool', 'message']);
    expect(lines[0].text).toContain('Search'); // formatToolSummary renders Grep as Search(…)
  });

  it('dedups the same tool id across hook lists and conversation turns', () => {
    const shared = tool({ id: 'dup', startedAt: 10, completedAt: 20 });
    const s = snap({
      completedToolCalls: [shared],
      conversation: [{ role: 'assistant', content: '', timestamp: 20, toolCalls: [{ ...shared }] }],
    });
    expect(collectRecentActivity(s, 5).filter((l) => l.kind === 'tool')).toHaveLength(1);
  });

  it('skips slash-command turns and empty assistant messages', () => {
    const s = snap({
      conversation: [
        { role: 'assistant', content: 'ran the linter', timestamp: 1, command: { name: 'lint' } },
        { role: 'assistant', content: '   ', timestamp: 2 },
        { role: 'user', content: 'do the thing', timestamp: 3 },
        { role: 'assistant', content: '## Header\nbody', timestamp: 4 },
      ],
    });
    const lines = collectRecentActivity(s, 5);
    expect(lines).toHaveLength(1);
    expect(lines[0].text).toBe('Header');
  });
});
