import { describe, it, expect, vi } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/react';
import React from 'react';
import { MODE_MANIFEST, type UiMode } from '../../src/lib/uiMode';

/**
 * Archived sessions (the hub's shared archive — set here, in the native
 * client, or in another browser) leave the sidebar's normal list but stay in
 * the layout, running if they were. The Archived view lists only them, and
 * Restore brings a row back. Archiving is never a stop: no terminate is sent.
 */

const mode: UiMode = 'fleet';
vi.mock('../../src/hooks/useUiMode', () => ({
  useUiMode: () => ({
    mode,
    manifest: MODE_MANIFEST[mode],
    setMode: () => {},
    toggle: () => {},
  }),
}));

const { default: SideBar, archiveKeyOf } = await import('../../src/components/SideBar');
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
const snapshot = (sessionId: string): any => ({
  sessionId,
  cwd: '/w',
  status: 'active',
  ambientState: 'thinking',
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
});

function Harness(props: {
  agents: any[];
  archived: ReadonlySet<string>;
  onSetArchived?: (id: string, archived: boolean) => void;
  onTerminateAgent?: (id: string) => void;
  collapsed?: boolean;
}) {
  const { agents } = props;
  const snapshotBySession = Object.fromEntries(
    agents.map((a) => [a.sessionId, snapshot(a.sessionId)]),
  );
  const statusBySession = Object.fromEntries(agents.map((a) => [a.sessionId, 'thinking']));
  const attention = useAttentionFeed(snapshotBySession, agents);
  return (
    <ConfigProvider>
      <NotificationsProvider>
        <AttentionProvider
          agents={agents}
          activeAgentId={agents[0].id}
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
            activeAgentId={agents[0].id}
            statusBySession={statusBySession as any}
            snapshotBySession={snapshotBySession}
            onSelectAgent={noop}
            onSpawnAgent={noop}
            onTerminateAgent={props.onTerminateAgent ?? noop}
            onRenameAgent={noop}
            onOpenHistory={noop}
            onOpenSettings={noop}
            collapsed={props.collapsed}
            archivedSessionIds={props.archived}
            onSetArchived={props.onSetArchived}
          />
        </AttentionProvider>
      </NotificationsProvider>
    </ConfigProvider>
  );
}

const fleet = () => [
  mkAgent('manager', 'manager'),
  mkAgent('worker', 'worker', 'manager'),
  mkAgent('done', 'old-task'),
  mkAgent('live', 'live-task'),
];

describe('SideBar archived sessions', () => {
  it('hides archived rows, keeps an archived manager’s live worker as a non-orphan root', () => {
    const terminate = vi.fn();
    render(
      <Harness
        agents={fleet()}
        archived={new Set(['s-manager', 's-done'])}
        onSetArchived={vi.fn()}
        onTerminateAgent={terminate}
      />,
    );
    expect(screen.queryByText('manager')).toBeNull();
    expect(screen.queryByText('old-task')).toBeNull();
    expect(screen.getByText('live-task')).toBeInTheDocument();
    // The parent is archived, not gone: its worker still lists, unflagged.
    expect(screen.getByText('worker')).toBeInTheDocument();
    expect(screen.queryByText('Unwatched')).toBeNull();
    const footer = screen.getByTitle(/Sessions hidden from this list/);
    expect(within(footer).getByText('2')).toBeInTheDocument();
    expect(terminate).not.toHaveBeenCalled();
  });

  it('a native-shaped archive update removes the row live; restoring returns it', () => {
    const agents = fleet();
    const view = render(<Harness agents={agents} archived={new Set()} onSetArchived={vi.fn()} />);
    expect(screen.getByText('old-task')).toBeInTheDocument();
    expect(screen.queryByTitle(/Sessions hidden from this list/)).toBeNull();

    // sessionArchive.changed from another client → new archived set.
    view.rerender(
      <Harness agents={agents} archived={new Set(['s-done'])} onSetArchived={vi.fn()} />,
    );
    expect(screen.queryByText('old-task')).toBeNull();

    view.rerender(<Harness agents={agents} archived={new Set()} onSetArchived={vi.fn()} />);
    expect(screen.getByText('old-task')).toBeInTheDocument();
  });

  it('the Archived view lists only archived rows and restores through the menu', () => {
    const setArchived = vi.fn();
    const terminate = vi.fn();
    render(
      <Harness
        agents={fleet()}
        archived={new Set(['s-done'])}
        onSetArchived={setArchived}
        onTerminateAgent={terminate}
      />,
    );
    fireEvent.click(screen.getByTitle(/Sessions hidden from this list/));
    expect(screen.getByText('Archived · 1')).toBeInTheDocument();
    expect(screen.getByText('old-task')).toBeInTheDocument();
    expect(screen.queryByText('live-task')).toBeNull();

    fireEvent.contextMenu(screen.getByText('old-task'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Restore' }));
    expect(setArchived).toHaveBeenCalledWith('s-done', false);

    // Back to the normal list.
    fireEvent.click(screen.getByText('Archived · 1'));
    expect(screen.getByText('live-task')).toBeInTheDocument();
    expect(terminate).not.toHaveBeenCalled();
  });

  it('Archive in the card menu archives by session id and never terminates', () => {
    const setArchived = vi.fn();
    const terminate = vi.fn();
    render(
      <Harness
        agents={fleet()}
        archived={new Set()}
        onSetArchived={setArchived}
        onTerminateAgent={terminate}
      />,
    );
    fireEvent.contextMenu(screen.getByText('live-task'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Archive' }));
    expect(setArchived).toHaveBeenCalledWith('s-live', true);
    expect(terminate).not.toHaveBeenCalled();
  });

  it('offers no Archive item when the hub has no shared archive', () => {
    render(<Harness agents={fleet()} archived={new Set()} />);
    fireEvent.contextMenu(screen.getByText('live-task'));
    expect(screen.queryByRole('menuitem', { name: 'Archive' })).toBeNull();
    expect(screen.getByRole('menuitem', { name: 'Terminate' })).toBeInTheDocument();
  });

  it('the collapsed rail hides archived tiles too', () => {
    const agents = fleet();
    const { container } = render(
      <Harness agents={agents} archived={new Set(['s-done'])} collapsed onSetArchived={vi.fn()} />,
    );
    const railButtons = container.querySelectorAll('button[title]');
    const titles = [...railButtons].map((b) => b.getAttribute('title') ?? '');
    expect(titles.some((t) => t.includes('old-task'))).toBe(false);
    expect(titles.some((t) => t.includes('live-task'))).toBe(true);
  });

  it('archives a stopped card under the session it last held', () => {
    expect(archiveKeyOf({ id: 'a', lastSessionId: 'prev', tabs: [] } as any)).toBe('prev');
    expect(archiveKeyOf({ id: 'a', sessionId: 'now', lastSessionId: 'prev' } as any)).toBe('now');
    expect(archiveKeyOf({ id: 'g', global: true, sessionId: 'x' } as any)).toBeUndefined();
  });
});
