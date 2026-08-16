/**
 * The facade's spawn contract. Two things are load-bearing and neither is
 * visible from the outside once the process is up:
 *   - the bus token travels in the environment, not argv, because
 *     /proc/<pid>/cmdline is world-readable while the token file is 0600;
 *   - WKS_MCP_TOKEN is NOT set. Setting it arms cmd/mcp's bearer check on /mcp
 *     and /sse, and no client can send that header yet (mcpConfig.ts's
 *     supervisor entry has no `headers`; managedSpawn passes claudemon a bare
 *     URL string), so arming it silently strips every mcp__workspacer__ tool
 *     from the supervisor and its workers. The assertion below is a tripwire:
 *     whoever sets it has to land the two client sides in the same change.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { EventEmitter } from 'events';

const killStaleListener = vi.fn();
const waitForHealth = vi.fn().mockResolvedValue(undefined);

vi.mock('../lib/daemonUtils', () => ({
  killStaleListener: (...a: unknown[]) => killStaleListener(...a),
  waitForHealth: (...a: unknown[]) => waitForHealth(...a),
  gracefulStop: vi.fn().mockResolvedValue(undefined),
  probeHealth: vi.fn().mockResolvedValue(false),
  daemonSpawnOptions: (extraEnv?: Record<string, string>) => ({
    stdio: ['pipe', 'pipe', 'pipe'],
    env: { ...extraEnv },
  }),
  PORTS: { claudemonHook: 7890, claudemonApi: 7891, hub: 7895, mcpFacade: 7897 },
  RestartBackoff: class {
    markStarted() {}
    reset() {}
    nextDelay() {
      return null; // never restart in tests
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
  mockConfig = {};
});

describe('mcp facade spawn', () => {
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
