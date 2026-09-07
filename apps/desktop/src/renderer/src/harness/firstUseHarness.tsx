/** Fresh-profile production App. All host/provider boundaries are in-memory;
 * never installs a backend, opens a provider, or reads a user's profile. */
import React from 'react';
import ReactDOM from 'react-dom/client';
import '../App.css';
import { CONFIG_DEFAULTS } from '../hooks/configDefaults.generated';
const params = new URLSearchParams(location.search);
let config: any = structuredClone(CONFIG_DEFAULTS);
config.ui = { ...config.ui, onboardingDismissed: false, theme: params.get('theme') ?? 'light' };
const calls: { method: string; args: any[] }[] = [];
let mode = params.get('spawn') ?? 'reject';
let detectionMode = params.get('providers') ?? 'installed';
let runtime = true;
const snapshots: Record<string, any> = {};
const listeners = new Set<(id: string, snapshot: any) => void>();
const configListeners = new Set<(config: any) => void>();
let layout: any = null;
let sequence = 0;
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
  layoutGet: async () => ({ version: 0, data: null }),
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
  listLiveClaudeSessionIds: async () => Object.keys(snapshots),
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
  worktreeInfo: async () => ({ isRepo: false }),
  listHubPlugins: async () => [],
  libraryList: async () => [],
  spawnClaude: async (opts: any) => {
    record('spawnClaude', opts);
    if (!runtime || mode === 'reject')
      throw new Error('fixture runtime unavailable secret=DO-NOT-DISPLAY');
    if (mode === 'malformed') return ' ';
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
  claudeMessage: async (id: string, message: string) => {
    record('claudeMessage', id, message);
    snapshots[id].conversation.push({ role: 'user', content: message, timestamp: Date.now() });
    emit(id);
    return { ok: true };
  },
};
(window as any).electronAPI = new Proxy(api, {
  get(target: any, name: string) {
    if (name in target) return target[name];
    if (name.startsWith('on')) return () => () => {};
    return async () => undefined;
  },
});
(window as any).firstUse = {
  calls,
  config: () => config,
  snapshots: () => snapshots,
  layout: () => layout,
  spawnMode: (value: string) => {
    mode = value;
  },
  providers: (value: string) => {
    detectionMode = value;
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
