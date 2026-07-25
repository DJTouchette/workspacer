import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import { MODE_MANIFEST, type UiMode } from '../../src/lib/uiMode';

/**
 * Focus mode's feed filter — the behavior the fleet/focus axis actually means
 * now that the sidebar is the live triage surface rather than a nav list.
 *
 * The contract: focus narrows WHICH agents get a full card to the one you're
 * piloting plus anything blocked on you, and folds the merely-working/finished
 * remainder into one expandable row. A blocked agent is never quieted — its
 * inline Approve is the whole reason the card exists — so "focus" can never
 * make you miss a block, only make the periphery quieter.
 */

// Drive the manifest directly: the config→manifest plumbing is useUiMode's own
// test, and SideBar should only ever read the flags.
let mode: UiMode = 'focus';
vi.mock('../../src/hooks/useUiMode', () => ({
  useUiMode: () => ({
    mode,
    manifest: MODE_MANIFEST[mode],
    setMode: vi.fn(),
    toggle: vi.fn(),
  }),
}));

const { default: SideBar } = await import('../../src/components/SideBar');
const { AttentionProvider } = await import('../../src/contexts/AttentionContext');
const { NotificationsProvider } = await import('../../src/contexts/NotificationsContext');
const { ConfigProvider } = await import('../../src/contexts/ConfigContext');
const { useAttentionFeed } = await import('../../src/hooks/useAttentionFeed');

const now = Date.now();
const noop = () => {};

const tabs = (id: string) => [
  {
    id: `tab-${id}`,
    title: 'Claude',
    panes: [{ id: `pane-${id}`, type: 'claude' as const, title: 'Claude' }],
    activePaneId: `pane-${id}`,
  },
];

// piloted = active; blocked = waiting on an approval; busy/finished = the
// remainder focus should quiet.
const agents: any[] = [
  { id: 'a-piloted', name: 'piloted', cwd: '/w/p', sessionId: 's-piloted', tabs: tabs('1') },
  { id: 'a-blocked', name: 'blocked', cwd: '/w/b', sessionId: 's-blocked', tabs: tabs('2') },
  { id: 'a-busy', name: 'busy', cwd: '/w/w', sessionId: 's-busy', tabs: tabs('3') },
  { id: 'a-finished', name: 'finished', cwd: '/w/d', sessionId: 's-finished', tabs: tabs('4') },
];

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
  lastActivity: now,
};

const snapshotBySession: Record<string, any> = {
  's-piloted': { ...base, sessionId: 's-piloted', ambientState: 'idle' },
  's-blocked': {
    ...base,
    sessionId: 's-blocked',
    ambientState: 'waiting_approval',
    pendingApproval: {
      toolName: 'Bash',
      toolInput: { command: 'rm -rf build' },
      timestamp: now - 1000,
    },
  },
  's-busy': { ...base, sessionId: 's-busy', ambientState: 'streaming' },
  's-finished': { ...base, sessionId: 's-finished', ambientState: 'idle' },
};

const statusBySession: Record<string, any> = Object.fromEntries(
  Object.values(snapshotBySession).map((s: any) => [s.sessionId, s.ambientState]),
);

const Harness: React.FC = () => {
  const attention = useAttentionFeed(snapshotBySession, agents);
  return (
    <ConfigProvider>
      <NotificationsProvider>
        <AttentionProvider
          agents={agents}
          activeAgentId="a-piloted"
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
            activeAgentId="a-piloted"
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

describe('SideBar feed filter (focus mode)', () => {
  beforeEach(() => {
    mode = 'focus';
  });

  it('keeps the piloted agent and the blocked agent as full cards', () => {
    render(<Harness />);
    expect(screen.getByText('piloted')).toBeInTheDocument();
    expect(screen.getByText('blocked')).toBeInTheDocument();
  });

  it('keeps a blocked agent resolvable inline even though it is not the active one', () => {
    render(<Harness />);
    // The inline Approve is the affordance the old rail-based focus mode threw
    // away; losing it again is the regression this test exists to catch.
    expect(screen.getByText('Approve')).toBeInTheDocument();
  });

  it('quiets the working and finished agents behind one summary row', () => {
    render(<Harness />);
    expect(screen.queryByText('busy')).toBeNull();
    expect(screen.queryByText('finished')).toBeNull();
    expect(screen.getByText('2 others')).toBeInTheDocument();
    expect(screen.getByText('1 working')).toBeInTheDocument();
  });

  it('reveals the quieted agents when the summary row is clicked', () => {
    render(<Harness />);
    fireEvent.click(screen.getByText('2 others'));
    expect(screen.getByText('busy')).toBeInTheDocument();
    expect(screen.getByText('finished')).toBeInTheDocument();
    expect(screen.queryByText('2 others')).toBeNull();
    expect(screen.getByText('Show less')).toBeInTheDocument();
  });

  it('fleet mode shows every agent and never renders the summary row', () => {
    mode = 'fleet';
    render(<Harness />);
    for (const name of ['piloted', 'blocked', 'busy', 'finished']) {
      expect(screen.getByText(name)).toBeInTheDocument();
    }
    expect(screen.queryByText(/others$/)).toBeNull();
  });
});
