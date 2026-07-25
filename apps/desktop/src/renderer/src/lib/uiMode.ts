/**
 * App-wide UI modes — a lens over the SAME workspace state, never a different
 * layout.
 *
 * The axis is HOW MANY AGENTS you are attending to, not how much chrome you
 * see. 'fleet' is supervising the whole fleet: every agent's card in the feed,
 * plus the Fleet Deck radar. 'focus' is working with one agent: the periphery
 * goes quiet, not blind — the sidebar keeps its live cards (and their inline
 * Approve/Reply) for the agent you're piloting and anything blocked on you,
 * while everything merely working or finished folds into one summary row.
 *
 * Deliberately NOT a width control: the sidebar collapse toggle (Ctrl+B,
 * `toggle-sidebar`) already owns that, and the two compose. An earlier version
 * of focus mode forced the icons-only rail, which duplicated the collapse
 * toggle and — once the sidebar became the live triage surface — threw away the
 * best attention affordance in the app to replace it with a bare count.
 *
 * Components must consume the manifest flags (via useUiMode) rather than
 * comparing mode strings, so what each mode shows stays declared in one place.
 * A field belongs here only if the two modes actually DIFFER on it; anything
 * that ends up identical in both entries should become unconditional behavior
 * instead of a flag nobody can act on.
 */

export type UiMode = 'fleet' | 'focus';

export interface ModeManifest {
  /**
   * Which agents the sidebar feed renders as full cards. 'active-and-blocked'
   * keeps the piloted agent and anything waiting on you (so a block is never
   * hidden and stays resolvable inline); the rest collapse into one expandable
   * "N others" row.
   */
  feed: 'all' | 'active-and-blocked';
  /** The Fleet Deck overlay may mount. */
  fleetDeck: boolean;
}

export const MODE_MANIFEST: Record<UiMode, ModeManifest> = {
  fleet: {
    feed: 'all',
    fleetDeck: true,
  },
  focus: {
    feed: 'active-and-blocked',
    fleetDeck: false,
  },
};

/** Normalize a raw config value to a concrete mode. Default 'fleet' — the full
 *  fleet view, so existing configs see zero behavior change. */
export function resolveUiMode(raw: string | undefined): UiMode {
  return raw === 'focus' ? 'focus' : 'fleet';
}
