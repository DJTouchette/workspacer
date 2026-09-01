/**
 * The per-harness model vocabulary — the oracle every spawn path now asks
 * before putting a configured model on a CLI's argv.
 *
 * The distinction under test is the whole design: `isForeignModel` fires only
 * when ANOTHER harness positively claims the id, never merely because this
 * harness's pattern doesn't recognize it. A whitelist test would have rejected
 * every model shipped after this file was written and silently downgraded a
 * user's deliberate choice; this only ever fires on an id whose real owner we
 * can name.
 */
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import * as path from 'path';
import { servesModel, providersServing, isForeignModel } from './modelVocabulary';

interface VocabularyOwnershipCase {
  name: string;
  model: string;
  owners: string[];
  note: string;
}

const here = path.dirname(fileURLToPath(import.meta.url));
const fixture = JSON.parse(
  readFileSync(
    path.resolve(here, '../../../../../contracts/model-vocabulary-ownership-cases.json'),
    'utf8',
  ),
) as { ownershipCases: VocabularyOwnershipCase[] };

const PROVIDERS = ['claude', 'codex', 'copilot', 'opencode', 'pi'];

describe('Fleet Manager model vocabulary ownership contract shared with Hub', () => {
  it('keeps the corpus substantive', () => {
    expect(fixture.ownershipCases.length).toBeGreaterThanOrEqual(20);
  });

  for (const testCase of fixture.ownershipCases) {
    it(testCase.name, () => {
      const owners = [...testCase.owners].sort();
      expect(providersServing(testCase.model).sort(), testCase.note).toEqual(owners);

      for (const provider of PROVIDERS) {
        const ownedHere = owners.includes(provider);
        expect(servesModel(provider, testCase.model), `${testCase.note} (${provider})`).toBe(
          ownedHere,
        );
        expect(isForeignModel(provider, testCase.model), `${testCase.note} (${provider})`).toBe(
          owners.length > 0 && !ownedHere,
        );
      }
    });
  }
});

describe('model vocabulary boundary inputs', () => {
  it('does not claim an absent id or an unknown harness', () => {
    expect(servesModel('claude', undefined)).toBe(false);
    expect(servesModel('nosuchharness', 'sonnet')).toBe(false);
    expect(providersServing(undefined)).toEqual([]);
    expect(isForeignModel('codex', null)).toBe(false);
  });
});
