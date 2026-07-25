import { describe, it, expect } from 'vitest';
import { MODE_MANIFEST, resolveUiMode, type ModeManifest } from '../src/lib/uiMode';

describe('uiMode manifest', () => {
  it('defaults to fleet (unset / unknown config values)', () => {
    expect(resolveUiMode(undefined)).toBe('fleet');
    expect(resolveUiMode('')).toBe('fleet');
    expect(resolveUiMode('bogus')).toBe('fleet');
    expect(resolveUiMode('fleet')).toBe('fleet');
    expect(resolveUiMode('focus')).toBe('focus');
  });

  it('fleet mode attends to the whole fleet', () => {
    expect(MODE_MANIFEST.fleet).toEqual({
      feed: 'all',
      fleetDeck: true,
    });
  });

  it('focus mode narrows the feed and drops the deck', () => {
    expect(MODE_MANIFEST.focus).toEqual({
      feed: 'active-and-blocked',
      fleetDeck: false,
    });
  });

  // The manifest earns its existence by declaring DIFFERENCES. A field with the
  // same value in both entries is not a mode difference — it should be
  // unconditional behavior instead of a flag no consumer can meaningfully read.
  // (`inspectorRail` and `hubFooter` were both removed for exactly this reason:
  // one became true in both modes, the other was never read at all.)
  it('every manifest field actually differs between the modes', () => {
    const identical = (Object.keys(MODE_MANIFEST.fleet) as Array<keyof ModeManifest>).filter(
      (k) => MODE_MANIFEST.fleet[k] === MODE_MANIFEST.focus[k],
    );
    expect(
      identical,
      `manifest field(s) identical in both modes — make them unconditional instead: ${identical.join(', ')}`,
    ).toEqual([]);
  });
});
