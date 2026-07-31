import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import {
  useAgentAutoTitle,
  openingExchange,
  agentsAwaitingTitle,
} from '../src/hooks/useAgentAutoTitle';
import type { AgentWorkspace } from '../src/types/pane';
import type { ClaudeSessionSnapshot, ConversationTurn } from '../src/types/claudeSession';

/**
 * Auto-titling decides WHEN an agent gets named; the title text itself is the
 * main process's problem (services/agentTitler). What matters here is that it
 * fires once, only for agents nobody has named, and only once the opening
 * exchange actually has an answer in it.
 */

function agent(over: Partial<AgentWorkspace> = {}): AgentWorkspace {
  return {
    id: 'agent-s1',
    name: 'workspacer',
    cwd: '/repo',
    sessionId: 's1',
    tabs: [],
    activeTabId: '',
    ...over,
  };
}

function snap(conversation: ConversationTurn[]): Record<string, ClaudeSessionSnapshot> {
  return { s1: { conversation } as ClaudeSessionSnapshot };
}

const EXCHANGE: ConversationTurn[] = [
  { role: 'user', content: 'fix the flaky sidebar test' },
  { role: 'assistant', content: 'Looking at the suite now' },
];

describe('openingExchange', () => {
  it('pairs the first real user message with the first assistant reply', () => {
    expect(openingExchange(EXCHANGE)).toEqual({
      userMessage: 'fix the flaky sidebar test',
      assistantReply: 'Looking at the suite now',
    });
  });

  it('waits for an answer — a question with no reply is not an exchange', () => {
    expect(openingExchange([{ role: 'user', content: 'fix it' }])).toBeNull();
  });

  it('skips slash-command turns, which say nothing about the work', () => {
    const turns: ConversationTurn[] = [
      { role: 'user', content: '/model sonnet', command: { name: 'model' } } as ConversationTurn,
      { role: 'assistant', content: 'Model set' },
      { role: 'user', content: 'now refactor the parser' },
      { role: 'assistant', content: 'On it' },
    ];
    expect(openingExchange(turns)?.userMessage).toBe('now refactor the parser');
  });

  it('ignores an empty conversation', () => {
    expect(openingExchange([])).toBeNull();
    expect(openingExchange(undefined)).toBeNull();
  });

  it('tolerates an assistant turn with no text (tool-only)', () => {
    const turns: ConversationTurn[] = [
      { role: 'user', content: 'fix it' },
      { role: 'assistant', content: '' },
      { role: 'assistant', content: 'Found the bug' },
    ];
    expect(openingExchange(turns)?.assistantReply).toBe('Found the bug');
  });
});

describe('agentsAwaitingTitle', () => {
  it('picks up a live, unnamed, untitled agent', () => {
    expect(agentsAwaitingTitle([agent()], snap(EXCHANGE))).toHaveLength(1);
  });

  it('leaves a name the user typed alone', () => {
    expect(agentsAwaitingTitle([agent({ nameSetByUser: true })], snap(EXCHANGE))).toHaveLength(0);
  });

  it('never titles the same agent twice', () => {
    expect(agentsAwaitingTitle([agent({ autoTitled: true })], snap(EXCHANGE))).toHaveLength(0);
  });

  it('skips the Overview workspace and stopped agents', () => {
    expect(agentsAwaitingTitle([agent({ global: true })], snap(EXCHANGE))).toHaveLength(0);
    expect(agentsAwaitingTitle([agent({ sessionId: undefined })], snap(EXCHANGE))).toHaveLength(0);
  });
});

