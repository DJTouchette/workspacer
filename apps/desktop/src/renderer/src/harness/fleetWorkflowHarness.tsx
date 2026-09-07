/** Isolated workflow fixture: every backend action is mocked; no live agents or ports. */
import React, { useEffect, useState } from 'react';
import ReactDOM from 'react-dom/client';
import '../App.css';
import { DEFAULT_CONFIG } from '../hooks/configDefaults';
import type { AgentWorkspace } from '../types/pane';

const theme = new URLSearchParams(location.search).get('theme') ?? 'everforest';
const config = {
  ...DEFAULT_CONFIG,
  ui: { ...DEFAULT_CONFIG.ui, mode: 'fleet', theme },
  claude: { ...DEFAULT_CONFIG.claude, transport: 'stream' },
};
const makeAgent = (id: string, manager = false, hub?: string): AgentWorkspace => ({
  id,
  name:
    id === 'manager' ? 'Fleet Manager' : id === 'manager-two' ? 'Release manager' : `${id} agent`,
  manager,
  hub,
  sessionId: `s-${id}`,
  cwd: '/fixture/project',
  provider: 'codex',
  tabs: [
    {
      id: `t-${id}`,
      title: id,
      activePaneId: `p-${id}`,
      panes: [
        {
          id: `p-${id}`,
          type: 'claude',
          title: id,
          attachSessionId: `s-${id}`,
          transport: 'stream',
          provider: 'codex',
        },
      ],
    },
  ],
  activeTabId: `t-${id}`,
});
const initialAgents = [
  makeAgent('worker'),
  makeAgent('manager', true),
  makeAgent('remote', false, 'fixture-peer'),
  makeAgent('manager-two', true),
  makeAgent('offline', false, 'offline-peer'),
];
let snapshots: Record<string, any> = Object.fromEntries(
  initialAgents.map((a) => [
    a.sessionId!,
    {
      sessionId: a.sessionId,
      cwd: a.cwd,
      provider: 'codex',
      transport: 'stream',
      hub: a.hub,
      hubOffline: a.id === 'offline',
      status: 'active',
      ambientState: a.manager ? 'idle' : 'streaming',
      lastActivity: Date.now(),
      activeToolCalls: [],
      completedToolCalls: [],
      fileChanges: [],
      subagents: [],
      workflows: [],
      pendingApproval: null,
      pendingQuestions: null,
      conversation: [
        { role: 'user', content: `Check ${a.name}`, timestamp: 100 },
        {
          role: 'assistant',
          content: a.manager
            ? 'I have the fleet in view. Select a worker to read its live chat, or send me the next task.'
            : `Working on ${a.name}. The transcript and composer stay in Fleet.`,
          timestamp: 101,
        },
      ],
    },
  ]),
);
snapshots['s-manager'].conversation.push({
  role: 'assistant',
  timestamp: 102,
  content:
    '```wks-html-card\n' +
    JSON.stringify({
      v: 1,
      title: 'Fleet notes',
      bodyHtml: '<label>Note <input aria-label="Inline note" /></label>',
      fallback: 'Fleet notes remain with this session.',
      actions: [{ kind: 'fill_composer', label: 'Use notes', text: 'Follow up on these notes' }],
    }) +
    '\n```',
});
const sessionListeners = new Set<(id: string, snapshot: any) => void>();
const calls: { method: string; args: unknown[] }[] = [];
let failTerminate = false;
let connected = true;
const hubListeners = new Set<(status: { connected: boolean }) => void>();
const record = async (method: string, ...args: unknown[]) => {
  calls.push({ method, args });
  if (failTerminate) throw new Error('Fixture authority denied termination');
};
(window as any).electronAPI = new Proxy(
  {
    platform: 'linux',
    getConfig: async () => config,
    reloadConfig: async () => config,
    saveConfig: async () => config,
    getHubStatus: async () => ({ connected }),
    onHubStatus: (fn: (status: { connected: boolean }) => void) => {
      hubListeners.add(fn);
      return () => hubListeners.delete(fn);
    },
    attachClaude: async (_pane: string, id: string) => id,
    detachClaude: async () => {},
    getClaudeSession: async (id: string) => snapshots[id],
    getAllClaudeSessions: async () => Object.values(snapshots),
    listLiveClaudeSessionIds: async () => Object.keys(snapshots),
    onClaudeSessionUpdate: (fn: (id: string, s: any) => void) => {
      sessionListeners.add(fn);
      return () => sessionListeners.delete(fn);
    },
    claudeMessage: async (id: string, text: string) => {
      calls.push({ method: 'message', args: [id, text] });
      return { ok: true };
    },
    claudeApprove: async (id: string, answer: string) => {
      calls.push({ method: 'approve', args: [id, answer] });
    },
    claudeAnswer: async (id: string, answer: unknown) => {
      calls.push({ method: 'answer', args: [id, answer] });
      return { ok: true };
    },
    claudeClose: (id: string) => record('close', id),
    claudeSignal: (id: string, signal: string) => record('signal', id, signal),
    federationConversation: async () => null,
    libraryList: async () => [],
  },
  {
    get(target: any, name: string) {
      if (name in target) return target[name];
      if (name.startsWith('on')) return () => () => {};
      return () => Promise.resolve(undefined);
    },
  },
);

