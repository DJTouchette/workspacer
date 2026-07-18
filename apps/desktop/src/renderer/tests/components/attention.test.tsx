import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import React from 'react';
import { AttentionProvider } from '../../src/contexts/AttentionContext';
import { ConfigProvider } from '../../src/contexts/ConfigContext';
import { useAttentionFeed } from '../../src/hooks/useAttentionFeed';
import InboxDrawer from '../../src/components/InboxDrawer';
import FleetDeck from '../../src/components/FleetDeck';
import type { AgentWorkspace } from '../../src/types/pane';
import { REVIEW_REQUEST_FILE_EVENT } from '../../src/lib/reviewBus';

const agents: AgentWorkspace[] = [
  {
    id: 'a1',
    name: 'Refactor agent',
    cwd: '/repo/refactor',
    sessionId: 'sess-1',
    tabs: [],
    activeTabId: '',
  },
  {
    id: 'a2',
    name: 'Builder agent',
    cwd: '/repo/builder',
    sessionId: 'sess-2',
    tabs: [],
    activeTabId: '',
  },
  { id: 'global', name: 'Overview', global: true, cwd: '', tabs: [], activeTabId: '' },
];

const snapshotBySession: Record<string, any> = {
  'sess-1': {
    sessionId: 'sess-1',
    ambientState: 'waiting_approval',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    subagents: [],
    workflows: [],
    pendingApproval: { toolName: 'Bash', toolInput: { command: 'npm test' }, timestamp: 1000 },
    pendingQuestions: null,
    usage: null,
  },
  'sess-2': {
    sessionId: 'sess-2',
    ambientState: 'streaming',
    conversation: [{ role: 'assistant', content: 'Wiring up the deck', timestamp: 1 }],
    activeToolCalls: [
      {
        id: 't1',
        name: 'Edit',
        input: { file_path: '/repo/builder/App.tsx' },
        status: 'running',
        startedAt: 1,
      },
    ],
    completedToolCalls: [],
    fileChanges: [],
    subagents: [],
    workflows: [],
    pendingApproval: null,
    pendingQuestions: null,
    usage: {
      model: 'claude-opus-4-8',
      contextTokens: 50000,
      contextLimit: 200000,
      totalInputTokens: 0,
      totalOutputTokens: 0,
      costUSD: 0.42,
    },
  },
};

// The feed is lifted to App in production, so the test mirrors that: a tiny
// harness calls the real useAttentionFeed and passes the same instance to the
// provider, keeping the test exercising real snapshot-derived attention.
function Harness({
  viewLevel,
  onOpenAgent,
  agentList = agents,
  snapshots = snapshotBySession,
}: {
  viewLevel: 'fleet' | 'piloting';
  onOpenAgent: (id: string) => void;
  agentList?: AgentWorkspace[];
  snapshots?: Record<string, any>;
}) {
  const attention = useAttentionFeed(snapshots, agentList);
  return (
    <ConfigProvider>
      <AttentionProvider
        agents={agentList}
        activeAgentId={agentList.find((a) => !a.global)?.id ?? ''}
        snapshotBySession={snapshots}
        inboxOpen
        openInbox={vi.fn()}
        closeInbox={vi.fn()}
        viewLevel={viewLevel}
        setViewLevel={vi.fn()}
        onOpenAgent={onOpenAgent}
        attention={attention}
      >
        <InboxDrawer />
        <FleetDeck top={40} left={196} />
      </AttentionProvider>
    </ConfigProvider>
  );
}

function renderSurfaces(viewLevel: 'fleet' | 'piloting' = 'fleet') {
  const onOpenAgent = vi.fn();
  render(<Harness viewLevel={viewLevel} onOpenAgent={onOpenAgent} />);
  return { onOpenAgent };
}

function renderCustomSurfaces(agentList: AgentWorkspace[], snapshots: Record<string, any>) {
  const onOpenAgent = vi.fn();
  render(
    <Harness
      viewLevel="fleet"
      onOpenAgent={onOpenAgent}
      agentList={agentList}
      snapshots={snapshots}
    />,
  );
  return { onOpenAgent };
}

