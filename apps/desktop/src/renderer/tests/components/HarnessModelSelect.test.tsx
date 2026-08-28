/**
 * The option list and stale-value verdict behind every harness-aware model
 * picker.
 *
 * Two rules, both learned the hard way. A value the catalog doesn't know is kept
 * VISIBLE — blanking the field would misreport what the config still holds, so
 * the user would "fix" a setting that was never blank. And the warning has to
 * distinguish another harness's id (which the spawn paths now drop outright)
 * from merely not-in-the-catalog (a retired model, a private deployment, a
 * catalog that failed to load): the first is a bug in the setting, the second
 * may be entirely deliberate, and telling the user the wrong one sends them
 * chasing the wrong thing.
 */
import { describe, it, expect } from 'vitest';
import { harnessModelOptions } from '../../src/components/settings/HarnessModelSelect';

const CODEX_CATALOG = [{ value: 'gpt-5-codex', label: 'GPT-5 Codex' }];

describe('harnessModelOptions', () => {
  it('always offers the harness default first', () => {
    const { options } = harnessModelOptions('codex', '', CODEX_CATALOG, true, 'codex default');
    expect(options[0]).toEqual({ value: '', label: 'codex default' });
    expect(options).toHaveLength(2);
  });

  it('does not duplicate a value the catalog already offers', () => {
    const { options, unknown, foreign } = harnessModelOptions(
      'codex',
      'gpt-5-codex',
      CODEX_CATALOG,
      true,
      'codex default',
    );
    expect(options.filter((o) => o.value === 'gpt-5-codex')).toHaveLength(1);
    expect(unknown).toBe(false);
    expect(foreign).toBe(false);
  });

  it('KEEPS an unknown value in the list rather than blanking the field', () => {
    const { options, unknown } = harnessModelOptions(
      'codex',
      'gpt-6-unreleased',
      CODEX_CATALOG,
      true,
      'codex default',
    );
    expect(options.map((o) => o.value)).toContain('gpt-6-unreleased');
    expect(unknown).toBe(true);
  });

  it('does not call a value unknown while the catalog is still loading', () => {
    // An empty list before the daemon answers is not evidence of anything, and
    // flashing a warning at every render is how a real warning stops being read.
    const { unknown } = harnessModelOptions('codex', 'gpt-5-codex', [], false, 'codex default');
    expect(unknown).toBe(false);
  });

  it('names a foreign id as foreign, not merely missing', () => {
    const { foreign, unknown } = harnessModelOptions(
      'codex',
      'sonnet',
      CODEX_CATALOG,
      true,
      'codex default',
    );
    expect(foreign).toBe(true);
    expect(unknown).toBe(true);
  });

  it('does NOT call an id nobody claims foreign — a private deployment is a real choice', () => {
    const { foreign, unknown } = harnessModelOptions(
      'codex',
      'my-finetune-v3',
      CODEX_CATALOG,
      true,
      'codex default',
    );
    expect(foreign).toBe(false);
    expect(unknown).toBe(true);
  });

  it('flags nothing at all for an empty value', () => {
    const { foreign, unknown } = harnessModelOptions('codex', '', [], true, 'codex default');
    expect(foreign).toBe(false);
    expect(unknown).toBe(false);
  });
});
