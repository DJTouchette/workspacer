import type {
  ManagerReplacementRequest,
  ManagerReplacementView,
} from '../../../main/shared/managerReplacement';
/** Isolated workflow fixture: every backend action is mocked; no live agents or ports. */
import React, { useEffect, useState } from 'react';
import ReactDOM from 'react-dom/client';
import { WebFontsAddon } from '@xterm/addon-web-fonts';
import '../App.css';
import { DEFAULT_CONFIG } from '../hooks/configDefaults';
import type { AgentWorkspace } from '../types/pane';
import { buildFleetMessage } from '../../../main/shared/fleetMessages';

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
let initialAgents = [
  makeAgent('worker'),
  makeAgent('manager', true),
  makeAgent('remote', false, 'fixture-peer'),
  makeAgent('manager-two', true),
  makeAgent('offline', false, 'offline-peer'),
];
const params = new URLSearchParams(location.search);
let releaseFontRelayouts = () => {};
let deferredFontRelayouts = 0;
if (params.get('fontRetirementProbe') === '1') {
  // Hold the real addon's internal await, not the pane's font loader. This
  // deterministically retires a terminal between relayout's guard and options
  // access, the window seen in the hosted handoff page error.
  const ready = document.fonts.ready;
  const gate = new Promise<FontFaceSet>((resolve) => {
    releaseFontRelayouts = () => resolve(document.fonts);
  });
  let insideRelayout = false;
  const relayout = WebFontsAddon.prototype.relayout;
  WebFontsAddon.prototype.relayout = function () {
    insideRelayout = true;
    try {
      return relayout.call(this);
    } finally {
      insideRelayout = false;
    }
  };
  Object.defineProperty(document.fonts, 'ready', {
    configurable: true,
    get() {
      if (insideRelayout) {
        deferredFontRelayouts++;
        return gate;
      }
      return ready;
    },
  });
}
if (params.get('fleet') === 'few') initialAgents = initialAgents.slice(0, 2);
if (params.get('fleet') === 'empty') initialAgents = [];
if (params.get('fleet') === 'busy')
  initialAgents.push(...Array.from({ length: 12 }, (_, i) => makeAgent(`task-${i}`)));
