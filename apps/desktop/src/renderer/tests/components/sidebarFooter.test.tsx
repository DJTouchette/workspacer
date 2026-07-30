import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import { MODE_MANIFEST, type UiMode } from '../../src/lib/uiMode';

/**
 * The sidebar's bottom strip: the quiet ways out of the live feed. History has
 * always been there; Settings joined it, so what's worth pinning down is that
 * the strip degrades cleanly — Settings must still be reachable on a fresh
 * install with no past sessions, when the History row isn't rendered at all.
 *
 * Scaffolding mirrors sidebarFeedFilter.test.tsx.
 */

const mode: UiMode = 'fleet';
vi.mock('../../src/hooks/useUiMode', () => ({
  useUiMode: () => ({
    mode: 'fleet',
    manifest: MODE_MANIFEST.fleet,
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

const agents: any[] = [
  {
    id: 'a1',
    name: 'workspacer',
    cwd: '/w',
    sessionId: 's1',
    tabs: [
      {
        id: 't1',
        title: 'Claude',
        panes: [{ id: 'p1', type: 'claude' as const, title: 'Claude' }],
        activePaneId: 'p1',
      },
    ],
    activeTabId: 't1',
  },
];

const snapshotBySession: Record<string, any> = {
  s1: {
    sessionId: 's1',
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
    ambientState: 'idle',
  },
};

/** Two resumable rows the layout doesn't hold — what History counts. */
const RECENT = [
  { sessionId: 'old-1', provider: 'claude', cwd: '/w', mode: 'stopped', updatedAt: now - 60_000 },
  { sessionId: 'old-2', provider: 'claude', cwd: '/w', mode: 'stopped', updatedAt: now - 90_000 },
] as any[];

const Harness: React.FC<{
  recentSessions?: any[];
  onOpenSettings?: () => void;
  onOpenHistory?: () => void;
}> = ({ recentSessions, onOpenSettings, onOpenHistory }) => {
  const attention = useAttentionFeed(snapshotBySession, agents);
  return (
    <ConfigProvider>
      <NotificationsProvider>
        <AttentionProvider
          agents={agents}
          activeAgentId="a1"
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
            activeAgentId="a1"
            statusBySession={{ s1: 'idle' } as any}
            snapshotBySession={snapshotBySession}
            onSelectAgent={noop}
            onSpawnAgent={noop}
            onTerminateAgent={noop}
            onRenameAgent={noop}
            onToggleCollapse={noop}
            onToggleHelp={noop}
            viewLevel="piloting"
            recentSessions={recentSessions ?? []}
            onOpenHistory={onOpenHistory ?? noop}
            onOpenSettings={onOpenSettings}
          />
        </AttentionProvider>
      </NotificationsProvider>
    </ConfigProvider>
  );
};

const row = (name: RegExp) => screen.getByRole('button', { name });

describe('sidebar footer strip', () => {
  it('offers Settings and opens it on click', () => {
    const onOpenSettings = vi.fn();
    render(<Harness recentSessions={RECENT} onOpenSettings={onOpenSettings} />);
    fireEvent.click(row(/^Settings$/));
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
  });

  it('opens Settings from the keyboard', () => {
    const onOpenSettings = vi.fn();
    render(<Harness recentSessions={RECENT} onOpenSettings={onOpenSettings} />);
    fireEvent.keyDown(row(/^Settings$/), { key: 'Enter' });
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
  });

  it('keeps Settings reachable on a fresh install with no history', () => {
    render(<Harness recentSessions={[]} onOpenSettings={noop} />);
    // No past sessions ⇒ no History row, but the strip must not vanish with it.
    expect(screen.queryByRole('button', { name: /^History/ })).toBeNull();
    expect(row(/^Settings$/)).toBeInTheDocument();
  });

  it('still shows History with its count when there are past sessions', () => {
    const onOpenHistory = vi.fn();
    render(<Harness recentSessions={RECENT} onOpenHistory={onOpenHistory} onOpenSettings={noop} />);
    const history = row(/^History/);
    expect(history.textContent).toContain('2');
    fireEvent.click(history);
    expect(onOpenHistory).toHaveBeenCalledTimes(1);
  });

  it('renders no strip at all when the host wires neither row', () => {
    render(<Harness recentSessions={[]} />);
    expect(screen.queryByRole('button', { name: /^Settings$/ })).toBeNull();
  });
});
