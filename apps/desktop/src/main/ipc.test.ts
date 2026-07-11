/**
 * The `claude:spawn` IPC gate: `transport` may ride the spawn-managed payload
 * ONLY for codex+stream (the daemon's other managed adapters reject/ignore it),
 * and claude+stream must route through the claude stream branch — never the
 * managed-provider branch.
 *
 * The gate is a one-line spread condition (see the managed branch in ipc.ts);
 * widening it leaks transport to opencode/pi, dropping it turns the spawn
 * dialog's headless-codex pill into a silent no-op (a hybrid spawns instead,
 * nothing fails loudly). The hub-bus twin path makes drift here easy to miss.
 *
 * Strategy (mirrors tests/main/hubCapabilitiesProfiles.test.ts): mock electron's
 * ipcMain to capture every registered handler, stub every service collaborator
 * so ipc.ts imports cleanly, and invoke the captured 'claude:spawn' handler with
 * spawnManagedAgent / spawnClaudeAgent mocked.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

const { handlers, spawnManagedAgent, spawnClaudeAgent } = vi.hoisted(() => ({
  handlers: new Map<string, (event: unknown, ...args: unknown[]) => unknown>(),
  spawnManagedAgent: vi.fn(async () => 'managed-1'),
  spawnClaudeAgent: vi.fn(async () => 'claude-1'),
}));

vi.mock('electron', () => ({
  ipcMain: {
    handle: (channel: string, handler: (event: unknown, ...args: unknown[]) => unknown) => {
      handlers.set(channel, handler);
    },
    on: (channel: string, handler: (event: unknown, ...args: unknown[]) => unknown) => {
      handlers.set(channel, handler);
    },
  },
  BrowserWindow: class {},
  dialog: {},
  shell: {},
}));

vi.mock('./services/managedSpawn', () => ({
  spawnManagedAgent: (...a: unknown[]) => spawnManagedAgent(...a),
}));
vi.mock('./services/claudeSpawn', () => ({
  spawnClaudeAgent: (...a: unknown[]) => spawnClaudeAgent(...a),
}));

// Everything else is stubbed just far enough for registerIpcHandlers to run
// (setMainWindow / setEmitSink are called at registration time; the rest only
// inside handler closures we never invoke).
vi.mock('./services/configService', () => ({
  configService: { getConfig: vi.fn(() => ({})) },
}));
vi.mock('./services/libraryService', () => ({
  libraryService: { setMainWindow: vi.fn() },
}));
vi.mock('./services/sessionService', () => ({ sessionService: {} }));
vi.mock('./services/pluginSettingsMigration', () => ({
  peekLegacyPluginSettings: vi.fn(),
  clearLegacyPluginSettings: vi.fn(),
}));
vi.mock('./services/sessionHistory', () => ({ sessionHistory: {} }));
vi.mock('./services/layoutService', () => ({ layoutService: {} }));
vi.mock('./services/updateService', () => ({ updateService: {} }));
vi.mock('./services/worktreeService', () => ({
  worktreeInfo: vi.fn(),
  createWorktree: vi.fn(),
}));
vi.mock('./services/claudeSessionStore', () => ({ claudeSessionStore: {} }));
vi.mock('./services/claudeModels', () => ({ listClaudeModels: vi.fn() }));
vi.mock('./services/workflowWatcher', () => ({ workflowWatcher: {} }));
vi.mock('./services/agentNotifier', () => ({ agentNotifier: {} }));
vi.mock('./services/claudemonSessionClient', () => ({
  claudemonSessionClient: { setMainWindow: vi.fn() },
}));
vi.mock('./services/agentHandoff', () => ({ agentHandoffBrief: vi.fn() }));
vi.mock('./services/agentProviders', () => ({
  resolveAgentBinary: vi.fn(),
  checkAllProviders: vi.fn(),
}));
vi.mock('./services/logFile', () => ({ logsDir: vi.fn(() => '/logs') }));
vi.mock('./services/supervisorSkill', () => ({ ensureSupervisorHome: vi.fn() }));
vi.mock('./services/chromeCookieImport', () => ({
  importChromeCookies: vi.fn(),
  importChromeCookiesViaCDP: vi.fn(),
}));
vi.mock('./services/claudeProfiles', () => ({ claudeProfiles: {} }));
vi.mock('./services/claudeSessionList', () => ({ listClaudeSessionsForDir: vi.fn() }));
vi.mock('./services/fileService', () => ({
  readTextFile: vi.fn(),
  writeTextFile: vi.fn(),
  listDir: vi.fn(),
}));
vi.mock('./services/fileWatchService', () => ({
  startWatch: vi.fn(),
  stopWatch: vi.fn(),
  setEmitSink: vi.fn(),
}));
vi.mock('./services/searchService', () => ({ searchProject: vi.fn() }));
vi.mock('./services/gitService', () => ({}));
vi.mock('./services/hubDaemon', () => ({
  HUB_HTTP_URL: 'http://127.0.0.1:0',
  HUB_PORT: 0,
  getHubToken: vi.fn(),
  getRemoteShareInfo: vi.fn(),
  setRemoteShare: vi.fn(),
}));
vi.mock('./services/tailscaleServe', () => ({
  getTailscaleInfo: vi.fn(),
  setTailscaleServe: vi.fn(),
}));
vi.mock('./services/hubClient', () => ({
  publishToHub: vi.fn(),
  isHubConnected: vi.fn(),
  callHub: vi.fn(),
}));

const { registerIpcHandlers } = await import('./ipc');

registerIpcHandlers({
  webContents: { send: vi.fn() },
  isDestroyed: () => false,
} as never);

const spawn = (opts: Record<string, unknown>) => handlers.get('claude:spawn')!(null, opts);

/** Options object of the most recent spawnManagedAgent call. */
function lastManagedOpts(): Record<string, unknown> {
  return spawnManagedAgent.mock.calls.at(-1)![0] as Record<string, unknown>;
}

