import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const io = vi.hoisted(() => ({
  mkdirSync: vi.fn(),
  openSync: vi.fn(),
  writeSync: vi.fn(),
  closeSync: vi.fn(),
  statSync: vi.fn(),
  rmSync: vi.fn(),
}));
vi.mock('fs', () => io);
import { withFileLock } from './fileLock';

beforeEach(() => {
  Object.values(io).forEach((mock) => mock.mockReset());
});
afterEach(() => vi.restoreAllMocks());

describe('shared file lock failure paths', () => {
  it('times out when a stale lock cannot be removed instead of spinning forever', () => {
    let clock = 1000;
    let attempts = 0;
    vi.spyOn(Date, 'now').mockImplementation(() => clock);
    io.openSync.mockImplementation(() => {
      // Kill the historical infinite loop without hanging the test worker.
      if (++attempts > 100) throw new Error('fixture watchdog: deadline was never checked');
      throw Object.assign(new Error('held'), { code: 'EEXIST' });
    });
    io.statSync.mockReturnValue({ mtimeMs: 0 });
    io.rmSync.mockImplementation(() => {
      clock += 7;
      throw Object.assign(new Error('unremovable stale lock'), { code: 'EACCES' });
    });
    const body = vi.fn();
    expect(() =>
      withFileLock(
        '/fixture/config.yaml',
        {
          staleMs: 10,
          maxWaitMs: 20,
          retryMs: 1,
          onTimeout: () => new Error('bounded lock timeout'),
        },
        body,
      ),
    ).toThrow('bounded lock timeout');
    expect(attempts).toBe(3);
    expect(body).not.toHaveBeenCalled();
    expect(io.writeSync).not.toHaveBeenCalled();
    expect(io.closeSync).not.toHaveBeenCalled();
  });

  it('keeps exclusive ownership when only diagnostic text fails to write', () => {
    io.openSync.mockReturnValue(17);
    io.writeSync.mockImplementation(() => {
      throw new Error('diagnostics failed');
    });
    const body = vi.fn(() => 'protected result');
    expect(
      withFileLock(
        '/fixture/config.yaml',
        {
          staleMs: 10000,
          maxWaitMs: 20,
          onTimeout: () => new Error('unexpected timeout'),
        },
        body,
      ),
    ).toBe('protected result');
    expect(io.openSync).toHaveBeenCalledWith('/fixture/config.yaml.lock', 'wx');
    expect(io.closeSync).toHaveBeenCalledWith(17);
    expect(body).toHaveBeenCalledOnce();
    expect(io.rmSync).toHaveBeenCalledWith('/fixture/config.yaml.lock', { force: true });
  });
});
