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

function harnessFor(agents: any[], snapshotOverrides: Record<string, any> = {}) {
  const snapshotBySession: Record<string, any> = Object.fromEntries(
    agents.map((a) => [
      a.sessionId,
      { ...base, sessionId: a.sessionId, ambientState: 'idle', ...snapshotOverrides[a.sessionId] },
    ]),
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

/**
 * The "Unwatched" chip surfaces the same dangling-parentId fact `rootOf`
 * already computes for grouping (see above) — a worker whose dispatcher card
 * is gone, so nobody is positioned to hear its result. It must appear only
 * for that exact case, never for an ordinary top-level agent or a normally
 * nested child whose parent resolves, and it must say something different
 * depending on whether the dispatcher is a *confirmed* Fleet Manager or
 * merely absent. That "confirmed" bit prefers the daemon's own live truth
 * (`snapshotBySession[...].orphan.confirmedManager`, recomputed from
 * managerTombstones on every push) and falls back to the renderer-local guess
 * snapshotted once at adopt time (`dispatchedByManager`) only when the daemon
 * hasn't reported one.
 */
describe('SideBar — orphaned-worker "Unwatched" chip', () => {
  it('renders on a worker whose parentId does not resolve to any known agent', () => {
    const agents = [
      // a-ghost-manager is never in the agents list — its card is gone.
      { ...mkAgent('a-worker', 'lone-worker', 'a-ghost-manager'), dispatchedByManager: true },
    ];
    const Harness = harnessFor(agents);
    render(<Harness />);

    expect(screen.getByText('lone-worker')).toBeInTheDocument();
    expect(screen.getByText('Unwatched')).toBeInTheDocument();
  });

  it('does NOT render for an ordinary top-level agent with no parentId at all', () => {
    const agents = [mkAgent('a-standalone', 'standalone')];
    const Harness = harnessFor(agents);
    render(<Harness />);

    expect(screen.getByText('standalone')).toBeInTheDocument();
    // This is the assertion that catches a false-positive implementation
    // (e.g. flagging any agent without a parentId instead of only a
    // *dangling* one) — watched it fail before the `!!agent.parentId` guard
    // was added: it matched Overview/global rows and every plain agent too.
    expect(screen.queryByText('Unwatched')).not.toBeInTheDocument();
  });

  it('does NOT render for a child whose parentId resolves to a live agent', () => {
    const agents = [mkAgent('a-manager', 'manager'), mkAgent('a-worker', 'worker', 'a-manager')];
    const Harness = harnessFor(agents);
    render(<Harness />);

    expect(screen.getByText('manager')).toBeInTheDocument();
    expect(screen.getByText('worker')).toBeInTheDocument();
    expect(screen.queryByText('Unwatched')).not.toBeInTheDocument();
  });

  it('distinguishes a confirmed manager from a merely-gone parent in the tooltip', () => {
    const confirmed = {
      ...mkAgent('a-confirmed', 'confirmed-worker', 'a-ghost-manager'),
      dispatchedByManager: true,
    };
    const inferred = {
      ...mkAgent('a-inferred', 'inferred-worker', 'a-ghost-parent'),
      dispatchedByManager: false,
    };
    const Harness = harnessFor([confirmed, inferred]);
    render(<Harness />);

    const chips = screen.getAllByText('Unwatched');
    expect(chips).toHaveLength(2);
    const titles = chips.map((el) => el.closest('span')?.getAttribute('title'));
    expect(titles.some((t) => t?.includes('manager session ended'))).toBe(true);
    expect(titles.some((t) => t?.includes('no longer here'))).toBe(true);
  });

  it("prefers the daemon's real orphan/confirmedManager truth over a stale or absent dispatchedByManager guess", () => {
    const agents = [
      // dispatchedByManager is FALSE — the renderer-local guess, snapshotted
      // once at adopt time — but the daemon's own snapshot (recomputed live
      // from managerTombstones on every push, see claudeSessionStore's
      // refreshOrphanStatus) says the dead parent WAS a confirmed manager.
      // That ground truth must win.
      { ...mkAgent('a-worker', 'lone-worker', 'a-ghost-manager'), dispatchedByManager: false },
    ];
    const Harness = harnessFor(agents, { 's-a-worker': { orphan: { confirmedManager: true } } });
    render(<Harness />);

    const chip = screen.getByText('Unwatched');
    expect(chip.closest('span')?.getAttribute('title')).toContain('manager session ended');
  });
});
