/**
 * ADDRESSES THIS PROCESS HANDS TO ITSELF.
 *
 * Two shapes cost a whole capability plane apiece and both were silent:
 *
 *  1. `bindAddr().split(':')[0]` is not IPv6-safe. WORKSPACER_REMOTE_ADDR takes
 *     `[fd7a:…]:7895` whenever it is pinned to a tailnet IPv6 (which this
 *     module's own docs recommend), and the split yields `[fd7a` — so busUrl,
 *     the phone QR and the /app link are all unparseable while install.ts still
 *     prints "desktop running on the hub bus" and hubBusClient swallows the
 *     WebSocket SyntaxError into a reconnect loop forever.
 *
 *  2. A CONCRETE non-loopback bind does not answer on loopback. Probing a
 *     hardcoded 127.0.0.1 means startHub() rejects for a hub that is perfectly
 *     healthy, so startHubClient()/startMcpFacade() never run and every
 *     capability main is the sole provider of is never registered — while the
 *     renderer, whose busUrl IS the tailnet IP, connects fine and the app looks
 *     alive.
 *
 * Twinned with services/hub/cmd/workspacer/dialhost_test.go (the Go copy of the
 * same decision, for `workspacer serve --host`).
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { EventEmitter } from 'events';

const probeHealth = vi.fn<(url: string, t?: number) => Promise<boolean>>();
const waitForHealth = vi.fn().mockResolvedValue(undefined);

vi.mock('../lib/daemonUtils', () => ({
  probeHealth: (...a: [string, number?]) => probeHealth(...a),
  killStaleListener: vi.fn(),
  waitForHealth: (...a: unknown[]) => waitForHealth(...a),
  gracefulStop: vi.fn().mockResolvedValue(undefined),
  daemonSpawnOptions: () => ({ stdio: ['pipe', 'pipe', 'pipe'], env: {} }),
  PORTS: { claudemonHook: 7890, claudemonApi: 7891, hub: 7895, mcpFacade: 7897 },
  RestartBackoff: class {
    markStarted() {}
    reset() {}
    nextDelay() {
      return null;
    }
  },
}));

function fakeChild() {
  const child = new EventEmitter() as EventEmitter & Record<string, unknown>;
  child.stdout = new EventEmitter();
  child.stderr = new EventEmitter();
  child.stdin = null;
  child.pid = 4243;
  child.kill = vi.fn();
  return child;
}
const spawnMock = vi.fn(() => fakeChild());
vi.mock('child_process', () => ({ spawn: (...a: unknown[]) => spawnMock(...(a as [])) }));
vi.mock('electron', () => ({
  app: { getPath: () => '/tmp', getAppPath: () => '/tmp/app', isPackaged: false },
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

async function loadModule(remoteAddr?: string) {
  vi.resetModules();
  if (remoteAddr) {
    process.env.WORKSPACER_REMOTE_SHARE = '1';
    process.env.WORKSPACER_REMOTE_ADDR = remoteAddr;
  } else {
    delete process.env.WORKSPACER_REMOTE_SHARE;
    delete process.env.WORKSPACER_REMOTE_ADDR;
  }
  return import('./hubDaemon');
}

beforeEach(() => {
  probeHealth.mockReset().mockResolvedValue(false);
  waitForHealth.mockClear().mockResolvedValue(undefined);
  spawnMock.mockClear();
});

describe('splitHostPort / urlHost — IPv6-safe authority handling', () => {
  it('splits every WORKSPACER_REMOTE_ADDR shape without eating the address', async () => {
    const { splitHostPort } = await loadModule();
    expect(splitHostPort('0.0.0.0:7895')).toEqual({ host: '0.0.0.0', port: '7895' });
    expect(splitHostPort('myhost.tail1234.ts.net:7895')).toEqual({
      host: 'myhost.tail1234.ts.net',
      port: '7895',
    });
    expect(splitHostPort('[::]:7895')).toEqual({ host: '::', port: '7895' });
    expect(splitHostPort('[::1]:7895')).toEqual({ host: '::1', port: '7895' });
    expect(splitHostPort('[fd7a:115c:a1e0::cc3b:4f4a]:7895')).toEqual({
      host: 'fd7a:115c:a1e0::cc3b:4f4a',
      port: '7895',
    });
    // A bare IPv6 literal with no port must not lose its tail.
    expect(splitHostPort('fd7a:115c:a1e0::cc3b:4f4a')).toEqual({
      host: 'fd7a:115c:a1e0::cc3b:4f4a',
      port: '',
    });
  });

  it('brackets an IPv6 literal so it can go in a URL', async () => {
    const { urlHost } = await loadModule();
    expect(urlHost('fd7a:115c:a1e0::1')).toBe('[fd7a:115c:a1e0::1]');
    expect(urlHost('[fd7a:115c:a1e0::1]')).toBe('[fd7a:115c:a1e0::1]');
    expect(urlHost('100.86.79.73')).toBe('100.86.79.73');
  });
});

describe('getRemoteShareInfo URLs are parseable for every supported bind', () => {
  for (const addr of [
    '0.0.0.0:7895',
    '[::]:7895',
    '[::1]:7895',
    '[fd7a:115c:a1e0::cc3b:4f4a]:7895',
    '100.86.79.73:7895',
    'myhost.tail1234.ts.net:7895',
  ]) {
    it(`WORKSPACER_REMOTE_ADDR=${addr}`, async () => {
      const mod = await loadModule(addr);
      const info = await mod.getRemoteShareInfo();
      // A URL the WebSocket constructor throws on is worse than no URL: the
      // transport selector still reports 'bridged' and the failure is a
      // reconnect loop with a success line in the log.
      expect(() => new URL(info.busUrl)).not.toThrow();
      expect(() => new URL(info.remoteUrl)).not.toThrow();
      expect(info.busUrl.startsWith('ws://')).toBe(true);
    });
  }
});

describe('the address main dials for its own hub follows the bind', () => {
  it('a wildcard bind is dialed on loopback (a wildcard names no host)', async () => {
    const mod = await loadModule('0.0.0.0:7895');
    expect(mod.dialAuthority('0.0.0.0:7895')).toBe('127.0.0.1:7895');
    expect(mod.dialAuthority('[::]:7895')).toBe('127.0.0.1:7895');
  });

  it('a CONCRETE bind is dialed at itself — it does not answer on loopback', async () => {
    const mod = await loadModule('100.86.79.73:7895');
    expect(mod.dialAuthority('100.86.79.73:7895')).toBe('100.86.79.73:7895');
    expect(mod.dialAuthority('[fd7a:115c:a1e0::1]:7895')).toBe('[fd7a:115c:a1e0::1]:7895');
    expect(mod.hubHttpUrl()).toBe('http://100.86.79.73:7895');
    expect(mod.hubBusUrl()).toBe('ws://100.86.79.73:7895/bus');
  });

  it('the health probe waits on the address the hub was actually spawned with', async () => {
    const mod = await loadModule('100.86.79.73:7895');
    await mod.startHub();

    const args = spawnMock.mock.calls[0]![1] as unknown as string[];
    const boundTo = args[args.indexOf('--addr') + 1];
    expect(boundTo).toBe('100.86.79.73:7895');

    const probed = String(waitForHealth.mock.calls[0]![0]);
    expect(probed).toBe(`http://${boundTo}/health`);
  });
});

describe('the hub (and therefore the brain) can say why it failed', () => {
  it('forwards the hub child stdout AND stderr to our own', async () => {
    const mod = await loadModule();
    await mod.startHub();
    const child = spawnMock.mock.results[0]!.value as unknown as {
      stdout: { emit: (e: string, d: unknown) => void } | null;
      stderr: { emit: (e: string, d: unknown) => void } | null;
    };

    const outSpy = vi.spyOn(process.stdout, 'write').mockReturnValue(true);
    const errSpy = vi.spyOn(process.stderr, 'write').mockReturnValue(true);
    try {
      child.stdout!.emit('data', 'brain: 403 host not allowed\n');
      child.stderr!.emit('data', 'brain: giving up\n');
      // The other half of today's fix (services/hub supervisor InheritOutput)
      // only reaches a human if this end forwards it: a packaged app has no
      // terminal, so an unforwarded child line goes nowhere at all.
      expect(outSpy.mock.calls.flat().join('')).toContain('403 host not allowed');
      expect(errSpy.mock.calls.flat().join('')).toContain('giving up');
    } finally {
      outSpy.mockRestore();
      errSpy.mockRestore();
    }
  });
});

describe('reverse-proxy hostnames reach the hub it spawns', () => {
  it('passes --trusted-host when the Tailscale HTTPS front is declared', async () => {
    // The persisted list, as setHubTrustedHosts writes it.
    const fs = await import('fs');
    vi.mocked(fs.readFileSync).mockImplementation((p: unknown) => {
      if (String(p).endsWith('hub-trusted-hosts')) return 'node.tail1234.ts.net\n';
      throw new Error('ENOENT');
    });
    const mod = await loadModule();
    await mod.startHub();

    const args = spawnMock.mock.calls[0]![1] as unknown as string[];
    const i = args.indexOf('--trusted-host');
    // Without this the hub's Host/Origin pins 403 every route behind
    // `tailscale serve` — the phone sees a bare error page and the desktop,
    // which dials loopback, sees nothing wrong at all.
    expect(i).toBeGreaterThan(-1);
    expect(args[i + 1]).toBe('node.tail1234.ts.net');

    vi.mocked(fs.readFileSync).mockImplementation(() => {
      throw new Error('ENOENT');
    });
  });

  it('passes no --trusted-host when nothing is declared (opt-in only)', async () => {
    const mod = await loadModule();
    await mod.startHub();
    const args = spawnMock.mock.calls[0]![1] as unknown as string[];
    expect(args).not.toContain('--trusted-host');
  });
});
