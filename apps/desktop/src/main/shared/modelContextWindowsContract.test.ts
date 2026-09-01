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
  claudeArgvModel,
  ModelSelectionError,
  normalizeModelSelection,
  windowFor,
  requestedWindowFor,
  resolveContextWindow,
} from './modelContextWindows';
import { DEFAULT_CODEX_CONTEXT_WINDOW, contextRequestForSpawn } from './providerContext';

interface WindowRowCase {
  match: string;
  kind: 'contains' | 'prefix' | 'suffix';
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
interface SelectionCase {
  name: string;
  model: string;
  contextWindow: number | null;
  expectedModel: string | null;
  expectedContextWindow: number | null;
  error: string | null;
  note: string;
}
interface ClaudeArgvCase {
  name: string;
  model: string;
  contextWindow: number | null;
  expected: string | null;
  error: string | null;
  note: string;
}
interface ProviderContextDefault {
  provider: string;
  freshContextWindow: number;
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
  selectionCases: SelectionCase[];
  claudeArgvCases: ClaudeArgvCase[];
  resolutionCases: ResolutionCase[];
  providerContextDefaults: ProviderContextDefault[];
};

describe('model context window contract (shared with Rust windows.rs and Go windows.go)', () => {
  it('the fixture loads and carries every block', () => {
    expect(fixture.windows.length).toBeGreaterThan(10);
    expect(fixture.lookupCases.length).toBeGreaterThanOrEqual(20);
    expect(fixture.markerCases.length).toBeGreaterThanOrEqual(8);
    expect(fixture.selectionCases.length).toBeGreaterThanOrEqual(10);
    expect(fixture.claudeArgvCases.length).toBeGreaterThanOrEqual(4);
    expect(fixture.resolutionCases.length).toBeGreaterThanOrEqual(12);
    expect(fixture.providerContextDefaults).toHaveLength(1);
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

  it.each(fixture.selectionCases)('selectionCases: $name', (c) => {
    if (c.error) {
      try {
        normalizeModelSelection(c.model, c.contextWindow);
        throw new Error(`expected ${c.error}`);
      } catch (err) {
        expect(err).toBeInstanceOf(ModelSelectionError);
        expect((err as ModelSelectionError).code, c.note).toBe(c.error);
      }
      return;
    }
    const got = normalizeModelSelection(c.model, c.contextWindow);
    expect(got, c.note).toEqual({
      model: c.expectedModel,
      contextWindow: c.expectedContextWindow,
    });
    expect(got.model).not.toMatch(/(?:\[1m\]|-1m)$/i);
    expect(normalizeModelSelection(got.model, got.contextWindow)).toEqual(got);
  });

  it.each(fixture.claudeArgvCases)('claudeArgvCases: $name', (c) => {
    if (c.error) {
      expect(() => claudeArgvModel({ model: c.model, contextWindow: c.contextWindow })).toThrow(
        ModelSelectionError,
      );
      return;
    }
    expect(claudeArgvModel({ model: c.model, contextWindow: c.contextWindow }), c.note).toBe(
      c.expected,
    );
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

  it('providerContextDefaults pins the fresh, model-less Codex spawn request', () => {
    const codex = fixture.providerContextDefaults.find((row) => row.provider === 'codex');
    expect(codex, 'Codex must have an explicit provider-level default').toBeDefined();
    expect(DEFAULT_CODEX_CONTEXT_WINDOW, codex?.note).toBe(codex?.freshContextWindow);
    expect(contextRequestForSpawn('codex', undefined)).toBe(codex?.freshContextWindow);
    // Compatibility decision: pre-feature resumes without a durable request
    // must keep the provider default, not be retroactively upgraded.
    expect(contextRequestForSpawn('codex', undefined, 'legacy-session')).toBeUndefined();
  });
});
