import { describe, it, expect } from 'vitest';
import { minimalConfigPatch, sameConfigValue } from './configPatch';

/**
 * The clobber this prevents, concretely: the renderer boots, an agent writes
 * `claude.seenModels` behind its back, the user toggles keep-warm in Settings,
 * and the toggle's `{ claude: { ...config.claude, keepWarm } }` carries the boot
 * snapshot's empty seenModels over the fresh one. Trimming to what changed makes
 * that impossible without touching a single caller.
 */

describe('minimalConfigPatch', () => {
  it('keeps only the leaf the caller actually changed', () => {
    const current = { ui: { theme: 'everforest', sidebarWidth: 296, guiFontScale: 1.15 } };
    const partial = { ui: { ...current.ui, sidebarWidth: 340 } };
    expect(minimalConfigPatch(current, partial)).toEqual({ ui: { sidebarWidth: 340 } });
  });

  it('drops a stale sibling spread from an out-of-date snapshot', () => {
    // The renderer still thinks seenModels is empty; main has since written one.
    const rendererSnapshot = { claude: { keepWarm: false, seenModels: [] as string[] } };
    const patch = minimalConfigPatch(rendererSnapshot, {
      claude: { ...rendererSnapshot.claude, keepWarm: true },
    });
    expect(patch).toEqual({ claude: { keepWarm: true } });
    // The array the renderer would have overwritten never leaves it.
    expect(JSON.stringify(patch)).not.toContain('seenModels');
  });

  it('never diffs INTO a wholesale map — a deletion must survive the trim', () => {
    const current = {
      ui: { customThemes: { 'custom:a': { name: 'A' }, 'custom:b': { name: 'B' } } },
    };
    const patch = minimalConfigPatch(current, {
      ui: { customThemes: { 'custom:a': { name: 'A' } } },
    });
    // Main replaces customThemes wholesale, so the whole surviving map goes.
    expect(patch).toEqual({ ui: { customThemes: { 'custom:a': { name: 'A' } } } });
  });

  it('sends an emptied map — deleting the last entry is a real change', () => {
    const current = { ui: { customThemes: { 'custom:a': { name: 'A' } } } };
    expect(minimalConfigPatch(current, { ui: { customThemes: {} } })).toEqual({
      ui: { customThemes: {} },
    });
  });

  it('leaves a wholesale map alone when it is genuinely unchanged', () => {
    const current = { ui: { theme: 'a', customThemes: { 'custom:a': { name: 'A' } } } };
    expect(minimalConfigPatch(current, { ui: { ...current.ui, theme: 'b' } })).toEqual({
      ui: { theme: 'b' },
    });
  });

  it('applies wholesale only at the real path, not to any key of that name', () => {
    // A nested `budgets` that is NOT claude.budgets still diffs normally.
    const current = { supervisor: { budgets: { a: 1, b: 2 } } };
    expect(minimalConfigPatch(current, { supervisor: { budgets: { a: 1, b: 3 } } })).toEqual({
      supervisor: { budgets: { b: 3 } },
    });
  });

  it('returns nothing at all when the save changes nothing', () => {
    const current = { ui: { theme: 'everforest', sidebarWidth: 296 } };
    expect(minimalConfigPatch(current, { ui: { ...current.ui } })).toEqual({});
  });

  it('treats an array as one value — a changed array is sent whole', () => {
    const current = { agents: { binaries: { claude: '' } }, apps: [{ name: 'A' }] };
    expect(minimalConfigPatch(current, { apps: [{ name: 'A' }, { name: 'B' }] })).toEqual({
      apps: [{ name: 'A' }, { name: 'B' }],
    });
    expect(minimalConfigPatch(current, { apps: [{ name: 'A' }] })).toEqual({});
  });

  it('keeps a key the snapshot has never seen', () => {
    expect(
      minimalConfigPatch({ ui: { theme: 'x' } }, { ui: { sidebarWidth: 304 } } as any),
    ).toEqual({ ui: { sidebarWidth: 304 } });
    expect(minimalConfigPatch(undefined, { ui: { theme: 'x' } } as any)).toEqual({
      ui: { theme: 'x' },
    });
  });

  it('distinguishes false and 0 from absent — they are real values', () => {
    const current = { ui: { animations: true, guiFontScale: 1.15 } };
    expect(minimalConfigPatch(current, { ui: { animations: false } })).toEqual({
      ui: { animations: false },
    });
    expect(minimalConfigPatch(current, { ui: { guiFontScale: 0 } })).toEqual({
      ui: { guiFontScale: 0 },
    });
  });

  it('nests as deep as the change', () => {
    const current = {
      agents: { binaries: { claude: '', codex: '/usr/bin/codex' }, defaultProvider: 'claude' },
    };
    expect(
      minimalConfigPatch(current, {
        agents: { ...current.agents, binaries: { ...current.agents.binaries, claude: '/opt/c' } },
      }),
    ).toEqual({ agents: { binaries: { claude: '/opt/c' } } });
  });
});

describe('sameConfigValue', () => {
  it('compares structurally, not by reference', () => {
    expect(sameConfigValue({ a: [1, { b: 2 }] }, { a: [1, { b: 2 }] })).toBe(true);
    expect(sameConfigValue({ a: [1, { b: 2 }] }, { a: [1, { b: 3 }] })).toBe(false);
    expect(sameConfigValue([1, 2], [2, 1])).toBe(false);
  });

  it('does not call a missing key equal to an explicit undefined-ish value', () => {
    expect(sameConfigValue({ a: 1 }, { a: 1, b: null })).toBe(false);
    expect(sameConfigValue('', 0)).toBe(false);
    expect(sameConfigValue(null, undefined)).toBe(false);
  });
});
