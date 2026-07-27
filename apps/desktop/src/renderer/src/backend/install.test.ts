/**
 * Backend mode selection guards: which transport the desktop renderer boots on,
 * from main's getRemoteInfo(). Remote-client mode must win outright (no local
 * daemons exist to bridge to), the WORKSPACER_DESKTOP_DIRECT kill switch must
 * keep pure IPC, and a missing bus URL/token must fall back to IPC.
 */
import { describe, it, expect, vi } from 'vitest';
import { selectBackendMode, getRemoteInfoWithRetry } from './install';

const local = {
  desktopBus: true,
  busUrl: 'ws://127.0.0.1:7895/bus',
  token: 'local-token',
  remoteClient: null,
};

describe('selectBackendMode', () => {
  it('picks remote when a remote server is configured — even over a live local bus', () => {
    expect(
      selectBackendMode({
        ...local,
        remoteClient: { busUrl: 'ws://100.64.1.2:7895/bus', token: 't' },
      }),
    ).toBe('remote');
    // The kill switch governs the LOCAL bus mirror, not client mode: with a
    // remote server configured there is nothing local to fall back to.
    expect(
      selectBackendMode({
        desktopBus: false,
        remoteClient: { busUrl: 'ws://100.64.1.2:7895/bus', token: 't' },
      }),
    ).toBe('remote');
  });

  it('defaults to bridged (desktop bus mode) with a reachable local bus', () => {
    expect(selectBackendMode(local)).toBe('bridged');
  });

  it('honors the WORKSPACER_DESKTOP_DIRECT kill switch (desktopBus:false → ipc)', () => {
    expect(selectBackendMode({ ...local, desktopBus: false })).toBe('ipc');
  });

  it('falls back to ipc when the bus URL or token is missing/unknown', () => {
    expect(selectBackendMode(null)).toBe('ipc');
    expect(selectBackendMode(undefined)).toBe('ipc');
    expect(selectBackendMode({ ...local, busUrl: '' })).toBe('ipc');
    expect(selectBackendMode({ ...local, token: '' })).toBe('ipc');
  });
});

/**
 * A transient getRemoteInfo() rejection used to fall straight through to "stay
 * on IPC" — silently fatal in remote-client mode, where main spawned no local
 * daemons for IPC to talk to. Zero backoff here keeps the tests instant; the
 * real delays live in the module default.
 */
describe('getRemoteInfoWithRetry', () => {
  const remote = { remoteClient: { busUrl: 'ws://100.64.1.2:7895/bus', token: 't' } };

  it('returns the first successful answer without retrying', async () => {
    const get = vi.fn().mockResolvedValue(remote);
    await expect(getRemoteInfoWithRetry(get, [0, 0])).resolves.toEqual(remote);
    expect(get).toHaveBeenCalledTimes(1);
  });

  it('recovers a remote-client answer from a handler that is not registered yet', async () => {
    const get = vi
      .fn()
      .mockRejectedValueOnce(new Error("No handler registered for 'hub:get-remote-info'"))
      .mockRejectedValueOnce(new Error('still booting'))
      .mockResolvedValue(remote);
    await expect(getRemoteInfoWithRetry(get, [0, 0, 0])).resolves.toEqual(remote);
    expect(get).toHaveBeenCalledTimes(3);
    // The whole point: the recovered answer still selects the remote transport.
    expect(selectBackendMode(await getRemoteInfoWithRetry(get, [0]))).toBe('remote');
  });

  it('makes attempts = backoff.length + 1, then rejects with the last error', async () => {
    const get = vi
      .fn()
      .mockRejectedValueOnce(new Error('first'))
      .mockRejectedValue(new Error('last'));
    await expect(getRemoteInfoWithRetry(get, [0, 0])).rejects.toThrow('last');
    expect(get).toHaveBeenCalledTimes(3);
  });
});
