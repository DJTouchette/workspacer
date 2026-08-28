/**
 * The `claude:spawn` IPC gate: `transport` may ride the spawn-managed payload
 * ONLY for codex (the daemon's other managed adapters reject/ignore it), and
 * claude+stream must route through the claude stream branch — never the
 * managed-provider branch.
 *
 * BOTH codex values forward, and an OMITTED one stays omitted. That triple is
 * the contract: 'stream' and 'pty' are two real shapes a caller can ask for,
 * while absence means "resolve the configured default", which spawnManagedAgent
 * does once for every entry point (main/lib/spawnTransport). Forwarding only
 * 'stream' — what this used to do — turned an explicit hybrid request into a
 * headless spawn the moment codex's default became headless.
 *
 * The gate is a one-line spread condition (see the managed branch in ipc.ts);
 * widening it leaks transport to opencode/pi. The hub-bus twin path makes drift
 * here easy to miss.
 *
 * Strategy (mirrors tests/main/hubCapabilitiesProfiles.test.ts): mock electron's
 * ipcMain to capture every registered handler, stub every service collaborator
 * so ipc.ts imports cleanly, and invoke the captured 'claude:spawn' handler with
 * spawnManagedAgent / spawnClaudeAgent mocked.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

const { handlers, spawnManagedAgent, spawnClaudeAgent, installWorkspacerCli, cfg } = vi.hoisted(
  () => ({
    handlers: new Map<string, (event: unknown, ...args: unknown[]) => unknown>(),
    spawnManagedAgent: vi.fn(async () => 'managed-1'),
    spawnClaudeAgent: vi.fn(async () => 'claude-1'),
    installWorkspacerCli: vi.fn(async () => ({ ok: true, message: 'installed' })),
    /** Mutable config the mocked configService serves — the role-harness
     *  settings this handler now resolves live here. */
    cfg: { value: {} as Record<string, unknown> },
  }),
);

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
vi.mock('./services/cliInstall', () => ({
  installWorkspacerCli: (...a: unknown[]) => installWorkspacerCli(...a),
}));

// Everything else is stubbed just far enough for registerIpcHandlers to run
// (setMainWindow / setEmitSink are called at registration time; the rest only
// inside handler closures we never invoke).
vi.mock('./services/configService', () => ({
  // onChange is subscribed at registration time (the config-changed push), so
  // the stub has to hand back an unsubscribe like the real one.
  configService: { getConfig: vi.fn(() => cfg.value), onChange: vi.fn(() => () => {}) },
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
  checkAllProvidersCached: vi.fn(),
}));
vi.mock('./services/logFile', () => ({ logsDir: vi.fn(() => '/logs') }));
vi.mock('./lib/workspacerHome', () => ({ ensureSupervisorHome: vi.fn() }));
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
  hubHttpUrl: () => 'http://127.0.0.1:0',
  HUB_PORT: 0,
  getHubToken: vi.fn(),
  getRemoteShareInfo: vi.fn(),
  setRemoteShare: vi.fn(),
}));
vi.mock('./services/remoteTokens', () => ({
  listRemoteTokens: vi.fn(),
  getOrCreateRemoteToken: vi.fn(),
  revokeRemoteToken: vi.fn(),
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
  cfg.value = {};
});

describe('claude:spawn — transport rides spawn-managed only for codex+stream', () => {
  it('codex + stream forwards transport:"stream"', async () => {
    await spawn({ provider: 'codex', transport: 'stream', cwd: '/proj' });
    expect(spawnManagedAgent).toHaveBeenCalledTimes(1);
    expect(lastManagedOpts().provider).toBe('codex');
    expect(lastManagedOpts().transport).toBe('stream');
  });

  it('codex + pty forwards transport:"pty" (an explicit hybrid request)', async () => {
    await spawn({ provider: 'codex', transport: 'pty', cwd: '/proj' });
    expect(lastManagedOpts().provider).toBe('codex');
    expect(lastManagedOpts().transport).toBe('pty');
  });

  it('codex with NO transport forwards no key — the default is resolved downstream', async () => {
    await spawn({ provider: 'codex', cwd: '/proj' });
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

describe('cli:install — delegates to installWorkspacerCli and returns its result', () => {
  it('is registered and passes the service result through untouched', async () => {
    const result = await handlers.get('cli:install')!(null);
    expect(installWorkspacerCli).toHaveBeenCalledTimes(1);
    expect(result).toEqual({ ok: true, message: 'installed' });
  });
});

/**
 * The reported bug: Settings said the Fleet Manager runs on codex, and
 * launching one produced a Claude session.
 *
 * `agents.managerProvider` was read in ONE renderer component, so every other
 * way the role starts — the hub bus (web client, phone, a hub job), a respawn
 * of a card that predates the field, the next entry point somebody adds —
 * arrived here with no provider and fell through `opts.provider ?? 'claude'`.
 * A silently-Claude manager is indistinguishable from a working one. The
 * resolution now lives in main (lib/roleProviders), so this handler honours
 * the setting whoever calls it.
 */
describe('claude:spawn — a role spawn with no provider resolves the configured harness', () => {
  it('spawns the Fleet Manager on config agents.managerProvider', async () => {
    cfg.value = { agents: { managerProvider: 'codex' } };
    await spawn({ manager: true, cwd: '/proj' });
    expect(lastManagedOpts().provider).toBe('codex');
    expect(lastManagedOpts().manager).toBe(true);
  });

  it('an EXPLICIT provider still wins — the launcher can override Settings', async () => {
    // "Ask the Fleet" offers a per-launch harness pick; a config default must
    // not quietly reclaim it.
    cfg.value = { agents: { managerProvider: 'codex' } };
    await spawn({ manager: true, provider: 'claude', transport: 'pty', cwd: '/proj' });
    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
    expect(spawnManagedAgent).not.toHaveBeenCalled();
  });

  it('ignores an unknown configured harness rather than passing it on', async () => {
    // A hand-edited config naming a harness we do not speak would otherwise
    // reach an adapter that has no idea what it is; claude at least runs.
    cfg.value = { agents: { managerProvider: 'nonesuch' } };
    await spawn({ manager: true, transport: 'pty', cwd: '/proj' });
    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
  });

  it('leaves a plain worker alone — no role flags means claude, as before', async () => {
    cfg.value = { agents: { managerProvider: 'codex' } };
    await spawn({ cwd: '/proj', transport: 'pty' });
    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
    expect(spawnManagedAgent).not.toHaveBeenCalled();
  });
});
