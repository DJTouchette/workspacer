const readinessMocks = vi.hoisted(() => ({
  read: vi.fn(() => ({ state: 'unchecked' })),
  check: vi.fn(async () => ({ state: 'responding', checkedAt: 1 })),
}));
vi.mock('./services/providerReadinessRuntime', () => ({
  providerReadinessService: readinessMocks,
}));
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

import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const {
  handlers,
  spawnManagedAgent,
  spawnClaudeAgent,
  installWorkspacerCli,
  cfg,
  readTextFileMock,
} = vi.hoisted(() => ({
  handlers: new Map<string, (event: unknown, ...args: unknown[]) => unknown>(),
  readTextFileMock: vi.fn(() => ({ path: '', contents: '', size: 0 })),
  spawnManagedAgent: vi.fn(async () => 'managed-1'),
  spawnClaudeAgent: vi.fn(async () => 'claude-1'),
  installWorkspacerCli: vi.fn(async () => ({ ok: true, message: 'installed' })),
  /** Mutable config the mocked configService serves. The role-harness
   *  settings this handler now resolves live here. */
  cfg: { value: {} as Record<string, unknown> },
}));

const cardMocks = vi.hoisted(() => ({ owner: vi.fn(), read: vi.fn() }));

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
  // pathConfinement reads this (the config dir is refused wholesale outside its
  // library/layouts/sessions carve-outs), and the file-read gate below goes
  // through it. A directory nobody in this suite writes to.
  getConfigDir: () => '/tmp/wks-ipc-test-cfg',
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
  claudeSessionStore: { getSnapshot: cardMocks.owner, getAllSnapshots: vi.fn(() => []) },
}));
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
  readTextFile: (...a: unknown[]) => readTextFileMock(...(a as [])),
  writeTextFile: vi.fn(),
  listDir: vi.fn(),
}));
vi.mock('./services/fileWatchService', () => ({
  startWatch: vi.fn(),
  stopWatch: vi.fn(),
  setEmitSink: vi.fn(),
}));
vi.mock('./services/searchService', () => ({ searchProject: vi.fn() }));
vi.mock('./services/gitService', () => ({ readHtmlCardDiff: cardMocks.read }));
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

  it('spawns a COPILOT manager when that is what Settings says', async () => {
    // The third manager harness. Copilot has no PTY leg at all, so the only
    // proof that the setting arrived is that it reached the managed funnel
    // naming copilot — a fall-through to claude would still have "worked".
    cfg.value = { agents: { managerProvider: 'copilot' } };
    await spawn({ manager: true, cwd: '/proj' });
    expect(lastManagedOpts().provider).toBe('copilot');
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

/**
 * The markdown detour's door, and the read behind it.
 *
 * `open_browser` on a `.md` file is routed away from the browser pane and into
 * the preview pane, and that detour used to be unconfined end to end: the
 * renderer only asked whether the URL ended in `.md`, and `file:read` applied no
 * confinement at all. So `file:///etc/ssl/README.md` rendered an out-of-root
 * file, and renaming any unreadable file to `.md` walked around the browser arm.
 */
describe('webview:check-preview / file:read confinement', () => {
  let tmp: string;
  let projectRoot: string;
  let outside: string;

  beforeAll(() => {
    tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-ipc-preview-'));
    projectRoot = path.join(tmp, 'project');
    outside = path.join(tmp, 'outside');
    fs.mkdirSync(path.join(projectRoot, '.git'), { recursive: true });
    fs.mkdirSync(outside, { recursive: true });
    fs.writeFileSync(path.join(projectRoot, 'NOTES.md'), '# notes');
    fs.writeFileSync(path.join(projectRoot, '.git', 'config'), '[core]\n');
    fs.writeFileSync(path.join(outside, 'README.md'), '# escaped');
  });

  afterAll(() => {
    fs.rmSync(tmp, { recursive: true, force: true });
  });

  const fileUrl = (p: string) => 'file://' + p.split('/').map(encodeURIComponent).join('/');
  const checkPreview = (url: string) =>
    handlers.get('webview:check-preview')!(null, url) as Promise<{
      allowed: boolean;
      reason?: string;
      canonicalPath?: string;
    }>;
  const readFile = (p: string, expectedCanonicalPath?: string) =>
    handlers.get('file:read')!(null, p, expectedCanonicalPath) as Promise<{ contents: string }>;

  beforeEach(() => {
    // The project directory is what makes tmp a root at all; home is the other.
    cfg.value = { projects: { [projectRoot]: { name: 'project' } } };
  });

  it('allows an in-root markdown file', async () => {
    expect((await checkPreview(fileUrl(path.join(projectRoot, 'NOTES.md')))).allowed).toBe(true);
  });

  it('refuses an out-of-root markdown file, before any read', async () => {
    const v = await checkPreview(fileUrl(path.join(outside, 'README.md')));
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/outside/);
  });

  it('refuses a %2e%2e traversal out of the root', async () => {
    const v = await checkPreview(fileUrl(projectRoot) + '/%2e%2e/outside/README.md');
    expect(v.allowed).toBe(false);
  });

  it('refuses a non-markdown file, so the detour is not a second browser door', async () => {
    fs.writeFileSync(path.join(projectRoot, 'index.html'), '<h1>x</h1>');
    const v = await checkPreview(fileUrl(path.join(projectRoot, 'index.html')));
    expect(v.allowed).toBe(false);
  });

  it('reads an ordinary file, and hands the reader the CANONICAL path', async () => {
    // check-path and opened-path may not differ: the gate resolves the string
    // and the reader must be given what the gate resolved, not what the caller
    // typed. `sub/..` here is the cheapest proof the two are the same value.
    readTextFileMock.mockClear();
    await readFile(path.join(projectRoot, 'sub', '..', 'NOTES.md'));
    expect(readTextFileMock).toHaveBeenCalledWith(path.join(projectRoot, 'NOTES.md'));
  });

  it('refuses to read a file the fs.* denial list refuses', async () => {
    await expect(readFile(path.join(projectRoot, '.git', 'config'))).rejects.toThrow(
      /credentials or agent configuration/,
    );
  });

  /**
   * assertPathAllowed (every fs.* caller) denies when canonicalizePath throws;
   * refuseSecretRead used to hand the RAW string back instead, on the theory
   * that readTextFile would fail on it the same way. It does not: canonicalize
   * requires an ABSOLUTE path and throws on a relative one, but fs.statSync /
   * fs.readFileSync resolve a relative path against process.cwd() just fine,
   * so a relative path skipped the secret gate entirely rather than merely
   * failing to canonicalize. A relative '.git/config' from a cwd that IS a
   * live project root is exactly the credential-adjacent file the gate exists
   * to catch.
   */
  it('refuses a relative path rather than skipping the secret gate entirely', async () => {
    const cwdBefore = process.cwd();
    process.chdir(projectRoot);
    try {
      await expect(readFile('.git/config')).rejects.toThrow(/could not be resolved/);
    } finally {
      process.chdir(cwdBefore);
    }
  });

  /**
   * The TOCTOU the re-review flagged: checkPreviewFile returns `canonicalPath`
   * over IPC, and a caller that hands it BACK to `file:read` as the expected
   * value gets a fresh canonicalization compared against it. Without this,
   * `checkPreview` approves `<root>/doc.md`, the file is swapped for a symlink
   * to somewhere outside every root, and `file:read` opens whatever the swap
   * now points at. The check and the open were never the same guarantee.
   */
  it('refuses a read when the checked file was swapped for a symlink afterward', async () => {
    const target = path.join(projectRoot, 'swap.md');
    fs.writeFileSync(target, '# before the swap');

    const v = await checkPreview(fileUrl(target));
    expect(v.allowed).toBe(true);
    expect(v.canonicalPath).toBe(target);

    fs.unlinkSync(target);
    fs.symlinkSync(path.join(outside, 'README.md'), target);

    await expect(readFile(target, v.canonicalPath)).rejects.toThrow(/changed since/);
  });

  it('reads an honest in-root markdown file at the canonical path checkPreview returned', async () => {
    const v = await checkPreview(fileUrl(path.join(projectRoot, 'NOTES.md')));
    expect(v.allowed).toBe(true);
    readTextFileMock.mockClear();
    await readFile(v.canonicalPath as string, v.canonicalPath);
    expect(readTextFileMock).toHaveBeenCalledWith(v.canonicalPath);
  });
});

describe('usage pacing schedule IPC', () => {
  it('reads the hub and answers null for a hub that does not know the method', async () => {
    const { callHub } = await import('./services/hubClient');
    const state = { schedule: 'five_day', configurable: true };
    vi.mocked(callHub).mockResolvedValueOnce(state);
    expect(await handlers.get('usage:pacingSchedule')!(null)).toEqual(state);
    expect(callHub).toHaveBeenLastCalledWith('usage.pacingSchedule', {});

    // An older hub. null is "cannot be asked", which the control renders as
    // unavailable — NOT as a default nobody chose.
    vi.mocked(callHub).mockRejectedValueOnce(new Error('unknown method'));
    expect(await handlers.get('usage:pacingSchedule')!(null)).toBeNull();
  });

  it('reports a failed save truthfully instead of swallowing it', async () => {
    const { callHub } = await import('./services/hubClient');
    const state = { schedule: 'seven_day', configurable: true };
    vi.mocked(callHub).mockResolvedValueOnce(state);
    expect(await handlers.get('usage:setPacingSchedule')!(null, 'seven_day')).toEqual({
      ok: true,
      state,
    });
    expect(callHub).toHaveBeenLastCalledWith('usage.setPacingSchedule', {
      schedule: 'seven_day',
    });

    // The read above degrades to null; the WRITE must not. A save that reports
    // success and stored nothing is the exact failure this plumbing exists to
    // avoid, so the hub's reason comes back to the UI.
    vi.mocked(callHub).mockRejectedValueOnce(new Error('requires host authority'));
    expect(await handlers.get('usage:setPacingSchedule')!(null, 'five_day')).toEqual({
      ok: false,
      error: 'requires host authority',
    });
  });

  it('never falls back to the daemon or to usage.report with a parameter', async () => {
    const { callHub } = await import('./services/hubClient');
    vi.mocked(callHub).mockRejectedValue(new Error('unknown method'));
    const fetcher = vi.spyOn(globalThis, 'fetch');
    try {
      await handlers.get('usage:pacingSchedule')!(null);
      await handlers.get('usage:setPacingSchedule')!(null, 'five_day');
      // claudemon has no schedule; a fallback here would invent one.
      expect(fetcher).not.toHaveBeenCalled();
      for (const call of vi.mocked(callHub).mock.calls) {
        expect(call[0]).not.toBe('usage.report');
      }
    } finally {
      fetcher.mockRestore();
      vi.mocked(callHub).mockReset();
    }
  });
});

describe('usage report hub-first IPC', () => {
  it('returns the hub projection without contacting the daemon', async () => {
    const { callHub } = await import('./services/hubClient');
    const report = { evaluated_at: 123, providers: [] };
    vi.mocked(callHub).mockResolvedValueOnce(report);
    const fetcher = vi.spyOn(globalThis, 'fetch');
    try {
      expect(await handlers.get('usage:report')!(null)).toEqual(report);
      expect(callHub).toHaveBeenLastCalledWith('usage.report', {});
      expect(fetcher).not.toHaveBeenCalled();
    } finally {
      fetcher.mockRestore();
    }
  });
  it('falls back to raw daemon data for an older hub and returns null if both fail', async () => {
    const { callHub } = await import('./services/hubClient');
    vi.mocked(callHub).mockRejectedValue(new Error('unknown method'));
    const report = { providers: [] };
    const fetcher = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce({ ok: true, json: async () => report } as Response)
      .mockRejectedValueOnce(new Error('offline'));
    try {
      expect(await handlers.get('usage:report')!(null)).toEqual(report);
      expect(fetcher).toHaveBeenCalledWith(expect.stringContaining('/usage/report'), {
        signal: expect.any(AbortSignal),
      });
      expect(await handlers.get('usage:report')!(null)).toBeNull();
    } finally {
      fetcher.mockRestore();
    }
  });
});

it('card diff IPC derives cwd from its live owner and refuses stale or remote owners', async () => {
  const invoke = (target: unknown, owner: unknown) =>
    handlers.get('html-card:read-diff')!(null, target, owner);
  cardMocks.read.mockResolvedValue({ ok: true, path: '/live/a', before: '', after: 'a' });
  cardMocks.owner.mockReturnValue({
    sessionId: 'owner',
    status: 'active',
    cwd: '/original',
    liveCwd: '/live',
  });
  expect(await invoke('a', 'owner')).toMatchObject({ ok: true });
  expect(cardMocks.read).toHaveBeenCalledWith('a', '/live');
  for (const owner of [null, { status: 'ended' }, { status: 'active', hub: 'remote' }]) {
    cardMocks.read.mockClear();
    cardMocks.owner.mockReturnValue(owner);
    expect(await invoke('a', 'owner')).toMatchObject({ ok: false });
    expect(cardMocks.read).not.toHaveBeenCalled();
  }
  expect(await invoke({}, 'owner')).toMatchObject({ ok: false });
});
it('card diff IPC drops bytes if the owning session changes during read', async () => {
  cardMocks.owner
    .mockReset()
    .mockReturnValueOnce({ sessionId: 'owner', status: 'active', cwd: '/live' })
    .mockReturnValue(null);
  cardMocks.read.mockResolvedValue({ ok: true, path: '/live/a', before: '', after: 'a' });
  expect(await handlers.get('html-card:read-diff')!(null, 'a', 'owner')).toMatchObject({
    ok: false,
  });
});

it('Fleet review IPC uses the record owner/selector guard without a trusted-user bypass', async () => {
  const { FleetReviewStore, fleetReviewStore } = await import('./services/fleetReviewStore');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'fleet-review-ipc-'));
  const file = path.join(dir, 'evidence.json');
  const evidence = {
    id: 'opaque',
    ownerSessionId: 'manager',
    workerSessionId: 'worker',
    availability: 'captured',
    files: [{ path: 'recorded.ts', status: 'M', diff: 'captured bytes' }],
  };
  fs.writeFileSync(file, JSON.stringify({ allocations: [], records: [evidence] }));
  const store = new FleetReviewStore(() => file);
  const read = vi
    .spyOn(fleetReviewStore, 'read')
    .mockImplementation((request) => store.read(request));
  const forget = vi
    .spyOn(fleetReviewStore, 'forget')
    .mockImplementation((request) => store.forget(request));
  try {
    const request = {
      ownerSessionId: 'manager',
      workerSessionId: 'worker',
      evidenceId: 'opaque',
      file: 'recorded.ts',
    };
    const invoke = (r: unknown) => handlers.get('fleet-review:read')!(null, r);
    expect(await invoke(request)).toMatchObject({
      ok: true,
      evidence: { files: [{ diff: 'captured bytes' }] },
    });
    for (const bad of [
      { ownerSessionId: 'other' },
      { workerSessionId: 'other' },
      { file: '../secret' },
      { revision: 'HEAD' },
      { cwd: '/primary' },
    ])
      expect(await invoke({ ...request, ...bad })).toMatchObject({ ok: false });
    expect(
      await handlers.get('fleet-review:forget')!(null, { ...request, ownerSessionId: 'other' }),
    ).toEqual({ ok: false });
    expect(await handlers.get('fleet-review:forget')!(null, request)).toEqual({ ok: true });
    expect(await invoke(request)).toMatchObject({ ok: false });
  } finally {
    read.mockRestore();
    forget.mockRestore();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

it('history read derives the current manager from live host state, ignoring caller row identity', async () => {
  const { claudeSessionStore } = await import('./services/claudeSessionStore');
  const { dispatchHistoryStore } = await import('./services/dispatchHistoryStore');
  const snaps = vi.spyOn(claudeSessionStore, 'getAllSnapshots').mockReturnValue([
    { sessionId: 'stopped', isWakeTarget: true, status: 'ended', startedAt: 99 },
    { sessionId: 'remote', isWakeTarget: true, status: 'active', startedAt: 100, hub: 'peer' },
    { sessionId: 'manager', isWakeTarget: true, status: 'active', startedAt: 5 },
  ] as never);
  const list = vi.spyOn(dispatchHistoryStore, 'list').mockReturnValue([]);
  try {
    expect(
      await handlers.get('dispatch-history:read')!(null, { ownerSessionId: 'forged' }),
    ).toEqual({ available: true, currentOwnerSessionId: 'manager', tasks: [], requests: [] });
  } finally {
    snaps.mockRestore();
    list.mockRestore();
  }
});

describe('routing preferences IPC', () => {
  it('forwards only the five connected-hub methods and preserves errors', async () => {
    const { callHub } = await import('./services/hubClient');
    vi.mocked(callHub).mockReset();
    const request = { baseRevision: 'revision', patch: { roles: { scout: 'cheap' } } };
    for (const method of [
      'routing.preferences.get',
      'routing.preferences.validate',
      'routing.preferences.save',
      'routing.preferences.reset',
      'routing.preview',
    ]) {
      vi.mocked(callHub).mockResolvedValueOnce({ status: 'applied' });
      expect(await handlers.get('routing:call')!(null, method, request)).toEqual({
        status: 'applied',
      });
      expect(callHub).toHaveBeenLastCalledWith(method, request);
    }
    for (const method of ['hub:peer/routing.preferences.save', 'config.save', 'fs.write']) {
      const calls = vi.mocked(callHub).mock.calls.length;
      await expect(handlers.get('routing:call')!(null, method, request)).rejects.toThrow(
        'unavailable',
      );
      expect(vi.mocked(callHub).mock.calls).toHaveLength(calls);
    }
    vi.mocked(callHub).mockRejectedValueOnce(new Error('source conflict'));
    await expect(
      handlers.get('routing:call')!(null, 'routing.preferences.save', request),
    ).rejects.toThrow('source conflict');
  });
});

it('task IPC edits recheck host sessions and open only store-resolved targets', async () => {
  const { dispatchHistoryStore } = await import('./services/dispatchHistoryStore');
  const { shell } = await import('electron');
  const edit = vi
    .spyOn(dispatchHistoryStore, 'editByHostUser')
    .mockReturnValue({ ok: false, code: 'ineligible', error: 'Worker is live' });
  const resolve = vi
    .spyOn(dispatchHistoryStore, 'openTarget')
    .mockReturnValue({ kind: 'url', target: 'https://example.test/recorded' });
  Object.assign(shell, { openExternal: vi.fn(async () => {}), openPath: vi.fn(async () => '') });
  try {
    const request = { taskId: 't', expectedTaskRevision: 1, action: 'waive', stepId: 'review' };
    expect(await handlers.get('task-inspector:edit')!(null, request)).toMatchObject({
      ok: false,
      error: 'Worker is live',
    });
    expect(edit).toHaveBeenCalledWith(request, expect.any(Function), expect.any(Function));
    const target = { taskId: 't', kind: 'url', reference: 'pullRequest' };
    expect(await handlers.get('task-inspector:open')!(null, target)).toEqual({ ok: true });
    expect(resolve).toHaveBeenCalledWith(target);
    expect(shell.openExternal).toHaveBeenCalledWith('https://example.test/recorded');
    resolve.mockImplementation(() => {
      throw new Error('Target unavailable');
    });
    expect(await handlers.get('task-inspector:open')!(null, target)).toMatchObject({ ok: false });
    expect(shell.openExternal).toHaveBeenCalledTimes(1);
  } finally {
    edit.mockRestore();
    resolve.mockRestore();
  }
});

it('routes provider readiness reads separately from explicit paid checks', async () => {
  expect(await handlers.get('provider:readiness')!(null, 'claude')).toEqual({ state: 'unchecked' });
  expect(readinessMocks.read).toHaveBeenCalledWith('claude');
  expect(readinessMocks.check).not.toHaveBeenCalled();
  expect(await handlers.get('provider:readiness')!(null, 'claude', true)).toEqual({
    state: 'responding',
    checkedAt: 1,
  });
  expect(readinessMocks.check).toHaveBeenCalledWith('claude');
});

it('mints a manager request before IPC delivery and binds the send to its stored content and exact owner', async () => {
  const { claudeSessionStore } = await import('./services/claudeSessionStore');
  const { managerRequests } = await import('./services/managerRequestService');
  const { claudemonSessionClient } = await import('./services/claudemonSessionClient');
  const snap = vi.spyOn(claudeSessionStore, 'getSnapshot').mockImplementation((id) => ({
    sessionId: id, cwd: '/project', status: 'active', isWakeTarget: id !== 'plain-chat',
  }) as never);
  const service = managerRequests();
  const send = vi.spyOn(claudemonSessionClient, 'message').mockImplementation(async (_id, content, _signatures, source) => {
    expect(content).toBe('Original request');
    expect(source?.requestId).toBeTruthy();
    service.finishDelivery(source!.requestId, source!.deliveryId, 'accepted');
    return { ok: true };
  });
  try {
    const prepared = await handlers.get('manager-request:prepare')!(null, 'request-manager', 'Original request');
    expect(prepared).toMatchObject({ available: true, delivery: 'pending' });
    expect(send).not.toHaveBeenCalled();
    const result = await handlers.get('claude:message')!(null, 'request-manager', 'Caller cannot replace the stored content', prepared.requestId);
    expect(result).toMatchObject({ ok: true, requestId: prepared.requestId, delivery: 'accepted' });
    await handlers.get('claude:message')!(null, 'request-manager', 'retry', prepared.requestId);
    expect(send).toHaveBeenCalledOnce();
    await expect(handlers.get('claude:message')!(null, 'foreign-manager', 'steal', prepared.requestId)).rejects.toThrow(/unavailable/);
    expect(await handlers.get('manager-request:prepare')!(null, 'plain-chat', 'Ordinary conversation')).toMatchObject({ available: false });
  } finally {
    snap.mockRestore();
    send.mockRestore();
  }
});
