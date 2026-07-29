/**
 * The point of these monitors is to speak up during a freeze. A silent
 * regression here (a swallowed error, an un-awaited handler, a wrapper that
 * drops the return value) would be invisible until the next time someone needed
 * the diagnostics and found nothing in the log — so pin the behaviour.
 *
 * The attribution rules get the most coverage because that is where this module
 * was wrong once already: it read a "currently executing" variable from a timer
 * that, by construction, can only run after the blocking has ended, so the one
 * case it existed to catch was the one case it always reported as "no IPC".
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const handlers = new Map<string, (...args: any[]) => any>();
const ipcMain = {
  handle: vi.fn((channel: string, listener: (...args: any[]) => any) => {
    handlers.set(channel, listener);
  }),
};
vi.mock('electron', () => ({ ipcMain }));

type Mod = typeof import('./stallDiagnostics');

/** Burn the event loop for `ms` — the whole point is that it is not awaitable. */
const blockFor = (ms: number): void => {
  const until = performance.now() + ms;
  while (performance.now() < until) {
    /* deliberately blocking */
  }
};

describe('instrumentIpcHandlers', () => {
  let warn: ReturnType<typeof vi.spyOn>;
  let mod: Mod;

  beforeEach(async () => {
    handlers.clear();
    ipcMain.handle = vi.fn((channel: string, listener: (...args: any[]) => any) => {
      handlers.set(channel, listener);
    });
    warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    mod = await import('./stallDiagnostics');
    mod.__resetStallDiagnostics();
    mod.instrumentIpcHandlers();
  });

  afterEach(() => {
    warn.mockRestore();
    vi.resetModules();
  });

  it('passes the handler result through untouched', async () => {
    ipcMain.handle('fast', () => 'payload');
    await expect(handlers.get('fast')!({}, 1)).resolves.toBe('payload');
    expect(warn).not.toHaveBeenCalled();
  });

  it('forwards the event and arguments to the wrapped handler', async () => {
    const inner = vi.fn(() => 'ok');
    ipcMain.handle('args', inner);
    await handlers.get('args')!({ sender: 1 }, 'a', 'b');
    expect(inner).toHaveBeenCalledWith({ sender: 1 }, 'a', 'b');
  });

  it('warns, naming the channel, when a handler blows the threshold', async () => {
    // Threshold is 100ms by default; block past it synchronously so the timing
    // is not at the mercy of the scheduler.
    ipcMain.handle('slow', () => {
      blockFor(120);
      return 'done';
    });
    await handlers.get('slow')!({});
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn.mock.calls[0][0]).toMatch(/^\[slow-ipc\] slow took \d+ms$/);
  });

  it('measures async handlers to completion, not to their first await', async () => {
    ipcMain.handle('async-slow', async () => {
      await new Promise((r) => setTimeout(r, 120));
      return 'late';
    });
    await expect(handlers.get('async-slow')!({})).resolves.toBe('late');
    expect(warn.mock.calls[0][0]).toMatch(/^\[slow-ipc\] async-slow took \d+ms$/);
  });

  it('still reports timing when the handler throws, and preserves the rejection', async () => {
    ipcMain.handle('boom', () => {
      throw new Error('nope');
    });
    await expect(handlers.get('boom')!({})).rejects.toThrow('nope');
    // Fast failure: timed, but under threshold, so nothing is logged.
    expect(warn).not.toHaveBeenCalled();
  });
});