if (params.get('labels') === 'tasks') {
  initialAgents = initialAgents.map((agent) => ({
    ...agent,
    name: agent.manager
      ? 'Coordinate release validation'
      : `Implement retained session navigation — ${agent.id}`,
  }));
}
if (new URLSearchParams(location.search).get('missingChat') === 'worker') {
  initialAgents[0].tabs = [];
}
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
      lastActivity: a.manager ? Date.now() - 3600000 : Date.now(),
      settings: { model: 'gpt-5.4' },
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
if (params.get('longChat') === '1' && snapshots['s-manager']) {
  snapshots['s-manager'].conversation.push(
    ...Array.from({ length: 40 }, (_, i) => ({
      role: i % 2 ? 'assistant' : 'user',
      content: `Recorded fixture turn ${i}: verify retained conversation state.`,
      timestamp: 200 + i,
    })),
  );
}
snapshots['s-manager']?.conversation.push({
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
const reviewId = '11111111-1111-4111-8111-111111111111';
if (params.get('longChat') === '1')
  snapshots['s-manager']?.conversation.push({
    role: 'user',
    timestamp: 500,
    content: buildFleetMessage('worker-finished', [
      {
        sessionId: 's-worker',
        label: 'Navigation implementation',
        cwd: '/fixture/project',
        reviewEvidenceId: reviewId,
        lastReply: 'Fixture implementation report.',
      },
    ]),
  });
const sessionListeners = new Set<(id: string, snapshot: any) => void>();
const calls: { method: string; args: unknown[] }[] = [];
const handoffMode = params.get('handoff');
let replacements: ManagerReplacementView[] = handoffMode
  ? JSON.parse(sessionStorage.getItem('handoff-fixture') ?? '[]')
  : [];
const saveReplacements = () =>
  sessionStorage.setItem('handoff-fixture', JSON.stringify(replacements));
const successorSnapshot = (op: ManagerReplacementView) => {
  snapshots[op.successorSessionId] = {
    ...snapshots[op.sourceSessionId],
    sessionId: op.successorSessionId,
    managerReplacementOperationId: op.operationId,
    isWakeTarget: true,
    conversation: [
      {
        role: 'assistant',
        content: 'Fresh manager context; pending decisions retained.',
        timestamp: Date.now(),
      },
    ],
  };
};
for (const op of replacements) if (op.committed) successorSnapshot(op);
async function replacementRequest(request: ManagerReplacementRequest) {
  if (!handoffMode || handoffMode === 'unavailable')
    return {
      available: false,
      operations: [],
      error:
        'Automatic manager replacement requires an owned local desktop manager and only local workers.',
    };
  if (request.action !== 'list') calls.push({ method: 'replacement', args: [request] });
  if (request.action === 'start' && !replacements.length) {
    const op: ManagerReplacementView = {
      operationId: 'fixture-operation',
      sourceSessionId: request.sourceSessionId,
      successorSessionId: 'fixture-successor',
      paneId: request.paneId,
      workspaceId: request.workspaceId,
      phase: 'preparing',
      createdAt: Date.now(),
      updatedAt: Date.now(),
      committed: false,
      bound: false,
      artifactPath: '/fixture/project/.workspacer/manager-handoffs/operation/handoff.json',
      workerIds: ['s-worker'],
      taskIds: ['ordinary-task', 'workflow-task'],
      deliveries: [],
    };
    replacements = [op];
    saveReplacements();
    setTimeout(() => {
      if (handoffMode === 'fail') {
        op.phase = 'failed';
        op.error = 'Invalid checkpoint receipt; old manager retained.';
      } else {
        op.phase = 'binding';
        op.committed = true;
        successorSnapshot(op);
      }
      op.updatedAt = Date.now();
      saveReplacements();
    }, 300);
  } else if (request.action === 'bind') {
    const op = replacements[0];
    if (op.phase === 'binding') {
      op.bound = true;
      op.phase = handoffMode === 'ambiguous' ? 'recovery-required' : 'complete';
      if (handoffMode === 'ambiguous') {
        op.error = 'Kickoff acknowledgement is uncertain. No automatic replay.';
        op.deliveries = [
          {
            id: 'kickoff',
            kind: 'kickoff',
            text: 'Preserved kickoff with pending decisions',
            status: 'uncertain',
          },
        ];
      }
      saveReplacements();
    }
  } else if (request.action === 'resolve-delivery') {
    replacements[0].deliveries[0].status = 'accepted';
    replacements[0].phase = 'complete';
    replacements[0].error = undefined;
    saveReplacements();
  }
  return { available: true, operations: structuredClone(replacements) };
}
let failTerminate = false;
let sendMode: 'accept' | 'defer' | 'reject' | 'throw' = 'accept';
let settleSend: (() => void) | undefined;
let connected = true;
const hubListeners = new Set<(status: { connected: boolean }) => void>();
const record = async (method: string, ...args: unknown[]) => {
  calls.push({ method, args });
  if (failTerminate) throw new Error('Fixture authority denied termination');
};
(window as any).electronAPI = new Proxy(
  {
    platform: 'linux',
    // These pre-inbox fleet cases deliberately exercise the legacy/missing
    // capability path. Modern capture is covered by firstUse and the real bridge.
    managerRequestPrepare:
      params.get('capture') === 'remote'
        ? async () => ({ available: false, reason: 'Remote request capture unavailable' })
        : undefined,
    managerReplacement: replacementRequest,
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
      if (sendMode === 'throw') throw new Error('Fixture message transport unavailable');
      if (sendMode === 'reject') return { ok: false, mode: 'stopped' };
      if (sendMode === 'defer')
        await new Promise<void>((resolve) => {
          settleSend = resolve;
        });
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
    fleetReviewRead: async (request: any) => ({
      ok: request.ownerSessionId === 's-manager' && request.evidenceId === reviewId,
      evidence: {
        id: reviewId,
        ownerSessionId: 's-manager',
        workerSessionId: 's-worker',
        projectRoot: '/fixture/project',
        allocatedCwd: '/fixture/project',
        branch: 'wks/fixture',
        baseCommit: 'a'.repeat(40),
        headCommit: 'b'.repeat(40),
        capturedAt: '2026-09-06T00:00:00Z',
        lifecycle: 'turn-ended',
        availability: 'captured',
        files: [
          {
            path: 'src/navigation.ts',
            status: 'M',
            diff: request.file
              ? '--- a/src/navigation.ts\n+++ b/src/navigation.ts\n@@ -1 +1 @@\n-old\n+retained\n'
              : undefined,
          },
        ],
      },
    }),
  },
  {
    get(target: any, name: string) {
      if (name in target) return target[name];
      if (name.startsWith('managerRequest')) return undefined;
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
  const [pilotId, setPilotId] = useState('manager');
  useEffect(() => {
    manager.loadAgentsFromSession(initialAgents, 'manager');
  }, []);
  useEffect(() => {
    (window as any).fleetHarness = {
      releaseFontRelayouts,
      fontRelayouts: () => deferredFontRelayouts,
      calls,
      sendMode: (mode: typeof sendMode) => {
        sendMode = mode;
      },
      settleSend: () => settleSend?.(),
      echo: (id: string, text: string) => {
        const snapshot = {
          ...snapshots[id],
          conversation: [
            ...snapshots[id].conversation,
            { role: 'user', content: text, timestamp: Date.now() },
          ],
        };
        snapshots = { ...snapshots, [id]: snapshot };
        setSnapshots(snapshots);
        sessionListeners.forEach((fn) => fn(id, snapshot));
      },
      repeatSnapshot: (id: string) =>
        sessionListeners.forEach((fn) =>
          fn(id, { ...snapshots[id], conversation: [...snapshots[id].conversation] }),
        ),
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
      agentRecords: () => manager.agents,
      pilot: (id = 'manager') => {
        setPilotId(id);
        setView('piloting');
      },
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
          .flatMap((a) =>
            a.tabs
              .flatMap((t) => t.panes)
              .filter((p) => p.type === 'claude')
              .map((p) => (
                <div
                  key={p.id}
                  style={{
                    display: view === 'piloting' && a.id === pilotId ? 'block' : 'none',
                    height: '100%',
                  }}
                >
                  <ClaudePane
                    manager={a.manager}
                    workspaceId={a.id}
                    paneId={p.id}
                    title={a.name}
                    cwd={a.cwd}
                    provider="codex"
                    transport="stream"
                    attachSessionId={a.sessionId}
                    isActive={view === 'piloting' && a.id === pilotId}
                  />
                </div>
              )),
          )}
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
