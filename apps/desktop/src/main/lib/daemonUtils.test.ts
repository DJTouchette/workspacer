import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { RestartBackoff, parseWindowsListenerPids } from './daemonUtils';

describe('parseWindowsListenerPids', () => {
  // Realistic `netstat -ano -p tcp` shape, including every row family that
  // fooled the old substring+first-match parse (Windows nightly bind loop):
  const NETSTAT = [
    '',
    'Active Connections',
    '',
    '  Proto  Local Address          Foreign Address        State           PID',
    '  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1052',
    '  TCP    127.0.0.1:7890         0.0.0.0:0              LISTENING       4242',
    '  TCP    127.0.0.1:78901        0.0.0.0:0              LISTENING       9999', // :7890 prefix trap
    '  TCP    127.0.0.1:52301        127.0.0.1:7890         ESTABLISHED     8888', // hook curl (foreign :7890)
    '  TCP    127.0.0.1:52302        127.0.0.1:7890         TIME_WAIT       0', // dead hook curl
    '  TCP    127.0.0.1:7891         0.0.0.0:0              LISTENING       4242',
  ].join('\r\n');

  it('matches only LISTENING rows whose local address ends in the exact port', () => {
    expect(parseWindowsListenerPids(NETSTAT, 7890)).toEqual([4242]);
    expect(parseWindowsListenerPids(NETSTAT, 7891)).toEqual([4242]);
  });

  it('the prefix-port and foreign-address traps yield nothing for their ports', () => {
    expect(parseWindowsListenerPids(NETSTAT, 78901)).toEqual([9999]);
    expect(parseWindowsListenerPids(NETSTAT, 52301)).toEqual([]); // ESTABLISHED, not a listener
  });

  it('empty / headers-only output yields nothing', () => {
    expect(parseWindowsListenerPids('', 7890)).toEqual([]);
    expect(parseWindowsListenerPids('Active Connections\r\n  Proto ...', 7890)).toEqual([]);
  });
});

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
