/**
 * The facade's spawn contract. Two things are load-bearing and neither is
 * visible from the outside once the process is up:
 *   - the bus token travels in the environment, not argv, because
 *     /proc/<pid>/cmdline is world-readable while the token file is 0600;
 *   - WKS_MCP_TOKEN is NOT a shared process credential. Each launched session
 *     gets its own lifecycle-bound identity bearer in the generated MCP URL;
 *     the facade verifies that token per request. Untokened access is a
 *     separate explicit compatibility dial. The assertion below prevents a
 *     shared secret from accidentally replacing those per-session identities.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { EventEmitter } from 'events';

const killStaleListener = vi.fn();
const waitForHealth = vi.fn().mockResolvedValue(undefined);
const facadeHealth = {
  status: 'ok',
  service: 'workspacer-mcp-facade',
  hubConnected: true,
  pluginCatalogReady: true,
  listenAddr: '127.0.0.1:7897',
  hubUrl: 'ws://127.0.0.1:7895/bus',
};
const fetchMock = vi.fn();
const gracefulStop = vi.fn().mockResolvedValue(undefined);
let restartDelay: number | null = null;

vi.mock('../lib/daemonUtils', () => ({
  killStaleListener: (...a: unknown[]) => killStaleListener(...a),
  waitForHealth: (...a: unknown[]) => waitForHealth(...a),
  gracefulStop: (...a: unknown[]) => gracefulStop(...a),
  daemonSpawnOptions: (extraEnv?: Record<string, string>) => ({
    stdio: ['pipe', 'pipe', 'pipe'],
    env: { ...extraEnv },
  }),
  PORTS: { claudemonHook: 7890, claudemonApi: 7891, hub: 7895, mcpFacade: 7897 },
  RestartBackoff: class {
    markStarted() {}
    reset() {}
    nextDelay() {
      return restartDelay;
    }
  },
}));

const HUB_TOKEN = 'tok-abc123';
vi.mock('./hubDaemon', () => ({
  hubBusUrl: () => 'ws://127.0.0.1:7895/bus',
  getHubToken: () => HUB_TOKEN,
}));

/** The config the daemon reads `facade.untokenedAccess` from; per-test shape. */
let mockConfig: Record<string, unknown> = {};
vi.mock('./configService', () => ({
  configService: { getConfig: () => mockConfig },
}));

vi.mock('electron', () => ({
  app: { getAppPath: () => '/tmp/app', isPackaged: false },
}));

vi.mock('fs', () => ({ existsSync: vi.fn(() => true) }));

function fakeChild() {
  const child = new EventEmitter() as EventEmitter & Record<string, unknown>;
  child.stdout = null;
  child.stderr = null;
  child.stdin = null;
  child.pid = 5150;
  child.exitCode = null;
  child.signalCode = null;
  child.kill = vi.fn();
  return child;
}

const spawnMock = vi.fn(() => fakeChild());
vi.mock('child_process', () => ({ spawn: (...a: unknown[]) => spawnMock(...(a as [])) }));

async function loadModule() {
  vi.resetModules();
  return import('./mcpFacadeDaemon');
}

beforeEach(() => {
  vi.spyOn(console, 'log').mockImplementation(() => {});
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  killStaleListener.mockClear();
  spawnMock.mockClear();
  fetchMock.mockReset();
  // First probe sees no adoptable listener; the post-launch readiness probe
  // sees this facade's identity + connected hub + synchronized catalog.
  fetchMock
    .mockRejectedValueOnce(new Error('connection refused'))
    .mockResolvedValue({ ok: true, json: async () => facadeHealth });
  vi.stubGlobal('fetch', fetchMock);
  gracefulStop.mockClear();
  mockConfig = {};
  restartDelay = null;
});

