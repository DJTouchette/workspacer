// Cross-language context-window drift guard.
//
// contracts/model-context-windows.json is the SHARED fixture: a Rust test
// (services/claudemon/src/session/windows.rs) and a Go test
// (services/hub/cmd/brain/windows_test.go) consume the exact same file. If the
// three tables — or, more importantly, the three RESOLVERS — ever disagree,
// one side's contract test fails.
//
// The fixture exists because there were five hand-maintained window tables in
// this repo and three of them disagreed for the same model id.

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import * as path from 'path';
import {
  CONTEXT_WINDOWS,
  windowFor,
  requestedWindowFor,
  resolveContextWindow,
} from './modelContextWindows';

interface WindowRowCase {
  match: string;
  kind: 'contains' | 'prefix';
  window: number;
  note: string;
}
interface LookupCase {
  model: string;
  expected: number | null;
  note: string;
}
interface MarkerCase {
  requested: string;
  expected: number | null;
  note: string;
}
interface ResolutionCase {
  name: string;
  model: string | null;
  requestedModel: string | null;
  reportedWindow: number | null;
  override: number | null;
  peakContext: number;
  expected: number | null;
  note: string;
}

// This file lives at apps/desktop/src/main/shared/ — five levels below the
// repo root, where contracts/ sits.
const here = path.dirname(fileURLToPath(import.meta.url));
const fixturePath = path.resolve(
  here,
  '..',
  '..',
  '..',
  '..',
  '..',
  'contracts',
  'model-context-windows.json',
);
const fixture = JSON.parse(readFileSync(fixturePath, 'utf-8')) as {
  windows: WindowRowCase[];
  lookupCases: LookupCase[];
  markerCases: MarkerCase[];
  resolutionCases: ResolutionCase[];
};

describe('model context window contract (shared with Rust windows.rs and Go windows.go)', () => {
  it('the fixture loads and carries every block', () => {
    expect(fixture.windows.length).toBeGreaterThan(10);
    expect(fixture.lookupCases.length).toBeGreaterThanOrEqual(20);
    expect(fixture.markerCases.length).toBeGreaterThanOrEqual(8);
    expect(fixture.resolutionCases.length).toBeGreaterThanOrEqual(12);
  });

  // The table itself, row for row and IN ORDER. A lookup corpus alone would not
  // catch a reordering that happens to leave every sampled id where it was; the
  // order is what makes `fable` beat `claude`.
  it('the table matches the contract row for row, in order', () => {
    expect(CONTEXT_WINDOWS.length).toBe(fixture.windows.length);
    fixture.windows.forEach((want, i) => {
      const got = CONTEXT_WINDOWS[i];
      expect({ match: got.match, kind: got.kind, window: got.window }).toEqual({
        match: want.match,
        kind: want.kind,
        window: want.window,
      });
    });
  });

  it.each(fixture.lookupCases)('lookupCases: $model → $expected ($note)', (c) => {
    expect(windowFor(c.model)).toBe(c.expected);
  });

  it.each(fixture.markerCases)('markerCases: $requested → $expected ($note)', (c) => {
    expect(requestedWindowFor(c.requested)).toBe(c.expected);
  });

  // The block that actually pins the twins: it exercises the RESOLVER, so a
  // stack that ports the table correctly but the hierarchy wrong still goes red.
  it.each(fixture.resolutionCases)('resolutionCases: $name', (c) => {
    expect(
      resolveContextWindow(c.model, {
        requestedModel: c.requestedModel,
        reportedWindow: c.reportedWindow,
        overrideWindow: c.override,
        peakContext: c.peakContext,
      }),
    ).toBe(c.expected);
  });

  // The alarm must not be reachable only through the fixture: assert the
  // boundary directly, because "off by one on the tolerance" is the way a full
  // 200k session loses its meter at exactly the moment it matters.
  it('the drift alarm fires only past the tolerance', () => {
    const at = (peakContext: number): number | null =>
      resolveContextWindow('claude-opus-5', { peakContext });
    expect(at(200_000)).toBe(200_000);
    expect(at(204_000)).toBe(200_000);
    expect(at(204_001)).toBeNull();
  });
});
