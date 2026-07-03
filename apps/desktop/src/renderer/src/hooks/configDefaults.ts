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
  'settings': 'ctrl+,',
  'save-session': 'ctrl+shift+s',
  'open-file': 'ctrl+shift+o',
  'toggle-help': 'f1',
  'toggle-terminal': 'ctrl+`',
  'toggle-sidebar': 'ctrl+shift+b',
  'toggle-inbox': 'ctrl+shift+i',
  'toggle-fleet': 'ctrl+shift+f',
  'toggle-inspector': 'ctrl+shift+e',
  'library-picker': 'ctrl+shift+l',
  'open-review': 'ctrl+shift+g',
  // ── Digit-range bindings: the modifier + any of 1–9 ──
  'jump-tab': 'ctrl+1-9',
  'move-tab': 'ctrl+shift+1-9',
  // ── Prefix chords (Ctrl+Space then …), grouped into submenus ──
  // New ▸
  'new-terminal': 'prefix n t',
  'new-claude': 'prefix n c',
  'new-browser': 'prefix n b',
  // Tab ▸
  'prev-tab': 'prefix t [',
  'next-tab': 'prefix t ]',
  'move-tab-left': 'prefix t ,',
  'move-tab-right': 'prefix t .',
  'rename-tab': 'prefix t r',
  'close-pane': 'prefix t w',
  // Pane ▸
  'split': 'prefix p s',
  'quick-split': 'prefix p c',
  'nav-left': 'prefix p h',
  'nav-down': 'prefix p j',
  'nav-up': 'prefix p k',
  'nav-right': 'prefix p l',
  // Top-level leaf
  'cycle-view': 'prefix v',
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
  },
  terminal: {
    shell: '',
    shells: [],
    fontFamily: '"JetBrainsMono Nerd Font Mono", "JetBrainsMono NF", "CaskaydiaMono Nerd Font Mono", "CaskaydiaMono NF", monospace',
    fontSize: 14,
    scrollback: 1500,
    cursorBlink: true,
    cursorStyle: 'block',
  },
  panes: {
    defaultWidth: 800,
    gap: 16,
    peek: 80,
    insertPosition: 'after',
    tabPosition: 'top',
    viewMode: 'tabs',
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
  claude: { defaultView: 'terminal', workLog: 'cards' },
  supervisor: { model: '', summarizerModel: 'sonnet', pollSeconds: 45 },
};
