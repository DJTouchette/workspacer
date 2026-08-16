/**
 * peers.json editing semantics, pinned:
 *
 * - Validation is the Go loader's (federation.go LoadPeersFile) — name
 *   `[A-Za-z0-9_-]+`, url ws://|wss:// — so a file this service writes is
 *   always one the hub will accept.
 * - Keep-token merge: an entry saved with `token === undefined` inherits the
 *   stored token for that peer name. This is what lets the renderer re-send
 *   existing rows without ever having seen their secrets.
 * - The read side redacts: name/url/hasToken only, never the token itself.
 * - The file carries bearer tokens for other machines → written 0o600.
 * - A successful save restarts the hub (links are dialed at hub startup);
 *   a rejected save must NOT touch the file or the hub.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const h = vi.hoisted(() => ({
  dir: '',
  stopHub: vi.fn<() => Promise<void>>(async () => {}),
  startHub: vi.fn<() => Promise<void>>(async () => {}),
  remoteServer: null as { httpUrl: string } | null,
  ipcHandle: vi.fn(),
}));

vi.mock('./configService', () => ({ getConfigDir: () => h.dir }));
vi.mock('./hubDaemon', () => ({
  stopHub: () => h.stopHub(),
  startHub: () => h.startHub(),
}));
vi.mock('./remoteServer', () => ({ getRemoteServer: () => h.remoteServer }));
vi.mock('electron', () => ({ ipcMain: { handle: h.ipcHandle } }));

import {
  readRedactedPeers,
  savePeersConfig,
  startFederationPeersConfig,
  peersConfigPath,
} from './federationPeersConfig';
import { IPC } from '../shared/ipcChannels';

function writePeersRaw(data: unknown): void {
  fs.writeFileSync(peersConfigPath(), JSON.stringify(data));
}

function readPeersRaw(): Array<Record<string, unknown>> {
  return JSON.parse(fs.readFileSync(peersConfigPath(), 'utf-8'));
}

beforeEach(() => {
  h.dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-fed-peers-'));
  h.remoteServer = null;
  h.stopHub.mockClear();
  h.startHub.mockClear();
  h.ipcHandle.mockClear();
});

afterEach(() => {
  fs.rmSync(h.dir, { recursive: true, force: true });
});

describe('readRedactedPeers', () => {
  it('reads peers.json with tokens redacted to hasToken', () => {
    writePeersRaw([
      { name: 'work', url: 'ws://100.64.1.2:7895/bus', token: 'secret-abc' },
      { name: 'laptop', url: 'wss://laptop.ts.net/bus' },
    ]);
    expect(readRedactedPeers()).toEqual([
      { name: 'work', url: 'ws://100.64.1.2:7895/bus', hasToken: true },
      { name: 'laptop', url: 'wss://laptop.ts.net/bus', hasToken: false },
    ]);
  });

  it('missing file = no peers; corrupt file reads empty (repairable via save)', () => {
    expect(readRedactedPeers()).toEqual([]);
    fs.writeFileSync(peersConfigPath(), '{not json');
    expect(readRedactedPeers()).toEqual([]);
  });
});

describe('savePeersConfig validation (mirrors LoadPeersFile)', () => {
  it.each([
    [[{ name: '', url: 'ws://x/bus' }], /need name and url/],
    [[{ name: 'ok', url: '' }], /need name and url/],
    [[{ name: 'bad name!', url: 'ws://x/bus' }], /letters, digits, - or _/],
    [[{ name: 'ok', url: 'http://x/bus' }], /must be ws:\/\/ or wss:\/\//],
    [
      [
        { name: 'twin', url: 'ws://a/bus' },
        { name: 'twin', url: 'ws://b/bus' },
      ],
      /duplicate peer name/,
    ],
    ['not-an-array', /must be an array/],
  ] as Array<[unknown, RegExp]>)('rejects %j', async (input, err) => {
    const res = await savePeersConfig(input);
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.error).toMatch(err);
    // A rejected save must not touch the file or restart the hub.
    expect(fs.existsSync(peersConfigPath())).toBe(false);
    expect(h.stopHub).not.toHaveBeenCalled();
    expect(h.startHub).not.toHaveBeenCalled();
  });

  it('accepts the exact character set the hub accepts', async () => {
    const res = await savePeersConfig([
      { name: 'Work_PC-2', url: 'wss://work.ts.net/bus', token: 't1' },
    ]);
    expect(res).toEqual({ ok: true });
    expect(readPeersRaw()).toEqual([
      { name: 'Work_PC-2', url: 'wss://work.ts.net/bus', token: 't1' },
    ]);
  });
});

describe('savePeersConfig keep-token merge', () => {
  it('token === undefined keeps the stored token for that peer name', async () => {
    writePeersRaw([{ name: 'work', url: 'ws://old:7895/bus', token: 'keep-me' }]);
    const res = await savePeersConfig([
      { name: 'work', url: 'ws://new-host:7895/bus' }, // token undefined → keep
      { name: 'fresh', url: 'ws://fresh:7895/bus', token: 'new-tok' },
    ]);
    expect(res).toEqual({ ok: true });
    expect(readPeersRaw()).toEqual([
      { name: 'work', url: 'ws://new-host:7895/bus', token: 'keep-me' },
      { name: 'fresh', url: 'ws://fresh:7895/bus', token: 'new-tok' },
    ]);
  });

  it('an explicit token replaces; an explicit empty string clears', async () => {
    writePeersRaw([
      { name: 'a', url: 'ws://a/bus', token: 'old-a' },
      { name: 'b', url: 'ws://b/bus', token: 'old-b' },
    ]);
    const res = await savePeersConfig([
      { name: 'a', url: 'ws://a/bus', token: 'new-a' },
      { name: 'b', url: 'ws://b/bus', token: '' },
    ]);
    expect(res).toEqual({ ok: true });
    expect(readPeersRaw()).toEqual([
      { name: 'a', url: 'ws://a/bus', token: 'new-a' },
      { name: 'b', url: 'ws://b/bus' },
    ]);
  });

  it('an omitted peer is removed, its token gone with it', async () => {
    writePeersRaw([
      { name: 'keep', url: 'ws://keep/bus', token: 'tk' },
      { name: 'drop', url: 'ws://drop/bus', token: 'td' },
    ]);
    await savePeersConfig([{ name: 'keep', url: 'ws://keep/bus' }]);
    expect(readPeersRaw()).toEqual([{ name: 'keep', url: 'ws://keep/bus', token: 'tk' }]);
  });
});

describe('savePeersConfig side effects', () => {
  it('writes 0o600 (bearer tokens, same posture as tokens.json)', async () => {
    await savePeersConfig([{ name: 'work', url: 'ws://x/bus', token: 's' }]);
    const mode = fs.statSync(peersConfigPath()).mode & 0o777;
    // Windows has no POSIX modes; elsewhere the file must be owner-only.
    if (process.platform !== 'win32') expect(mode).toBe(0o600);
  });

  it('restarts the hub after a successful save (stop, then start)', async () => {
    const order: string[] = [];
    h.stopHub.mockImplementation(async () => {
      order.push('stop');
    });
    h.startHub.mockImplementation(async () => {
      order.push('start');
    });
    const res = await savePeersConfig([{ name: 'work', url: 'ws://x/bus' }]);
    expect(res).toEqual({ ok: true });
    expect(order).toEqual(['stop', 'start']);
  });

  it('a failed hub restart reports ok:false but the file is already saved', async () => {
    h.startHub.mockRejectedValueOnce(new Error('no binary'));
    const res = await savePeersConfig([{ name: 'work', url: 'ws://x/bus' }]);
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.error).toMatch(/peers saved.*restart failed/);
    expect(readPeersRaw()).toEqual([{ name: 'work', url: 'ws://x/bus' }]);
  });

  it('remote-client mode saves the file but spawns no local hub', async () => {
    h.remoteServer = { httpUrl: 'http://100.64.1.9:7895' };
    const res = await savePeersConfig([{ name: 'work', url: 'ws://x/bus' }]);
    expect(res).toEqual({ ok: true });
    expect(readPeersRaw()).toEqual([{ name: 'work', url: 'ws://x/bus' }]);
    expect(h.stopHub).not.toHaveBeenCalled();
    expect(h.startHub).not.toHaveBeenCalled();
  });
});

describe('startFederationPeersConfig', () => {
  it('registers both IPC handlers on the contract channels', () => {
    startFederationPeersConfig();
    const channels = h.ipcHandle.mock.calls.map((c) => c[0]);
    expect(channels).toContain(IPC.FEDERATION_PEERS_CONFIG);
    expect(channels).toContain(IPC.FEDERATION_SAVE_PEERS_CONFIG);
  });
});
