// Cross-language deepMerge drift guard.
//
// contracts/deepmerge-cases.json is the SHARED fixture: a Go test (config.go's
// deepMerge) consumes the exact same file. Each case pins deepMerge(target,
// source) => expected for the config.yaml overlay semantics (null = unset,
// objects recurse, arrays/scalars replace wholesale). If the TS and Go merges
// diverge, one side's contract test fails.

import { describe, it, expect, vi } from 'vitest';
import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import * as path from 'path';

// Importing configService instantiates its module-level singleton, whose
// constructor reads/writes the real config dir. Neutralize the WRITE side of the
// real fs so the import has no side effects; keep reads real so the fixture
// readFileSync below still works (and the constructor safely runs on whatever is
// on disk / defaults).
vi.mock('fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('fs')>();
  const noop = () => undefined;
  return {
    ...actual,
    default: actual,
    writeFileSync: noop,
    mkdirSync: noop,
    renameSync: noop,
    chmodSync: noop,
    rmSync: noop,
    copyFileSync: noop,
  };
});

import { deepMerge } from './configService';

interface MergeCase {
  name: string;
  target: unknown;
  source: unknown;
  expected: unknown;
}

// apps/desktop/src/main/services/ → five levels below the repo root.
const here = path.dirname(fileURLToPath(import.meta.url));
const fixturePath = path.resolve(
  here,
  '..',
  '..',
  '..',
  '..',
  '..',
  'contracts',
  'deepmerge-cases.json',
);
const fixture = JSON.parse(readFileSync(fixturePath, 'utf-8')) as { cases: MergeCase[] };

describe('deepMerge contract (shared with Go config.go)', () => {
  it('the fixture loads and has cases', () => {
    expect(Array.isArray(fixture.cases)).toBe(true);
    expect(fixture.cases.length).toBeGreaterThan(0);
  });

  for (const c of fixture.cases) {
    it(c.name, () => {
      // Clone inputs so a case can't be mutated across the shared fixture.
      const target = structuredClone(c.target);
      const source = structuredClone(c.source);
      expect(deepMerge(target, source)).toEqual(c.expected);
    });
  }
});
