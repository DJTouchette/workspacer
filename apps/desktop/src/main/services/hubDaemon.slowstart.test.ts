/**
 * Slow-start patience guards for hubDaemon: a hub that misses the first
 * health round but whose PROCESS is still alive must not be abandoned.
 * The old single-round wait rejected startHub() permanently (the child never
 * exits, so the exit-driven restart path never re-arms), orphaning the desktop
 * from a hub that came up healthy seconds later — plugins, remote sharing,
 * federation, and the MCP facade all skipped for the whole session.
 *
 * Contract under test (awaitHealthPatiently):
 *   - healthy in round one → resolve, no notices (happy path byte-identical),
 *   - late health → warn notice, keep polling, info notice + resolve,
 *   - child exit mid-wait → reject (the exit handler owns restarts),
 *   - patience exhausted with a live-but-wedged child → reject.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { EventEmitter } from 'events';

const probeHealth = vi.fn<(url: string, t?: number) => Promise<boolean>>();
const killStaleListener = vi.fn();
const waitForHealth = vi.fn<(...a: unknown[]) => Promise<void>>();
const gracefulStop = vi.fn().mockResolvedValue(undefined);

vi.mock('../lib/daemonUtils', () => ({
  probeHealth: (...a: [string, number?]) => probeHealth(...a),
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
      return null; // never restart in tests
    }
  },
}));

function fakeChild() {
  const child = new EventEmitter() as EventEmitter & {
    stdout: null;
    stderr: null;
    stdin: null;
    pid: number;
    exitCode: null;
    signalCode: null;
    kill: () => void;
  };
  child.stdout = null;
  child.stderr = null;
  child.stdin = null;
  child.pid = 4243;
  child.exitCode = null;
  child.signalCode = null;
  child.kill = vi.fn();
  return child;
}

const spawnMock = vi.fn(() => fakeChild());
vi.mock('child_process', () => ({ spawn: (...a: unknown[]) => spawnMock(...(a as [])) }));

vi.mock('electron', () => ({
  app: {
    getPath: () => '/tmp',
    getAppPath: () => '/tmp/app',
    isPackaged: false,
  },
}));

vi.mock('fs', () => ({
  readFileSync: vi.fn(() => {
    throw new Error('ENOENT');
  }),
  writeFileSync: vi.fn(),
  mkdirSync: vi.fn(),
  rmSync: vi.fn(),
  existsSync: vi.fn(() => true),
  readdirSync: vi.fn(() => ['editor']),
  cpSync: vi.fn(),
}));

const notifySystem = vi.fn();
vi.mock('./systemNotice', () => ({ notifySystem: (...a: unknown[]) => notifySystem(...a) }));
vi.mock('./configService', () => ({ getConfigDir: () => '/tmp/wks-test-config' }));
vi.mock('./claudemonDaemon', () => ({
  CLAUDEMON_API_URL: 'http://127.0.0.1:7891',
  isClaudemonAdopted: () => false,
}));
vi.mock('./brainDelegation', () => ({
  DELEGATE_CATALOG_TO_BRAIN: true,
  DESKTOP_RENDERER_USES_BUS: true,
}));
vi.mock('./remoteServer', () => ({ getRemoteServer: () => null }));

async function loadModule() {
  vi.resetModules();
  return import('./hubDaemon');
}

/** Yield until pending microtask chains (the poll loop's awaits) settle. */
async function drain() {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}

beforeEach(() => {
  probeHealth.mockReset().mockResolvedValue(false); // never adopt → spawn path
  killStaleListener.mockClear();
  waitForHealth.mockReset();
  gracefulStop.mockClear().mockResolvedValue(undefined);
  spawnMock.mockClear();
  notifySystem.mockClear();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('hubDaemon slow start', () => {
  it('healthy in the first round: resolves with no notices', async () => {
    waitForHealth.mockResolvedValue(undefined);
    const mod = await loadModule();
    await mod.startHub();
    expect(waitForHealth).toHaveBeenCalledTimes(1);
    expect(notifySystem).not.toHaveBeenCalled();
  });

  it('late health: warns, keeps polling, then reports recovery and resolves', async () => {
    waitForHealth
      .mockRejectedValueOnce(new Error('hub /health did not respond within 5000ms'))
      .mockResolvedValueOnce(undefined);
    const mod = await loadModule();
    await mod.startHub();

    expect(waitForHealth).toHaveBeenCalledTimes(2);
    expect(spawnMock).toHaveBeenCalledTimes(1); // never respawned, just waited
    const notices = notifySystem.mock.calls.map((c) => c[0] as Record<string, unknown>);
    expect(notices).toHaveLength(2);
    expect(notices[0]).toMatchObject({ level: 'warn', key: 'hub-start' });
    expect(notices[1]).toMatchObject({ level: 'info', key: 'hub-start' });
  });

  it('child exit mid-wait rejects instead of polling a corpse', async () => {
    let signalSeen: AbortSignal | undefined;
    waitForHealth.mockImplementation(async (...a: unknown[]) => {
      signalSeen = a[3] as AbortSignal;
      if (signalSeen?.aborted) throw new Error('hub health check cancelled (daemon exited early)');
      // Real rounds take ≥ timeoutMs; a real timer here lets the exit event land
      // between rounds instead of the loop spinning on microtasks.
      await new Promise((r) => setTimeout(r, 5));
      if (signalSeen?.aborted) throw new Error('hub health check cancelled (daemon exited early)');
      throw new Error('hub /health did not respond within 5000ms');
    });
    const mod = await loadModule();
    const ready = mod.startHub();
    const guard = expect(ready).rejects.toThrow(/cancelled/);
    await drain();
    const child = spawnMock.mock.results[0]!.value as EventEmitter;
    child.emit('exit', 1, null);
    await guard;
    expect(signalSeen?.aborted).toBe(true);
  });

  it('gives up on a live-but-wedged hub once patience runs out', async () => {
    vi.useFakeTimers({ toFake: ['Date'] }); // freeze Date.now; timers stay real
    const t0 = Date.now();
    waitForHealth
      .mockRejectedValueOnce(new Error('hub /health did not respond within 5000ms'))
      .mockImplementation(async () => {
        // The wedged daemon: rounds keep failing while the clock runs out.
        vi.setSystemTime(t0 + 300_000);
        throw new Error('hub /health did not respond within 5000ms');
      });
    const mod = await loadModule();
    await expect(mod.startHub()).rejects.toThrow(/never answered/);
    const notices = notifySystem.mock.calls.map((c) => c[0] as Record<string, unknown>);
    expect(notices).toHaveLength(1); // the slow-start warn; no recovery info
    expect(notices[0]).toMatchObject({ level: 'warn', key: 'hub-start' });
  });
});
