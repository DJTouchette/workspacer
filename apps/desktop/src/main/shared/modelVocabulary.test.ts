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
import { servesModel, providersServing, isForeignModel } from './modelVocabulary';

describe('servesModel', () => {
  it('claims claude aliases (case-insensitively) and concrete claude ids', () => {
    for (const id of ['haiku', 'sonnet', 'sonnet[1m]', 'opus', 'opusplan', 'fable', 'default']) {
      expect(servesModel('claude', id)).toBe(true);
    }
    expect(servesModel('claude', 'Sonnet')).toBe(true);
    expect(servesModel('claude', 'claude-opus-4-5-20251101')).toBe(true);
    // `[1m]` on a concrete id is still a claude id.
    expect(servesModel('claude', 'claude-opus-4-5[1m]')).toBe(true);
  });

  it('claims codex families', () => {
    for (const id of ['gpt-5.1-codex-max', 'gpt-5', 'o3', 'o1-preview', 'codex-mini', 'gpt4']) {
      expect(servesModel('codex', id)).toBe(true);
    }
  });

  it('claims the provider/model form for opencode and pi', () => {
    expect(servesModel('opencode', 'anthropic/claude-sonnet-4')).toBe(true);
    expect(servesModel('pi', 'google/gemini-2.5-pro')).toBe(true);
  });

  it('is false for a blank model, an unknown provider, and a cross-harness id', () => {
    expect(servesModel('claude', '')).toBe(false);
    expect(servesModel('claude', '   ')).toBe(false);
    expect(servesModel('claude', undefined)).toBe(false);
    expect(servesModel('nosuchharness', 'sonnet')).toBe(false);
    expect(servesModel('codex', 'sonnet')).toBe(false);
    expect(servesModel('claude', 'gpt-5')).toBe(false);
  });

  it('trims before matching, so a config value with stray whitespace still resolves', () => {
    expect(servesModel('claude', '  sonnet  ')).toBe(true);
  });
});

describe('providersServing', () => {
  it('names the owner of an id', () => {
    expect(providersServing('sonnet')).toEqual(['claude']);
    expect(providersServing('gpt-5.1-codex-max')).toEqual(['codex']);
  });

  it('reports BOTH slash-form harnesses, because the string genuinely cannot tell them apart', () => {
    expect(providersServing('anthropic/claude-sonnet-4').sort()).toEqual(['opencode', 'pi']);
  });

  it('is empty for an id nobody claims — ignorance, not invalidity', () => {
    expect(providersServing('my-finetune-v3')).toEqual([]);
    expect(providersServing('')).toEqual([]);
  });
});

describe('isForeignModel', () => {
  it('catches the exact bugs this exists for', () => {
    // `supervisor.summarizerModel` / `agents.autoTitle.model` ship claude ids.
    expect(isForeignModel('codex', 'sonnet')).toBe(true);
    expect(isForeignModel('codex', 'haiku')).toBe(true);
    expect(isForeignModel('opencode', 'fable')).toBe(true);
    // …and the reverse, which a codex-configured field would produce.
    expect(isForeignModel('claude', 'gpt-5.1-codex-max')).toBe(true);
    expect(isForeignModel('claude', 'o3')).toBe(true);
  });

  it('is false for a model this harness serves', () => {
    expect(isForeignModel('claude', 'sonnet')).toBe(false);
    expect(isForeignModel('codex', 'gpt-5')).toBe(false);
    expect(isForeignModel('opencode', 'anthropic/claude-sonnet-4')).toBe(false);
  });

  it('PASSES THROUGH an id nobody claims — a private deployment is not a bug', () => {
    expect(isForeignModel('codex', 'my-finetune-v3')).toBe(false);
    expect(isForeignModel('claude', 'internal-eval-model')).toBe(false);
  });

  it('is false for a blank model — there is nothing to be wrong about', () => {
    expect(isForeignModel('codex', '')).toBe(false);
    expect(isForeignModel('codex', '   ')).toBe(false);
    expect(isForeignModel('codex', undefined)).toBe(false);
    expect(isForeignModel('codex', null)).toBe(false);
  });

  it('does NOT call a slash id foreign to opencode or pi (they share the form)', () => {
    expect(isForeignModel('opencode', 'google/gemini-2.5-pro')).toBe(false);
    expect(isForeignModel('pi', 'anthropic/claude-sonnet-4')).toBe(false);
  });
});
