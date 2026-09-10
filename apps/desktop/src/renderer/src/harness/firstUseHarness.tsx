import {
  buildFleetMessage,
  type FleetMessageKind,
  type FleetMessageEntry,
} from '../../../main/shared/fleetMessages';
/** Fresh-profile production App. All host/provider boundaries are in-memory;
 * never installs a backend, opens a provider, or reads a user's profile. */
import React from 'react';
import ReactDOM from 'react-dom/client';
import '../App.css';
import { CONFIG_DEFAULTS } from '../hooks/configDefaults.generated';
const params = new URLSearchParams(location.search);
let config: any = structuredClone(CONFIG_DEFAULTS);
if (params.get('managerProvider') === 'codex') config.agents.managerProvider = 'codex';
config.ui = { ...config.ui, onboardingDismissed: false, theme: params.get('theme') ?? 'light' };
const calls: { method: string; args: any[] }[] = [];
let mode = params.get('spawn') ?? 'reject';
let detectionMode = params.get('providers') ?? 'installed';
let runtime = true;
let readiness = params.get('runtime') ?? 'unknown';
const folderPending = new Map<string, (info: any) => void>();
const restored = sessionStorage.getItem('first-use-restart');
const restart = restored ? JSON.parse(restored) : null;
if (restart) config = restart.config;
const snapshots: Record<string, any> = restart?.snapshots ?? {};
const listeners = new Set<(id: string, snapshot: any) => void>();
const noticeListeners = new Set<(notice: any) => void>();
const configListeners = new Set<(config: any) => void>();
let layout: any = null;
let sequence = 0;
const captureMode = params.get('capture') ?? 'default';
const requests = new Map<string, { requestId: string; owner: string; text: string; bootstrap: boolean; delivery: 'pending' | 'accepted' }>();
function merge(target: any, patch: any): any {
  for (const [key, value] of Object.entries(patch)) {
    target[key] =
      value && typeof value === 'object' && !Array.isArray(value)
        ? merge(target[key] ?? {}, value)
        : value;
  }
  return target;
}
function record(method: string, ...args: any[]) {
  calls.push({ method, args: structuredClone(args) });
}
function emit(id: string) {
  for (const fn of listeners) fn(id, structuredClone(snapshots[id]));
}
const api = {
  platform: 'linux',
  managerRequestPrepare: ['legacy', 'missing'].includes(captureMode) ? undefined : async (owner: string, text: string, bootstrap = false) => {
    if (captureMode === 'remote') return { available: false, reason: 'Remote request capture unavailable' };
    if (!snapshots[owner]) throw new Error('Unknown request owner');
    const requestId = crypto.randomUUID();
    requests.set(requestId, { requestId, owner, text, bootstrap, delivery: 'pending' });
    record('managerRequestPrepare', owner, text, bootstrap, requestId);
    return { available: true, requestId, delivery: 'pending' };
  },
  onSystemNotice: (fn: (notice: any) => void) => {
    noticeListeners.add(fn);
    return () => noticeListeners.delete(fn);
  },
  openExternalUrl: async (url: string) => record('openExternalUrl', url),
  getConfig: async () => structuredClone(config),
  reloadConfig: async () => structuredClone(config),
  onConfigChanged: (fn: (cfg: any) => void) => {
    configListeners.add(fn);
    return () => configListeners.delete(fn);
  },
  saveConfig: async (patch: any) => {
    record('saveConfig', patch);
    merge(config, patch);
    for (const fn of configListeners) fn(structuredClone(config));
    return structuredClone(config);
  },
  getCwd: async () => '/fixture/project',
  getAppCwd: async () => '/fixture/project',
  getSupervisorHome: async () => '/fixture/.workspacer',
  getHubStatus: async () => ({ connected: runtime }),
  providerReadiness: async (provider: string, check = false) => {
    record('providerReadiness', provider, check);
    return { state: params.get('providerReadiness') ?? 'unchecked' };
  },
  agentRuntimeStatus: async () => {
    record('agentRuntimeStatus');
    if (readiness === 'unknown') return undefined;
    return {
      claudemon: readiness === 'down' ? 'failed' : readiness === 'starting' ? 'starting' : 'ready',
      hub: readiness === 'degraded' ? 'failed' : 'ready',
      facade: readiness === 'degraded' ? 'failed' : 'ready',
    };
  },
  federationPeers: async () =>
    params.has('remote')
      ? [{ hubId: 'remote-fixture', name: 'Remote fixture', connected: true }]
      : [],
  layoutGet: async () => ({ version: restart ? 1 : 0, data: restart?.layout ?? null }),
  layoutSet: async (data: any) => {
    record('layoutSet', data);
    layout = data;
    return { version: 1, data };
  },
  listRecentAgentSessions: async () => [],
  listSessions: async () => [],
  loadSession: async () => null,
  saveSession: async (data: any) => {
    record('saveSession', data);
    return 'fixture';
  },
  setActiveSession: async (id: string) => record('setActiveSession', id),
  listLiveClaudeSessionIds: async () =>
    Object.keys(snapshots).filter((id) => snapshots[id].status !== 'stopped'),
  getAllClaudeSessions: async () => Object.values(snapshots),
  getClaudeSession: async (id: string) => structuredClone(snapshots[id] ?? null),
  onClaudeSessionUpdate: (fn: (id: string, snapshot: any) => void) => {
    listeners.add(fn);
    return () => listeners.delete(fn);
  },
  attachClaude: async (_pane: string, id: string) => id,
  providerCheckAll: async (force: boolean) => {
    record('providerCheckAll', force);
    if (detectionMode === 'failed') throw new Error('fixture old/unavailable host');
    if (detectionMode === 'unknown') return [];
    if (detectionMode === 'malformed') return [{ provider: 'claude' }];
    return ['claude', 'codex', 'copilot', 'opencode', 'pi'].map((provider) => ({
      provider,
      found:
        !!config.agents.binaries?.[provider] ||
        detectionMode === 'installed' ||
        (detectionMode === 'codex-only' && provider === 'codex'),
      resolvedPath: `/fixture/bin/${provider}`,
      customBin: config.agents.binaries?.[provider] ?? '',
    }));
  },
  claudeListModels: async () => ({ ...config.claude, aliases: [], seen: [] }),
  providerListModels: async () => [],
  claudeProfilesList: async () => [],
  claudeListSessionsForDir: async () => [],
  pickFolder: async () => '/fixture/project',
  // Empty chats query git metadata before the user sends their first message.
  gitStatus: async () => ({ branch: null, files: [] }),
  gitLog: async () => [],
  worktreeInfo: async (cwd: string) => {
    record('worktreeInfo', cwd);
    if (cwd.includes('deferred')) return new Promise((resolve) => folderPending.set(cwd, resolve));
    if (cwd.includes('invalid') || cwd.startsWith('~'))
      return { isRepo: false, directory: 'invalid' };
    if (cwd.includes('git-folder'))
      return { isRepo: true, directory: 'accessible', gitStatus: 'repo', branch: 'main' };
    return { isRepo: false, directory: 'accessible', gitStatus: 'non-git' };
  },
  listHubPlugins: async () => [],
  libraryList: async () => [],
  spawnClaude: async (opts: any) => {
    record('spawnClaude', opts);
    if (!runtime || mode === 'reject')
      throw new Error('fixture runtime unavailable secret=DO-NOT-DISPLAY');
    if (mode === 'malformed') return ' ';
    if (opts.resumeSessionId && snapshots[opts.resumeSessionId]) {
      const id = opts.resumeSessionId;
      snapshots[id].status = 'active';
      snapshots[id].ambientState = 'idle';
      emit(id);
      return id;
    }
    const id = `fixture-session-${++sequence}`;
    const provider = opts.provider ?? 'claude';
    const transport =
      opts.transport ?? (provider === 'claude' ? config.claude.transport : config.codex.transport);
    snapshots[id] = {
      sessionId: id,
      cwd: opts.cwd,
      provider,
      transport,
      status: 'active',
      ambientState: 'idle',
      lastActivity: Date.now(),
      settings: {},
      activeToolCalls: [],
      completedToolCalls: [],
      fileChanges: [],
      subagents: [],
      workflows: [],
      pendingApproval: null,
      pendingQuestions: null,
      conversation: opts.message
        ? [{ role: 'user', content: opts.message, timestamp: Date.now() }]
        : [],
    };
    // A managed host may publish before IPC resolves. Adoption must converge.
    emit(id);
    return id;
  },
  claudeMessage: async (id: string, message: string, requestId?: string) => {
    if (requestId !== undefined) {
      const request = requests.get(requestId);
      if (!request || request.owner !== id) throw new Error('Foreign request owner');
      if (request.delivery === 'accepted') return { ok: true, requestId, delivery: 'accepted' };
      request.delivery = 'accepted';
      record('claudeMessage', id, message, requestId);
    } else record('claudeMessage', id, message);
    snapshots[id].conversation.push({ role: 'user', content: message, timestamp: Date.now() });
    emit(id);
    return { ok: true, ...(requestId ? { requestId, delivery: 'accepted' } : {}) };
  },
};
(window as any).electronAPI = new Proxy(api, {
  get(target: any, name: string) {
    if (name in target) return target[name];
    if (name.startsWith('managerRequest')) return undefined;
    if (name.startsWith('on')) return () => () => {};
    return async () => undefined;
  },
});
(window as any).firstUse = {
  requests: () => [...requests.values()],
  calls,
  notice: (notice: any) => {
    for (const fn of noticeListeners) fn(notice);
  },
  config: () => config,
  snapshots: () => snapshots,
  layout: () => layout,
  spawnMode: (value: string) => {
    mode = value;
  },
  providers: (value: string) => {
    detectionMode = value;
  },
  readiness: (value: string) => {
    readiness = value;
  },
  folderReply: (cwd: string, info: any) => {
    folderPending.get(cwd)?.(info);
  },
  update: (id: string, patch: any) => {
    Object.assign(snapshots[id], patch);
    emit(id);
  },
  wake: (id: string, kind: FleetMessageKind, entry: FleetMessageEntry) => {
    snapshots[id].conversation.push({
      role: 'user',
      content: buildFleetMessage(kind, [entry]),
      timestamp: Date.now(),
    });
    emit(id);
  },
  reply: (id: string, content: string) => {
    snapshots[id].conversation.push({ role: 'assistant', content, timestamp: Date.now() });
    snapshots[id].ambientState = 'idle';
    emit(id);
  },
  restart: () => {
    for (const snapshot of Object.values(snapshots)) {
      snapshot.status = 'stopped';
      snapshot.ambientState = 'idle';
    }
    sessionStorage.setItem('first-use-restart', JSON.stringify({ config, snapshots, layout }));
  },
  runtime: (value: boolean) => {
    runtime = value;
  },
};
const [{ default: App }, { ConfigProvider }, { PluginsProvider }] = await Promise.all([
  import('../App'),
  import('../contexts/ConfigContext'),
  import('../contexts/PluginsContext'),
]);
ReactDOM.createRoot(document.getElementById('root')!).render(
  <ConfigProvider>
    <PluginsProvider>
      <App />
    </PluginsProvider>
  </ConfigProvider>,
);
