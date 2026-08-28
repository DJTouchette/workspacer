import { describe, it, expect } from 'vitest';
import {
  providerAvailability,
  isProviderOffered,
  visibleProviderOptions,
  type ProviderDetection,
} from '../src/lib/providerAvailability';

/**
 * The rule every provider picker shares. Two properties matter more than the
 * filtering itself, because getting either wrong removes a working harness
 * from the UI with no way for the user to tell why:
 *
 *  1. FAIL OPEN — no answer, or an answer that doesn't mention a provider, is
 *     `unknown` and stays visible.
 *  2. NEVER VANISH WHAT'S IN USE — a harness named by config or by a live
 *     session survives the filter flagged, not dropped.
 */

const det = (rows: Array<[string, boolean]>): ProviderDetection[] =>
  rows.map(([provider, found]) => ({
    provider,
    found,
    resolvedPath: found ? `/usr/bin/${provider}` : null,
    customBin: '',
  }));

const OPTIONS = [
  { value: 'claude', label: 'Claude Code' },
  { value: 'codex', label: 'Codex' },
  { value: 'copilot', label: 'GitHub Copilot' },
  { value: 'opencode', label: 'OpenCode' },
  { value: 'pi', label: 'Pi' },
];

describe('providerAvailability', () => {
  it('reads a detection row', () => {
    const d = det([
      ['claude', true],
      ['codex', false],
    ]);
    expect(providerAvailability(d, 'claude')).toBe('installed');
    expect(providerAvailability(d, 'codex')).toBe('missing');
  });

  it('is unknown — not missing — with no answer at all', () => {
    expect(providerAvailability(null, 'codex')).toBe('unknown');
    expect(providerAvailability(undefined, 'codex')).toBe('unknown');
    // An empty list is what a host that doesn't serve detection returns; it is
    // not the claim "nothing is installed".
    expect(providerAvailability([], 'codex')).toBe('unknown');
    // A provider the answer simply doesn't cover (an older host, a new
    // harness) is also unknown.
    expect(providerAvailability(det([['claude', true]]), 'pi')).toBe('unknown');
    expect(isProviderOffered(det([['claude', true]]), 'pi')).toBe(true);
  });
});

describe('visibleProviderOptions', () => {
  it('hides what detection says is missing', () => {
    const visible = visibleProviderOptions(
      OPTIONS,
      det([
        ['claude', true],
        ['codex', true],
        ['copilot', false],
        ['opencode', false],
        ['pi', false],
      ]),
    );
    expect(visible.map((o) => o.value)).toEqual(['claude', 'codex']);
    expect(visible.every((o) => !o.missing)).toBe(true);
  });

  it('collapses to a single option when only claude is installed', () => {
    const visible = visibleProviderOptions(
      OPTIONS,
      det([
        ['claude', true],
        ['codex', false],
        ['copilot', false],
        ['opencode', false],
        ['pi', false],
      ]),
    );
    expect(visible.map((o) => o.value)).toEqual(['claude']);
  });

  it('keeps a missing harness that is the current value, flagged', () => {
    const visible = visibleProviderOptions(
      OPTIONS,
      det([
        ['claude', true],
        ['codex', false],
        ['copilot', false],
        ['opencode', false],
        ['pi', false],
      ]),
      ['codex'],
    );
    expect(visible.map((o) => o.value)).toEqual(['claude', 'codex']);
    expect(visible.find((o) => o.value === 'codex')!.missing).toBe(true);
    expect(visible.find((o) => o.value === 'claude')!.missing).toBe(false);
  });

  it('ignores undefined keep entries so optional config values pass straight through', () => {
    const visible = visibleProviderOptions(OPTIONS, det([['codex', false]]), [undefined, null, '']);
    expect(visible.map((o) => o.value)).not.toContain('codex');
  });

  it('shows everything while detection has not answered', () => {
    expect(visibleProviderOptions(OPTIONS, null).map((o) => o.value)).toEqual(
      OPTIONS.map((o) => o.value),
    );
  });

  it('preserves the option shape (labels, beta flags) it was given', () => {
    const withBeta = [{ value: 'pi', label: 'Pi', beta: true }];
    const [only] = visibleProviderOptions(withBeta, det([['pi', true]]));
    expect(only).toMatchObject({ value: 'pi', label: 'Pi', beta: true, missing: false });
  });
});
