import { afterEach, expect, it, vi } from 'vitest';

const state = vi.hoisted(() => ({ paired: true }));
vi.mock('./remoteServer', () => ({
  getPairedWorkerTarget: () =>
    state.paired
      ? {
          httpUrl: 'https://worker.example',
          busUrl: 'wss://worker.example/bus',
          token: 'synthetic-pairing-secret',
        }
      : null,
}));
vi.mock('./hubClient', () => ({ callHub: vi.fn() }));
vi.mock('./pairedWorkerConnection', () => ({ pairedWorkerConnection: { call: vi.fn() } }));
vi.mock('./federationBridge', () => ({ listFederationPeers: () => [] }));
vi.mock('./federationPeersConfig', () => ({ readRedactedPeers: () => [] }));

import { DISPATCH_PROTOCOL, listDispatchTargets } from './dispatchTargets';
afterEach(() => {
  state.paired = true;
});

it('describes the actual paired host route without exposing its credential', async () => {
  const result = await listDispatchTargets(async () => ({
    protocol: DISPATCH_PROTOCOL,
    executes: true,
    providers: [{ provider: 'claude', found: true, authenticated: true, note: 'remote login' }],
    cwds: [{ path: '/remote/repo', source: 'project', git: true }],
  }));
  expect(result.targets[0]).toMatchObject({ name: 'paired', ready: true, host: 'worker.example' });
  expect(result.note).toContain('executionTarget="paired"');
  expect(result.note).toContain('LOCAL task project');
  expect(JSON.stringify(result)).not.toContain('synthetic-pairing-secret');
});

it('does not advertise a legacy federation link as a usable paired route', async () => {
  state.paired = false;
  const probe = vi.fn();
  const result = await listDispatchTargets(probe, () => [
    {
      name: 'legacy',
      url: 'wss://legacy.example/bus',
      hasToken: true,
      dispatch: true,
    },
  ]);
  expect(result.targets[0].ready).toBe(false);
  expect(result.targets[0].readiness).toContain('not a paired dispatch route');
  expect(probe).not.toHaveBeenCalled();
});

it('keeps an older peer explicitly unsupported', async () => {
  const result = await listDispatchTargets(async () => ({ protocol: 1, executes: true }));
  expect(result.targets[0].ready).toBe(false);
  expect(result.targets[0].readiness).toContain('protocol mismatch');
});
