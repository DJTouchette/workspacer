import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { RestartBackoff } from './daemonUtils';

describe('RestartBackoff', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(100);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('backs off exponentially, caps delay, then exhausts the restart budget', () => {
    const backoff = new RestartBackoff({ baseMs: 100, maxMs: 250, maxAttempts: 4 });
    backoff.markStarted();

    expect(backoff.nextDelay()).toBe(100);
    expect(backoff.nextDelay()).toBe(200);
    expect(backoff.nextDelay()).toBe(250);
    expect(backoff.nextDelay()).toBe(250);
    expect(backoff.nextDelay()).toBeNull();
  });

  it('resets the crash budget after a healthy uptime window', () => {
    const backoff = new RestartBackoff({
      baseMs: 100,
      maxAttempts: 2,
      resetAfterMs: 1000,
    });
    backoff.markStarted();

    expect(backoff.nextDelay()).toBe(100);
    expect(backoff.nextDelay()).toBe(200);
    expect(backoff.nextDelay()).toBeNull();

    vi.setSystemTime(1200);
    expect(backoff.nextDelay()).toBe(100);
  });

  it('manual reset starts the next crash sequence from the base delay', () => {
    const backoff = new RestartBackoff({ baseMs: 100, maxAttempts: 2 });
    backoff.markStarted();

    expect(backoff.nextDelay()).toBe(100);
    expect(backoff.nextDelay()).toBe(200);
    backoff.reset();

    expect(backoff.nextDelay()).toBe(100);
  });
});
