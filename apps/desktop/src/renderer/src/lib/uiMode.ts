/**
 * App-wide UI modes — a lens over the SAME workspace state, never a different
 * layout. 'fleet' is the full mission-control chrome (today's UI); 'focus'
 * strips it down to the piloted agent: rail sidebar, no inspector rail, no
 * Fleet Deck, attention reduced to a compact "N need you" badge.
 *
 * Components must consume the manifest flags (via useUiMode) rather than
 * comparing mode strings, so what each mode shows stays declared in one place.
 */

export type UiMode = 'fleet' | 'focus';

export interface ModeManifest {
  /** Sidebar presentation: the full agent panel, or the icons-only rail. */
  sidebar: 'full' | 'rail';
  /** Per-pane inspector rail available (mount + composer toggle + hotkey). */
  inspectorRail: boolean;
  /** The Fleet Deck overlay may mount. */
  fleetDeck: boolean;
  /** Attention surface: full pills/counters, or one compact needs-you badge. */
  attention: 'full' | 'badge';
  /** Hub footer: full status row, or the compact dot (comes with the rail). */
  hubFooter: 'full' | 'compact';
}

export const MODE_MANIFEST: Record<UiMode, ModeManifest> = {
  fleet: {
    sidebar: 'full',
    inspectorRail: true,
    fleetDeck: true,
    attention: 'full',
    hubFooter: 'full',
  },
  focus: {
    sidebar: 'rail',
    inspectorRail: false,
    fleetDeck: false,
    attention: 'badge',
    hubFooter: 'compact',
  },
};

/** Normalize a raw config value to a concrete mode. Default 'fleet' — today's
 *  UI, so existing configs see zero behavior change. */
export function resolveUiMode(raw: string | undefined): UiMode {
  return raw === 'focus' ? 'focus' : 'fleet';
}
