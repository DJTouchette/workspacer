/**
 * Standalone Fleet Deck harness — renders the deck with fabricated agents for
 * design screenshots without launching Electron (or fighting the live
 * claudemon's ports).
 *
 * Served by the normal Vite dev server: /deck-harness.html
 * Not part of the app build — nothing imports this except the harness page.
 *
 * Sibling of `sidebarHarness.tsx`. The two deliberately keep their own fixtures:
 * the sidebar one is the source for the marketing shots in `landing/shots/`, and
 * coupling them means a tweak made for a design review silently restages
 * published screenshots.
 *
 * The fixture exists to exercise the cases that make the deck LOOK busy, which
 * is what this harness is for:
 *   - one agent blocked on a multi-option question — the tall card that used to
 *     make the grid ragged and the one that stacks card → footer → picker →
 *     option row (the deepest nesting in the app)
 *   - one blocked on an approval
 *   - two short cards, so uneven row heights are visible
 *
 * `?theme=<id>` switches theme (default everforest). Worth flipping through
 * everforest / kanagawa / one-dark when touching `Surface`: those three put
 * `--wks-bg-surface` within a few RGB units of `--wks-bg-base`, so they are
 * where a fill-only elevation goes invisible.
 */
/* eslint-disable @typescript-eslint/no-explicit-any */
import React from 'react';
import ReactDOM from 'react-dom/client';
import '../App.css';

const params = new URLSearchParams(window.location.search);

// Minimal electronAPI stub BEFORE importing anything that touches it at module
// scope. Proxy: any method returns a quiet promise; on* subscriptions return an
// unsubscribe.
(window as any).electronAPI = new Proxy(
  {
    platform: 'linux',
    getConfig: async () => ({}),
    reloadConfig: async () => ({}),
    saveConfig: async () => ({}),
    getHubStatus: async () => ({ connected: true }),
  },
  {
    get(target: any, prop: string) {
      if (prop in target) return target[prop];
      if (typeof prop === 'string' && prop.startsWith('on')) {
        return () => () => {};
      }
      return () => Promise.resolve(undefined);
    },
  },
);

// Deferred imports so the stub is installed first.
const { default: FleetDeck } = await import('../components/FleetDeck');
const { AttentionProvider } = await import('../contexts/AttentionContext');
const { NotificationsProvider } = await import('../contexts/NotificationsContext');
const { ConfigProvider } = await import('../contexts/ConfigContext');
const { useAttentionFeed } = await import('../hooks/useAttentionFeed');
const { resolveTheme, applyTheme } = await import('../themes');

applyTheme(resolveTheme(params.get('theme') || 'everforest'));

const now = Date.now();
const min = 60_000;

const tabs = (id: string) => [
  {
    id: `tab-${id}`,
    title: 'Claude',
    panes: [{ id: `pane-${id}`, type: 'claude' as const, title: 'Claude' }],
    activePaneId: `pane-${id}`,
  },
];

const agents: any[] = [
  {
    id: 'agent-workspacer',
    name: 'workspacer',
    cwd: '/work/workspacer',
    sessionId: 's-workspacer',
    model: 'fable',
    tabs: tabs('w'),
    activeTabId: 'tab-w',
  },
  {
    id: 'agent-prep',
    name: 'prep',
    cwd: '/work/prep',
    sessionId: 's-prep',
    model: 'opus',
    tabs: tabs('p'),
    activeTabId: 'tab-p',
  },
  {
    id: 'agent-rivet',
    name: 'rivet',
    cwd: '/work/rivet',
    provider: 'codex',
    sessionId: 's-rivet',
    tabs: tabs('r'),
    activeTabId: 'tab-r',
  },
  {
    id: 'agent-recon',
    name: 'recon',
    cwd: '/work/recon',
    sessionId: 's-recon',
    model: 'opus',
    tabs: tabs('c'),
    activeTabId: 'tab-c',
  },
];

const tc = (
  id: string,
  name: string,
  input: any,
  at: number,
  status: 'running' | 'complete' = 'complete',
): any => ({
  id,
  name,
  input,
  status,
  startedAt: at,
  completedAt: status === 'complete' ? at + 400 : undefined,
});

const base = {
  cwd: '/w',
  status: 'active',
  activeToolCalls: [],
  completedToolCalls: [],
  fileChanges: [],
  pendingApproval: null,
  pendingQuestions: null,
  subagents: [],
  workflows: [],
  totalToolCalls: 4,
  usage: null,
};

