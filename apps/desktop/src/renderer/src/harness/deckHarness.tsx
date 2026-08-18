/**
 * Standalone Fleet Deck harness — renders the deck with fabricated agents for
 * design screenshots without launching Electron (or fighting the live
 * claudemon's ports).
 *
 * Served by the normal Vite dev server: /deck-harness.html
 * Not part of the app build — nothing imports this except the harness page.
 *
 * Sibling of `sidebarHarness.tsx`. The two deliberately keep their own fixtures
 * so a tweak made while reviewing one never silently restages the other's
 * published screenshots.
 *
 * THIS HARNESS IS THE SOURCE OF TWO PUBLISHED SHOTS — edit the fixture with that
 * in mind, and restage both if you change it:
 *   landing/shots/fleet-deck.webp    ← 1600x862, no query string
 *   landing/shots/triage-inbox.webp  ← 1600x862, ?inbox=1
 * Both are captured straight from `/usr/bin/chromium --headless --screenshot`
 * against `npx vite --port 5199 --strictPort` run from `src/renderer` (a spare
 * port, so it never collides with a live `dev:renderer` on 5173), then
 * `magick <png> -quality 86 -define webp:method=6 <webp>`.
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

// The real default config, not a hand-written stub. ConfigProvider passes the
// bridge's object straight through without merging defaults, and the chrome
// reads nested fields directly (`config.ui.navBarHeight`, `config.terminal
// .shells`, ...), so anything less than the real defaults fails one field at a
// time. This is a static import on purpose — it touches no electronAPI.
import { DEFAULT_CONFIG } from '../hooks/configDefaults';

// Minimal electronAPI stub BEFORE importing anything that touches it at module
// scope. Proxy: any method returns a quiet promise; on* subscriptions return an
// unsubscribe.
(window as any).electronAPI = new Proxy(
  {
    platform: 'linux',
    getConfig: async () => ({
      ...DEFAULT_CONFIG,
      ui: { ...DEFAULT_CONFIG.ui, mode: 'fleet', theme: params.get('theme') || 'everforest' },
      panes: { ...DEFAULT_CONFIG.panes, viewLevel: 'fleet' },
    }),
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
const { default: InboxDrawer } = await import('../components/InboxDrawer');
const { default: NavBar } = await import('../components/NavBar');
const { default: SideBar } = await import('../components/SideBar');
const { SIDEBAR_RAIL_WIDTH } = await import('../lib/sidebarWidth');
const { AttentionProvider } = await import('../contexts/AttentionContext');
const { NotificationsProvider } = await import('../contexts/NotificationsContext');
const { ConfigProvider } = await import('../contexts/ConfigContext');
const { useAttentionFeed } = await import('../hooks/useAttentionFeed');
const { resolveNavHeight } = await import('../lib/layoutUtils');
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

const statusBySession: Record<string, any> = Object.fromEntries(
  Object.values(snapshotBySession).map((s: any) => [s.sessionId, s.ambientState]),
);

const navTabs = [
  {
    id: 'tab-overview',
    title: 'Overview',
    panes: [{ id: 'pane-overview', type: 'overview' as const, title: 'Overview' }],
    activePaneId: 'pane-overview',
  },
  {
    id: 'tab-ask',
    title: 'Ask',
    panes: [{ id: 'pane-ask', type: 'ask' as const, title: 'Ask' }],
    activePaneId: 'pane-ask',
  },
];

/**
 * The real fleet chrome, composed the way `App.tsx` does it: NavBar across the
 * top with `leftOffset`, the sidebar as its collapsed rail, and the deck inset
 * by both. Reproducing the composition (rather than screenshotting the deck on
 * its own) is what makes this usable for `landing/shots/` — the rail is also
 * the point, since at fleet altitude the sidebar collapses.
 */
function Harness() {
  const navHeight = resolveNavHeight(undefined, false);
  return (
    <div className="app-root" style={{ height: '100vh' }}>
      <SideBar
        agents={agents}
        activeAgentId="agent-workspacer"
        statusBySession={statusBySession}
        snapshotBySession={snapshotBySession}
        onSelectAgent={noop}
        onSpawnAgent={noop}
        onTerminateAgent={noop}
        onRenameAgent={noop}
        onToggleCollapse={noop}
        onOpenRemote={noop}
        viewLevel="fleet"
        collapsed
        onOpenHistory={noop}
      />
      <NavBar
        tabs={navTabs as any}
        activeTabId="tab-overview"
        onTabClick={noop}
        onAddTab={noop}
        onCloseTab={noop}
        onRenameTab={noop}
        onSplitTab={noop}
        onMoveTab={noop}
        leftOffset={SIDEBAR_RAIL_WIDTH}
        cwd="/home/djtouchette/Work/worky/workspacer"
      />
      <FleetDeck top={navHeight} left={SIDEBAR_RAIL_WIDTH} />
      {/* `?inbox=1` docks the Triage Inbox over the deck. It is propless — it
          reads inboxOpen and the feed off AttentionContext — so the only thing
          that opens it is the provider below. */}
      <InboxDrawer />
    </div>
  );
}

function Root() {
  const attention = useAttentionFeed(snapshotBySession, agents);
  return (
    <ConfigProvider>
      <NotificationsProvider>
        <AttentionProvider
          agents={agents}
          activeAgentId="agent-workspacer"
          snapshotBySession={snapshotBySession}
          inboxOpen={params.get('inbox') === '1'}
          openInbox={noop}
          closeInbox={noop}
          viewLevel="fleet"
          setViewLevel={noop}
          onOpenAgent={noop}
          attention={attention}
        >
          <Harness />
        </AttentionProvider>
      </NotificationsProvider>
    </ConfigProvider>
  );
}

ReactDOM.createRoot(document.getElementById('root')!).render(<Root />);
