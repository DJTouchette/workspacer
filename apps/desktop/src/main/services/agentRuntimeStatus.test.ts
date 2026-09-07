import { beforeEach, describe, expect, it, vi } from 'vitest';
const probe = vi.hoisted(() => vi.fn(async (_url: string) => true));
vi.mock('../lib/daemonUtils', () => ({
  PORTS: { claudemonApi: 1, hub: 2, mcpFacade: 3 },
  probeHealth: probe,
}));
import {
  noteRuntimePhase,
  noteRuntimePending,
  observeRuntimeStart,
  readAgentRuntimeStatus,
} from './agentRuntimeStatus';
import { runtimeLaunchState } from '../shared/agentRuntimeStatus';
beforeEach(() => {
  for (const key of ['claudemon', 'hub', 'facade'] as const) noteRuntimePhase(key, 'unknown');
  probe.mockReset().mockResolvedValue(true);
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => ({ ok: true, json: async () => ({ hubConnected: true }) })),
  );
});
describe('read-only runtime lifecycle', () => {
  it('keeps unknown hosts unknown without probing or promising readiness', async () => {
    const status = await readAgentRuntimeStatus();
    expect(status.claudemon).toBe('unknown');
    expect(runtimeLaunchState(status, true)).toMatchObject({ blocked: false });
    expect(probe).not.toHaveBeenCalled();
  });
  it('follows slow startup through ready, including adoption health promises', async () => {
    let done!: () => void;
    const start = observeRuntimeStart(
      'claudemon',
      new Promise<void>((resolve) => {
        done = resolve;
      }),
    );
    expect(runtimeLaunchState(await readAgentRuntimeStatus(), false).blocked).toBe(true);
    expect(probe).not.toHaveBeenCalled();
    done();
    await start;
    expect((await readAgentRuntimeStatus()).claudemon).toBe('ready');
  });
  it('reports failed claudemon independently of hub-only degradation', async () => {
    await expect(
      observeRuntimeStart('claudemon', Promise.reject(new Error('failed'))),
    ).rejects.toThrow();
    probe.mockResolvedValue(false);
    expect(runtimeLaunchState(await readAgentRuntimeStatus(), false).blocked).toBe(true);
    noteRuntimePhase('claudemon', 'ready');
    noteRuntimePhase('hub', 'failed');
    noteRuntimePhase('facade', 'failed');
    probe.mockImplementation(async (url) => url.includes(':1/'));
    const status = await readAgentRuntimeStatus();
    expect(runtimeLaunchState(status, false).blocked).toBe(false);
    expect(runtimeLaunchState(status, true).blocked).toBe(true);
  });
  it('rechecks adopted processes and recovers without starting any process', async () => {
    await observeRuntimeStart('claudemon', Promise.resolve());
    probe.mockResolvedValue(false);
    expect((await readAgentRuntimeStatus()).claudemon).toBe('failed');
    probe.mockResolvedValue(true);
    expect((await readAgentRuntimeStatus()).claudemon).toBe('ready');
  });
  it('does not let an in-flight health reply overwrite a later exit/start event', async () => {
    noteRuntimePhase('claudemon', 'ready');
    let done!: (value: boolean) => void;
    probe.mockImplementation(
      () =>
        new Promise((resolve) => {
          done = resolve;
        }),
    );
    const reading = readAgentRuntimeStatus();
    noteRuntimePhase('claudemon', 'failed');
    done(true);
    expect((await reading).claudemon).toBe('failed');
  });
});

it('requires the facade bus connection, not merely a responding HTTP listener', async () => {
  noteRuntimePhase('claudemon', 'ready');
  noteRuntimePhase('hub', 'ready');
  noteRuntimePhase('facade', 'ready');
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => ({ ok: true, json: async () => ({ hubConnected: false }) })),
  );
  expect(runtimeLaunchState(await readAgentRuntimeStatus(), true).blocked).toBe(true);
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => ({ ok: true, json: async () => ({}) })),
  );
  expect((await readAgentRuntimeStatus()).facade).toBe('unknown');
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => ({ ok: true, json: async () => ({ hubConnected: true }) })),
  );
  expect((await readAgentRuntimeStatus()).facade).toBe('ready');
});

it('rechecks the health address chosen by the lifecycle owner', async () => {
  await observeRuntimeStart('hub', Promise.resolve(), 'http://192.0.2.10:9999/health');
  await readAgentRuntimeStatus();
  expect(probe).toHaveBeenCalledWith('http://192.0.2.10:9999/health');
});

it('does not reset healthy dependencies when a desktop window is reopened', async () => {
  noteRuntimePending('hub');
  expect((await readAgentRuntimeStatus()).hub).toBe('starting');
  await observeRuntimeStart('hub', Promise.resolve());
  noteRuntimePending('hub');
  expect((await readAgentRuntimeStatus()).hub).toBe('ready');
});
