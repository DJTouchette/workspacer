/**
 * Keybinding presets — a chosen-on-first-run, switchable-in-Settings keymap.
 *
 * Each preset is a full map keyed by the same action ids as ACTION_REGISTRY
 * (shortcuts.ts), so it can't drift from the canonical action list (guarded by a
 * test). Presets only define the PERSISTED actions (the config.keybindings
 * .shortcuts subset); the renderer-only Fleet/Inbox/digit bindings
 * (configDefaults.ts) are surface-local vim-style nav and stay constant across
 * presets.
 *
 * Combos use the `mod` token (resolved to Cmd on macOS / Ctrl elsewhere — see
 * shortcuts.ts). Structural + pane-nav commands stay behind the prefix chord so
 * a focused terminal keeps Ctrl+C / Ctrl+L / etc. — the whole app is
 * deliberately terminal-safe, and presets honour that (no raw Ctrl+letter for
 * structural work).
 *
 * The leader is a literal `ctrl+space` on EVERY preset (Cmd+Space is Spotlight
 * on macOS, and a bare key like `space` would eat the spacebar in this non-modal
 * GUI). On Linux it can't stay ctrl+space — fcitx/ibus grab that as their
 * input-method toggle — so it's substituted at use-time with a single Alt tap;
 * see resolveLeader (shortcuts.ts). The stored value is untouched, so nothing
 * migrates. That terminal-safety is also why structural/nav commands live on the
 * leader, so presets differ mainly in their direct OVERLAY combos, their chord
 * grouping (flat vs which-key submenus), and whether they enable editor Vim
 * mode — not in the leader or the nav keys.
 */
import type { Config, KeybindingsConfig } from '../hooks/useConfig';

export type PresetId = 'vscode' | 'vim' | 'jetbrains';

export interface KeybindingPreset {
  id: PresetId;
  label: string;
  /** One-line pitch shown in the picker. */
  description: string;
  /** Chord leader for this preset. */
  prefix: string;
  chordHints: boolean;
  /** When true, applying the preset also switches the code editor into Vim mode. */
  editorVim?: boolean;
  /** Persisted action id → combo. Direct combos use `mod`; chords use `prefix …`. */
  shortcuts: Record<string, string>;
}

// ── VS Code ────────────────────────────────────────────────────────────────
// Direct mod-combos for overlays/tools/agents (VS Code's Ctrl+Shift+letter
// vocabulary); structural + pane-nav on the leader so terminals stay untouched.
const VSCODE: KeybindingPreset = {
  id: 'vscode',
  label: 'VS Code',
  description: 'Familiar Ctrl/Cmd+Shift combos, quick-open on ⌘P.',
  prefix: 'ctrl+space',
  chordHints: true,
  shortcuts: {
    'command-palette': 'mod+shift+p',
    'open-file': 'mod+p',
    // Literal ctrl (not mod): Cmd+Tab is the macOS app switcher, so mod+tab
    // would be hijacked on Mac. Ctrl+Tab works on every platform.
    'next-agent': 'ctrl+tab',
    'prev-agent': 'ctrl+shift+tab',
    'next-attention': 'mod+shift+space',
    'spawn-agent': 'mod+shift+n',
    settings: 'mod+,',
    'save-session': 'mod+s',
    'toggle-help': 'f1',
    'text-size-up': 'mod+=',
    'text-size-down': 'mod+-',
    'text-size-reset': 'mod+0',
    'toggle-terminal': 'mod+`',
    'toggle-sidebar': 'mod+b',
    'toggle-inbox': 'mod+shift+i',
    'toggle-fleet': 'mod+shift+f',
    'toggle-ui-mode': 'mod+shift+m',
    'toggle-inspector': 'mod+shift+e',
    'library-picker': 'mod+shift+l',
    'open-review': 'mod+shift+g',
    // Structural + nav behind the leader (terminal-safe).
    'new-terminal': 'prefix t',
    'new-claude': 'prefix c',
    'new-browser': 'prefix b',
    split: 'prefix s',
    'quick-split': 'prefix q',
    'close-pane': 'prefix w',
    'rename-tab': 'prefix r',
    'prev-tab': 'prefix [',
    'next-tab': 'prefix ]',
    'move-tab-left': 'prefix ,',
    'move-tab-right': 'prefix .',
    'nav-left': 'prefix h',
    'nav-down': 'prefix j',
    'nav-up': 'prefix k',
    'nav-right': 'prefix l',
  },
};

// ── Vim ──────────────────────────────────────────────────────────────────────
// A which-key / LazyVim-flavoured layer: overlays AND structural live on leader
// chords (nothing on mod-combos), with Window (`w`) and Agent (`a`) submenus
// that exercise the chord tree, plus the CodeMirror editor flipped into Vim
// mode. Shares the ctrl+space leader (a bare `space` leader would eat the
// spacebar in this non-modal GUI). This is the standout preset — pressing the
// leader opens a which-key menu instead of a modifier reflex.
const VIM: KeybindingPreset = {
  id: 'vim',
  label: 'Vim',
  description: 'Which-key leader chords + editor Vim mode.',
  prefix: 'ctrl+space',
  chordHints: true,
  editorVim: true,
  shortcuts: {
    'command-palette': 'prefix p',
    'open-file': 'prefix f',
    settings: 'prefix ,',
    'save-session': 'prefix s',
    'open-review': 'prefix g',
    'toggle-help': 'prefix ?',
    'text-size-up': 'mod+=',
    'text-size-down': 'mod+-',
    'text-size-reset': 'mod+0',
    'toggle-sidebar': 'prefix e',
    'toggle-terminal': 'prefix `',
    'toggle-inbox': 'prefix i',
    'toggle-fleet': 'prefix o',
    'toggle-ui-mode': 'prefix m',
    'toggle-inspector': 'prefix d',
    'library-picker': 'prefix l',
    'new-terminal': 'prefix t',
    'new-claude': 'prefix c',
    'new-browser': 'prefix b',
    'rename-tab': 'prefix r',
    'prev-tab': 'prefix [',
    'next-tab': 'prefix ]',
    'move-tab-left': 'prefix <',
    'move-tab-right': 'prefix >',
    // Window (pane) group.
    'nav-left': 'prefix w h',
    'nav-down': 'prefix w j',
    'nav-up': 'prefix w k',
    'nav-right': 'prefix w l',
    split: 'prefix w s',
    'quick-split': 'prefix w v',
    'close-pane': 'prefix w q',
    // Agent group.
    'prev-agent': 'prefix a k',
    'next-agent': 'prefix a j',
    'spawn-agent': 'prefix a n',
    'next-attention': 'prefix a a',
  },
};

