import { describe, expect, it } from 'vitest';
import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import * as path from 'path';
import {
  canonicalManagerPreferences,
  managerContextPreference,
  managerSelectionErrorCode,
} from './managerSelection';
import { contextRequestForSpawn, DEFAULT_CODEX_CONTEXT_WINDOW } from './providerContext';

interface PreferenceCase {
  name: string;
  agents: Record<string, unknown>;
  expectedModels: Record<string, string>;
  expectedEfforts: Record<string, string>;
  expectedContexts: Record<string, number | null>;
  error: string | null;
  note: string;
}

const here = path.dirname(fileURLToPath(import.meta.url));
const fixture = JSON.parse(
  readFileSync(path.resolve(here, '../../../../../contracts/model-context-windows.json'), 'utf8'),
) as { managerPreferenceCases: PreferenceCase[] };

describe('Fleet Manager preference contract shared with Hub', () => {
  it('keeps the corpus substantive', () => {
    expect(fixture.managerPreferenceCases.length).toBeGreaterThanOrEqual(6);
  });

  for (const testCase of fixture.managerPreferenceCases) {
    it(testCase.name, () => {
      try {
        const got = canonicalManagerPreferences(testCase.agents);
        expect(testCase.error, testCase.note).toBeNull();
        expect(got.managerModels ?? {}).toEqual(testCase.expectedModels);
        expect(got.managerEfforts ?? {}).toEqual(testCase.expectedEfforts);
        expect(got.managerContextWindows ?? {}).toEqual(testCase.expectedContexts);
      } catch (error) {
        expect(managerSelectionErrorCode(error), testCase.note).toBe(testCase.error);
      }
    });
  }
});

describe('fresh/default/explicit manager context precedence', () => {
  it('an absent Codex entry takes the shared 1M policy', () => {
    expect(contextRequestForSpawn('codex', managerContextPreference('codex', undefined))).toBe(
      DEFAULT_CODEX_CONTEXT_WINDOW,
    );
  });

  it('explicit null and an explicit numeric preference defeat the default', () => {
    // Mutation guard: changing either stored value changes the resolved spawn
    // request. A resolver that blindly reapplies 1M makes both assertions fail.
    expect(
      contextRequestForSpawn('codex', managerContextPreference('codex', { codex: null })),
    ).toBeNull();
    expect(
      contextRequestForSpawn('codex', managerContextPreference('codex', { codex: 400_000 })),
    ).toBe(400_000);
  });

  it('a resume without a durable request stays provider-default', () => {
    expect(contextRequestForSpawn('codex', undefined, 'old-session')).toBeUndefined();
  });
});
