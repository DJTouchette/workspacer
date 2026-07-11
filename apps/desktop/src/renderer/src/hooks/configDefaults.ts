// Default config values, split out of useConfig.ts so they form a dependency
// LEAF: ConfigContext.tsx imports DEFAULT_CONFIG from here, and useConfig.ts
// re-exports from here, but nothing here imports a *value* back from either —
// breaking the old useConfig ↔ ConfigContext import cycle that, under Vite
// HMR, could duplicate the ConfigContext module and make lazy-loaded panes
// throw "useConfig must be used inside <ConfigProvider>".
//
// The persisted defaults come from the SINGLE SOURCE OF TRUTH shared with the
// headless brain (Go): services/hub/cmd/brain/config_defaults.json, surfaced
// here as CONFIG_DEFAULTS via the generated leaf. So the renderer's fallback
// values can no longer drift from the desktop main / brain defaults. Only two
// things stay hand-maintained here, because they are genuinely renderer-only:
//   1. UI-surface keybindings (Fleet Deck, Inbox drawer, digit jumps) — not part
//      of the persisted config schema, so they don't belong in the shared JSON.
//   2. The empty list placeholders (shells / panes / bookmarks / apps) — this
//      config is only shown for the instant before the backend config loads
//      (ConfigContext replaces it), and keeping those lists empty avoids a flash
//      of default apps/terminals before the user's real config arrives.
//
// The Config type is pulled in type-only (erased at build), so it adds no
// runtime edge; the generated leaf imports nothing.
import type { Config } from './useConfig';
import { CONFIG_DEFAULTS } from './configDefaults.generated';

// Renderer-only keybindings: UI surfaces that aren't user-persisted config keys,
// so they live here rather than in the shared config_defaults.json. FleetDeck /
// InboxDrawer layer these UNDER the live config (`{ ...DEFAULT_SHORTCUTS, ...live }`).
const RENDERER_ONLY_SHORTCUTS: Record<string, string> = {
  // ── Digit-range bindings: the modifier + any of 1–9 ──
  'jump-tab': 'ctrl+1-9',
  'move-tab': 'ctrl+shift+1-9',
  // ── Fleet Deck (only while the deck is open; bare keys are fine there) ──
  // Movement is per fleet view: the Cards grid navigates spatially (vim-style
  // hjkl), the List moves linearly through rows.
  'fleet-open': 'enter',
  'fleet-approve-yes': 'y',
  'fleet-approve-no': 'n',
  'fleet-answer': '1-9',
  'fleet-cards-left': 'h',
  'fleet-cards-down': 'j',
  'fleet-cards-up': 'k',
  'fleet-cards-right': 'l',
  'fleet-list-down': 'j',
  'fleet-list-up': 'k',
  // ── Inbox drawer (only while the drawer is open) ──
  'inbox-move-down': 'j',
  'inbox-move-up': 'k',
  'inbox-open': 'o',
  'inbox-approve-yes': 'y',
  'inbox-approve-no': 'n',
  'inbox-answer': '1-9',
  'inbox-dismiss': 'e',
  'inbox-snooze': 's',
  'inbox-clear-reviewed': 'shift+e',
};

// Persisted keybindings come from the shared single source; the renderer layers
// its UI-only bindings on top so the settings picker + Fleet/Inbox see them all.
export const DEFAULT_SHORTCUTS: Record<string, string> = {
  ...CONFIG_DEFAULTS.keybindings.shortcuts,
  ...RENDERER_ONLY_SHORTCUTS,
};

// Pre-load placeholder (ConfigContext's initial state, replaced once the backend
// config loads). Built from the shared defaults so every scalar can't drift; the
// big list fields stay empty to avoid a flash of defaults before the real config.
const shared = structuredClone(CONFIG_DEFAULTS) as unknown as Config;

export const DEFAULT_CONFIG: Config = {
  ...shared,
  terminal: { ...shared.terminal, shells: [] },
  panes: { ...shared.panes, default: [] },
  browser: { ...shared.browser, bookmarks: [] },
  apps: [],
  keybindings: { ...shared.keybindings, shortcuts: { ...DEFAULT_SHORTCUTS } },
};