describe('mcp facade spawn', () => {
  it('restarts through the shared launch gate when a once-ready facade becomes unready', async () => {
    fetchMock.mockReset();
    fetchMock
      .mockRejectedValueOnce(new Error('no listener'))
      .mockResolvedValueOnce({ ok: true, json: async () => facadeHealth })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({ ...facadeHealth, pluginCatalogReady: false }),
      })
      .mockRejectedValueOnce(new Error('old listener stopped'))
      .mockResolvedValue({ ok: true, json: async () => facadeHealth });
    const mod = await loadModule();
    await mod.ensureMcpFacadeReady();

    expect(spawnMock).toHaveBeenCalledTimes(2);
    expect(gracefulStop).toHaveBeenCalledWith(expect.anything(), 'mcp');
  });

  it('adopts a healthy externally supervised facade and leaves it running on stop', async () => {
    fetchMock.mockReset().mockResolvedValue({ ok: true, json: async () => facadeHealth });
    const mod = await loadModule();
    await mod.startMcpFacade();

    expect(spawnMock).not.toHaveBeenCalled();
    expect(killStaleListener).not.toHaveBeenCalled();
    await mod.stopMcpFacade();
    expect(gracefulStop).toHaveBeenCalledWith(null, 'mcp');
  });

  it('cancels a stale owned-child restart when an external facade is adopted', async () => {
    vi.useFakeTimers();
    restartDelay = 50;
    try {
      const mod = await loadModule();
      await mod.startMcpFacade();
      const owned = spawnMock.mock.results[0]?.value as EventEmitter;

      // The owned process crashes and schedules a restart. Before its timer
      // fires, a later explicit start sees workspacer serve's healthy facade.
      owned.emit('exit', 1, null);
      fetchMock.mockReset().mockResolvedValue({ ok: true, json: async () => facadeHealth });
      await mod.startMcpFacade();
      expect(spawnMock).toHaveBeenCalledTimes(1);

      await vi.advanceTimersByTimeAsync(100);
      expect(spawnMock).toHaveBeenCalledTimes(1);
      expect(killStaleListener).toHaveBeenCalledTimes(1);
      // Stopping an adopted listener only clears local state; there is no
      // owned child left for gracefulStop to terminate.
      await mod.stopMcpFacade();
      expect(gracefulStop).toHaveBeenLastCalledWith(null, 'mcp');
    } finally {
      vi.useRealTimers();
    }
  });

  it.each([
    { status: 'ok' },
    { ...facadeHealth, service: 'some-other-daemon' },
    { ...facadeHealth, hubConnected: false },
    { ...facadeHealth, pluginCatalogReady: false },
    { ...facadeHealth, listenAddr: '127.0.0.1:9999' },
    { ...facadeHealth, hubUrl: 'ws://127.0.0.1:9998/bus' },
  ])('does not adopt a 200 response without facade readiness: %j', async (body) => {
    fetchMock.mockReset();
    fetchMock
      .mockResolvedValueOnce({ ok: true, json: async () => body })
      .mockResolvedValue({ ok: true, json: async () => facadeHealth });
    const mod = await loadModule();
    await mod.startMcpFacade();
    expect(spawnMock).toHaveBeenCalledTimes(1);
    expect(killStaleListener).toHaveBeenCalledTimes(1);
  });

  it('cleans up an owned process whose facade-specific readiness fails', async () => {
    fetchMock.mockReset();
    fetchMock.mockRejectedValueOnce(new Error('connection refused')).mockResolvedValueOnce({
      ok: true,
      json: async () => ({ ...facadeHealth, hubConnected: false }),
    });
    const mod = await loadModule();
    await expect(mod.startMcpFacade()).rejects.toThrow(/hub and plugin catalog readiness/);
    expect(gracefulStop).toHaveBeenCalledWith(expect.anything(), 'mcp');
  });

  it('gives the facade the bus token in the environment', async () => {
    const mod = await loadModule();
    await mod.startMcpFacade();

    const [, , opts] = spawnMock.mock.calls[0] as unknown as [
      string,
      string[],
      { env: Record<string, string> },
    ];
    expect(mod.getMcpFacadeToken()).toBe(HUB_TOKEN);
    expect(opts.env.HUB_TOKEN).toBe(HUB_TOKEN);
  });

  it("does not arm the facade's bearer check while no client can send the header", async () => {
    const mod = await loadModule();
    await mod.startMcpFacade();

    const [, , opts] = spawnMock.mock.calls[0] as unknown as [
      string,
      string[],
      { env: Record<string, string> },
    ];
    expect(opts.env.WKS_MCP_TOKEN).toBeUndefined();
  });

  it('never puts a token in argv', async () => {
    const mod = await loadModule();
    await mod.startMcpFacade();

    const [, args] = spawnMock.mock.calls[0] as unknown as [string, string[]];
    expect(args).not.toContain('--token');
    expect(args).not.toContain('--mcp-token');
    expect(args).not.toContain(HUB_TOKEN);
    // Also the absent-key case for the untokened dial: no facade config key,
    // no --untokened flag — the facade keeps its own operator default.
    expect(args).toEqual(['--addr', '127.0.0.1:7897', '--hub', 'ws://127.0.0.1:7895/bus']);
  });

  it.each(['operator', 'view', 'deny'] as const)(
    'passes --untokened %s from facade.untokenedAccess',
    async (dial) => {
      mockConfig = { facade: { untokenedAccess: dial } };
      const mod = await loadModule();
      await mod.startMcpFacade();

      const [, args] = spawnMock.mock.calls[0] as unknown as [string, string[]];
      expect(args).toEqual([
        '--addr',
        '127.0.0.1:7897',
        '--hub',
        'ws://127.0.0.1:7895/bus',
        '--untokened',
        dial,
      ]);
    },
  );

  it('omits --untokened for an invalid dial value (the binary would refuse to start)', async () => {
    mockConfig = { facade: { untokenedAccess: 'viewer' } };
    const mod = await loadModule();
    await mod.startMcpFacade();

    const [, args] = spawnMock.mock.calls[0] as unknown as [string, string[]];
    expect(args).not.toContain('--untokened');
    expect(args).not.toContain('viewer');
  });
});
