/**
 * contracts/provider-parity-cases.json, TypeScript side.
 *
 * Path containment has its own corpus. This one pins the other half of "the same
 * bus call must give the same answer whichever provider ran": the ORDER a list
 * comes back in, and how each side reads a scalar that is not the type it
 * expected. Both were live divergences — see the fixture's header.
 *
 * TWIN: services/hub/cmd/brain/parity_ordering_test.go.
 */
import * as fs from 'fs';
import { SweepTally, itSweptTheWholeCorpus } from '../../../tests/support/sweepTally';
import * as os from 'os';
import * as path from 'path';
import { describe, expect, it } from 'vitest';

import { asString, byteCompare, trimSuffix, trimSuffixFold } from './providerParity';

interface ParityFixture {
  order: { name: string; input: string[]; expected: string[]; why?: string }[];
  scalar: { name: string; value: unknown; expected: string }[];
  suffix: {
    name: string;
    value: string;
    suffix: string;
    fold: boolean;
    expected: string;
    why?: string;
  }[];
}

const fixture: ParityFixture = JSON.parse(
  fs.readFileSync(
    path.resolve(__dirname, '../../../../../contracts/provider-parity-cases.json'),
    'utf-8',
  ),
);

describe('contracts/provider-parity-cases.json', () => {
  it('is not silently empty', () => {
    expect(fixture.order.length).toBeGreaterThan(0);
    expect(fixture.scalar.length).toBeGreaterThan(0);
    expect(fixture.suffix.length).toBeGreaterThan(0);
  });

  // One tally per block: three independent corpora share this file, and a total
  // would let one of them empty itself behind the other two.
  const order = new SweepTally();
  const scalar = new SweepTally();
  const suffix = new SweepTally();
  for (const c of fixture.order) {
    it(`order: ${c.name}`, () => {
      order.ran('other');
      expect([...c.input].sort(byteCompare)).toEqual(c.expected);
    });
  }

  it('order: localeCompare really does disagree, so these cases are not vacuous', () => {
    const disagreements = fixture.order.filter(
      (c) =>
        JSON.stringify([...c.input].sort((a, b) => a.localeCompare(b))) !==
        JSON.stringify(c.expected),
    );
    expect(disagreements.length).toBeGreaterThan(0);
  });

  for (const c of fixture.scalar) {
    it(`scalar: ${c.name}`, () => {
      scalar.ran('other');
      expect(asString(c.value)).toBe(c.expected);
    });
  }

  for (const c of fixture.suffix) {
    it(`suffix: ${c.name}`, () => {
      suffix.ran('other');
      const got = c.fold ? trimSuffixFold(c.value, c.suffix) : trimSuffix(c.value, c.suffix);
      expect(got).toBe(c.expected);
    });
  }

  it('suffix: String#replace really does disagree, so these cases are not vacuous', () => {
    const disagreements = fixture.suffix.filter(
      (c) => !c.fold && c.value.replace(c.suffix, '') !== c.expected,
    );
    expect(disagreements.length).toBeGreaterThan(0);
  });

  itSweptTheWholeCorpus(order, 'the provider-parity order block', 4, { allow: 0, deny: 0 });
  itSweptTheWholeCorpus(scalar, 'the provider-parity scalar block', 7, { allow: 0, deny: 0 });
  itSweptTheWholeCorpus(suffix, 'the provider-parity suffix block', 10, { allow: 0, deny: 0 });
});

// The helpers pinned in isolation are not enough — the divergence shipped at the
// CALL SITES. These drive the real listers with the fixture's own input.
describe('the listers use the fixture ordering', () => {
  it('fs.listEntries sorts directories first, then byte-wise', async () => {
    const { listDir } = await import('../services/fileService');
    const c = fixture.order[0];
    const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-parity-')));
    for (const name of c.input) {
      fs.mkdirSync(path.join(root, name));
      fs.writeFileSync(path.join(root, `${name}.txt`), 'x');
    }
    const { entries } = listDir(root);
    expect(entries.filter((e) => e.isDir).map((e) => e.name)).toEqual(c.expected);
    expect(entries.filter((e) => !e.isDir).map((e) => e.name)).toEqual(
      c.expected.map((n) => `${n}.txt`),
    );
    fs.rmSync(root, { recursive: true, force: true });
  });
});
