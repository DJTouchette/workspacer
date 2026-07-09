// Default config values, split out of useConfig.ts so they form a dependency
// LEAF: ConfigContext.tsx imports DEFAULT_CONFIG from here, and useConfig.ts
// re-exports from here, but nothing here imports a *value* back from either —
// breaking the old useConfig ↔ ConfigContext import cycle that, under Vite
// HMR, could duplicate the ConfigContext module and make lazy-loaded panes
// throw "useConfig must be used inside <ConfigProvider>".
//
// The Config type is pulled in type-only (erased at build), so it adds no
// runtime edge.
import type { Config } from './useConfig';

export const DEFAULT_SHORTCUTS: Record<string, string> = {
  // ── Direct, terminal-safe ──
  'command-palette': 'ctrl+shift+p',
  'next-agent': 'ctrl+tab',
  'prev-agent': 'ctrl+shift+tab',
  'next-attention': 'ctrl+shift+space',
  'spawn-agent': 'ctrl+shift+n',
  settings: 'ctrl+,',
  'save-session': 'ctrl+shift+s',
  'open-file': 'ctrl+shift+o',
  'toggle-help': 'f1',
  'toggle-terminal': 'ctrl+`',
  'toggle-sidebar': 'ctrl+shift+b',
  'toggle-inbox': 'ctrl+shift+i',
  'toggle-fleet': 'ctrl+shift+f',
  'toggle-ui-mode': 'ctrl+shift+m',
  'toggle-inspector': 'ctrl+shift+e',
  'library-picker': 'ctrl+shift+l',
  'open-review': 'ctrl+shift+g',
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
  // ── Prefix chords (Ctrl+Space then one key) — flat, single-key per action ──
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
};

export const DEFAULT_CONFIG: Config = {
  ui: {
    animations: false,
    theme: 'dark',
    cornerStyle: '',
    borderColor: '',
    fontFamily: 'Inter, system-ui, sans-serif',
    fontSize: 14,
    borderRadius: 8,
    navBarHeight: 34,
    paneHeaderHeight: 22,
    showComposerSend: true,
    guiFontScale: 1.15,
    diffView: 'stacked',
    mode: 'fleet',
  },
  terminal: {
    shell: '',
    shells: [],
    fontFamily:
      '"JetBrainsMono Nerd Font Mono", "JetBrainsMono NF", "CaskaydiaMono Nerd Font Mono", "CaskaydiaMono NF", monospace',
    fontSize: 14,
    scrollback: 1500,
    cursorBlink: true,
    cursorStyle: 'block',
  },
  panes: {
    defaultWidth: 800,
    gap: 0,
    peek: 0,
    insertPosition: 'after',
    tabPosition: 'top',
    viewLevel: 'piloting',
    default: [],
  },
  browser: {
    homepage: 'https://google.com',
    bookmarks: [],
    hibernateAfter: 300,
  },
  keybindings: {
    prefix: 'ctrl+space',
    chordHints: true,
    shortcuts: { ...DEFAULT_SHORTCUTS },
  },
  notifications: {
    enabled: true,
    notifyDone: true,
    onlyWhenUnwatched: true,
    sound: false,
  },
  scripts: {},
  apps: [],
  session: { autoResume: false },
  updates: { enabled: true, channel: 'latest' },
  claude: { defaultView: 'terminal', workLog: 'cards', transport: 'stream' },
  supervisor: { model: '', summarizerModel: 'sonnet', pollSeconds: 45 },
};
