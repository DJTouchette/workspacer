import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createWebBackend } from '../../src/backend/webBackend';

const busMock = vi.hoisted(() => {
  type Handler = (ev: {
    id: string;
    type: string;
    source: string;
    time: string;
    data?: unknown;
  }) => void;

  class FakeHubBusClient {
    calls: { method: string; params: unknown }[] = [];
    reconnectHandlers: (() => void)[] = [];
    subscriptions = new Map<string, Set<Handler>>();
    connected = true;

    constructor(
      readonly token: string,
      readonly busUrl?: string,
    ) {}

    start() {}

    isConnected() {
      return this.connected;
    }

    onStatus(handler: (connected: boolean) => void) {
      handler(this.connected);
      return () => {};
    }

    onReconnect(handler: () => void) {
      this.reconnectHandlers.push(handler);
      return () => {};
    }

    call(method: string, params: unknown = {}) {
      this.calls.push({ method, params });
      if (method === 'layout.get') {
        return Promise.resolve({
          version: 3,
          data: {
            agents: [{ id: 'agent-a', sessionId: 's1', tabs: [] }],
            activeAgentId: 'agent-a',
          },
        });
      }
      if (method === 'layout.set')
        return Promise.resolve({ version: 4, data: (params as any).data });
      if (method === 'sessions.snapshots') return Promise.resolve([]);
      if (method === 'sessions.snapshot') return Promise.resolve(null);
      return Promise.resolve({});
    }

    subscribe(topic: string, handler: Handler) {
      const handlers = this.subscriptions.get(topic) ?? new Set<Handler>();
      handlers.add(handler);
      this.subscriptions.set(topic, handlers);
      return () => handlers.delete(handler);
    }

    emit(type: string, data: unknown) {
      const ev = { id: 'e1', type, source: 'test', time: new Date(0).toISOString(), data };
      for (const [topic, handlers] of this.subscriptions) {
        if (
          topic === '*' ||
          topic === type ||
          (topic.endsWith('.*') && type.startsWith(topic.slice(0, -1)))
        ) {
          for (const handler of handlers) handler(ev);
        }
      }
    }

    reconnect() {
      for (const handler of this.reconnectHandlers) handler();
    }
  }

  return {
    FakeHubBusClient,
    instances: [] as FakeHubBusClient[],
  };
});

vi.mock('../../src/backend/hubBusClient', () => ({
  HubBusClient: class extends busMock.FakeHubBusClient {
    constructor(token: string, busUrl?: string) {
      super(token, busUrl);
      busMock.instances.push(this);
    }
  },
}));

function client() {
  const instance = busMock.instances.at(-1);
  if (!instance) throw new Error('expected fake hub client');
  return instance;
}

function call(method: string) {
  return client().calls.filter((entry) => entry.method === method);
}

describe('web backend bus integration', () => {
  beforeEach(() => {
    busMock.instances.length = 0;
  });

  it('uses the shared layout document surface for browser clients', async () => {
    const api = createWebBackend('token', 'ws://host.test/bus');

    await expect(api.layoutGet()).resolves.toMatchObject({
      version: 3,
      data: { activeAgentId: 'agent-a' },
    });

    await api.layoutSet({
      agents: [{ id: 'agent-b', sessionId: 's1', tabs: [] }],
      activeAgentId: 'agent-b',
    });
    expect(call('layout.set').at(-1)?.params).toEqual({
      data: { agents: [{ id: 'agent-b', sessionId: 's1', tabs: [] }], activeAgentId: 'agent-b' },
    });

    const changed = vi.fn();
    const unsubscribe = api.onLayoutChanged(changed);
    client().emit('layout.changed', {
      version: 5,
      data: { agents: [{ id: 'agent-b', sessionId: 's1', tabs: [] }], activeAgentId: 'agent-b' },
    });

    expect(changed).toHaveBeenCalledWith({
      version: 5,
      data: { agents: [{ id: 'agent-b', sessionId: 's1', tabs: [] }], activeAgentId: 'agent-b' },
    });
    unsubscribe();
  });

  it('routes attention resolution over the bus and applies the resulting snapshot', async () => {
    const api = createWebBackend('token', 'ws://host.test/bus');
    const snapshots = vi.fn();

    const unsubscribe = api.onClaudeSessionUpdate(snapshots);
    await api.claudeApprove('s1', 'yes', 'reviewed in web');
    await api.claudeAnswer('s1', { option: 2 });

    expect(call('claude.approve').at(-1)?.params).toEqual({
      sessionId: 's1',
      decision: 'yes',
      reason: 'reviewed in web',
    });
    expect(call('claude.answer').at(-1)?.params).toEqual({ sessionId: 's1', option: 2 });

    client().emit('agent.snapshot', {
      sessionId: 's1',
      id: 's1',
      status: 'running',
      mode: 'input',
      pending: null,
    });

    expect(snapshots).toHaveBeenCalledWith(
      's1',
      expect.objectContaining({ sessionId: 's1', mode: 'input', pending: null }),
    );
    unsubscribe();
  });

  it('attaches terminal streams once per viewer and replays them after reconnect', async () => {
    vi.useFakeTimers();
    const api = createWebBackend('token', 'ws://host.test/bus');
    const output = vi.fn();

    const unsubscribe = api.onClaudeOutput('s1', output);
    expect(call('sessions.attachTerminal')).toHaveLength(1);
    expect(call('sessions.attachTerminal')[0].params).toEqual({ sessionId: 's1' });

    client().emit('pty.bytes.s1', btoa('current screen'));
    expect(output).toHaveBeenCalledWith('current screen');

    client().reconnect();
    await vi.advanceTimersByTimeAsync(120);
    expect(call('sessions.attachTerminal')).toHaveLength(2);

    unsubscribe();
    expect(call('sessions.detachTerminal').at(-1)?.params).toEqual({ sessionId: 's1' });
    vi.useRealTimers();
  });
});