describe('attributeStall', () => {
  let warn: ReturnType<typeof vi.spyOn>;
  let mod: Mod;
  /** A window that certainly contains everything the test just did. */
  const wholeTest = () => performance.now() - 1_000;

  beforeEach(async () => {
    handlers.clear();
    ipcMain.handle = vi.fn((channel: string, listener: (...args: any[]) => any) => {
      handlers.set(channel, listener);
    });
    warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    mod = await import('./stallDiagnostics');
    mod.__resetStallDiagnostics();
    mod.instrumentIpcHandlers();
  });

  afterEach(() => {
    warn.mockRestore();
    vi.resetModules();
  });

  it('names a synchronous blocker that has already returned', async () => {
    // The regression case: the handler is long gone by the time a timer runs,
    // so anything keying off "in flight" reports nothing.
    ipcMain.handle('claude:spawn', () => {
      blockFor(120);
      return 'spawned';
    });
    await handlers.get('claude:spawn')!({});
    expect(mod.attributeStall(110, wholeTest())).toMatch(
      /blocked synchronously in ipc:claude:spawn \(\d+ms\)/,
    );
  });

  it('sums several short blockers that together explain the stall', async () => {
    ipcMain.handle('a', () => blockFor(40));
    ipcMain.handle('b', () => blockFor(40));
    await handlers.get('a')!({});
    await handlers.get('b')!({});
    await handlers.get('a')!({});
    const report = mod.attributeStall(100, wholeTest());
    expect(report).toContain('blocked synchronously in');
    expect(report).toContain('ipc:a');
    expect(report).toContain('ipc:b');
    // Heaviest contributor first: two 'a' calls beat one 'b'.
    expect(report.indexOf('ipc:a')).toBeLessThan(report.indexOf('ipc:b'));
  });

  it('does not blame IPC when the recorded handlers cannot account for the lag', async () => {
    ipcMain.handle('tiny', () => blockFor(10));
    await handlers.get('tiny')!({});
    // 10ms of IPC cannot explain a 2s freeze — the culprit is elsewhere (a
    // watcher callback, a serializer, GC), and saying so is the useful answer.
    expect(mod.attributeStall(2_000, wholeTest())).toBe(' (no IPC in flight)');
  });

  it('ignores blocking that happened before the stall window', async () => {
    ipcMain.handle('earlier', () => blockFor(120));
    await handlers.get('earlier')!({});
    // Window starts in the future relative to that call.
    expect(mod.attributeStall(110, performance.now() + 1_000)).toBe(' (no IPC in flight)');
  });

  it('reports awaiting handlers as context, not as the cause', async () => {
    let release!: () => void;
    ipcMain.handle('net', () => new Promise<void>((r) => (release = r)));
    const pending = handlers.get('net')!({});
    const report = mod.attributeStall(300, wholeTest());
    expect(report).toContain('not IPC');
    expect(report).toContain('net');
    release();
    await pending;
  });

  it('does not leave a stale channel when overlapping handlers finish out of order', async () => {
    // The LIFO save/restore this replaced pinned the reported channel forever
    // as soon as two handlers overlapped and the outer one finished first.
    let releaseLong!: () => void;
    ipcMain.handle('short', () => 'ok');
    ipcMain.handle('long', () => new Promise<void>((r) => (releaseLong = r)));
    const short = handlers.get('short')!({});
    const long = handlers.get('long')!({});
    await short;
    expect(mod.attributeStall(300, wholeTest())).toContain('awaiting: long');
    releaseLong();
    await long;
    expect(mod.attributeStall(300, wholeTest())).toBe(' (no IPC in flight)');
  });

  it('does not record an async handler that only awaited as a blocker', async () => {
    ipcMain.handle('awaits', async () => {
      await new Promise((r) => setTimeout(r, 120));
    });
    await handlers.get('awaits')!({});
    // 120ms elapsed, ~0ms of it synchronous: it never held the loop.
    expect(mod.attributeStall(110, wholeTest())).toBe(' (no IPC in flight)');
  });
});

describe('startEventLoopLagMonitor', () => {
  let warn: ReturnType<typeof vi.spyOn>;
  let clock: ReturnType<typeof vi.spyOn>;
  let mod: Mod;

  /** Drive the monitor's monotonic clock explicitly; real jitter is untestable. */
  const scriptClock = (readings: number[]): void => {
    let i = 0;
    clock = vi
      .spyOn(performance, 'now')
      .mockImplementation(() => readings[Math.min(i++, readings.length - 1)]);
  };

  beforeEach(async () => {
    vi.useFakeTimers();
    warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    mod = await import('./stallDiagnostics');
    mod.__resetStallDiagnostics();
  });

  afterEach(() => {
    clock?.mockRestore();
    warn.mockRestore();
    vi.useRealTimers();
    vi.resetModules();
  });

  it('warns when the loop lags past the threshold', () => {
    // start=0, tick reads 1400 => 400ms of lag on a 1000ms timer.
    scriptClock([0, 1_400]);
    mod.startEventLoopLagMonitor();
    vi.advanceTimersByTime(1_000);
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn.mock.calls[0][0]).toMatch(/^\[stall\] main process blocked ~400ms/);
  });

  it('stays quiet for ordinary scheduling jitter', () => {
    scriptClock([0, 1_020]);
    mod.startEventLoopLagMonitor();
    vi.advanceTimersByTime(1_000);
    expect(warn).not.toHaveBeenCalled();
  });

  it('is immune to wall-clock jumps from suspend or an NTP step', () => {
    // performance.now() is monotonic and excludes suspend; Date.now() is not.
    // An eight-hour lid-close must not print an eight-hour stall.
    scriptClock([0, 1_000]);
    const dateNow = vi.spyOn(Date, 'now').mockReturnValue(8 * 60 * 60 * 1_000);
    mod.startEventLoopLagMonitor();
    vi.advanceTimersByTime(1_000);
    expect(warn).not.toHaveBeenCalled();
    dateNow.mockRestore();
  });

  it('does not hold the process open on quit', () => {
    scriptClock([0]);
    const spy = vi.spyOn(global, 'setInterval');
    mod.startEventLoopLagMonitor();
    const timer = spy.mock.results[0].value as NodeJS.Timeout;
    expect(timer.hasRef?.()).toBe(false);
    spy.mockRestore();
  });
});