// ── JetBrains (lite) ─────────────────────────────────────────────────────────
// A JetBrains-flavoured approximation within the current combo grammar — no
// double-tap (Search Everywhere / double-Shift can't be represented), so Find
// Action stands in as the palette. Direct combos dominate; structural + nav
// stay on the leader for terminal safety.
const JETBRAINS: KeybindingPreset = {
  id: 'jetbrains',
  label: 'JetBrains',
  description: 'IntelliJ-style combos (approximate — no double-tap).',
  prefix: 'ctrl+space',
  chordHints: true,
  shortcuts: {
    'command-palette': 'mod+shift+a', // Find Action (Search Everywhere needs double-Shift)
    'open-file': 'mod+shift+o', // Go to File
    settings: 'mod+,',
    'save-session': 'mod+s',
    'toggle-help': 'f1',
    'text-size-up': 'mod+=',
    'text-size-down': 'mod+-',
    'text-size-reset': 'mod+0',
    'toggle-terminal': 'alt+f12',
    'toggle-sidebar': 'alt+1', // Project tool window
    'prev-agent': 'alt+left',
    'next-agent': 'alt+right',
    'spawn-agent': 'mod+shift+n',
    'next-attention': 'mod+shift+space',
    'toggle-inbox': 'mod+shift+i',
    'toggle-fleet': 'mod+shift+f',
    'toggle-ui-mode': 'mod+shift+m',
    'toggle-inspector': 'mod+shift+e',
    'library-picker': 'mod+shift+l',
    'open-review': 'mod+shift+k', // Commit (mod+k alone would steal terminal Ctrl+K)
    // Structural + nav behind the leader (terminal-safe).
    'new-terminal': 'prefix t',
    'new-claude': 'prefix c',
    'new-browser': 'prefix b',
    split: 'prefix s',
    'quick-split': 'prefix v',
    'close-pane': 'prefix w',
    'rename-tab': 'prefix r',
    'prev-tab': 'prefix [',
    'next-tab': 'prefix ]',
    'move-tab-left': 'prefix ,',
    'move-tab-right': 'prefix .',
    'nav-left': 'prefix h',
    'nav-down': 'prefix j',
    'nav-up': 'prefix k',
    'nav-right': 'prefix l',
  },
};

export const KEYBINDING_PRESETS: Record<PresetId, KeybindingPreset> = {
  vscode: VSCODE,
  vim: VIM,
  jetbrains: JETBRAINS,
};

/** Display order for the picker. */
export const PRESET_ORDER: PresetId[] = ['vscode', 'vim', 'jetbrains'];

/** The default preset a fresh install lands on. */
export const DEFAULT_PRESET_ID: PresetId = 'vscode';

export function isPresetId(v: unknown): v is PresetId {
  return v === 'vscode' || v === 'vim' || v === 'jetbrains';
}

/**
 * Compute the keybindings config for switching to `id`. Preserves per-action
 * rebinds the user made on top of the *previous* preset (same rule as
 * migrateFlatChords): an action still equal to the previous preset's value is
 * replaced with the new preset's; a diverged value is a user override and
 * survives. With no previous preset (or `force`), the new preset wins wholesale.
 */
export function applyPresetKeybindings(
  id: PresetId,
  current: KeybindingsConfig | undefined,
  force = false,
): KeybindingsConfig {
  const preset = KEYBINDING_PRESETS[id];
  const prev =
    !force && current?.presetId && isPresetId(current.presetId)
      ? KEYBINDING_PRESETS[current.presetId]
      : undefined;
  const userShortcuts = current?.shortcuts ?? {};
  const nextShortcuts: Record<string, string> = { ...preset.shortcuts };
  if (prev) {
    for (const [action, combo] of Object.entries(userShortcuts)) {
      // A value that diverges from the previous preset's default is a user
      // rebind — carry it forward instead of clobbering it.
      if (combo && combo !== prev.shortcuts[action]) nextShortcuts[action] = combo;
    }
  }
  return {
    ...current,
    prefix: preset.prefix,
    chordHints: preset.chordHints,
    shortcuts: nextShortcuts,
    presetId: id,
  };
}

/**
 * A full config patch for applying a preset: the keybindings above, plus the
 * editor Vim toggle when the preset asks for it (only ever turned ON — switching
 * away leaves the editor mode alone, since a user may want Vim editing
 * independently of their workspace keymap).
 */
export function presetConfigPatch(id: PresetId, config: Config, force = false): Partial<Config> {
  const patch: Partial<Config> = {
    keybindings: applyPresetKeybindings(id, config.keybindings, force),
  };
  if (KEYBINDING_PRESETS[id].editorVim) {
    patch.editor = { ...config.editor!, vim: true };
  }
  return patch;
}
