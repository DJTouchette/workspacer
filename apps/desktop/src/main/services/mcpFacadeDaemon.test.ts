import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
const fetchMock = vi.fn();
const spawn = vi.fn();
const kill = vi.fn();
vi.mock('child_process', () => ({ spawn, execFile: spawn }));
vi.mock('../lib/daemonUtils', () => ({ PORTS: { mcpFacade: 7897 }, killStaleListener: kill }));
vi.mock('./hubDaemon', () => ({
  hubBusUrl: () => 'ws://127.0.0.1:7895/bus',
  getHubToken: () => 'fixture-owner',
}));
vi.mock('./agentRuntimeStatus', () => ({
  noteRuntimePhase: vi.fn(),
  observeRuntimeStart: (_: string, promise: Promise<void>) => promise,
}));
const healthy = {
  status: 'ok',
  service: 'workspacer-mcp-facade',
  hubConnected: true,
  pluginCatalogReady: true,
  listenAddr: '127.0.0.1:7897',
  hubUrl: 'ws://127.0.0.1:7895/bus',
};
const response = (body = healthy) => ({ ok: true, json: async () => body });
let module: typeof import('./mcpFacadeDaemon');
beforeEach(async () => {
  vi.resetModules();
  fetchMock.mockReset();
  spawn.mockClear();
  kill.mockClear();
  fetchMock.mockResolvedValue(response());
  vi.stubGlobal('fetch', fetchMock);
  module = await import('./mcpFacadeDaemon');
});
afterEach(async () => {
  await module.stopMcpFacade();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
describe('hub-owned MCP readiness', () => {
  it('adopts a matching ready facade without starting or stopping a process', async () => {
    await module.startMcpFacade();
    await module.startMcpFacade();
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(spawn).not.toHaveBeenCalled();
    expect(kill).not.toHaveBeenCalled();
    expect(fetchMock.mock.calls[0][1].headers).toBeUndefined();
    expect(module.getMcpFacadeToken()).toBe('fixture-owner');
  });
  it.each([
    { service: 'another-service' },
    { listenAddr: '127.0.0.1:9999' },
    { hubUrl: 'ws://another-host:7895/bus' },
    { hubConnected: false },
    { pluginCatalogReady: false },
  ])('rejects mismatched or unready identity %j', async (different) => {
    fetchMock
      .mockResolvedValueOnce(response({ ...healthy, ...different }))
      .mockResolvedValue(response());
    await module.startMcpFacade();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(spawn).not.toHaveBeenCalled();
    expect(kill).not.toHaveBeenCalled();
  });
  it('coalesces simultaneous checks and probes again before later spawns', async () => {
    await Promise.all([module.ensureMcpFacadeReady(), module.ensureMcpFacadeReady()]);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    await module.ensureMcpFacadeReady();
    expect(fetchMock).toHaveBeenCalledTimes(3);
  });
  it('waits for an unhealthy cached listener without creating a second supervisor', async () => {
    await module.startMcpFacade();
    fetchMock
      .mockResolvedValueOnce(response({ ...healthy, hubConnected: false }))
      .mockResolvedValue(response());
    await module.ensureMcpFacadeReady();
    expect(fetchMock).toHaveBeenCalledTimes(4);
    expect(spawn).not.toHaveBeenCalled();
    expect(kill).not.toHaveBeenCalled();
  });
  it('a stopped generation cannot accept a late response or clear its successor', async () => {
    let release!: (value: ReturnType<typeof response>) => void;
    fetchMock.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    const previous = module.startMcpFacade();
    const refused = expect(previous).rejects.toThrow('canceled');
    await module.stopMcpFacade();
    await module.startMcpFacade();
    release(response());
    await refused;
    const calls = fetchMock.mock.calls.length;
    await module.startMcpFacade();
    expect(fetchMock).toHaveBeenCalledTimes(calls);
  });
  it('reports unavailable readiness after a bounded wait without killing the owner', async () => {
    vi.useFakeTimers();
    fetchMock.mockResolvedValue({ ok: false });
    const pending = module.startMcpFacade();
    const failed = expect(pending).rejects.toThrow('not ready');
    await vi.advanceTimersByTimeAsync(5100);
    await failed;
    expect(spawn).not.toHaveBeenCalled();
    expect(kill).not.toHaveBeenCalled();
  });
});
