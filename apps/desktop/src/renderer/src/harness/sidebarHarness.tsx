/**
 * Standalone SideBar harness — renders the expanded sidebar with fabricated
 * agents/snapshots covering every card state, for design screenshots without
 * launching Electron (or fighting the live claudemon's ports).
 *
 * Served by the normal Vite dev server: /sidebar-harness.html
 * Not part of the app build — nothing imports this except the harness page.
 */
/* eslint-disable @typescript-eslint/no-explicit-any */
import React from 'react';
import ReactDOM from 'react-dom/client';
import '../App.css';

// Minimal electronAPI stub BEFORE importing anything that touches it at
// module scope. Proxy: any method returns a quiet promise; on* subscriptions
// return an unsubscribe.
(window as any).electronAPI = new Proxy(
  {
    platform: 'linux',
    getConfig: async () => ({}),
    reloadConfig: async () => ({}),
    saveConfig: async () => ({}),
    // Show the hub as live so the footer reads "hub" (green), not "hub offline".
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
const { default: SideBar } = await import('../components/SideBar');
const { AttentionProvider } = await import('../contexts/AttentionContext');
const { ConfigProvider } = await import('../contexts/ConfigContext');
const { useAttentionFeed } = await import('../hooks/useAttentionFeed');
const { resolveTheme, applyTheme } = await import('../themes');

applyTheme(resolveTheme('everforest'));
document.documentElement.style.setProperty('--wks-font-mono', 'ui-monospace, monospace');

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
  { id: 'global', name: 'Overview', cwd: '', global: true, tabs: tabs('g'), activeTabId: 'tab-g' },
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
    name: 'os-prep',
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
  {
    id: 'agent-docs',
    name: 'docs-site',
    cwd: '/work/docs',
    lastSessionId: 's-docs-old',
    tabs: tabs('d'),
    activeTabId: 'tab-d',
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
  // Idle after a finished turn — history lives on the conversation turns.
  's-workspacer': {
    ...base,
    sessionId: 's-workspacer',
    ptyId: 's-workspacer',
    ambientState: 'idle',
    lastActivity: now - 4 * min,
    conversation: [
      { role: 'user', content: 'final prep pass', timestamp: now - 9 * min },
      {
        role: 'assistant',
        content: 'Running the secret scanner over the full history first.',
        timestamp: now - 8 * min,
      },
      {
        role: 'assistant',
        content: '',
        timestamp: now - 6 * min,
        toolCalls: [tc('w1', 'Edit', { file_path: 'landing/index.html' }, now - 6 * min)],
      },
      {
        role: 'assistant',
        content: 'All three prep items are done — history clean, downloads wired.',
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
  // Working — completed Read, then an active Bash (green line).
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
  // Waiting on an approval — amber.
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
        content: 'I need to run the coverage query against the prod snapshot.',
        timestamp: now - 4 * min,
      },
      {
        role: 'assistant',
        content: '',
        timestamp: now - 3.5 * min,
        toolCalls: [tc('r1', 'Read', { file_path: 'schema/coverage.sql' }, now - 3.5 * min)],
      },
    ],
    statusLine: { modelDisplay: 'GPT-5.2', contextUsedPct: 8, costUSD: 0.34 },
  },
  // Working, no tools yet — freshest line is its own words.
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

const statusBySession: Record<string, any> = Object.fromEntries(
  Object.values(snapshotBySession).map((s: any) => [s.sessionId, s.ambientState]),
);

// RECENT rows show the provider's auto-generated conversation title (name is
// the spawn-default dirname, so recentSessionLabel prefers the title).
const recent = (
  sessionId: string,
  provider: string,
  cwd: string,
  title: string,
  ageMin: number,
): any => ({
  sessionId,
  provider,
  cwd,
  mode: 'stopped',
  transport: provider === 'claude' ? 'stream' : 'pty',
  archived: false,
  updatedAt: now - ageMin * min,
  startedAt: now - (ageMin + 20) * min,
  name: cwd.split('/').filter(Boolean).pop() || '',
  title,
  model: provider === 'claude' ? 'opus' : '',
  costUSD: 1.2,
});

const recentSessions: any[] = [
  recent('old-1', 'claude', '/work/infra-tf', 'Split the Terraform state per environment', 3 * 60),
  recent('old-2', 'codex', '/work/api', 'Add rate limiting to the public endpoints', 26 * 60),
  recent('old-3', 'claude', '/work/docs-site', 'Rewrite the getting-started guide', 2 * 24 * 60),
  recent('old-4', 'claude', '/work/release', 'Cut the v0.14 release and tag it', 9 * 24 * 60),
];

const noop = () => {};

// Scenarios: default shows every card state (design review); 'recent' trims to
// one working + one done card so the RECENT list (with conversation titles)
// stays prominent — that's the marketing shot for landing/shots/sidebar-recent.
const scenario = new URLSearchParams(window.location.search).get('scenario');
const shownAgents =
  scenario === 'recent'
    ? agents.filter((a) => ['global', 'agent-prep', 'agent-workspacer'].includes(a.id))
    : agents;

function Harness() {
  const attention = useAttentionFeed(snapshotBySession, shownAgents);
  return (
    <ConfigProvider>
      <AttentionProvider
        agents={shownAgents}
        activeAgentId="agent-prep"
        snapshotBySession={snapshotBySession}
        inboxOpen={false}
        openInbox={noop}
        closeInbox={noop}
        viewLevel="piloting"
        setViewLevel={noop}
        onOpenAgent={noop}
        attention={attention}
      >
        <div className="app-root" style={{ height: '100vh', display: 'flex' }}>
          <SideBar
            agents={shownAgents}
            activeAgentId="agent-prep"
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
            recentSessions={recentSessions}
            onResumeSession={noop}
          />
        </div>
      </AttentionProvider>
    </ConfigProvider>
  );
}

ReactDOM.createRoot(document.getElementById('root')!).render(<Harness />);