describe('Mission Control surfaces', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the inbox with an approval card derived from the snapshot', () => {
    renderSurfaces();
    expect(screen.getByText('Inbox')).toBeInTheDocument();
    // "Needs approval" shows on the inbox card and the agent's fleet card.
    expect(screen.getAllByText(/Needs approval/i).length).toBeGreaterThanOrEqual(1);
    // The approval card embeds ClaudePane's ApprovalPrompt verbatim.
    expect(screen.getByText('Permission Required: Bash')).toBeInTheDocument();
  });

  it('resolves an approval by sessionId without owning the pane', () => {
    renderSurfaces();
    const allow = screen.getAllByText('Allow')[0];
    fireEvent.click(allow);
    expect(window.electronAPI.claudeApprove).toHaveBeenCalledWith('sess-1', 'yes');
  });

  it('renders a Fleet card per real agent, excluding the global workspace', () => {
    renderSurfaces();
    expect(screen.getByText('Fleet')).toBeInTheDocument();
    // "Refactor agent" appears in both the inbox card and its fleet card.
    expect(screen.getAllByText('Refactor agent').length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText('Builder agent')).toBeInTheDocument();
    // The streaming agent shows its live tool, not a stale message.
    expect(screen.getByText(/Edit\(App\.tsx\)/)).toBeInTheDocument();
  });

  it('opens an agent (drops to piloting) when a card is clicked', () => {
    const { onOpenAgent } = renderSurfaces();
    fireEvent.click(screen.getByText('Builder agent'));
    expect(onOpenAgent).toHaveBeenCalledWith('a2');
  });

  it("clears an agent's inbox items when you open it", () => {
    const { onOpenAgent } = renderSurfaces();
    // The Refactor agent (sess-1) has an approval card in the inbox.
    expect(screen.getByText('Permission Required: Bash')).toBeInTheDocument();
    // Opening it from the card footer is the triage action.
    fireEvent.click(screen.getAllByText('Open')[0]);
    expect(onOpenAgent).toHaveBeenCalledWith('a1');
    // Its item is dismissed, so the card no longer lingers in the inbox.
    expect(screen.queryByText('Permission Required: Bash')).not.toBeInTheDocument();
  });

  it("keeps the piloted agent's inbox clear as new items arrive", () => {
    // activeAgentId is 'a1'; while piloting it, its approval must not surface in
    // the inbox even though the snapshot has a pending approval (you're already
    // looking at the agent, and its pane shows the live prompt).
    renderSurfaces('piloting');
    expect(screen.queryByText('Permission Required: Bash')).not.toBeInTheDocument();
  });

  it('reviewing a large-diff card opens Review and clears the card', () => {
    const reviewAgents: AgentWorkspace[] = [
      {
        id: 'review-agent',
        name: 'Review agent',
        cwd: '/repo/review',
        sessionId: 'sess-review',
        tabs: [],
        activeTabId: '',
      },
      { id: 'global', name: 'Overview', global: true, cwd: '', tabs: [], activeTabId: '' },
    ];
    const reviewSnapshots = {
      'sess-review': {
        sessionId: 'sess-review',
        ambientState: 'idle',
        conversation: [],
        activeToolCalls: [],
        completedToolCalls: [],
        fileChanges: [
          {
            path: 'src/App.tsx',
            input: {
              old_string: `${'old\n'.repeat(90)}`,
              new_string: `${'new\n'.repeat(90)}`,
            },
          },
        ],
        subagents: [],
        workflows: [],
        pendingApproval: null,
        pendingQuestions: null,
        usage: null,
        lastActivity: 1000,
      },
    };
    const events: Array<{ cwd?: string; path?: string; agentId?: string }> = [];
    const handler = (e: Event) => {
      events.push((e as CustomEvent).detail);
    };
    window.addEventListener(REVIEW_REQUEST_FILE_EVENT, handler);
    try {
      renderCustomSurfaces(reviewAgents, reviewSnapshots);
      expect(screen.getByText(/1 file, ±\d+ lines/)).toBeInTheDocument();

      fireEvent.click(screen.getAllByRole('button', { name: 'Review' }).at(-1)!);

      expect(events).toEqual([
        { cwd: '/repo/review', path: '/repo/review', agentId: 'review-agent' },
      ]);
      expect(screen.queryByText(/1 file, ±\d+ lines/)).not.toBeInTheDocument();
    } finally {
      window.removeEventListener(REVIEW_REQUEST_FILE_EVENT, handler);
    }
  });

  it('flips a Fleet card in place into the live InspectorCard, then collapses', () => {
    renderSurfaces();
    // No inspector chrome until a card is expanded.
    expect(screen.queryByRole('button', { name: /Usage/ })).not.toBeInTheDocument();

    // Click a card's expand affordance → the InspectorCard (its Usage tab +
    // collapse control) appears in place.
    fireEvent.click(screen.getAllByTitle(/Inspect \(plan/)[0]);
    expect(screen.getByRole('button', { name: /Usage/ })).toBeInTheDocument();
    const collapse = screen.getByTitle('Collapse (Esc)');
    expect(collapse).toBeInTheDocument();

    // Collapsing returns to the telemetry card (inspector chrome gone).
    fireEvent.click(collapse);
    expect(screen.queryByRole('button', { name: /Usage/ })).not.toBeInTheDocument();
  });
});

// ── idx 13: Needs-you / Review tab badges must match their contents ──
import { useAttention } from '../../src/contexts/AttentionContext';

