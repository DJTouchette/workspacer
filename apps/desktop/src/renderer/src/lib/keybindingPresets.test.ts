import { describe, it, expect } from 'vitest';
import {
  KEYBINDING_PRESETS,
  PRESET_ORDER,
  applyPresetKeybindings,
  presetConfigPatch,
} from './keybindingPresets';
import { ACTION_REGISTRY, resolveMod, eventMatchesCombo } from './shortcuts';
import { CONFIG_DEFAULTS } from '../hooks/configDefaults.generated';
import { DEFAULT_CONFIG } from '../hooks/configDefaults';

// The actions a preset must bind: everything persisted (i.e. not a scoped
// Fleet/Inbox binding and not a renderer-only digit range).
const PERSISTED_ACTIONS = ACTION_REGISTRY.filter((a) => !a.scope && !a.digitRange).map(
  (a) => a.action,
);

describe('keybinding presets', () => {
  for (const id of PRESET_ORDER) {
    const preset = KEYBINDING_PRESETS[id];

    it(`${id} binds every persisted action and nothing extra`, () => {
      for (const action of PERSISTED_ACTIONS) {
        expect(preset.shortcuts[action], `${id} missing ${action}`).toBeTruthy();
      }
      // No stray keys that aren't real actions (guards drift when actions change).
      for (const key of Object.keys(preset.shortcuts)) {
        expect(PERSISTED_ACTIONS, `${id} binds unknown action ${key}`).toContain(key);
      }
    });

    it(`${id} has no conflicting bindings`, () => {
      const entries = Object.entries(preset.shortcuts);
      const directs = entries.filter(([, c]) => !c.startsWith('prefix ')).map(([, c]) => c);
      expect(new Set(directs).size, `${id} has a duplicate direct combo`).toBe(directs.length);

      const chords = entries
        .filter(([, c]) => c.startsWith('prefix '))
        .map(([, c]) => c.replace(/^prefix\s+/, '').trim());
      expect(new Set(chords).size, `${id} has a duplicate chord`).toBe(chords.length);
      // No chord path may be a strict prefix of another (ambiguous which-key node).
      for (const a of chords) {
        for (const b of chords) {
          if (a !== b) expect(b.startsWith(a + ' '), `${id}: "${a}" shadows "${b}"`).toBe(false);
        }
      }
    });
  }

  it('the vscode preset is the shared default (no drift with config_defaults.json)', () => {
    expect(KEYBINDING_PRESETS.vscode.shortcuts).toEqual(CONFIG_DEFAULTS.keybindings.shortcuts);
    expect(KEYBINDING_PRESETS.vscode.prefix).toBe(CONFIG_DEFAULTS.keybindings.prefix);
    expect(CONFIG_DEFAULTS.keybindings.presetId).toBe('vscode');
  });
});

describe('applyPresetKeybindings', () => {
  it('preserves a user rebind when switching presets', () => {
    const base = {
      presetId: 'vscode',
      prefix: 'mod+space',
      chordHints: true,
      shortcuts: { ...KEYBINDING_PRESETS.vscode.shortcuts, settings: 'mod+9' /* user rebind */ },
    };
    const next = applyPresetKeybindings('vim', base);
    expect(next.presetId).toBe('vim');
    expect(next.prefix).toBe('ctrl+space');
    // Diverged binding survives; untouched one takes the new preset's value.
    expect(next.shortcuts?.settings).toBe('mod+9');
    expect(next.shortcuts?.['nav-left']).toBe(KEYBINDING_PRESETS.vim.shortcuts['nav-left']);
  });

  it('force overwrites everything (reset-to-preset)', () => {
    const base = {
      presetId: 'vscode',
      prefix: 'mod+space',
      chordHints: true,
      shortcuts: { ...KEYBINDING_PRESETS.vscode.shortcuts, settings: 'mod+9' },
    };
    const reset = applyPresetKeybindings('vim', base, true);
    expect(reset.shortcuts?.settings).toBe(KEYBINDING_PRESETS.vim.shortcuts.settings);
  });
});

describe('presetConfigPatch', () => {
  it('flips editor Vim on for the vim preset only', () => {
    expect(presetConfigPatch('vim', DEFAULT_CONFIG).editor?.vim).toBe(true);
    expect(presetConfigPatch('vscode', DEFAULT_CONFIG).editor).toBeUndefined();
    expect(presetConfigPatch('jetbrains', DEFAULT_CONFIG).editor).toBeUndefined();
  });
});

describe('resolveMod', () => {
  it('expands mod to the platform primary modifier', () => {
    expect(resolveMod('mod+p', true)).toBe('meta+p');
    expect(resolveMod('mod+p', false)).toBe('ctrl+p');
    expect(resolveMod('mod+shift+p', true)).toBe('meta+shift+p');
  });
  it('leaves non-mod combos and chords untouched', () => {
    expect(resolveMod('ctrl+tab', false)).toBe('ctrl+tab');
    expect(resolveMod('prefix w h', true)).toBe('prefix w h');
    expect(resolveMod('f1', true)).toBe('f1');
  });
  it('matches a keydown against a mod combo on non-mac (ctrl)', () => {
    const e = { key: 'p', ctrlKey: true, altKey: false, shiftKey: false, metaKey: false };
    // jsdom is not macOS, so mod resolves to ctrl.
    expect(eventMatchesCombo(e as unknown as KeyboardEvent, 'mod+p')).toBe(true);
    const withMeta = { key: 'p', ctrlKey: false, altKey: false, shiftKey: false, metaKey: true };
    expect(eventMatchesCombo(withMeta as unknown as KeyboardEvent, 'mod+p')).toBe(false);
  });
});
