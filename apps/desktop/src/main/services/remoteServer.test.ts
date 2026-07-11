/**
 * "Connect to remote server" persistence + normalization guards: what the user
 * types must resolve to a dialable hub bus URL, survive a save/load roundtrip,
 * and never persist an address the next launch couldn't parse.
 */
import { describe, it, expect, vi, beforeEach, afterAll } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-remote-server-test-'));
vi.mock('./configService', () => ({ getConfigDir: () => tmpDir }));

import { normalizeRemoteServerUrl } from './remoteServer';

afterAll(() => {
  fs.rmSync(tmpDir, { recursive: true, force: true });
});

describe('normalizeRemoteServerUrl', () => {
  it('bare host gets http + the default hub port', () => {
    expect(normalizeRemoteServerUrl('100.64.1.2')).toEqual({
      httpUrl: 'http://100.64.1.2:7895',
      busUrl: 'ws://100.64.1.2:7895/bus',
    });
  });

  it('bare host:port keeps the explicit port', () => {
    expect(normalizeRemoteServerUrl('myserver:9000')).toEqual({
      httpUrl: 'http://myserver:9000',
      busUrl: 'ws://myserver:9000/bus',
    });
  });

  it('an explicit scheme without a port keeps the scheme default (tailscale serve)', () => {
    // https://node.ts.net fronts the hub at 443 — forcing :7895 would break it.
    expect(normalizeRemoteServerUrl('https://node.tail1234.ts.net')).toEqual({
      httpUrl: 'https://node.tail1234.ts.net',
      busUrl: 'wss://node.tail1234.ts.net/bus',
    });
  });

  it('accepts a pasted ws bus URL', () => {
    expect(normalizeRemoteServerUrl('ws://100.64.1.2:7895/bus')).toEqual({
      httpUrl: 'http://100.64.1.2:7895',
      busUrl: 'ws://100.64.1.2:7895/bus',
    });
  });

  it('secure schemes stay secure', () => {
    expect(normalizeRemoteServerUrl('wss://host:7895/bus')).toEqual({
      httpUrl: 'https://host:7895',
      busUrl: 'wss://host:7895/bus',
    });
  });

  it('rejects empty / unparseable / non-http(s)/ws(s) input', () => {
    expect(normalizeRemoteServerUrl('')).toBeNull();
    expect(normalizeRemoteServerUrl('   ')).toBeNull();
    expect(normalizeRemoteServerUrl('ftp://host')).toBeNull();
    expect(normalizeRemoteServerUrl('http://')).toBeNull();
  });
});

describe('setRemoteServer / getRemoteServer roundtrip', () => {
  beforeEach(async () => {
    const { setRemoteServer } = await import('./remoteServer');
    setRemoteServer(null); // clean slate (also resets the cache)
  });

  it('persists, normalizes, and reads back the target', async () => {
    const { setRemoteServer, getRemoteServer, isRemoteClientMode } = await import('./remoteServer');
    setRemoteServer({ url: '100.64.1.2', token: 'secret-token' });
    expect(getRemoteServer()).toEqual({
      httpUrl: 'http://100.64.1.2:7895',
      busUrl: 'ws://100.64.1.2:7895/bus',
      token: 'secret-token',
    });
    expect(isRemoteClientMode()).toBe(true);
  });

  it('clearing returns to local mode', async () => {
    const { setRemoteServer, getRemoteServer, isRemoteClientMode } = await import('./remoteServer');
    setRemoteServer({ url: 'host:1234', token: 't' });
    setRemoteServer(null);
    expect(getRemoteServer()).toBeNull();
    expect(isRemoteClientMode()).toBe(false);
  });

  it('refuses to persist an unparseable address (fails closed)', async () => {
    const { setRemoteServer, getRemoteServer } = await import('./remoteServer');
    expect(() => setRemoteServer({ url: 'http://', token: 't' })).toThrow(/unrecognized/);
    expect(getRemoteServer()).toBeNull();
  });
});