describe('Inbox tab badges match tab contents', () => {
  const now = Date.now();
  const errAgents: AgentWorkspace[] = [
    {
      id: 'e1',
      name: 'Erroring agent',
      cwd: '/repo/err',
      sessionId: 'sess-err',
      tabs: [],
      activeTabId: '',
    },
    { id: 'global', name: 'Overview', global: true, cwd: '', tabs: [], activeTabId: '' },
  ];
  const errSnapshots: Record<string, any> = {
    'sess-err': {
      sessionId: 'sess-err',
      ambientState: 'idle',
      conversation: [],
      activeToolCalls: [],
      completedToolCalls: [
        { id: 'c1', name: 'Bash', status: 'failed', completedAt: now, response: 'boom' },
      ],
      fileChanges: [],
      subagents: [],
      workflows: [],
      pendingApproval: null,
      pendingQuestions: null,
      usage: null,
      lastActivity: now,
    },
  };

  function InboxOnly() {
    const attention = useAttentionFeed(errSnapshots, errAgents);
    return (
      <ConfigProvider>
        <AttentionProvider
          agents={errAgents}
          activeAgentId=""
          snapshotBySession={errSnapshots}
          inboxOpen
          openInbox={vi.fn()}
          closeInbox={vi.fn()}
          viewLevel="fleet"
          setViewLevel={vi.fn()}
          onOpenAgent={vi.fn()}
          attention={attention}
        >
          <InboxDrawer />
        </AttentionProvider>
      </ConfigProvider>
    );
  }

  it('lists an error item under the tab whose badge counts it (Needs you)', () => {
    render(<InboxOnly />);
    const needsTab = screen.getByRole('button', { name: /Needs you/ });
    expect(needsTab).toHaveTextContent('1');
    expect(screen.getByText('boom')).toBeInTheDocument();

    // Switching to the "Needs you" tab must list the very item its badge counted.
    fireEvent.click(needsTab);
    expect(screen.getByText('boom')).toBeInTheDocument();
  });
});

// ── idx 14: piloting auto-dismiss must cover ALL active-agent items ──
describe('Piloting auto-dismiss ignores the inbox filter', () => {
  const bugAgents: AgentWorkspace[] = [
    {
      id: 'a1',
      name: 'Refactor agent',
      cwd: '/repo/refactor',
      sessionId: 'sess-1',
      tabs: [],
      activeTabId: '',
    },
    { id: 'global', name: 'Overview', global: true, cwd: '', tabs: [], activeTabId: '' },
  ];
  const workingSnap = {
    sessionId: 'sess-1',
    ambientState: 'streaming',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    subagents: [],
    workflows: [],
    pendingApproval: null,
    pendingQuestions: null,
    usage: null,
  };
  // Idle agent that left a large (>80 line) unreviewed change → review-class item.
  const bigdiffSnap = {
    ...workingSnap,
    ambientState: 'idle',
    fileChanges: [
      {
        path: 'src/App.tsx',
        input: { old_string: 'old\n'.repeat(90), new_string: 'new\n'.repeat(90) },
      },
    ],
    lastActivity: 1000,
  };

  function Inside() {
    const { topByAgent, setInboxFilter } = useAttention();
    React.useEffect(() => {
      setInboxFilter('needs');
    }, [setInboxFilter]);
    const top = topByAgent.get('a1');
    return <span data-testid="top-a1">{top ? top.kind : 'none'}</span>;
  }

  function PilotBugHarness({ snaps }: { snaps: Record<string, any> }) {
    const attention = useAttentionFeed(snaps, bugAgents);
    return (
      <ConfigProvider>
        <AttentionProvider
          agents={bugAgents}
          activeAgentId="a1"
          snapshotBySession={snaps}
          inboxOpen
          openInbox={vi.fn()}
          closeInbox={vi.fn()}
          viewLevel="piloting"
          setViewLevel={vi.fn()}
          onOpenAgent={vi.fn()}
          attention={attention}
        >
          <Inside />
        </AttentionProvider>
      </ConfigProvider>
    );
  }

  it('auto-dismisses ALL active-agent items while piloting, even ones the inbox filter hides', () => {
    const { rerender } = render(<PilotBugHarness snaps={{ 'sess-1': workingSnap }} />);
    expect(screen.getByTestId('top-a1').textContent).toBe('none');
    // While piloting a1 with the 'needs' filter set, a1 finishes and leaves a
    // review-class item. Because you're actively looking at a1, the piloting
    // effect must auto-dismiss it even though 'needs' filters it out of `feed`.
    rerender(<PilotBugHarness snaps={{ 'sess-1': bigdiffSnap }} />);
    expect(screen.getByTestId('top-a1').textContent).toBe('none');
  });
});
