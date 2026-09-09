import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  ProviderReadinessService,
  completionReadiness,
  type ReadinessContext,
} from './providerReadiness';
import { normalizeProviderReadiness } from '../shared/providerReadiness';

const ok = {
  ok: true as const,
  text: 'SECRET',
  provider: 'claude' as const,
  model: 'haiku',
  elapsedMs: 1,
};
const fail = (reason: string, message = 'SECRET') => ({
  ok: false as const,
  reason,
  message,
  provider: 'claude' as const,
  model: null,
  elapsedMs: 1,
});
afterEach(() => vi.useRealTimers());
function setup() {
  let ctx: ReadinessContext = {
    key: 'local:claude:bin1',
    local: true,
    enabled: true,
    provider: 'claude',
    bin: '/bin/fixture',
  };
  const ping = vi.fn().mockResolvedValue(ok);
  const service = new ProviderReadinessService({ context: () => ctx, ping });
  return {
    service,
    ping,
    change: (patch: Partial<ReadinessContext>) => {
      ctx = { ...ctx, ...patch };
      service.invalidate();
    },
  };
}
describe('provider startup scheduler', () => {
  it('checks only the selected manager once, reads/remounts never ping', async () => {
    vi.useFakeTimers();
    const { service, ping } = setup();
    service.start();
    service.start();
    service.read('claude');
    expect(ping).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2000);
    expect(ping).toHaveBeenCalledTimes(1);
    expect(ping.mock.calls[0].slice(0, 2)).toEqual(['claude', '/bin/fixture']);
    expect(service.read('claude')).toEqual({ state: 'responding', checkedAt: expect.any(Number) });
    await vi.advanceTimersByTimeAsync(60000);
    expect(ping).toHaveBeenCalledTimes(1);
    service.dispose();
  });
  it('persisted opt-out and disabling during startup delay prevent all automatic work; manual is explicit', async () => {
    vi.useFakeTimers();
    const { service, ping, change } = setup();
    service.start();
    change({ enabled: false });
    await vi.advanceTimersByTimeAsync(60000);
    expect(ping).not.toHaveBeenCalled();
    await service.check('claude', true);
    expect(ping).not.toHaveBeenCalled();
    await service.check('claude');
    expect(ping).toHaveBeenCalledTimes(1);
    service.dispose();
  });
  it('never checks a remote owner or missing executable', async () => {
    const { service, ping, change } = setup();
    change({ local: false });
    expect(await service.check('claude')).toEqual({ state: 'unsupported' });
    change({ local: true, bin: null });
    expect(await service.check('claude')).toEqual({ state: 'unchecked' });
    expect(ping).not.toHaveBeenCalled();
  });
  it('startup reuses an earlier manual result without another paid request', async () => {
    vi.useFakeTimers();
    const { service, ping } = setup();
    service.start();
    await service.check('claude');
    await vi.advanceTimersByTimeAsync(4000);
    expect(ping).toHaveBeenCalledTimes(1);
    service.dispose();
  });
  it('deduplicates concurrent and immediate refreshes, permits later explicit refresh', async () => {
    vi.useFakeTimers();
    const { service, ping } = setup();
    await Promise.all([service.check('claude'), service.check('claude')]);
    await service.check('claude');
    expect(ping).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(2001);
    await service.check('claude');
    expect(ping).toHaveBeenCalledTimes(2);
  });
  it.each([
    { provider: 'codex', key: 'codex' },
    { bin: '/new/bin', key: 'new-bin' },
    { local: false, key: 'remote' },
    { enabled: false, key: 'disabled' },
  ])('cancels stale work after %j and never publishes its result', async (patch) => {
    const { service, ping, change } = setup();
    let finish!: (value: unknown) => void;
    ping.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const pending = service.check('claude');
    const signal = ping.mock.calls[0][2] as AbortSignal;
    change(patch);
    expect(signal.aborted).toBe(true);
    finish(ok);
    expect(await pending).toEqual({ state: 'unchecked' });
    expect(service.read('claude').state).not.toBe('responding');
  });
  it('bounds even an adapter that ignores cancellation without retries', async () => {
    vi.useFakeTimers();
    const { service, ping } = setup();
    ping.mockImplementation(() => new Promise(() => {}));
    const pending = service.check('claude');
    await vi.advanceTimersByTimeAsync(20000);
    expect((await pending).state).toBe('timeout');
    expect(ping.mock.calls[0][2].aborted).toBe(true);
    expect(ping).toHaveBeenCalledTimes(1);
  });
});
describe('safe advisory projection', () => {
  it.each([
    ['not-authed', 'unauthenticated'],
    ['rate-limited', 'limited'],
    ['timeout', 'timeout'],
    ['no-tools-unsupported', 'unsupported'],
    ['unsupported-model', 'unsupported'],
    ['daemon-unavailable', 'unsupported'],
    ['failed', 'error'],
    ['empty', 'error'],
    ['cancelled', 'unchecked'],
  ])('%s is %s, never raw output', (reason, state) => {
    expect(completionReadiness(fail(reason) as never, 123)).toEqual({ state, checkedAt: 123 });
  });
  it('distinguishes connectivity and rejects future/invalid contracts and extra fields', () => {
    expect(completionReadiness(fail('failed', 'ECONNRESET SECRET') as never, 1).state).toBe(
      'network-error',
    );
    expect(normalizeProviderReadiness({ state: 'future', token: 'SECRET' })).toEqual({
      state: 'unchecked',
    });
    expect(
      normalizeProviderReadiness({ state: 'responding', checkedAt: Infinity, stdout: 'SECRET' }),
    ).toEqual({ state: 'responding' });
    expect(completionReadiness(ok, 1)).toEqual({ state: 'responding', checkedAt: 1 });
  });
});