const { default: FleetDeck } = await import('../components/FleetDeck');
const { default: ClaudePane } = await import('../panes/ClaudePane');
const { ConfigProvider } = await import('../contexts/ConfigContext');
const { AttentionProvider } = await import('../contexts/AttentionContext');
const { NotificationsProvider } = await import('../contexts/NotificationsContext');
const { useAttentionFeed } = await import('../hooks/useAttentionFeed');
const { useAgentManager } = await import('../hooks/useAgentManager');
const { resolveTheme, applyTheme } = await import('../themes');
applyTheme(resolveTheme(theme));
const noop = () => {};
function Harness() {
  const manager = useAgentManager();
  const [currentSnapshots, setSnapshots] = useState(snapshots);
  const [view, setView] = useState<'fleet' | 'piloting'>('fleet');
  const [recent, setRecent] = useState(false);
  useEffect(() => {
    manager.loadAgentsFromSession(initialAgents, 'manager');
  }, []);
  useEffect(() => {
    (window as any).fleetHarness = {
      calls,
      connection: (value: boolean) => {
        connected = value;
        hubListeners.forEach((fn) => fn({ connected }));
      },
      failTerminate: (fail: boolean) => {
        failTerminate = fail;
      },
      activity: (id: string, patch: any) => {
        snapshots = { ...snapshots, [id]: { ...snapshots[id], ...patch } };
        setSnapshots(snapshots);
        sessionListeners.forEach((fn) => fn(id, snapshots[id]));
      },
      addWorker: () =>
        manager.loadAgentsFromSession([...manager.agents, makeAgent('new-worker')], 'manager'),
      agents: () => manager.agents.map((a) => a.id),
    };
  }, [manager.agents]);
  const attention = useAttentionFeed(currentSnapshots, manager.agents);
  return (
    <AttentionProvider
      agents={manager.agents}
      activeAgentId="manager"
      snapshotBySession={currentSnapshots}
      inboxOpen={false}
      openInbox={noop}
      closeInbox={noop}
      viewLevel={view}
      setViewLevel={setView}
      onOpenAgent={() => setView('piloting')}
      attention={attention}
    >
      <div className="app-root" style={{ height: '100vh', fontFamily: 'var(--wks-font-sans)' }}>
        {manager.agents
          .filter((a) => !a.global)
          .map((a) => (
            <div
              key={a.id}
              style={{
                display: view === 'piloting' && a.id === 'manager' ? 'block' : 'none',
                height: '100%',
              }}
            >
              <ClaudePane
                paneId={`p-${a.id}`}
                title={a.name}
                cwd={a.cwd}
                provider="codex"
                transport="stream"
                attachSessionId={a.sessionId}
                isActive={view === 'piloting' && a.id === 'manager'}
              />
            </div>
          ))}
        {view === 'fleet' ? (
          <FleetDeck
            top={0}
            left={0}
            onOpenRecentAgents={() => setRecent(true)}
            onEnsureAgentChat={manager.ensureAgentChat}
            onTerminateAgent={async (id) => {
              await manager.terminateAgent(id);
            }}
          />
        ) : (
          <button onClick={() => setView('fleet')}>Return to Fleet</button>
        )}
        {recent && (
          <div
            role="dialog"
            aria-label="Recent agents"
            style={{
              position: 'fixed',
              inset: '20%',
              zIndex: 300,
              background: 'var(--wks-bg-surface)',
              padding: 20,
            }}
          >
            Recent agents destination<button onClick={() => setRecent(false)}>Close history</button>
          </div>
        )}
      </div>
    </AttentionProvider>
  );
}
ReactDOM.createRoot(document.getElementById('root')!).render(
  <ConfigProvider>
    <NotificationsProvider>
      <Harness />
    </NotificationsProvider>
  </ConfigProvider>,
);
