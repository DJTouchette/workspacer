// The `spawnCwds` block of contracts/path-containment-cases.json — the desktop
// half of the twin.
//
// agents.spawn and terminals.create are registered by BOTH providers, and
// capspec lists both under `unscopedByDecision`: the cwd is the point of the
// call, not something the path guard confines. That makes the string each
// provider hands the daemon the whole contract, and the two disagreed on five of
// eight probe spellings — the brain trimmed, tilde-expanded and stripped
// trailing slashes; this side existence-checked and silently fell back to
// $HOME. Each suite tested its own rule, so neither noticed.

import { describe, it, expect, vi } from 'vitest';
import { SweepTally, itSweptTheWholeCorpus } from '../../../tests/support/sweepTally';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { normalizeSpawnCwd, spawnCwdProblem, assertSpawnCwd } from './spawnCwd';
import { notifySystem } from '../services/systemNotice';

// The guard's whole point is that the failure is USER-visible; capture the notice.
vi.mock('../services/systemNotice', () => ({ notifySystem: vi.fn() }));

interface SpawnCwdCase {
  in: string;
  out: string;
  why: string;
}

const fixture = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '..', '..', '..', '..', '..', 'contracts', 'path-containment-cases.json'),
    'utf-8',
  ),
) as { spawnCwds: { cases: SpawnCwdCase[] } };

describe('spawn cwd normalization — cross-language contract', () => {
  it('has vectors at all', () => {
    expect(fixture.spawnCwds.cases.length).toBeGreaterThan(0);
    // The block's whole reason for existing: without a '~' vector this file is
    // satisfied by the tilde-expanding rule the brain shipped.
    expect(
      fixture.spawnCwds.cases.some((c) => c.in.startsWith('~')),
      "no '~' vector — BINDING DECISION 1 is unpinned on this seam again",
    ).toBe(true);
  });

  // 'has vectors at all' is a floor of one, and the '~' check next to it reads
  // the fixture rather than the run — so a block whose cases all failed to
  // register would satisfy both. The tally counts bodies.
  const tally = new SweepTally();
  for (const c of fixture.spawnCwds.cases) {
    it(`${JSON.stringify(c.in)} -> ${JSON.stringify(c.out)}`, () => {
      tally.ran('other');
      const want = c.out.split('${HOME}').join(os.homedir());
      expect(want, 'the only token this block defines is ${HOME}').not.toContain('${');
      expect(normalizeSpawnCwd(c.in), c.why).toBe(want);
    });
  }

  // undefined is the shape a bus caller produces by omitting the key, and it has
  // to land on the same answer as ''. The fixture cannot spell it in JSON.
  it('an omitted cwd is the empty cwd', () => {
    expect(normalizeSpawnCwd(undefined)).toBe(os.homedir());
    expect(normalizeSpawnCwd(null)).toBe(os.homedir());
  });

  itSweptTheWholeCorpus(tally, 'the spawnCwds block', 14, { allow: 0, deny: 0 });
});

/**
 * The guard that makes the rule above survivable. normalizeSpawnCwd passes '~'
 * through by design; claudemon then answers 200, registers the id and only
 * afterwards fails to launch the child — so without this pre-flight the user
 * gets an agent card whose session is already stopped and whose messages all
 * 409. Reproduced exactly that way before this guard existed.
 */
describe('spawnCwdProblem — a cwd no process could run in', () => {
  it('accepts a real directory', () => {
    expect(spawnCwdProblem(os.tmpdir())).toBeNull();
  });

  it('rejects a path that does not exist', () => {
    const problem = spawnCwdProblem(path.join(os.tmpdir(), 'wks-no-such-dir-4f3a9c'));
    expect(problem).toContain('not an existing directory');
  });

  it('rejects a FILE — existing is not the same as usable as a cwd', () => {
    const file = path.join(os.tmpdir(), `wks-spawncwd-${process.pid}.txt`);
    fs.writeFileSync(file, 'x');
    try {
      expect(spawnCwdProblem(file)).toContain('not an existing directory');
    } finally {
      fs.unlinkSync(file);
    }
  });

  it("explains the '~' case rather than just reporting it — that spelling is the trap", () => {
    // The literal that broke the Fleet Manager: agents.fleetRoot typed as '~/',
    // trailing slash stripped by normalizeSpawnCwd, handed to the daemon as a
    // directory named '~'.
    const problem = spawnCwdProblem(normalizeSpawnCwd('~/'));
    expect(problem).toContain('"~"');
    expect(problem).toContain('not expanded');
    // And a real path that merely CONTAINS a tilde gets no such lecture.
    expect(spawnCwdProblem(path.join(os.tmpdir(), 'a~b'))).not.toContain('not expanded');
  });

  it('assertSpawnCwd raises a user-visible notice AND throws — a console line was the bug', () => {
    expect(() => assertSpawnCwd('~')).toThrow(/not an existing directory/);
    expect(notifySystem).toHaveBeenCalledWith(
      expect.objectContaining({ level: 'error', title: 'Agent could not start' }),
    );
    expect(() => assertSpawnCwd(os.tmpdir())).not.toThrow();
  });
});
