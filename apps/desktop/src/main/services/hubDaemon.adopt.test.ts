/**
 * Adopt-don't-kill regression guards for hubDaemon, mirroring
 * claudemonDaemon.adopt.test.ts:
 *   - a HEALTHY external hub (`workspacer serve`) is ADOPTED (no kill/spawn),
 *   - an UNHEALTHY/absent listener gets the classic kill-stale + spawn,
 *   - stop/setRemoteShare never signal or restart an adopted hub,
 *   - getRemoteShareInfo surfaces the adopted state for the UI.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { EventEmitter } from 'events';

const probeHealth = vi.fn<(url: string, t?: number, signal?: AbortSignal) => Promise<boolean>>();
let restartDelay: number | null = null;
const killStaleListener = vi.fn();
const waitForHealth = vi.fn().mockResolvedValue(undefined);
const gracefulStop = vi.fn().mockResolvedValue(undefined);

vi.mock('../lib/daemonUtils', () => ({
  probeHealth: (...a: [string, number?, AbortSignal?]) => probeHealth(...a),
  killStaleListener: (...a: unknown[]) => killStaleListener(...a),
  waitForHealth: (...a: unknown[]) => waitForHealth(...a),
  gracefulStop: (...a: unknown[]) => gracefulStop(...a),
  // Mirrors the real helper closely enough to assert what lands in the child's
  // environment (the token must ride there, not in argv).
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

// In-memory fs: the module reads/writes the token + remote-share flag at load
// time; none of that should touch the real config dir from a test.
vi.mock('fs', () => ({
  readFileSync: vi.fn(() => {
    throw new Error('ENOENT');
  }),
  writeFileSync: vi.fn(),
  mkdirSync: vi.fn(),
  rmSync: vi.fn(),
  existsSync: vi.fn(() => true),
  readdirSync: vi.fn(() => ['editor']), // plugins dir non-empty → no seeding
  cpSync: vi.fn(),
}));

vi.mock('./systemNotice', () => ({ notifySystem: vi.fn() }));
vi.mock('./configService', () => ({ getConfigDir: () => '/tmp/wks-test-config' }));
vi.mock('./claudemonDaemon', () => ({
  CLAUDEMON_API_URL: 'http://127.0.0.1:7891',
  isClaudemonAdopted: () => false,
}));
vi.mock('./brainDelegation', () => ({
  DELEGATE_CATALOG_TO_BRAIN: true,
  DESKTOP_RENDERER_USES_BUS: true,
}));
vi.mock('./remoteServer', () => ({ getRemoteServer: () => null, getPairedWorkerInfo: () => null }));

async function loadModule() {
  vi.resetModules();
  return import('./hubDaemon');
}

beforeEach(() => {
  probeHealth.mockReset();
  killStaleListener.mockClear();
  waitForHealth.mockClear().mockResolvedValue(undefined);
  gracefulStop.mockClear().mockResolvedValue(undefined);
  spawnMock.mockClear();
  delete process.env.WORKSPACER_REMOTE_SHARE;
  delete process.env.WORKSPACER_REMOTE_ADDR;
  delete process.env.WORKSPACER_RUST_HUB_BINARY;
  restartDelay = null;
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('hubDaemon adopt-vs-spawn', () => {
  it('adopts a healthy external hub: no kill, no spawn, surfaced in share info', async () => {
    probeHealth.mockResolvedValue(true);
    const mod = await loadModule();
    await mod.startHub();
    expect(spawnMock).not.toHaveBeenCalled();
    expect(killStaleListener).not.toHaveBeenCalled();
    expect(mod.isHubAdopted()).toBe(true);
    expect((await mod.getRemoteShareInfo()).hubAdopted).toBe(true);
  });

  it('kills stale + spawns when the probe finds nothing healthy', async () => {
    probeHealth.mockResolvedValue(false);
    const mod = await loadModule();
    await mod.startHub();
    // Third arg is the hub's own binary path (zombie-owner kill escalation).
    expect(killStaleListener).toHaveBeenCalledWith(7895, 'hub', expect.stringContaining('hub'));
    expect(spawnMock).toHaveBeenCalledTimes(1);
    expect(mod.isHubAdopted()).toBe(false);
    expect((await mod.getRemoteShareInfo()).hubAdopted).toBe(false);
  });

  it('stopHub never signals an adopted hub', async () => {
    probeHealth.mockResolvedValue(true);
    const mod = await loadModule();
    await mod.startHub();
    await mod.stopHub();
    expect(gracefulStop).not.toHaveBeenCalled();
    expect(mod.isHubAdopted()).toBe(false); // re-probe fresh on a later start
  });

  it('stopHub gracefully stops an OWNED child', async () => {
    probeHealth.mockResolvedValue(false);
    const mod = await loadModule();
    await mod.startHub();
    const child = spawnMock.mock.results[0]!.value;
    await mod.stopHub();
    expect(gracefulStop).toHaveBeenCalledWith(child, 'hub', 6000);
  });

  // The bus token is minted on every run, not just when remote sharing is on,
  // so an argv flag would publish it through /proc/<pid>/cmdline (0444) on the
  // default desktop launch — while the same secret sits at 0600 on disk.
  it('passes the hub token in the environment, never in argv', async () => {
    probeHealth.mockResolvedValue(false);
    const mod = await loadModule();
    await mod.startHub();

    const [, args, opts] = spawnMock.mock.calls[0] as unknown as [
      string,
      string[],
      { env: Record<string, string> },
    ];
    const token = mod.getHubToken();
    expect(token).not.toBe('');
    expect(args).not.toContain('--token');
    expect(args).not.toContain(token);
    expect(opts.env.HUB_TOKEN).toBe(token);
  });

  it('setRemoteShare on an adopted hub persists the flag but does not restart it', async () => {
    probeHealth.mockResolvedValue(true);
    const mod = await loadModule();
    await mod.startHub();
    const info = await mod.setRemoteShare(true);
    expect(gracefulStop).not.toHaveBeenCalled();
    expect(spawnMock).not.toHaveBeenCalled();
    expect(info.hubAdopted).toBe(true);
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}
async function drain() {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}

describe('Rust hub owner generations', () => {
  for (const healthy of [true, false]) {
    it(`stop fences a late initial probe (${healthy ? 'healthy' : 'absent'})`, async () => {
      const probe = deferred<boolean>();
      probeHealth.mockReturnValue(probe.promise);
      const mod = await loadModule();
      const starting = mod.startHub();
      const rejected = expect(starting).rejects.toThrow(/cancelled/);
      const signal = probeHealth.mock.calls[0][2];
      await mod.stopHub();
      expect(signal?.aborted).toBe(true);
      probe.resolve(healthy);
      await rejected;
      expect(mod.isHubAdopted()).toBe(false);
      expect(spawnMock).not.toHaveBeenCalled();
      expect(killStaleListener).not.toHaveBeenCalled();
    });
  }
  it('an earlier probe cannot adopt or reset a replacement startup', async () => {
    const oldProbe = deferred<boolean>();
    probeHealth.mockReturnValueOnce(oldProbe.promise).mockResolvedValue(false);
    const mod = await loadModule();
    const oldStart = mod.startHub();
    const rejected = expect(oldStart).rejects.toThrow(/cancelled/);
    await mod.stopHub();
    await mod.startHub();
    oldProbe.resolve(true);
    await rejected;
    expect(mod.isHubAdopted()).toBe(false);
    await mod.startHub();
    expect(spawnMock).toHaveBeenCalledTimes(1);
    await mod.stopHub();
  });
  it('an old child exit cannot clear the new child or schedule its restart', async () => {
    probeHealth.mockResolvedValue(false);
    const mod = await loadModule();
    await mod.startHub();
    const old = spawnMock.mock.results[0].value;
    await mod.stopHub();
    await mod.startHub();
    const runtime = await import('./agentRuntimeStatus');
    const phase = vi.spyOn(runtime, 'noteRuntimePhase');
    old.emit('exit', 1, null);
    await drain();
    expect(phase).not.toHaveBeenCalled();
    await mod.startHub();
    expect(spawnMock).toHaveBeenCalledTimes(2);
    await mod.stopHub();
    expect(gracefulStop.mock.calls.at(-1)?.[0]).toBe(spawnMock.mock.results[1].value);
  });
  it('stop aborts owned health and a late success cannot complete the old start', async () => {
    probeHealth.mockResolvedValue(false);
    const oldHealth = deferred<void>();
    waitForHealth.mockReturnValueOnce(oldHealth.promise).mockResolvedValue(undefined);
    const mod = await loadModule();
    const oldStart = mod.startHub();
    const rejected = expect(oldStart).rejects.toThrow(/cancelled/);
    await drain();
    const signal = waitForHealth.mock.calls[0][3] as AbortSignal;
    await mod.stopHub();
    expect(signal.aborted).toBe(true);
    await mod.startHub();
    oldHealth.resolve();
    await rejected;
    await mod.startHub();
    expect(spawnMock).toHaveBeenCalledTimes(2);
    await mod.stopHub();
  });
  it('start waits for owned shutdown before probing or spawning again', async () => {
    probeHealth.mockResolvedValue(false);
    const mod = await loadModule();
    await mod.startHub();
    const ended = deferred<void>();
    gracefulStop.mockReturnValueOnce(ended.promise);
    const stopping = mod.stopHub();
    const restarting = mod.startHub();
    await drain();
    expect(probeHealth).toHaveBeenCalledTimes(1);
    expect(spawnMock).toHaveBeenCalledTimes(1);
    ended.resolve();
    await stopping;
    await restarting;
    expect(spawnMock).toHaveBeenCalledTimes(2);
    await mod.stopHub();
  });
  it('stop cancels queued restarts even if a later start has reset the stop flag', async () => {
    vi.useFakeTimers();
    restartDelay = 100;
    probeHealth.mockResolvedValue(false);
    const mod = await loadModule();
    await mod.startHub();
    spawnMock.mock.results[0].value.emit('exit', 1, null);
    await mod.stopHub();
    await mod.startHub();
    await vi.advanceTimersByTimeAsync(1000);
    expect(spawnMock).toHaveBeenCalledTimes(2);
    await mod.stopHub();
  });
  it('a crash retry adopts a healthy external replacement without killing it', async () => {
    vi.useFakeTimers();
    restartDelay = 100;
    probeHealth.mockResolvedValueOnce(false).mockResolvedValue(true);
    const mod = await loadModule();
    await mod.startHub();
    spawnMock.mock.results[0].value.emit('exit', 1, null);
    await vi.advanceTimersByTimeAsync(200);
    expect(mod.isHubAdopted()).toBe(true);
    expect(spawnMock).toHaveBeenCalledTimes(1);
    expect(killStaleListener).toHaveBeenCalledTimes(1);
    await mod.stopHub();
    expect(gracefulStop).not.toHaveBeenCalled();
  });
  it('launches only the Rust executable and Rust serve flags', async () => {
    probeHealth.mockResolvedValue(false);
    const mod = await loadModule();
    await mod.startHub();
    const [binary, args] = spawnMock.mock.calls[0] as unknown as [string, string[]];
    expect(binary).toMatch(/workspacer-rust(?:\.exe)?$/);
    expect(binary).toContain('hub-rs');
    expect(args).toContain('serve');
    expect(args).toContain('--hub-only');
    expect(args).toContain('--external-claudemon');
    expect(args).not.toContain('--brain-scope');
    expect(args).not.toContain('--claudemon-events');
    expect(args).not.toContain('--addr');
    await mod.stopHub();
  });
});
