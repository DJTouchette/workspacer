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

  it('renders a Fleet Deck card per real agent, excluding the global workspace', () => {
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

  it('flips a Fleet Deck card in place into the live InspectorCard, then collapses', () => {
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
