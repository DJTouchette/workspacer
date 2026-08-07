// Cross-language host-trusted-config drift guard.
//
// contracts/host-trusted-config-cases.json is the SHARED fixture: a Go test
// (cmd/brain/config.go's dropHostTrusted) consumes the exact same file. Both bus
// entry points must strip the same sections — the brain answers config.save
// under the default DELEGATE_CATALOG_TO_BRAIN, this copy when delegation is off.
// If either side's list drifts, one of the two contract tests fails.
//
// The finding this exists for: only the Go twin had the drop, so with delegation
// off a bus caller could set updates.channel — which is concatenated into the
// electron-updater feed URL the app downloads and installs from.

import { describe, it, expect } from 'vitest';
import { SweepTally, itSweptTheWholeCorpus } from '../../../tests/support/sweepTally';
import { readFileSync } from 'fs';
import * as path from 'path';
import { dropHostTrusted, HOST_TRUSTED_PATHS, HOST_TRUSTED_SECTIONS } from './hostTrustedConfig';

interface Fixture {
  sections: string[];
  paths: string[];
  cases: { name: string; partial: Record<string, unknown>; expected: Record<string, unknown> }[];
}

const fixture: Fixture = JSON.parse(
  readFileSync(
    path.join(__dirname, '../../../../../contracts/host-trusted-config-cases.json'),
    'utf-8',
  ),
);

describe('dropHostTrusted — cross-language contract', () => {
  it('guards exactly the sections the fixture names', () => {
    expect([...HOST_TRUSTED_SECTIONS].sort()).toEqual([...fixture.sections].sort());
  });

  // The sub-key list too, or agents.binaries (argv[0] for every spawned agent)
  // can be dropped from the guard with every case still green — the cases only
  // see the drop, not the list that drives it.
  it('guards exactly the sub-keys the fixture names', () => {
    expect([...HOST_TRUSTED_PATHS].sort()).toEqual([...fixture.paths].sort());
  });

  const tally = new SweepTally();
  for (const c of fixture.cases) {
    it(c.name, () => {
      tally.ran('other');
      expect(dropHostTrusted(c.partial)).toEqual(c.expected);
    });
  }

  it('does not mutate the caller-supplied object', () => {
    const partial = { updates: { channel: 'nightly' }, ui: { theme: 'dark' } };
    dropHostTrusted(partial);
    expect(partial.updates, 'the caller still owns what it passed in').toEqual({
      channel: 'nightly',
    });
  });

  it('returns the same object when there is nothing to drop', () => {
    const partial = { ui: { theme: 'dark' } };
    expect(dropHostTrusted(partial)).toBe(partial);
  });

  itSweptTheWholeCorpus(tally, 'the host-trusted-config corpus', 13, { allow: 0, deny: 0 });
});
