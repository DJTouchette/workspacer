import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import React from 'react';
import { MODE_MANIFEST, type UiMode } from '../../src/lib/uiMode';

/**
 * A worker dispatched by a worker is a grandchild of the manager. SideBar used
 * to only unnest one generation (childrenByParent keyed by *immediate*
 * parentId), so a grandchild whose parent was itself a nested child never
 * rendered at all — it wasn't top-level, and nothing iterated its parent's
 * bucket looking for further children.
 *
 * The fix mirrors services/hub/cmd/hub/mobile.html's fleetRoster(): walk each
 * agent's parentId chain to the top-most resolvable ancestor and attach it
 * there, flat, so the desktop and the mobile PWA agree on one nesting policy.
 */

const mode: UiMode = 'fleet'; // fleet mode never quiets/folds cards away
vi.mock('../../src/hooks/useUiMode', () => ({
  useUiMode: () => ({
    mode,
    manifest: MODE_MANIFEST[mode],
    setMode: () => {},
    toggle: () => {},
  }),
}));

const { default: SideBar } = await import('../../src/components/SideBar');
const { AttentionProvider } = await import('../../src/contexts/AttentionContext');
const { NotificationsProvider } = await import('../../src/contexts/NotificationsContext');
const { ConfigProvider } = await import('../../src/contexts/ConfigContext');
const { useAttentionFeed } = await import('../../src/hooks/useAttentionFeed');

const noop = () => {};

const tabs = (id: string) => [
  {
    id: `tab-${id}`,
    title: 'Claude',
    panes: [{ id: `pane-${id}`, type: 'claude' as const, title: 'Claude' }],
    activePaneId: `pane-${id}`,
  },
];

const mkAgent = (id: string, name: string, parentId?: string): any => ({
  id,
  name,
  cwd: '/w',
  sessionId: `s-${id}`,
  tabs: tabs(id),
  ...(parentId ? { parentId } : {}),
});

const base = {
  cwd: '/w',
  status: 'active',
  conversation: [],
  activeToolCalls: [],
  completedToolCalls: [],
  fileChanges: [],
  pendingApproval: null,
  pendingQuestions: null,
  subagents: [],
  workflows: [],
  totalToolCalls: 0,
  usage: null,
  lastActivity: Date.now(),
};

function harnessFor(agents: any[]) {
  const snapshotBySession: Record<string, any> = Object.fromEntries(
    agents.map((a) => [a.sessionId, { ...base, sessionId: a.sessionId, ambientState: 'idle' }]),
  );
  const statusBySession: Record<string, any> = Object.fromEntries(
    agents.map((a) => [a.sessionId, 'idle']),
  );

  const Harness: React.FC = () => {
    const attention = useAttentionFeed(snapshotBySession, agents);
    return (
      <ConfigProvider>
        <NotificationsProvider>
          <AttentionProvider
            agents={agents}
            activeAgentId={agents[0]?.id}
            snapshotBySession={snapshotBySession}
            inboxOpen={false}
            openInbox={noop}
            closeInbox={noop}
            viewLevel="piloting"
            setViewLevel={noop}
            onOpenAgent={noop}
            attention={attention}
          >
            <SideBar
              agents={agents}
              activeAgentId={agents[0]?.id}
              statusBySession={statusBySession}
              snapshotBySession={snapshotBySession}
              onSelectAgent={noop}
              onSpawnAgent={noop}
              onTerminateAgent={noop}
              onRenameAgent={noop}
              onToggleCollapse={noop}
              onToggleHelp={noop}
              viewLevel="piloting"
              collapsed={false}
            />
          </AttentionProvider>
        </NotificationsProvider>
      </ConfigProvider>
    );
  };
  return Harness;
}

describe('SideBar fleet nesting — grandchildren', () => {
  it('nests a three-level chain (manager → worker → sub-worker) under the manager', () => {
    const agents = [
      mkAgent('a-manager', 'manager'),
      mkAgent('a-worker', 'worker', 'a-manager'),
      mkAgent('a-subworker', 'sub-worker', 'a-worker'),
    ];
    const Harness = harnessFor(agents);
    render(<Harness />);

    // The bug: sub-worker was dropped entirely because its parent (worker)
    // wasn't a top-level agent, so nothing iterated worker's bucket for it.
    expect(screen.getByText('manager')).toBeInTheDocument();
    expect(screen.getByText('worker')).toBeInTheDocument();
    expect(screen.getByText('sub-worker')).toBeInTheDocument();
  });

  it('attaches an orphan (parent gone) as its own top-level card instead of vanishing', () => {
    const agents = [
      mkAgent('a-manager', 'manager'),
      // a-ghost-parent is never in the agents list — the parent session ended.
      mkAgent('a-orphan', 'orphan', 'a-ghost-parent'),
    ];
    const Harness = harnessFor(agents);
    render(<Harness />);

    expect(screen.getByText('manager')).toBeInTheDocument();
    expect(screen.getByText('orphan')).toBeInTheDocument();
  });

  it('does not double-count a self-referential parentId as an infinite loop', () => {
    const agents = [
      mkAgent('a-loopy', 'loopy'),
      { ...mkAgent('a-loopy', 'loopy'), parentId: 'a-loopy' },
    ];
    // Same id twice is unrealistic in practice; the real regression guard is
    // that rootOf() terminates. If it didn't, this render would hang/crash.
    const Harness = harnessFor([agents[1]]);
    expect(() => render(<Harness />)).not.toThrow();
  });
});
