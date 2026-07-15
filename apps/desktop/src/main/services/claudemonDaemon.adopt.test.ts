/**
 * Adopt-don't-kill regression guards for claudemonDaemon:
 *   - a HEALTHY external daemon on the port is ADOPTED (no kill, no spawn),
 *   - an UNHEALTHY/absent listener gets the classic kill-stale + spawn,
 *   - stop only signals an OWNED child — an adopted daemon is left running.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { EventEmitter } from 'events';

const probeHealth = vi.fn<(url: string, t?: number) => Promise<boolean>>();
const killStaleListener = vi.fn();
const waitForHealth = vi.fn().mockResolvedValue(undefined);
const gracefulStop = vi.fn().mockResolvedValue(undefined);

vi.mock('../lib/daemonUtils', () => ({
  probeHealth: (...a: [string, number?]) => probeHealth(...a),
  killStaleListener: (...a: unknown[]) => killStaleListener(...a),
  waitForHealth: (...a: unknown[]) => waitForHealth(...a),
  gracefulStop: (...a: unknown[]) => gracefulStop(...a),
  daemonSpawnOptions: () => ({ stdio: ['pipe', 'pipe', 'pipe'] }),
  PORTS: { claudemonHook: 7890, claudemonApi: 7891, hub: 7895, mcpFacade: 7897 },
  RestartBackoff: class {
    markStarted() {}
    reset() {}
    nextDelay() {
      return null; // never restart in tests
    }
  },
}));

// A minimal fake child: enough surface for launch()'s wiring + gracefulStop.
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
  child.pid = 4242;
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

vi.mock('fs', () => ({ existsSync: vi.fn(() => true) }));
vi.mock('./systemNotice', () => ({ notifySystem: vi.fn() }));
vi.mock('./configService', () => ({
  configService: { getConfig: () => ({}) },
  getConfigDir: () => '/tmp/wks-test-config',
}));

async function loadModule() {
  vi.resetModules();
  return import('./claudemonDaemon');
}

beforeEach(() => {
  probeHealth.mockReset();
  killStaleListener.mockClear();
  waitForHealth.mockClear().mockResolvedValue(undefined);
  gracefulStop.mockClear().mockResolvedValue(undefined);
  spawnMock.mockClear();
});

describe('claudemonDaemon adopt-vs-spawn', () => {
  it('adopts a healthy external daemon: no kill, no spawn, adopted flagged', async () => {
    probeHealth.mockResolvedValue(true);
    const mod = await loadModule();
    await mod.startClaudemon();
    expect(spawnMock).not.toHaveBeenCalled();
    expect(killStaleListener).not.toHaveBeenCalled();
    expect(mod.isClaudemonAdopted()).toBe(true);
  });

  it('kills stale + spawns when the probe finds nothing healthy', async () => {
    probeHealth.mockResolvedValue(false);
    const mod = await loadModule();
    await mod.startClaudemon();
    // Third arg is the daemon's own binary path — the zombie-owner escalation
    // (kill orphaned instances of our exe by path) needs it.
    expect(killStaleListener).toHaveBeenCalledWith(
      7890,
      'claudemon',
      expect.stringContaining('claudemon'),
    );
    expect(killStaleListener).toHaveBeenCalledWith(
      7891,
      'claudemon',
      expect.stringContaining('claudemon'),
    );
    expect(spawnMock).toHaveBeenCalledTimes(1);
    expect(waitForHealth).toHaveBeenCalled();
    expect(mod.isClaudemonAdopted()).toBe(false);
  });

  it('stopClaudemon never signals an adopted daemon', async () => {
    probeHealth.mockResolvedValue(true);
    const mod = await loadModule();
    await mod.startClaudemon();
    await mod.stopClaudemon();
    expect(gracefulStop).not.toHaveBeenCalled();
    // The adopted flag clears on stop so a later start re-probes fresh.
    expect(mod.isClaudemonAdopted()).toBe(false);
  });

  it('stopClaudemon gracefully stops an OWNED child', async () => {
    probeHealth.mockResolvedValue(false);
    const mod = await loadModule();
    await mod.startClaudemon();
    const child = spawnMock.mock.results[0]!.value;
    await mod.stopClaudemon();
    expect(gracefulStop).toHaveBeenCalledWith(child, 'claudemon');
  });

  it('startClaudemon is idempotent while adopted (no second probe/spawn)', async () => {
    probeHealth.mockResolvedValue(true);
    const mod = await loadModule();
    await mod.startClaudemon();
    await mod.startClaudemon();
    expect(probeHealth).toHaveBeenCalledTimes(1);
    expect(spawnMock).not.toHaveBeenCalled();
  });
});
