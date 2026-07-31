// The cross-process config lock. Uses a real temp directory rather than a mocked
// fs — the whole point is the O_EXCL create, which a mock would not model.
//
// The Go twin (cmd/brain/configlock_test.go) covers the same cases, and both
// assert their parameters against contracts/config-lock.json. staleMs in
// particular MUST agree: a side that expires locks sooner will steal one the
// other still believes it holds, which is worse than having no lock at all.
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { withConfigLock, ConfigLockTimeout, LOCK_STALE_MS } from './configLock';

let dir: string;
let cfg: string;
const lockOf = (p: string): string => p + '.lock';

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lock-'));
  cfg = path.join(dir, 'config.yaml');
  fs.writeFileSync(cfg, 'ui: {}\n');
});
afterEach(() => {
  fs.rmSync(dir, { recursive: true, force: true });
});

describe('withConfigLock', () => {
  it('runs the body and releases the lock', () => {
    let sawLock = false;
    const out = withConfigLock(cfg, () => {
      sawLock = fs.existsSync(lockOf(cfg));
      return 'result';
    });
    expect(out).toBe('result');
    expect(sawLock, 'the lock must be held FOR the body').toBe(true);
    expect(fs.existsSync(lockOf(cfg)), 'and released after').toBe(false);
  });

  it('releases the lock when the body throws', () => {
    expect(() =>
      withConfigLock(cfg, () => {
        throw new Error('write failed');
      }),
    ).toThrow('write failed');
    expect(
      fs.existsSync(lockOf(cfg)),
      'a throwing write must not wedge config until the stale timeout',
    ).toBe(false);
  });

  it('refuses rather than writing when another process holds it', () => {
    // Stand in for the brain mid-write: a fresh lock file it has not released.
    fs.writeFileSync(lockOf(cfg), '9999 held\n');

    let ran = false;
    expect(() =>
      withConfigLock(cfg, () => {
        ran = true;
      }),
    ).toThrow(ConfigLockTimeout);
    expect(ran, 'the body must NOT run — writing anyway is the bug this prevents').toBe(false);
    expect(fs.existsSync(lockOf(cfg)), "the other side's lock is left alone").toBe(true);
  });

  it('steals a lock whose holder died mid-write', () => {
    fs.writeFileSync(lockOf(cfg), '9999 crashed\n');
    // Backdate it past the stale threshold.
    const old = Date.now() - (LOCK_STALE_MS + 1000);
    fs.utimesSync(lockOf(cfg), new Date(old), new Date(old));

    let ran = false;
    withConfigLock(cfg, () => {
      ran = true;
    });
    expect(ran, 'a dead holder must not wedge config forever').toBe(true);
    expect(fs.existsSync(lockOf(cfg))).toBe(false);
  });

  it('is re-entrant across sequential calls', () => {
    withConfigLock(cfg, () => undefined);
    withConfigLock(cfg, () => undefined);
    expect(fs.existsSync(lockOf(cfg))).toBe(false);
  });

  it('agrees with the Go twin on the stale threshold', () => {
    const fixture = JSON.parse(
      fs.readFileSync(path.join(__dirname, '../../../../../contracts/config-lock.json'), 'utf-8'),
    );
    expect(LOCK_STALE_MS).toBe(fixture.staleMs);
    expect(lockOf(cfg)).toBe(cfg + fixture.lockFileSuffix);
  });
});
