/**
 * Adopt-don't-kill regression guards for hubDaemon, mirroring
 * claudemonDaemon.adopt.test.ts:
 *   - a HEALTHY external hub (`workspacer serve`) is ADOPTED (no kill/spawn),
 *   - an UNHEALTHY/absent listener gets the classic kill-stale + spawn,
 *   - stop/setRemoteShare never signal or restart an adopted hub,
 *   - getRemoteShareInfo surfaces the adopted state for the UI.
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
vi.mock('./remoteServer', () => ({ getRemoteServer: () => null }));

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
});

describe('hubDaemon adopt-vs-spawn', () => {
  it('adopts a healthy external hub: no kill, no spawn, surfaced in share info', async () => {
    probeHealth.mockResolvedValue(true);
    const mod = await loadModule();
    await mod.startHub();
    expect(spawnMock).not.toHaveBeenCalled();
    expect(killStaleListener).not.toHaveBeenCalled();
    expect(mod.isHubAdopted()).toBe(true);
    expect(mod.getRemoteShareInfo().hubAdopted).toBe(true);
  });

  it('kills stale + spawns when the probe finds nothing healthy', async () => {
    probeHealth.mockResolvedValue(false);
    const mod = await loadModule();
    await mod.startHub();
    expect(killStaleListener).toHaveBeenCalledWith(7895, 'hub');
    expect(spawnMock).toHaveBeenCalledTimes(1);
    expect(mod.isHubAdopted()).toBe(false);
    expect(mod.getRemoteShareInfo().hubAdopted).toBe(false);
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