describe('useAgentAutoTitle', () => {
  const suggest = vi.fn();

  beforeEach(() => {
    suggest.mockReset();
    suggest.mockResolvedValue('Fix the flaky sidebar test');
    (window.electronAPI as any).agentSuggestTitle = suggest;
  });

  it('asks for a title once the first exchange has an answer', async () => {
    const onTitle = vi.fn();
    renderHook(() =>
      useAgentAutoTitle({
        agents: [agent()],
        snapshotBySession: snap(EXCHANGE),
        enabled: true,
        onTitle,
      }),
    );
    await waitFor(() =>
      expect(onTitle).toHaveBeenCalledWith('agent-s1', 'Fix the flaky sidebar test'),
    );
    expect(suggest).toHaveBeenCalledWith({
      userMessage: 'fix the flaky sidebar test',
      assistantReply: 'Looking at the suite now',
    });
  });

  it('does not fire twice while the first call is still in flight', async () => {
    let resolve!: (v: string) => void;
    suggest.mockReturnValue(new Promise<string>((r) => (resolve = r)));
    const props = {
      agents: [agent()],
      snapshotBySession: snap(EXCHANGE),
      enabled: true,
      onTitle: vi.fn(),
    };
    const { rerender } = renderHook((p: typeof props) => useAgentAutoTitle(p), {
      initialProps: props,
    });
    // A stream of snapshots arrives while the round-trip is outstanding.
    rerender({ ...props, snapshotBySession: snap([...EXCHANGE]) });
    rerender({ ...props, snapshotBySession: snap([...EXCHANGE]) });
    expect(suggest).toHaveBeenCalledTimes(1);
    resolve('Fix it');
    await waitFor(() => expect(props.onTitle).toHaveBeenCalled());
  });

  it('marks the agent titled even when the call fails, so it cannot loop', async () => {
    suggest.mockRejectedValue(new Error('no claude on PATH'));
    const onTitle = vi.fn();
    renderHook(() =>
      useAgentAutoTitle({
        agents: [agent()],
        snapshotBySession: snap(EXCHANGE),
        enabled: true,
        onTitle,
      }),
    );
    await waitFor(() => expect(onTitle).toHaveBeenCalledWith('agent-s1', null));
  });

  it('does nothing at all when the feature is off', () => {
    renderHook(() =>
      useAgentAutoTitle({
        agents: [agent()],
        snapshotBySession: snap(EXCHANGE),
        enabled: false,
        onTitle: vi.fn(),
      }),
    );
    expect(suggest).not.toHaveBeenCalled();
  });

  it('waits for the reply — an unanswered question triggers nothing', () => {
    renderHook(() =>
      useAgentAutoTitle({
        agents: [agent()],
        snapshotBySession: snap([{ role: 'user', content: 'fix it' }]),
        enabled: true,
        onTitle: vi.fn(),
      }),
    );
    expect(suggest).not.toHaveBeenCalled();
  });

  // Every promoted snapshot is compacted at 12 turns, and conversationOffset
  // only grows — so an agent whose opening burst ran past that was never titled
  // at all. The opening is banked while the snapshot is still whole.
  it('still titles an agent whose conversation compacts after the opening', async () => {
    const onTitle = vi.fn();
    const whole = {
      s1: {
        conversationOffset: 0,
        conversation: [
          { role: 'user', content: 'add retries to the uploader' },
          { role: 'assistant', content: 'on it' },
        ],
      },
    } as unknown as Record<string, ClaudeSessionSnapshot>;
    const compacted = {
      s1: {
        conversationOffset: 30,
        conversation: [
          { role: 'user', content: 'and now something unrelated' },
          { role: 'assistant', content: 'sure' },
        ],
      },
    } as unknown as Record<string, ClaudeSessionSnapshot>;

    const { rerender } = renderHook(
      (p: { snaps: Record<string, ClaudeSessionSnapshot> }) =>
        useAgentAutoTitle({
          agents: [agent()],
          snapshotBySession: p.snaps,
          enabled: true,
          onTitle,
        }),
      { initialProps: { snaps: whole } },
    );
    await waitFor(() => expect(suggest).toHaveBeenCalled());
    expect(suggest.mock.calls[0][0].userMessage).toBe('add retries to the uploader');

    // The same session, now compacted past its opening. Any further request must
    // still carry the ORIGINAL opening, never the oldest surviving turn.
    rerender({ snaps: compacted });
    await waitFor(() => expect(onTitle).toHaveBeenCalled());
    for (const [arg] of suggest.mock.calls) {
      expect(arg.userMessage).toBe('add retries to the uploader');
    }
  });
});
