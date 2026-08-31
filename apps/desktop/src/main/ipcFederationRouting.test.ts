/**
 * Federation-aware session-action routing in ipc.ts: an action on a session
 * whose store entry carries `hub` must go out as a qualified bus call
 * (`hub:<peer>/<method>`) and must NOT touch the local claudemon HTTP client —
 * the local daemon has no such session, and hitting it would at best 404 and
 * at worst act on the wrong session. Local sessions keep the claudemon path
 * byte-for-byte; the operations that cannot work remotely v1 (resize, close,
 * mode/model/effort switches, handoff) refuse loudly; and the pane-housekeeping
 * pair is special-cased so the remote chat surface can exist at all: attach
 * adopts the id as a stream-less GUI viewer (web-polyfill parity) and gate is
 * a silent no-op (the peer's own client gates its claudemon).
 *
 * Strategy (mirrors ipc.test.ts): mock electron's ipcMain to capture every
 * registered handler, stub every service collaborator so ipc.ts imports
 * cleanly, and invoke the captured handlers with claudeSessionStore.getSnapshot
 * answering a hub-tagged or local snapshot.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

const { handlers, callHub, sessionClient, getSnapshot, clearPendingQuestions } = vi.hoisted(() => ({
  handlers: new Map<string, (event: unknown, ...args: unknown[]) => unknown>(),
  callHub: vi.fn(async (): Promise<unknown> => ({ ok: true })),
  sessionClient: {
    setMainWindow: vi.fn(),
    message: vi.fn(async () => ({ ok: true, mode: 'input' })),
    approve: vi.fn(async () => undefined),
    answer: vi.fn(async () => undefined),
    signal: vi.fn(async () => undefined),
    resize: vi.fn(async () => undefined),
    close: vi.fn(async () => undefined),
    attach: vi.fn(async () => undefined),
    setGate: vi.fn(async () => undefined),
    setModel: vi.fn(async () => ({ ok: true })),
    setPermissionMode: vi.fn(async () => ({ ok: true, mode: 'default' })),
    handoffBrief: vi.fn(async () => ({ path: '/brief' })),
  },
  getSnapshot: vi.fn((_sessionId: string): unknown => null),
  clearPendingQuestions: vi.fn(),
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

vi.mock('./services/managedSpawn', () => ({ spawnManagedAgent: vi.fn() }));
vi.mock('./services/claudeSpawn', () => ({ spawnClaudeAgent: vi.fn() }));
vi.mock('./services/cliInstall', () => ({ installWorkspacerCli: vi.fn() }));
vi.mock('./services/configService', () => ({
  configService: { getConfig: vi.fn(() => ({})), onChange: vi.fn(() => () => {}) },
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
vi.mock('./services/claudeSessionStore', () => ({
  claudeSessionStore: {
    getSnapshot: (sessionId: string) => getSnapshot(sessionId),
    clearPendingQuestions: (sessionId: string) => clearPendingQuestions(sessionId),
    notePermissionMode: vi.fn(),
    noteRequestedModel: vi.fn(),
  },
}));
vi.mock('./services/claudeModels', () => ({ listClaudeModels: vi.fn() }));
vi.mock('./services/workflowWatcher', () => ({ workflowWatcher: {} }));
vi.mock('./services/agentNotifier', () => ({ agentNotifier: {} }));
vi.mock('./services/claudemonSessionClient', () => ({
  claudemonSessionClient: sessionClient,
}));
vi.mock('./services/agentHandoff', () => ({ agentHandoffBrief: vi.fn() }));
vi.mock('./services/agentProviders', () => ({
  resolveAgentBinary: vi.fn(),
  checkAllProviders: vi.fn(),
  checkAllProvidersCached: vi.fn(),
}));
vi.mock('./services/liveEffort', () => ({ applyLiveEffort: vi.fn(async () => ({ ok: true })) }));
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
  setHubTrustedHosts: vi.fn(),
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
  callHub: (...a: unknown[]) => callHub(...(a as [])),
}));
vi.mock('./services/federationBridge', () => ({
  listFederationPeers: vi.fn(() => [{ name: 'work', connected: true }]),
}));

const { registerIpcHandlers } = await import('./ipc');

registerIpcHandlers({
  webContents: { send: vi.fn() },
  isDestroyed: () => false,
} as never);

const invoke = (channel: string, ...args: unknown[]) => handlers.get(channel)!(null, ...args);

beforeEach(() => {
  callHub.mockClear();
  clearPendingQuestions.mockClear();
  for (const fn of Object.values(sessionClient)) (fn as ReturnType<typeof vi.fn>).mockClear();
  // Default: sess-remote lives on peer "work"; anything else is local.
  getSnapshot.mockImplementation((sessionId: string) =>
    sessionId === 'sess-remote' ? { sessionId, hub: 'work' } : { sessionId },
  );
});

describe('remote sessions route over the bus, not claudemon', () => {
  it('claude:approve on a hub-tagged session issues hub:work/claude.approve', async () => {
    await invoke('claude:approve', 'sess-remote', 'yes', 'because');
    expect(callHub).toHaveBeenCalledWith('hub:work/claude.approve', {
      sessionId: 'sess-remote',
      decision: 'yes',
      reason: 'because',
    });
    expect(sessionClient.approve).not.toHaveBeenCalled();
  });

  it('claude:answer routes and optimistically clears pending questions', async () => {
    await invoke('claude:answer', 'sess-remote', { option: 2 });
    expect(callHub).toHaveBeenCalledWith('hub:work/claude.answer', {
      sessionId: 'sess-remote',
      option: 2,
    });
    expect(sessionClient.answer).not.toHaveBeenCalled();
    expect(clearPendingQuestions).toHaveBeenCalledWith('sess-remote');
  });

  it('claude:message routes to hub:work/agents.sendMessage and keeps the { ok } shape', async () => {
    const res = await invoke('claude:message', 'sess-remote', 'hello');
    expect(callHub).toHaveBeenCalledWith('hub:work/agents.sendMessage', {
      sessionId: 'sess-remote',
      text: 'hello',
    });
    expect(sessionClient.message).not.toHaveBeenCalled();
    expect(res).toEqual({ ok: true });
  });

  it('a refused remote send folds into { ok: false } instead of throwing', async () => {
    callHub.mockRejectedValueOnce(new Error('session is not accepting input'));
    const res = await invoke('claude:message', 'sess-remote', 'hello');
    expect(res).toEqual({ ok: false });
  });

  it('claude:signal routes to hub:work/claude.signal', async () => {
    await invoke('claude:signal', 'sess-remote', 'SIGINT');
    expect(callHub).toHaveBeenCalledWith('hub:work/claude.signal', {
      sessionId: 'sess-remote',
      signal: 'SIGINT',
    });
    expect(sessionClient.signal).not.toHaveBeenCalled();
  });
});

describe('local sessions keep the claudemon path untouched', () => {
  it('claude:approve on a local session calls claudemon and never the bus', async () => {
    await invoke('claude:approve', 'sess-local', 'no');
    expect(sessionClient.approve).toHaveBeenCalledWith('sess-local', 'no', undefined);
    expect(callHub).not.toHaveBeenCalled();
  });

  it('claude:message on a local session calls claudemon', async () => {
    const res = await invoke('claude:message', 'sess-local', 'hi');
    expect(sessionClient.message).toHaveBeenCalledWith('sess-local', 'hi');
    expect(callHub).not.toHaveBeenCalled();
    expect(res).toEqual({ ok: true, mode: 'input' });
  });

  it('claude:setModel forwards the canonical pair beside the legacy companion', async () => {
    await invoke('claude:setModel', 'sess-local', 'opus[1m]', undefined, 'opus', 1_000_000);
    expect(sessionClient.setModel).toHaveBeenCalledWith(
      'sess-local',
      'opus[1m]',
      undefined,
      'opus',
      1_000_000,
    );
  });

  it('an unknown session (no store entry) stays on the local path', async () => {
    getSnapshot.mockReturnValue(null);
    await invoke('claude:signal', 'sess-unknown', 'SIGTERM');
    expect(sessionClient.signal).toHaveBeenCalledWith('sess-unknown', 'SIGTERM');
    expect(callHub).not.toHaveBeenCalled();
  });
});

describe('local-only operations refuse loudly for remote sessions', () => {
  it.each([
    ['claude:setPermissionMode', ['sess-remote', 'plan'], 'setPermissionMode'],
    ['claude:setEffort', ['sess-remote', 'high'], null],
    ['claude:setModel', ['sess-remote', 'opus'], 'setModel'],
    ['claude:handoffBrief', ['sess-remote'], 'handoffBrief'],
    ['claude:resize', ['sess-remote', 80, 24], 'resize'],
    ['claude:close', ['sess-remote'], 'close'],
  ] as Array<[string, unknown[], string | null]>)(
    '%s throws "not available for remote sessions"',
    async (channel, args, clientMethod) => {
      // Async wrapper: some of these handlers throw synchronously, and the
      // rejects matcher needs that surfaced as a rejection either way.
      await expect(async () => invoke(channel, ...args)).rejects.toThrow(
        /not available for remote sessions/,
      );
      if (clientMethod) {
        expect(
          (sessionClient as Record<string, unknown>)[clientMethod] as ReturnType<typeof vi.fn>,
        ).not.toHaveBeenCalled();
      }
      expect(callHub).not.toHaveBeenCalled();
    },
  );
});

describe('pane housekeeping adopts or no-ops for remote sessions', () => {
  // These two are what the pane needs to exist: refusing attach left remote
  // panes sessionless (cards rendered, chat could neither read nor send).
  it('claude:attach adopts the remote id as a GUI-only viewer, touching nothing', async () => {
    const res = await invoke('claude:attach', 'pane-1', 'sess-remote');
    expect(res).toBe('sess-remote');
    expect(sessionClient.attach).not.toHaveBeenCalled();
    expect(callHub).not.toHaveBeenCalled();
  });

  it('claude:gate is a silent no-op for a remote session', async () => {
    await expect(invoke('claude:gate', 'sess-remote', true)).resolves.toBeUndefined();
    expect(sessionClient.setGate).not.toHaveBeenCalled();
    expect(callHub).not.toHaveBeenCalled();
  });
});

// A federated spawn carries its first message to the peer, and — because
// federation is where version skew actually lives — verifies the peer took it.
// A peer that predates `message` answers a perfectly ordinary successful spawn
// with the prompt nowhere, and the worker over there just sits, which is
// indistinguishable from a wedge.
describe('a federated spawn delivers its first message', () => {
  it('forwards `message` to the peer and sends nothing extra when the peer confirms', async () => {
    callHub.mockResolvedValueOnce({ sessionId: 'peer-1', messageQueued: true });
    const id = await invoke('claude:spawn', {
      targetHub: 'work',
      cwd: '/proj',
      message: 'ship the thing',
    });
    expect(id).toBe('peer-1');
    expect(callHub).toHaveBeenCalledTimes(1);
    expect(callHub.mock.calls[0][0]).toBe('hub:work/agents.spawn');
    expect((callHub.mock.calls[0][1] as { message?: string }).message).toBe('ship the thing');
  });

  it('falls back to hub:<peer>/agents.sendMessage when the peer does not confirm', async () => {
    callHub.mockResolvedValueOnce({ sessionId: 'peer-1' });
    const id = await invoke('claude:spawn', {
      targetHub: 'work',
      cwd: '/proj',
      message: 'ship the thing',
    });
    expect(id).toBe('peer-1');
    expect(callHub).toHaveBeenCalledTimes(2);
    expect(callHub.mock.calls[1]).toEqual([
      'hub:work/agents.sendMessage',
      { sessionId: 'peer-1', text: 'ship the thing' },
    ]);
  });

  it('sends nothing extra when there was no first message', async () => {
    callHub.mockResolvedValueOnce({ sessionId: 'peer-1' });
    await invoke('claude:spawn', { targetHub: 'work', cwd: '/proj' });
    expect(callHub).toHaveBeenCalledTimes(1);
  });
});

describe('federation:peers', () => {
  it('returns the bridge peer list', async () => {
    const res = await invoke('federation:peers');
    expect(res).toEqual([{ name: 'work', connected: true }]);
  });
});
