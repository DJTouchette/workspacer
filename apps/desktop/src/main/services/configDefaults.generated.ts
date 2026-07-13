// GENERATED FILE — do not edit by hand.
// Source of truth: services/hub/cmd/brain/config_defaults.json (the brain go:embeds it).
// Regenerate: npm run gen:config-defaults  (apps/desktop/scripts/gen-config-defaults.mjs).
//
// The main process (configService.ts) and the renderer (hooks/configDefaults.ts)
// both build their defaults from this; drift tests assert each generated copy still
// deep-equals the JSON, so the desktop + brain defaults can never drift.

export const CONFIG_DEFAULTS = {
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
    shells: [
      {
        name: 'default',
        path: '',
        label: 'Default ($SHELL)',
      },
      {
        name: 'bash',
        path: '/bin/bash',
        label: 'Bash',
      },
      {
        name: 'zsh',
        path: '/bin/zsh',
        label: 'Zsh',
      },
      {
        name: 'fish',
        path: '/usr/bin/fish',
        label: 'Fish',
      },
    ],
    fontFamily:
      '"JetBrainsMono Nerd Font Mono", "JetBrainsMonoNL Nerd Font Mono", "JetBrainsMono NFM", "JetBrainsMonoNL NFM", "JetBrainsMono NF", "CaskaydiaMono Nerd Font Mono", "CaskaydiaCove Nerd Font Mono", "CaskaydiaMono NF", "Cascadia Mono", monospace',
    fontSize: 14,
    scrollback: 1500,
    cursorBlink: true,
    cursorStyle: 'block',
  },
  browser: {
    homepage: 'https://google.com',
    bookmarks: [
      {
        name: 'Go Docs',
        url: 'https://pkg.go.dev',
      },
      {
        name: 'MDN',
        url: 'https://developer.mozilla.org',
      },
      {
        name: 'Localhost 3000',
        url: 'http://localhost:3000',
      },
      {
        name: 'Localhost 8080',
        url: 'http://localhost:8080',
      },
    ],
    hibernateAfter: 300,
  },
  panes: {
    defaultWidth: 800,
    gap: 0,
    peek: 0,
    insertPosition: 'after',
    tabPosition: 'top',
    viewLevel: 'piloting',
    default: [
      {
        id: 'terminal-1',
        type: 'terminal',
        title: 'Terminal 1',
        width: 800,
        order: 0,
      },
      {
        id: 'terminal-2',
        type: 'terminal',
        title: 'Terminal 2',
        width: 800,
        order: 1,
      },
      {
        id: 'terminal-3',
        type: 'terminal',
        title: 'Terminal 3',
        width: 800,
        order: 2,
      },
      {
        id: 'notes-1',
        type: 'notes',
        title: 'Notes',
        width: 800,
        order: 3,
      },
    ],
  },
  keybindings: {
    prefix: 'ctrl+space',
    chordHints: true,
    presetId: 'vscode',
    shortcuts: {
      'command-palette': 'mod+shift+p',
      'open-file': 'mod+p',
      'next-agent': 'ctrl+tab',
      'prev-agent': 'ctrl+shift+tab',
      'next-attention': 'mod+shift+space',
      'spawn-agent': 'mod+shift+n',
      settings: 'mod+,',
      'save-session': 'mod+s',
      'toggle-help': 'f1',
      'toggle-terminal': 'mod+`',
      'toggle-sidebar': 'mod+b',
      'toggle-inbox': 'mod+shift+i',
      'toggle-fleet': 'mod+shift+f',
      'toggle-ui-mode': 'mod+shift+m',
      'toggle-inspector': 'mod+shift+e',
      'library-picker': 'mod+shift+l',
      'open-review': 'mod+shift+g',
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
  },
  notifications: {
    enabled: true,
    notifyDone: true,
    onlyWhenUnwatched: true,
    sound: false,
  },
  editor: {
    engine: 'codemirror',
    terminalCommand: 'nvim',
    vim: true,
  },
  claude: {
    defaultModel: 'opus[1m]',
    seenModels: [],
    skipPermissionsDefault: false,
    defaultPermissionMode: '',
    defaultView: 'terminal',
    workLog: 'cards',
    transport: 'stream',
    settingsOverlay: false,
  },
  agents: {
    defaultProvider: 'claude',
    defaultCwd: '',
    binaries: {
      claude: '',
      codex: '',
      opencode: '',
      pi: '',
    },
  },
  supervisor: {
    model: '',
    summarizerModel: 'sonnet',
    pollSeconds: 45,
  },
  directories: {
    recent: [],
    favourites: [],
  },
  scripts: {},
  session: {
    autoResume: false,
  },
  updates: {
    enabled: true,
    channel: 'latest',
  },
  apps: [
    {
      name: 'GitHub',
      url: 'https://github.com',
      icon: '💻',
    },
    {
      name: 'ChatGPT',
      url: 'https://chat.openai.com',
      icon: '🤖',
    },
    {
      name: 'Claude',
      url: 'https://claude.ai',
      icon: '✨',
    },
    {
      name: 'Stack Overflow',
      url: 'https://stackoverflow.com',
      icon: '📚',
    },
    {
      name: 'Localhost 3000',
      url: 'http://localhost:3000',
      icon: '🌐',
    },
    {
      name: 'Localhost 8080',
      url: 'http://localhost:8080',
      icon: '🌐',
    },
  ],
} as const;
