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

import { describe, it, expect } from 'vitest';
import { SweepTally, itSweptTheWholeCorpus } from '../../../tests/support/sweepTally';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { normalizeSpawnCwd } from './spawnCwd';

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