const snapshotBySession: Record<string, any> = {
  // THE TALL CARD. Blocked on a 4-option question — this is the one that made
  // the grid ragged (≈700px against ≈270px siblings) and it is also the deepest
  // surface stack in the app.
  's-workspacer': {
    ...base,
    sessionId: 's-workspacer',
    ptyId: 's-workspacer',
    ambientState: 'waiting_input',
    lastActivity: now - 4 * min,
    pendingQuestions: [
      {
        question: 'Workspacer is ready to go public — what should we do next?',
        header: 'Next move',
        options: [
          {
            label: 'Tag v0.124.0 and ship it (Recommended)',
            description:
              'Push the commits, tag a release so CI builds installers for all three OSes, and publish the draft so the landing page download button goes live.',
          },
          {
            label: 'One more design pass',
            description: 'Give another pane the Inspector treatment before the public debut.',
          },
          {
            label: 'Write the announcement',
            description:
              'Draft the launch post: control plane for a fleet of coding agents, alpha, source-available.',
          },
          {
            label: 'Just keep hacking',
            description: 'Public can wait — spawn an agent and build the next feature.',
          },
        ],
      },
    ],
    fileChanges: [{ path: 'landing/index.html', additions: 14, deletions: 2 }],
    conversation: [
      { role: 'user', content: 'final prep pass', timestamp: now - 9 * min },
      {
        role: 'assistant',
        content: 'All three prep items are done — history is clean, downloads wired.',
        timestamp: now - 4 * min,
      },
    ],
    statusLine: {
      modelDisplay: 'Fable 5',
      contextUsedPct: 32,
      totalInputTokens: 96000,
      totalOutputTokens: 145000,
      costUSD: 46.24,
    },
  },
  // Working — completed Read, then an active Bash.
  's-prep': {
    ...base,
    sessionId: 's-prep',
    ptyId: 's-prep',
    ambientState: 'streaming',
    lastActivity: now - 20_000,
    completedToolCalls: [
      tc('p1', 'Read', { file_path: '.github/workflows/release.yml' }, now - 2 * min),
    ],
    activeToolCalls: [
      tc('p2', 'Bash', { command: 'gh release list --limit 5' }, now - 30_000, 'running'),
    ],
    conversation: [
      { role: 'user', content: 'prep status', timestamp: now - 5 * min },
      {
        role: 'assistant',
        content: 'Checking the release pipeline before I summarize.',
        timestamp: now - 3 * min,
      },
    ],
    statusLine: {
      modelDisplay: 'Opus 4.8',
      contextUsedPct: 17,
      totalInputTokens: 33000,
      totalOutputTokens: 4200,
      costUSD: 0.21,
    },
  },
  // Blocked on an approval — the ApprovalPrompt surface stack.
  's-rivet': {
    ...base,
    sessionId: 's-rivet',
    ptyId: 's-rivet',
    provider: 'codex',
    ambientState: 'waiting_approval',
    lastActivity: now - 3 * min,
    pendingApproval: {
      toolName: 'Bash',
      toolInput: { command: 'psql prod -c "select count(*) from coverage"' },
      timestamp: now - 3 * min,
    },
    conversation: [
      { role: 'user', content: 'verify coverage', timestamp: now - 7 * min },
      {
        role: 'assistant',
        content: 'I need to run the coverage query against the prod snapshot to be sure.',
        timestamp: now - 4 * min,
      },
    ],
    statusLine: { modelDisplay: 'GPT-5.2', contextUsedPct: 8, costUSD: 0.34 },
  },
  // Working, no tools yet — short card, for row-height contrast.
  's-recon': {
    ...base,
    sessionId: 's-recon',
    ptyId: 's-recon',
    ambientState: 'thinking',
    lastActivity: now - 10_000,
    conversation: [
      { role: 'user', content: 'map the cache', timestamp: now - 2 * min },
      {
        role: 'assistant',
        content: 'Mapping the cache layout — three tiers, symbol index is the hot one.',
        timestamp: now - 40_000,
      },
    ],
    statusLine: { modelDisplay: 'Opus 4.8', contextUsedPct: 3, costUSD: 0.05 },
  },
};

const noop = () => {};

function Harness() {
  const attention = useAttentionFeed(snapshotBySession, agents);
  return (
    <ConfigProvider>
      <NotificationsProvider>
        <AttentionProvider
          agents={agents}
          activeAgentId="agent-workspacer"
          snapshotBySession={snapshotBySession}
          inboxOpen={false}
          openInbox={noop}
          closeInbox={noop}
          viewLevel="fleet"
          setViewLevel={noop}
          onOpenAgent={noop}
          attention={attention}
        >
          {/* top/left mimic the navbar + collapsed rail the deck sits inside.
              74 is SIDEBAR_RAIL_WIDTH — at fleet altitude the sidebar collapses,
              which is the layout this harness is here to show. */}
          <div className="app-root" style={{ height: '100vh' }}>
            <FleetDeck top={0} left={74} />
          </div>
        </AttentionProvider>
      </NotificationsProvider>
    </ConfigProvider>
  );
}

ReactDOM.createRoot(document.getElementById('root')!).render(<Harness />);