beforeEach(() => {
  spawnManagedAgent.mockClear();
  spawnClaudeAgent.mockClear();
});

describe('claude:spawn — transport rides spawn-managed only for codex+stream', () => {
  it('codex + stream forwards transport:"stream"', async () => {
    await spawn({ provider: 'codex', transport: 'stream', cwd: '/proj' });
    expect(spawnManagedAgent).toHaveBeenCalledTimes(1);
    expect(lastManagedOpts().provider).toBe('codex');
    expect(lastManagedOpts().transport).toBe('stream');
  });

  it('codex + pty forwards NO transport key (hybrid default)', async () => {
    await spawn({ provider: 'codex', transport: 'pty', cwd: '/proj' });
    expect(lastManagedOpts().provider).toBe('codex');
    expect(lastManagedOpts()).not.toHaveProperty('transport');
  });

  it.each(['opencode', 'pi'])(
    '%s + stream forwards NO transport key (their adapters have no headless mode)',
    async (provider) => {
      await spawn({ provider, transport: 'stream', cwd: '/proj' });
      expect(lastManagedOpts().provider).toBe(provider);
      expect(lastManagedOpts()).not.toHaveProperty('transport');
    },
  );

  it('claude + stream routes through the claude stream branch, not the managed one', async () => {
    // mcpItemIds only ride the claude branch — their presence in the forwarded
    // options proves which branch handled the spawn.
    await spawn({
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      mcpItemIds: ['srv1'],
      profileId: 'p1',
    });
    expect(spawnManagedAgent).toHaveBeenCalledTimes(1);
    const opts = lastManagedOpts();
    expect(opts.provider).toBe('claude');
    expect(opts.transport).toBe('stream');
    expect(opts.mcpItemIds).toEqual(['srv1']);
    expect(opts.profileId).toBe('p1');
    expect(spawnClaudeAgent).not.toHaveBeenCalled();
  });

  it('claude + pty is a Tier-1 PTY spawn — spawnManagedAgent is never touched', async () => {
    await spawn({ provider: 'claude', transport: 'pty', cwd: '/proj' });
    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
    expect(spawnManagedAgent).not.toHaveBeenCalled();
  });
});
