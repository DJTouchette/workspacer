import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import { MODE_MANIFEST, type UiMode } from '../../src/lib/uiMode';

/**
 * The sidebar's bottom strip: the quiet ways out of the live feed. History
 * and Settings are both standing rows — History opens the project-grouped
 * transcript browser, whose contents don't depend on what the daemon still
 * holds, so it must stay reachable even with zero daemon sessions (it used to
 * hide itself on a daemon count of 0, which orphaned transcript-only
 * history). The strip renders only when the host wires at least one row.
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

const Harness: React.FC<{
  onOpenSettings?: () => void;
  onOpenHistory?: () => void;
}> = ({ onOpenSettings, onOpenHistory }) => {
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
            onOpenHistory={onOpenHistory}
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
    render(<Harness onOpenSettings={onOpenSettings} />);
    fireEvent.click(row(/^Settings$/));
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
  });

  it('opens Settings from the keyboard', () => {
    const onOpenSettings = vi.fn();
    render(<Harness onOpenSettings={onOpenSettings} />);
    fireEvent.keyDown(row(/^Settings$/), { key: 'Enter' });
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
  });

  it('offers History whenever it is wired, and opens it on click', () => {
    // No daemon-count gate anymore: the pane's transcript contents exist
    // independently of the daemon, so the door is always there.
    const onOpenHistory = vi.fn();
    render(<Harness onOpenHistory={onOpenHistory} onOpenSettings={noop} />);
    fireEvent.click(row(/^History/));
    expect(onOpenHistory).toHaveBeenCalledTimes(1);
  });

  it('renders no strip at all when the host wires neither row', () => {
    render(<Harness />);
    expect(screen.queryByRole('button', { name: /^Settings$/ })).toBeNull();
  });
});
