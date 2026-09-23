import React, { memo } from 'react';
import { act, render } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import {
  AttentionProvider,
  useAttention,
  useAttentionActions,
  useAttentionNavigationOptional,
} from '../src/contexts/AttentionContext';
import type { AttentionFeed } from '../src/hooks/useAttentionFeed';
import type { AgentWorkspace } from '../src/types/pane';
import type { ClaudeSessionSnapshot } from '../src/types/claudeSession';
import { resolveAnswer } from '../src/lib/resolveAttention';
vi.mock('../src/lib/resolveAttention', () => ({
  resolveAnswer: vi.fn(),
  resolveApproval: vi.fn(),
  resolveReply: vi.fn(),
}));

it('isolates action and directory consumers while actions use the latest session transport', () => {
  const actionsRendered = vi.fn();
  const chipsRendered = vi.fn();
  let actions!: ReturnType<typeof useAttentionActions>;
  const ActionConsumer = memo(() => {
    actions = useAttentionActions();
    actionsRendered();
    return null;
  });
  const ChipConsumer = memo(() => {
    const navigation = useAttentionNavigationOptional();
    chipsRendered(navigation?.agentsBySession.get('s')?.name);
    return null;
  });
  const agents: AgentWorkspace[] = [
    { id: 'a', sessionId: 's', name: 'Agent', cwd: '/repo', tabs: [], activeTabId: '' },
  ];
  const noop = vi.fn();
  const feed: AttentionFeed = {
    items: [],
    counts: { total: 0, needsYou: 0, byKind: {} as any },
    topByAgent: new Map(),
    dismiss: noop,
    snooze: noop,
  };
  const snapshots = {
    s: { sessionId: 's', provider: 'claude', transport: 'pty' } as ClaudeSessionSnapshot,
  };
  const children = (
    <>
      <ActionConsumer />
      <ChipConsumer />
    </>
  );
  const tree = (snapshotBySession: typeof snapshots, attention = feed, directory = agents) => (
    <AttentionProvider
      agents={directory}
      activeAgentId="a"
      snapshotBySession={snapshotBySession}
      inboxOpen={false}
      openInbox={noop}
      closeInbox={noop}
      viewLevel="fleet"
      setViewLevel={noop}
      onOpenAgent={noop}
      attention={attention}
    >
      {children}
    </AttentionProvider>
  );
  const { rerender } = render(tree(snapshots));
  const stableActions = actions;
  rerender(
    tree(
      { s: { ...snapshots.s, transport: 'stream' } },
      { ...feed, items: [], counts: { ...feed.counts } },
    ),
  );
  expect(actionsRendered).toHaveBeenCalledTimes(1);
  expect(chipsRendered).toHaveBeenCalledTimes(1);
  expect(actions).toBe(stableActions);
  act(() => actions.answer({ sessionId: 's' } as any, { text: 'yes' }));
  expect(resolveAnswer).toHaveBeenCalledWith('s', { text: 'yes' }, 'claude', 'stream');
  rerender(tree(snapshots, feed, [{ ...agents[0], name: 'Renamed' }]));
  expect(chipsRendered).toHaveBeenLastCalledWith('Renamed');
});

it('indexes the first decision per agent and follows the selected inbox filter', () => {
  let context!: ReturnType<typeof useAttention>;
  function Reader() {
    context = useAttention();
    return null;
  }
  const noop = vi.fn();
  const approval = { agentId: 'a', sessionId: 's', kind: 'approval', signature: 'first' } as any;
  const second = { ...approval, signature: 'second' };
  const question = { ...approval, kind: 'question', signature: 'question' };
  const attention = {
    items: [approval, second, question],
    topByAgent: new Map(),
    counts: { total: 3, needsYou: 3, byKind: {} },
    dismiss: noop,
    snooze: noop,
  } as AttentionFeed;
  render(
    <AttentionProvider
      agents={[]}
      activeAgentId=""
      snapshotBySession={{}}
      inboxOpen={false}
      openInbox={noop}
      closeInbox={noop}
      viewLevel="fleet"
      setViewLevel={noop}
      onOpenAgent={noop}
      attention={attention}
    >
      <Reader />
    </AttentionProvider>,
  );
  expect(context.decisionsByAgent.get('a')).toEqual({ approval, question });
  act(() => context.setInboxFilter('review'));
  expect(context.decisionsByAgent.size).toBe(0);
});
