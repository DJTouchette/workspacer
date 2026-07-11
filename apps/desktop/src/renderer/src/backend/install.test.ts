/**
 * Backend mode selection guards: which transport the desktop renderer boots on,
 * from main's getRemoteInfo(). Remote-client mode must win outright (no local
 * daemons exist to bridge to), the WORKSPACER_DESKTOP_DIRECT kill switch must
 * keep pure IPC, and a missing bus URL/token must fall back to IPC.
 */
import { describe, it, expect } from 'vitest';
import { selectBackendMode } from './install';

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
