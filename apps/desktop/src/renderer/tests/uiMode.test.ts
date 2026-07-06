import { describe, it, expect } from 'vitest';
import { MODE_MANIFEST, resolveUiMode } from '../src/lib/uiMode';

describe('uiMode manifest', () => {
  it('defaults to fleet (unset / unknown config values)', () => {
    expect(resolveUiMode(undefined)).toBe('fleet');
    expect(resolveUiMode('')).toBe('fleet');
    expect(resolveUiMode('bogus')).toBe('fleet');
    expect(resolveUiMode('fleet')).toBe('fleet');
    expect(resolveUiMode('focus')).toBe('focus');
  });

  it("fleet mode is today's full chrome (zero behavior change)", () => {
    expect(MODE_MANIFEST.fleet).toEqual({
      sidebar: 'full',
      inspectorRail: true,
      fleetDeck: true,
      attention: 'full',
      hubFooter: 'full',
    });
  });

  it('focus mode strips the chrome down to the piloted agent', () => {
    expect(MODE_MANIFEST.focus).toEqual({
      sidebar: 'rail',
      inspectorRail: false,
      fleetDeck: false,
      attention: 'badge',
      hubFooter: 'compact',
    });
  });
});
