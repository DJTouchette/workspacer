// Theme system — CSS custom properties + xterm.js theme objects

export interface TerminalTheme {
  background: string;
  foreground: string;
  cursor: string;
  cursorAccent: string;
  selectionBackground: string;
  selectionForeground?: string;
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
}

/** Corner treatment for the whole UI. Themes declare a default; the user can
 *  override it in Settings (the override wins until the theme is switched). */
export type CornerStyle = 'rounded' | 'soft' | 'square';

export interface Theme {
  name: string;

  /** Default corner style for this theme (falls back to 'soft' if absent). */
  corners?: CornerStyle;

  // Surfaces
  bgBase: string;
  bgRaised: string;
  bgSurface: string;
  bgElevated: string;
  bgHeader: string;
  bgInput: string;
  bgHover: string;
  bgSelected: string;
  bgTerminal: string;

  // Borders
  border: string;
  borderSubtle: string;
  borderInput: string;

  // Text
  textPrimary: string;
  textSecondary: string;
  textTertiary: string;
  textMuted: string;
  textFaint: string;
  textDisabled: string;

  // Accent
  accent: string;
  accentText: string;
  accentGlow: string;
  accentBg: string;

  /** Focused-pane border color. Defaults to `accent` when omitted; the user can
   *  override it in Settings (override wins until the theme is switched). */
  borderActive?: string;

  // Status
  success: string;
  error: string;
  warning: string;
  purple?: string;
  /** "Busy / actively working" accent (streaming + thinking). Distinct from the
   *  brand accent so an agent mid-run reads differently from idle. Defaults to
   *  `accent` when omitted. */
  busy?: string;

  // Overlay / shadow
  overlay: string;
  shadow: string;

  // Scrollbar
  scrollbarThumb: string;
  scrollbarHover: string;

  // Claude pane
  claudeBg: string;
  claudeUserBubble: string;
  claudeUserBorder: string;
  claudeDivider: string;
  claudeBorder: string;
  claudeBorderSubtle: string;

  // Terminal
  terminal: TerminalTheme;
}

// ── Dark Theme (current look, exact color match) ──

export const darkTheme: Theme = {
  name: 'dark',
  bgBase: 'rgb(24, 24, 27)',
  bgRaised: 'rgb(28, 28, 32)',
  bgSurface: 'rgb(30, 30, 33)',
  bgElevated: 'rgb(32, 32, 36)',
  bgHeader: 'rgb(40, 42, 54)',
  bgInput: 'rgb(20, 20, 24)',
  bgHover: 'rgb(38, 38, 44)',
  bgSelected: 'rgb(45, 48, 60)',
  bgTerminal: '#1e1e22',
  border: 'rgb(50, 50, 55)',
  borderSubtle: 'rgb(45, 45, 50)',
  borderInput: 'rgb(55, 55, 60)',
  textPrimary: 'rgb(220, 220, 235)',
  textSecondary: 'rgb(200, 200, 210)',
  textTertiary: 'rgb(180, 180, 195)',
  textMuted: 'rgb(140, 140, 155)',
  textFaint: 'rgb(123, 123, 137)',
  textDisabled: 'rgb(90, 90, 100)',
  accent: 'rgb(80, 120, 200)',
  accentText: 'rgb(96, 165, 250)',
  accentGlow: 'rgba(80, 120, 200, 0.15)',
  accentBg: 'rgba(96, 165, 250, 0.15)',
  success: 'rgb(74, 222, 128)',
  error: 'rgb(248, 113, 113)',
  warning: '#facc15',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.4)',
  scrollbarThumb: 'rgba(255, 255, 255, 0.08)',
  scrollbarHover: 'rgba(255, 255, 255, 0.15)',
  claudeBg: '#0d0d10',
  claudeUserBubble: 'rgba(60, 90, 160, 0.15)',
  claudeUserBorder: 'rgba(60, 90, 160, 0.25)',
  claudeDivider: 'rgba(255, 255, 255, 0.05)',
  claudeBorder: 'rgba(255, 255, 255, 0.06)',
  claudeBorderSubtle: 'rgba(255, 255, 255, 0.04)',
  purple: '#c084fc',
  terminal: {
    background: '#1e1e22',
    foreground: '#e4e4e7',
    cursor: '#e4e4e7',
    cursorAccent: '#1e1e22',
    selectionBackground: 'rgba(128, 160, 255, 0.3)',
    black: '#1e1e21',
    red: '#f87171',
    green: '#4ade80',
    yellow: '#facc15',
    blue: '#60a5fa',
    magenta: '#c084fc',
    cyan: '#22d3ee',
    white: '#e4e4e7',
    brightBlack: '#71717a',
    brightRed: '#fca5a5',
    brightGreen: '#86efac',
    brightYellow: '#fde68a',
    brightBlue: '#93c5fd',
    brightMagenta: '#d8b4fe',
    brightCyan: '#67e8f9',
    brightWhite: '#fafafa',
  },
};

// ── Light Theme ──

export const lightTheme: Theme = {
  name: 'light',
  bgBase: '#f4f4f5',
  bgRaised: '#ffffff',
  bgSurface: '#ffffff',
  bgElevated: '#e4e4e7',
  bgHeader: '#e8e8ec',
  bgInput: '#ffffff',
  bgHover: '#e4e4e7',
  bgSelected: '#dbeafe',
  bgTerminal: '#fafafa',
  border: '#d4d4d8',
  borderSubtle: '#e4e4e7',
  borderInput: '#a1a1aa',
  textPrimary: '#18181b',
  textSecondary: '#27272a',
  textTertiary: '#3f3f46',
  textMuted: '#65656e',
  textFaint: '#737380',
  textDisabled: '#d4d4d8',
  accent: '#2563eb',
  accentText: '#1d4ed8',
  accentGlow: 'rgba(37, 99, 235, 0.15)',
  accentBg: 'rgba(37, 99, 235, 0.1)',
  success: '#16a34a',
  error: '#dc2626',
  warning: '#ca8a04',
  overlay: 'rgba(0, 0, 0, 0.3)',
  shadow: 'rgba(0, 0, 0, 0.1)',
  scrollbarThumb: 'rgba(0, 0, 0, 0.15)',
  scrollbarHover: 'rgba(0, 0, 0, 0.25)',
  claudeBg: '#fafafa',
  claudeUserBubble: 'rgba(37, 99, 235, 0.08)',
  claudeUserBorder: 'rgba(37, 99, 235, 0.2)',
  claudeDivider: 'rgba(0, 0, 0, 0.06)',
  claudeBorder: 'rgba(0, 0, 0, 0.08)',
  claudeBorderSubtle: 'rgba(0, 0, 0, 0.04)',
  purple: '#9333ea',
  terminal: {
    background: '#fafafa',
    foreground: '#18181b',
    cursor: '#18181b',
    cursorAccent: '#fafafa',
    selectionBackground: 'rgba(37, 99, 235, 0.2)',
    black: '#18181b',
    red: '#dc2626',
    green: '#16a34a',
    yellow: '#ca8a04',
    blue: '#2563eb',
    magenta: '#9333ea',
    cyan: '#0891b2',
    white: '#e4e4e7',
    brightBlack: '#71717a',
    brightRed: '#ef4444',
    brightGreen: '#22c55e',
    brightYellow: '#eab308',
    brightBlue: '#3b82f6',
    brightMagenta: '#a855f7',
    brightCyan: '#06b6d4',
    brightWhite: '#fafafa',
  },
};

// ── Midnight Theme (deep dark, purple accent, catppuccin-inspired) ──

export const midnightTheme: Theme = {
  name: 'midnight',
  bgBase: '#0a0a12',
  bgRaised: '#11111b',
  bgSurface: '#14142a',
  bgElevated: '#1a1a2e',
  bgHeader: '#1e1e3a',
  bgInput: '#080810',
  bgHover: '#1e1e3a',
  bgSelected: '#2a2a50',
  bgTerminal: '#0c0c14',
  border: '#2a2a40',
  borderSubtle: '#1e1e35',
  borderInput: '#3a3a55',
  textPrimary: '#cdd6f4',
  textSecondary: '#bac2de',
  textTertiary: '#a6adc8',
  textMuted: '#7f849c',
  textFaint: '#6f738d',
  textDisabled: '#45475a',
  accent: '#8b5cf6',
  accentText: '#a78bfa',
  accentGlow: 'rgba(139, 92, 246, 0.2)',
  accentBg: 'rgba(139, 92, 246, 0.15)',
  success: '#a6e3a1',
  error: '#f38ba8',
  warning: '#f9e2af',
  overlay: 'rgba(0, 0, 0, 0.6)',
  shadow: 'rgba(0, 0, 0, 0.5)',
  scrollbarThumb: 'rgba(255, 255, 255, 0.06)',
  scrollbarHover: 'rgba(255, 255, 255, 0.12)',
  claudeBg: '#080810',
  claudeUserBubble: 'rgba(139, 92, 246, 0.12)',
  claudeUserBorder: 'rgba(139, 92, 246, 0.25)',
  claudeDivider: 'rgba(255, 255, 255, 0.04)',
  claudeBorder: 'rgba(255, 255, 255, 0.06)',
  claudeBorderSubtle: 'rgba(255, 255, 255, 0.03)',
  terminal: {
    background: '#0c0c14',
    foreground: '#cdd6f4',
    cursor: '#f5e0dc',
    cursorAccent: '#0c0c14',
    selectionBackground: 'rgba(139, 92, 246, 0.3)',
    black: '#11111b',
    red: '#f38ba8',
    green: '#a6e3a1',
    yellow: '#f9e2af',
    blue: '#89b4fa',
    magenta: '#cba6f7',
    cyan: '#94e2d5',
    white: '#cdd6f4',
    brightBlack: '#585b70',
    brightRed: '#f38ba8',
    brightGreen: '#a6e3a1',
    brightYellow: '#f9e2af',
    brightBlue: '#89b4fa',
    brightMagenta: '#cba6f7',
    brightCyan: '#94e2d5',
    brightWhite: '#f5e0dc',
  },
};

// ── Solarized Dark Theme ──

export const solarizedDarkTheme: Theme = {
  name: 'solarized-dark',
  bgBase: '#002b36',
  bgRaised: '#073642',
  bgSurface: '#073642',
  bgElevated: '#0a4050',
  bgHeader: '#073642',
  bgInput: '#002028',
  bgHover: '#0a4050',
  bgSelected: '#0d4e60',
  bgTerminal: '#002b36',
  border: '#586e75',
  borderSubtle: '#2a4a52',
  borderInput: '#657b83',
  textPrimary: '#fdf6e3',
  textSecondary: '#eee8d5',
  textTertiary: '#a1adad',
  textMuted: '#8c9b9d',
  textFaint: '#748c94',
  textDisabled: '#586e75',
  accent: '#268bd2',
  accentText: '#268bd2',
  accentGlow: 'rgba(38, 139, 210, 0.2)',
  accentBg: 'rgba(38, 139, 210, 0.15)',
  success: '#859900',
  error: '#dc322f',
  warning: '#b58900',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.4)',
  scrollbarThumb: 'rgba(147, 161, 161, 0.15)',
  scrollbarHover: 'rgba(147, 161, 161, 0.25)',
  claudeBg: '#001e28',
  claudeUserBubble: 'rgba(38, 139, 210, 0.12)',
  claudeUserBorder: 'rgba(38, 139, 210, 0.25)',
  claudeDivider: 'rgba(238, 232, 213, 0.06)',
  claudeBorder: 'rgba(238, 232, 213, 0.08)',
  claudeBorderSubtle: 'rgba(238, 232, 213, 0.04)',
  terminal: {
    background: '#002b36',
    foreground: '#839496',
    cursor: '#93a1a1',
    cursorAccent: '#002b36',
    selectionBackground: 'rgba(38, 139, 210, 0.3)',
    black: '#073642',
    red: '#dc322f',
    green: '#859900',
    yellow: '#b58900',
    blue: '#268bd2',
    magenta: '#d33682',
    cyan: '#2aa198',
    white: '#eee8d5',
    brightBlack: '#586e75',
    brightRed: '#cb4b16',
    brightGreen: '#859900',
    brightYellow: '#b58900',
    brightBlue: '#6c71c4',
    brightMagenta: '#d33682',
    brightCyan: '#2aa198',
    brightWhite: '#fdf6e3',
  },
};

// ── Dracula Theme ──

export const draculaTheme: Theme = {
  name: 'dracula',
  bgBase: '#282a36',
  bgRaised: '#2d2f3d',
  bgSurface: '#343746',
  bgElevated: '#44475a',
  bgHeader: '#44475a',
  bgInput: '#21222c',
  bgHover: '#44475a',
  bgSelected: '#4d5070',
  bgTerminal: '#282a36',
  border: '#44475a',
  borderSubtle: '#3a3c4e',
  borderInput: '#6272a4',
  textPrimary: '#f8f8f2',
  textSecondary: '#e0e0ea',
  textTertiary: '#bfbfcf',
  textMuted: '#909cbf',
  textFaint: '#848ba9',
  textDisabled: '#44475a',
  accent: '#bd93f9',
  accentText: '#bd93f9',
  accentGlow: 'rgba(189, 147, 249, 0.2)',
  accentBg: 'rgba(189, 147, 249, 0.15)',
  success: '#50fa7b',
  error: '#ff5555',
  warning: '#f1fa8c',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.4)',
  scrollbarThumb: 'rgba(255, 255, 255, 0.08)',
  scrollbarHover: 'rgba(255, 255, 255, 0.15)',
  claudeBg: '#1e1f29',
  claudeUserBubble: 'rgba(189, 147, 249, 0.12)',
  claudeUserBorder: 'rgba(189, 147, 249, 0.25)',
  claudeDivider: 'rgba(248, 248, 242, 0.05)',
  claudeBorder: 'rgba(248, 248, 242, 0.06)',
  claudeBorderSubtle: 'rgba(248, 248, 242, 0.04)',
  terminal: {
    background: '#282a36',
    foreground: '#f8f8f2',
    cursor: '#f8f8f2',
    cursorAccent: '#282a36',
    selectionBackground: 'rgba(189, 147, 249, 0.3)',
    black: '#21222c',
    red: '#ff5555',
    green: '#50fa7b',
    yellow: '#f1fa8c',
    blue: '#bd93f9',
    magenta: '#ff79c6',
    cyan: '#8be9fd',
    white: '#f8f8f2',
    brightBlack: '#6272a4',
    brightRed: '#ff6e6e',
    brightGreen: '#69ff94',
    brightYellow: '#ffffa5',
    brightBlue: '#d6acff',
    brightMagenta: '#ff92df',
    brightCyan: '#a4ffff',
    brightWhite: '#ffffff',
  },
};

// ── Nord Theme ──

export const nordTheme: Theme = {
  name: 'nord',
  bgBase: '#2e3440',
  bgRaised: '#3b4252',
  bgSurface: '#3b4252',
  bgElevated: '#434c5e',
  bgHeader: '#434c5e',
  bgInput: '#272c36',
  bgHover: '#434c5e',
  bgSelected: '#4c566a',
  bgTerminal: '#2e3440',
  border: '#4c566a',
  borderSubtle: '#3b4252',
  borderInput: '#4c566a',
  textPrimary: '#eceff4',
  textSecondary: '#e5e9f0',
  textTertiary: '#d8dee9',
  textMuted: '#8eabc7',
  textFaint: '#8b96ad',
  textDisabled: '#4c566a',
  accent: '#81a1c1',
  accentText: '#88c0d0',
  accentGlow: 'rgba(129, 161, 193, 0.2)',
  accentBg: 'rgba(129, 161, 193, 0.15)',
  success: '#a3be8c',
  error: '#bf616a',
  warning: '#ebcb8b',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.4)',
  scrollbarThumb: 'rgba(216, 222, 233, 0.1)',
  scrollbarHover: 'rgba(216, 222, 233, 0.2)',
  claudeBg: '#252a33',
  claudeUserBubble: 'rgba(129, 161, 193, 0.12)',
  claudeUserBorder: 'rgba(129, 161, 193, 0.25)',
  claudeDivider: 'rgba(236, 239, 244, 0.05)',
  claudeBorder: 'rgba(236, 239, 244, 0.06)',
  claudeBorderSubtle: 'rgba(236, 239, 244, 0.04)',
  terminal: {
    background: '#2e3440',
    foreground: '#d8dee9',
    cursor: '#d8dee9',
    cursorAccent: '#2e3440',
    selectionBackground: 'rgba(129, 161, 193, 0.3)',
    black: '#3b4252',
    red: '#bf616a',
    green: '#a3be8c',
    yellow: '#ebcb8b',
    blue: '#81a1c1',
    magenta: '#b48ead',
    cyan: '#88c0d0',
    white: '#e5e9f0',
    brightBlack: '#4c566a',
    brightRed: '#bf616a',
    brightGreen: '#a3be8c',
    brightYellow: '#ebcb8b',
    brightBlue: '#81a1c1',
    brightMagenta: '#b48ead',
    brightCyan: '#8fbcbb',
    brightWhite: '#eceff4',
  },
};

// ── Monokai Theme ──

export const monokaiTheme: Theme = {
  name: 'monokai',
  bgBase: '#272822',
  bgRaised: '#2d2e27',
  bgSurface: '#33342c',
  bgElevated: '#3e3d32',
  bgHeader: '#3e3d32',
  bgInput: '#20211b',
  bgHover: '#3e3d32',
  bgSelected: '#49483e',
  bgTerminal: '#272822',
  border: '#49483e',
  borderSubtle: '#3e3d32',
  borderInput: '#5b5a4f',
  textPrimary: '#f8f8f2',
  textSecondary: '#e0e0da',
  textTertiary: '#c0c0b8',
  textMuted: '#9e9986',
  textFaint: '#8b8a72',
  textDisabled: '#49483e',
  accent: '#f92672',
  accentText: '#f92672',
  accentGlow: 'rgba(249, 38, 114, 0.2)',
  accentBg: 'rgba(249, 38, 114, 0.15)',
  success: '#a6e22e',
  error: '#f92672',
  warning: '#e6db74',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.4)',
  scrollbarThumb: 'rgba(248, 248, 242, 0.08)',
  scrollbarHover: 'rgba(248, 248, 242, 0.15)',
  claudeBg: '#1e1f1a',
  claudeUserBubble: 'rgba(249, 38, 114, 0.12)',
  claudeUserBorder: 'rgba(249, 38, 114, 0.25)',
  claudeDivider: 'rgba(248, 248, 242, 0.05)',
  claudeBorder: 'rgba(248, 248, 242, 0.06)',
  claudeBorderSubtle: 'rgba(248, 248, 242, 0.04)',
  terminal: {
    background: '#272822',
    foreground: '#f8f8f2',
    cursor: '#f8f8f0',
    cursorAccent: '#272822',
    selectionBackground: 'rgba(249, 38, 114, 0.3)',
    black: '#272822',
    red: '#f92672',
    green: '#a6e22e',
    yellow: '#f4bf75',
    blue: '#66d9ef',
    magenta: '#ae81ff',
    cyan: '#a1efe4',
    white: '#f8f8f2',
    brightBlack: '#75715e',
    brightRed: '#f92672',
    brightGreen: '#a6e22e',
    brightYellow: '#e6db74',
    brightBlue: '#66d9ef',
    brightMagenta: '#ae81ff',
    brightCyan: '#a1efe4',
    brightWhite: '#f9f8f5',
  },
};

// ── GitHub Dark Theme ──

export const githubDarkTheme: Theme = {
  name: 'github-dark',
  bgBase: '#0d1117',
  bgRaised: '#161b22',
  bgSurface: '#161b22',
  bgElevated: '#1c2128',
  bgHeader: '#161b22',
  bgInput: '#0a0e14',
  bgHover: '#1c2128',
  bgSelected: '#1f2937',
  bgTerminal: '#0d1117',
  border: '#30363d',
  borderSubtle: '#21262d',
  borderInput: '#484f58',
  textPrimary: '#e6edf3',
  textSecondary: '#c9d1d9',
  textTertiary: '#b1bac4',
  textMuted: '#8b949e',
  textFaint: '#6d7885',
  textDisabled: '#30363d',
  accent: '#58a6ff',
  accentText: '#58a6ff',
  accentGlow: 'rgba(88, 166, 255, 0.2)',
  accentBg: 'rgba(88, 166, 255, 0.15)',
  success: '#3fb950',
  error: '#f85149',
  warning: '#d29922',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.4)',
  scrollbarThumb: 'rgba(139, 148, 158, 0.15)',
  scrollbarHover: 'rgba(139, 148, 158, 0.25)',
  claudeBg: '#080b10',
  claudeUserBubble: 'rgba(88, 166, 255, 0.12)',
  claudeUserBorder: 'rgba(88, 166, 255, 0.25)',
  claudeDivider: 'rgba(230, 237, 243, 0.05)',
  claudeBorder: 'rgba(230, 237, 243, 0.06)',
  claudeBorderSubtle: 'rgba(230, 237, 243, 0.04)',
  terminal: {
    background: '#0d1117',
    foreground: '#e6edf3',
    cursor: '#e6edf3',
    cursorAccent: '#0d1117',
    selectionBackground: 'rgba(88, 166, 255, 0.3)',
    black: '#0d1117',
    red: '#f85149',
    green: '#3fb950',
    yellow: '#d29922',
    blue: '#58a6ff',
    magenta: '#bc8cff',
    cyan: '#39c5cf',
    white: '#b1bac4',
    brightBlack: '#484f58',
    brightRed: '#ff7b72',
    brightGreen: '#56d364',
    brightYellow: '#e3b341',
    brightBlue: '#79c0ff',
    brightMagenta: '#d2a8ff',
    brightCyan: '#56d4dd',
    brightWhite: '#f0f6fc',
  },
};

// ── Gruvbox Dark Theme (warm retro) ──

export const gruvboxTheme: Theme = {
  name: 'gruvbox',
  bgBase: '#282828',
  bgRaised: '#3c3836',
  bgSurface: '#32302f',
  bgElevated: '#3c3836',
  bgHeader: '#504945',
  bgInput: '#1d2021',
  bgHover: '#504945',
  bgSelected: '#665c54',
  bgTerminal: '#282828',
  border: '#504945',
  borderSubtle: '#3c3836',
  borderInput: '#665c54',
  textPrimary: '#ebdbb2',
  textSecondary: '#d5c4a1',
  textTertiary: '#bdae93',
  textMuted: '#a89984',
  textFaint: '#958778',
  textDisabled: '#665c54',
  accent: '#fe8019',
  accentText: '#fe8019',
  accentGlow: 'rgba(254, 128, 25, 0.2)',
  accentBg: 'rgba(254, 128, 25, 0.15)',
  success: '#b8bb26',
  error: '#fb4934',
  warning: '#fabd2f',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.4)',
  scrollbarThumb: 'rgba(235, 219, 178, 0.1)',
  scrollbarHover: 'rgba(235, 219, 178, 0.18)',
  claudeBg: '#1d2021',
  claudeUserBubble: 'rgba(254, 128, 25, 0.12)',
  claudeUserBorder: 'rgba(254, 128, 25, 0.25)',
  claudeDivider: 'rgba(235, 219, 178, 0.06)',
  claudeBorder: 'rgba(235, 219, 178, 0.08)',
  claudeBorderSubtle: 'rgba(235, 219, 178, 0.04)',
  terminal: {
    background: '#282828',
    foreground: '#ebdbb2',
    cursor: '#ebdbb2',
    cursorAccent: '#282828',
    selectionBackground: 'rgba(254, 128, 25, 0.3)',
    black: '#282828',
    red: '#cc241d',
    green: '#98971a',
    yellow: '#d79921',
    blue: '#458588',
    magenta: '#b16286',
    cyan: '#689d6a',
    white: '#a89984',
    brightBlack: '#928374',
    brightRed: '#fb4934',
    brightGreen: '#b8bb26',
    brightYellow: '#fabd2f',
    brightBlue: '#83a598',
    brightMagenta: '#d3869b',
    brightCyan: '#8ec07c',
    brightWhite: '#ebdbb2',
  },
};

// ── Tokyo Night Theme (inspired by Tokyo city lights) ──

export const tokyoNightTheme: Theme = {
  name: 'tokyo-night',
  bgBase: '#1a1b26',
  bgRaised: '#1f2335',
  bgSurface: '#24283b',
  bgElevated: '#292e42',
  bgHeader: '#24283b',
  bgInput: '#16161e',
  bgHover: '#292e42',
  bgSelected: '#33467c',
  bgTerminal: '#1a1b26',
  border: '#3b4261',
  borderSubtle: '#292e42',
  borderInput: '#414868',
  textPrimary: '#c0caf5',
  textSecondary: '#a9b1d6',
  textTertiary: '#9aa5ce',
  textMuted: '#848db2',
  textFaint: '#737ca7',
  textDisabled: '#3b4261',
  accent: '#7aa2f7',
  accentText: '#7aa2f7',
  accentGlow: 'rgba(122, 162, 247, 0.2)',
  accentBg: 'rgba(122, 162, 247, 0.15)',
  success: '#9ece6a',
  error: '#f7768e',
  warning: '#e0af68',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.4)',
  scrollbarThumb: 'rgba(192, 202, 245, 0.08)',
  scrollbarHover: 'rgba(192, 202, 245, 0.15)',
  claudeBg: '#16161e',
  claudeUserBubble: 'rgba(122, 162, 247, 0.12)',
  claudeUserBorder: 'rgba(122, 162, 247, 0.25)',
  claudeDivider: 'rgba(192, 202, 245, 0.05)',
  claudeBorder: 'rgba(192, 202, 245, 0.06)',
  claudeBorderSubtle: 'rgba(192, 202, 245, 0.04)',
  terminal: {
    background: '#1a1b26',
    foreground: '#c0caf5',
    cursor: '#c0caf5',
    cursorAccent: '#1a1b26',
    selectionBackground: 'rgba(122, 162, 247, 0.3)',
    black: '#15161e',
    red: '#f7768e',
    green: '#9ece6a',
    yellow: '#e0af68',
    blue: '#7aa2f7',
    magenta: '#bb9af7',
    cyan: '#7dcfff',
    white: '#a9b1d6',
    brightBlack: '#414868',
    brightRed: '#f7768e',
    brightGreen: '#9ece6a',
    brightYellow: '#e0af68',
    brightBlue: '#7aa2f7',
    brightMagenta: '#bb9af7',
    brightCyan: '#7dcfff',
    brightWhite: '#c0caf5',
  },
};

// ── One Dark Theme (Atom's iconic theme) ──

export const oneDarkTheme: Theme = {
  name: 'one-dark',
  bgBase: '#282c34',
  bgRaised: '#2c313a',
  bgSurface: '#21252b',
  bgElevated: '#323842',
  bgHeader: '#21252b',
  bgInput: '#1b1f27',
  bgHover: '#323842',
  bgSelected: '#3e4452',
  bgTerminal: '#282c34',
  border: '#3e4452',
  borderSubtle: '#2c313a',
  borderInput: '#4b5263',
  textPrimary: '#abb2bf',
  textSecondary: '#a4abb9',
  textTertiary: '#a6abb4',
  textMuted: '#989eaa',
  textFaint: '#848da2',
  textDisabled: '#3e4452',
  accent: '#61afef',
  accentText: '#61afef',
  accentGlow: 'rgba(97, 175, 239, 0.2)',
  accentBg: 'rgba(97, 175, 239, 0.15)',
  success: '#98c379',
  error: '#e06c75',
  warning: '#e5c07b',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.4)',
  scrollbarThumb: 'rgba(171, 178, 191, 0.1)',
  scrollbarHover: 'rgba(171, 178, 191, 0.18)',
  claudeBg: '#1b1f27',
  claudeUserBubble: 'rgba(97, 175, 239, 0.12)',
  claudeUserBorder: 'rgba(97, 175, 239, 0.25)',
  claudeDivider: 'rgba(171, 178, 191, 0.06)',
  claudeBorder: 'rgba(171, 178, 191, 0.08)',
  claudeBorderSubtle: 'rgba(171, 178, 191, 0.04)',
  terminal: {
    background: '#282c34',
    foreground: '#abb2bf',
    cursor: '#528bff',
    cursorAccent: '#282c34',
    selectionBackground: 'rgba(97, 175, 239, 0.3)',
    black: '#282c34',
    red: '#e06c75',
    green: '#98c379',
    yellow: '#e5c07b',
    blue: '#61afef',
    magenta: '#c678dd',
    cyan: '#56b6c2',
    white: '#abb2bf',
    brightBlack: '#5c6370',
    brightRed: '#e06c75',
    brightGreen: '#98c379',
    brightYellow: '#e5c07b',
    brightBlue: '#61afef',
    brightMagenta: '#c678dd',
    brightCyan: '#56b6c2',
    brightWhite: '#ffffff',
  },
};

// ── Rose Pine Theme (elegant dark with rose accents) ──

export const rosePineTheme: Theme = {
  name: 'rose-pine',
  bgBase: '#191724',
  bgRaised: '#1f1d2e',
  bgSurface: '#26233a',
  bgElevated: '#2a2837',
  bgHeader: '#1f1d2e',
  bgInput: '#15131f',
  bgHover: '#26233a',
  bgSelected: '#312e4a',
  bgTerminal: '#191724',
  border: '#26233a',
  borderSubtle: '#1f1d2e',
  borderInput: '#393552',
  textPrimary: '#e0def4',
  textSecondary: '#d0cde8',
  textTertiary: '#9e9ab4',
  textMuted: '#8c89a1',
  textFaint: '#7d7896',
  textDisabled: '#403d52',
  accent: '#c4a7e7',
  accentText: '#c4a7e7',
  accentGlow: 'rgba(196, 167, 231, 0.2)',
  accentBg: 'rgba(196, 167, 231, 0.15)',
  success: '#9ccfd8',
  error: '#eb6f92',
  warning: '#f6c177',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.4)',
  scrollbarThumb: 'rgba(224, 222, 244, 0.08)',
  scrollbarHover: 'rgba(224, 222, 244, 0.15)',
  claudeBg: '#15131f',
  claudeUserBubble: 'rgba(196, 167, 231, 0.12)',
  claudeUserBorder: 'rgba(196, 167, 231, 0.25)',
  claudeDivider: 'rgba(224, 222, 244, 0.05)',
  claudeBorder: 'rgba(224, 222, 244, 0.06)',
  claudeBorderSubtle: 'rgba(224, 222, 244, 0.04)',
  terminal: {
    background: '#191724',
    foreground: '#e0def4',
    cursor: '#ebbcba',
    cursorAccent: '#191724',
    selectionBackground: 'rgba(196, 167, 231, 0.3)',
    black: '#26233a',
    red: '#eb6f92',
    green: '#31748f',
    yellow: '#f6c177',
    blue: '#9ccfd8',
    magenta: '#c4a7e7',
    cyan: '#9ccfd8',
    white: '#e0def4',
    brightBlack: '#6e6a86',
    brightRed: '#eb6f92',
    brightGreen: '#31748f',
    brightYellow: '#f6c177',
    brightBlue: '#9ccfd8',
    brightMagenta: '#c4a7e7',
    brightCyan: '#9ccfd8',
    brightWhite: '#e0def4',
  },
};

// ── Cyberpunk Theme (neon on dark, futuristic) ──

export const cyberpunkTheme: Theme = {
  name: 'cyberpunk',
  bgBase: '#0a0a0f',
  bgRaised: '#12121a',
  bgSurface: '#0e0e16',
  bgElevated: '#1a1a2a',
  bgHeader: '#12121a',
  bgInput: '#08080c',
  bgHover: '#1a1a2a',
  bgSelected: '#22223a',
  bgTerminal: '#0a0a0f',
  border: '#1a1a2a',
  borderSubtle: '#12121a',
  borderInput: '#2a2a44',
  textPrimary: '#e0e0ff',
  textSecondary: '#c0c0e0',
  textTertiary: '#a0a0c0',
  textMuted: '#81819e',
  textFaint: '#717198',
  textDisabled: '#2a2a44',
  accent: '#05d9e8',
  accentText: '#05d9e8',
  accentGlow: 'rgba(5, 217, 232, 0.2)',
  accentBg: 'rgba(5, 217, 232, 0.15)',
  success: '#01c38d',
  error: '#ff2a6d',
  warning: '#d1f700',
  overlay: 'rgba(0, 0, 0, 0.6)',
  shadow: 'rgba(0, 0, 0, 0.5)',
  scrollbarThumb: 'rgba(224, 224, 255, 0.06)',
  scrollbarHover: 'rgba(224, 224, 255, 0.12)',
  claudeBg: '#08080c',
  claudeUserBubble: 'rgba(5, 217, 232, 0.1)',
  claudeUserBorder: 'rgba(5, 217, 232, 0.25)',
  claudeDivider: 'rgba(224, 224, 255, 0.04)',
  claudeBorder: 'rgba(224, 224, 255, 0.06)',
  claudeBorderSubtle: 'rgba(224, 224, 255, 0.03)',
  terminal: {
    background: '#0a0a0f',
    foreground: '#e0e0ff',
    cursor: '#05d9e8',
    cursorAccent: '#0a0a0f',
    selectionBackground: 'rgba(5, 217, 232, 0.3)',
    black: '#0a0a0f',
    red: '#ff2a6d',
    green: '#01c38d',
    yellow: '#d1f700',
    blue: '#05d9e8',
    magenta: '#b537f2',
    cyan: '#05d9e8',
    white: '#e0e0ff',
    brightBlack: '#6a6a8a',
    brightRed: '#ff4f8a',
    brightGreen: '#01e8a2',
    brightYellow: '#e0ff1a',
    brightBlue: '#33e1f0',
    brightMagenta: '#c966f5',
    brightCyan: '#33e1f0',
    brightWhite: '#ffffff',
  },
};

// ── Catppuccin Mocha (the soothing pastel dark) ──

export const catppuccinMochaTheme: Theme = {
  name: 'catppuccin-mocha',
  corners: 'rounded',
  bgBase: '#1e1e2e',
  bgRaised: '#1c1c2c',
  bgSurface: '#24243a',
  bgElevated: '#313244',
  bgHeader: '#24243a',
  bgInput: '#181825',
  bgHover: '#313244',
  bgSelected: '#3b3b54',
  bgTerminal: '#1e1e2e',
  border: '#45475a',
  borderSubtle: '#313244',
  borderInput: '#585b70',
  textPrimary: '#cdd6f4',
  textSecondary: '#bac2de',
  textTertiary: '#a6adc8',
  textMuted: '#8c91a6',
  textFaint: '#7c8095',
  textDisabled: '#585b70',
  accent: '#cba6f7',
  accentText: '#cba6f7',
  accentGlow: 'rgba(203, 166, 247, 0.2)',
  accentBg: 'rgba(203, 166, 247, 0.15)',
  success: '#a6e3a1',
  error: '#f38ba8',
  warning: '#f9e2af',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.4)',
  scrollbarThumb: 'rgba(205, 214, 244, 0.08)',
  scrollbarHover: 'rgba(205, 214, 244, 0.15)',
  claudeBg: '#181825',
  claudeUserBubble: 'rgba(203, 166, 247, 0.12)',
  claudeUserBorder: 'rgba(203, 166, 247, 0.25)',
  claudeDivider: 'rgba(205, 214, 244, 0.05)',
  claudeBorder: 'rgba(205, 214, 244, 0.06)',
  claudeBorderSubtle: 'rgba(205, 214, 244, 0.04)',
  terminal: {
    background: '#1e1e2e',
    foreground: '#cdd6f4',
    cursor: '#f5e0dc',
    cursorAccent: '#1e1e2e',
    selectionBackground: 'rgba(203, 166, 247, 0.3)',
    black: '#45475a',
    red: '#f38ba8',
    green: '#a6e3a1',
    yellow: '#f9e2af',
    blue: '#89b4fa',
    magenta: '#f5c2e7',
    cyan: '#94e2d5',
    white: '#bac2de',
    brightBlack: '#585b70',
    brightRed: '#f38ba8',
    brightGreen: '#a6e3a1',
    brightYellow: '#f9e2af',
    brightBlue: '#89b4fa',
    brightMagenta: '#f5c2e7',
    brightCyan: '#94e2d5',
    brightWhite: '#a6adc8',
  },
};

// ── Everforest (warm, low-contrast forest green) ──

export const everforestTheme: Theme = {
  name: 'everforest',
  corners: 'soft',
  bgBase: '#2d353b',
  bgRaised: '#272e33',
  bgSurface: '#2f373d',
  bgElevated: '#333c43',
  bgHeader: '#272e33',
  bgInput: '#232a2e',
  bgHover: '#333c43',
  bgSelected: '#3a454a',
  bgTerminal: '#2d353b',
  border: '#444f55',
  borderSubtle: '#374149',
  borderInput: '#4f585e',
  textPrimary: '#d3c6aa',
  textSecondary: '#c7c1a6',
  textTertiary: '#b4beb7',
  textMuted: '#a0aaa3',
  textFaint: '#90998e',
  textDisabled: '#56635f',
  accent: '#a7c080',
  accentText: '#a7c080',
  accentGlow: 'rgba(167, 192, 128, 0.2)',
  accentBg: 'rgba(167, 192, 128, 0.15)',
  busy: '#e69875',
  success: '#a7c080',
  error: '#e67e80',
  warning: '#dbbc7f',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.4)',
  scrollbarThumb: 'rgba(211, 198, 170, 0.08)',
  scrollbarHover: 'rgba(211, 198, 170, 0.15)',
  claudeBg: '#232a2e',
  claudeUserBubble: 'rgba(167, 192, 128, 0.12)',
  claudeUserBorder: 'rgba(167, 192, 128, 0.25)',
  claudeDivider: 'rgba(211, 198, 170, 0.05)',
  claudeBorder: 'rgba(211, 198, 170, 0.06)',
  claudeBorderSubtle: 'rgba(211, 198, 170, 0.04)',
  terminal: {
    background: '#2d353b',
    foreground: '#d3c6aa',
    cursor: '#d3c6aa',
    cursorAccent: '#2d353b',
    selectionBackground: 'rgba(167, 192, 128, 0.3)',
    black: '#343f44',
    red: '#e67e80',
    green: '#a7c080',
    yellow: '#dbbc7f',
    blue: '#7fbbb3',
    magenta: '#d699b6',
    cyan: '#83c092',
    white: '#d3c6aa',
    brightBlack: '#859289',
    brightRed: '#e67e80',
    brightGreen: '#a7c080',
    brightYellow: '#dbbc7f',
    brightBlue: '#7fbbb3',
    brightMagenta: '#d699b6',
    brightCyan: '#83c092',
    brightWhite: '#d3c6aa',
  },
};

// ── Kanagawa (inspired by Hokusai's Great Wave) ──

export const kanagawaTheme: Theme = {
  name: 'kanagawa',
  corners: 'soft',
  bgBase: '#1f1f28',
  bgRaised: '#1c1c25',
  bgSurface: '#21212b',
  bgElevated: '#2a2a37',
  bgHeader: '#21212b',
  bgInput: '#16161d',
  bgHover: '#2a2a37',
  bgSelected: '#223249',
  bgTerminal: '#1f1f28',
  border: '#2a2a37',
  borderSubtle: '#222230',
  borderInput: '#363646',
  textPrimary: '#dcd7ba',
  textSecondary: '#c8c093',
  textTertiary: '#a8a48f',
  textMuted: '#939289',
  textFaint: '#86817b',
  textDisabled: '#4a4a57',
  accent: '#7e9cd8',
  accentText: '#7e9cd8',
  accentGlow: 'rgba(126, 156, 216, 0.2)',
  accentBg: 'rgba(126, 156, 216, 0.15)',
  success: '#98bb6c',
  error: '#e46876',
  warning: '#e6c384',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.5)',
  scrollbarThumb: 'rgba(220, 215, 186, 0.08)',
  scrollbarHover: 'rgba(220, 215, 186, 0.15)',
  claudeBg: '#16161d',
  claudeUserBubble: 'rgba(126, 156, 216, 0.12)',
  claudeUserBorder: 'rgba(126, 156, 216, 0.25)',
  claudeDivider: 'rgba(220, 215, 186, 0.05)',
  claudeBorder: 'rgba(220, 215, 186, 0.06)',
  claudeBorderSubtle: 'rgba(220, 215, 186, 0.04)',
  terminal: {
    background: '#1f1f28',
    foreground: '#dcd7ba',
    cursor: '#c8c093',
    cursorAccent: '#1f1f28',
    selectionBackground: 'rgba(45, 79, 103, 0.5)',
    black: '#16161d',
    red: '#e46876',
    green: '#98bb6c',
    yellow: '#e6c384',
    blue: '#7e9cd8',
    magenta: '#957fb8',
    cyan: '#7aa89f',
    white: '#dcd7ba',
    brightBlack: '#727169',
    brightRed: '#ff5d62',
    brightGreen: '#98bb6c',
    brightYellow: '#e6c384',
    brightBlue: '#7fb4ca',
    brightMagenta: '#938aa9',
    brightCyan: '#7aa89f',
    brightWhite: '#dcd7ba',
  },
};

// ── Ayu Dark (crisp, warm-amber accent) ──

export const ayuDarkTheme: Theme = {
  name: 'ayu-dark',
  corners: 'soft',
  bgBase: '#0d1017',
  bgRaised: '#0f131a',
  bgSurface: '#131721',
  bgElevated: '#1a1f29',
  bgHeader: '#131721',
  bgInput: '#0b0e14',
  bgHover: '#1a1f29',
  bgSelected: '#233040',
  bgTerminal: '#0d1017',
  border: '#1f2430',
  borderSubtle: '#1a1f29',
  borderInput: '#2a3038',
  textPrimary: '#bfbdb6',
  textSecondary: '#aca89e',
  textTertiary: '#999790',
  textMuted: '#808694',
  textFaint: '#6f7787',
  textDisabled: '#3a3f49',
  accent: '#e6b450',
  accentText: '#ffcc66',
  accentGlow: 'rgba(230, 180, 80, 0.2)',
  accentBg: 'rgba(230, 180, 80, 0.15)',
  success: '#aad94c',
  error: '#f26d78',
  warning: '#ffb454',
  overlay: 'rgba(0, 0, 0, 0.5)',
  shadow: 'rgba(0, 0, 0, 0.5)',
  scrollbarThumb: 'rgba(191, 189, 182, 0.08)',
  scrollbarHover: 'rgba(191, 189, 182, 0.15)',
  claudeBg: '#0b0e14',
  claudeUserBubble: 'rgba(230, 180, 80, 0.1)',
  claudeUserBorder: 'rgba(230, 180, 80, 0.22)',
  claudeDivider: 'rgba(191, 189, 182, 0.05)',
  claudeBorder: 'rgba(191, 189, 182, 0.06)',
  claudeBorderSubtle: 'rgba(191, 189, 182, 0.04)',
  terminal: {
    background: '#0d1017',
    foreground: '#bfbdb6',
    cursor: '#e6b450',
    cursorAccent: '#0d1017',
    selectionBackground: 'rgba(230, 180, 80, 0.25)',
    black: '#11151c',
    red: '#ea6c73',
    green: '#91b362',
    yellow: '#f9af4f',
    blue: '#53bdfa',
    magenta: '#fae994',
    cyan: '#90e1c6',
    white: '#c7c7c7',
    brightBlack: '#686868',
    brightRed: '#f07178',
    brightGreen: '#c2d94c',
    brightYellow: '#ffb454',
    brightBlue: '#59c2ff',
    brightMagenta: '#ffee99',
    brightCyan: '#95e6cb',
    brightWhite: '#ffffff',
  },
};

// ── Synthwave '84 (neon retro-future) ──

export const synthwaveTheme: Theme = {
  name: 'synthwave',
  corners: 'square',
  bgBase: '#262335',
  bgRaised: '#2a2540',
  bgSurface: '#2d2b40',
  bgElevated: '#34304d',
  bgHeader: '#2d2b40',
  bgInput: '#1e1a2e',
  bgHover: '#34304d',
  bgSelected: '#463465',
  bgTerminal: '#262335',
  border: '#463465',
  borderSubtle: '#34304d',
  borderInput: '#534b76',
  textPrimary: '#f0eff1',
  textSecondary: '#d6d3e0',
  textTertiary: '#be9dd2',
  textMuted: '#8d94c2',
  textFaint: '#8683a4',
  textDisabled: '#534b76',
  accent: '#ff7edb',
  accentText: '#ff7edb',
  accentGlow: 'rgba(255, 126, 219, 0.25)',
  accentBg: 'rgba(255, 126, 219, 0.15)',
  success: '#72f1b8',
  error: '#fe4450',
  warning: '#fede5d',
  overlay: 'rgba(0, 0, 0, 0.55)',
  shadow: 'rgba(0, 0, 0, 0.5)',
  scrollbarThumb: 'rgba(255, 126, 219, 0.12)',
  scrollbarHover: 'rgba(255, 126, 219, 0.25)',
  claudeBg: '#1e1a2e',
  claudeUserBubble: 'rgba(255, 126, 219, 0.12)',
  claudeUserBorder: 'rgba(255, 126, 219, 0.3)',
  claudeDivider: 'rgba(240, 239, 241, 0.05)',
  claudeBorder: 'rgba(240, 239, 241, 0.06)',
  claudeBorderSubtle: 'rgba(240, 239, 241, 0.04)',
  terminal: {
    background: '#262335',
    foreground: '#f0eff1',
    cursor: '#ff7edb',
    cursorAccent: '#262335',
    selectionBackground: 'rgba(255, 126, 219, 0.3)',
    black: '#2a2139',
    red: '#fe4450',
    green: '#72f1b8',
    yellow: '#fede5d',
    blue: '#57c7ff',
    magenta: '#ff7edb',
    cyan: '#36f9f6',
    white: '#f0eff1',
    brightBlack: '#848bbd',
    brightRed: '#fe4450',
    brightGreen: '#72f1b8',
    brightYellow: '#fede5d',
    brightBlue: '#57c7ff',
    brightMagenta: '#ff7edb',
    brightCyan: '#36f9f6',
    brightWhite: '#ffffff',
  },
};

// ── Theme registry ──

export const themes: Record<string, Theme> = {
  dark: darkTheme,
  light: lightTheme,
  midnight: midnightTheme,
  'solarized-dark': solarizedDarkTheme,
  dracula: draculaTheme,
  nord: nordTheme,
  monokai: monokaiTheme,
  'github-dark': githubDarkTheme,
  gruvbox: gruvboxTheme,
  'tokyo-night': tokyoNightTheme,
  'one-dark': oneDarkTheme,
  'rose-pine': rosePineTheme,
  cyberpunk: cyberpunkTheme,
  'catppuccin-mocha': catppuccinMochaTheme,
  everforest: everforestTheme,
  kanagawa: kanagawaTheme,
  'ayu-dark': ayuDarkTheme,
  synthwave: synthwaveTheme,
};

// ── Custom themes ──

/** Editable token set for a custom theme: every flat Theme color plus a
 *  partial terminal palette. `name` / `corners` are managed separately
 *  (display name lives on CustomTheme; corners via the cornerStyle override). */
export type ThemeColors = Omit<Partial<Theme>, 'name' | 'corners' | 'terminal'> & {
  terminal?: Partial<TerminalTheme>;
};

/** A user-made theme, persisted in config.ui.customThemes. Colors are fully
 *  resolved from the base theme at creation time, so resolution stays trivial
 *  and later changes to built-ins never restyle saved themes. `base` is kept
 *  for backfilling tokens added after the theme was saved, and as the
 *  fallback when the theme is deleted. */
export interface CustomTheme {
  /** Display name (the id stays stable across renames). */
  name: string;
  /** Built-in theme id this was forked from (fallback + token backfill). */
  base?: string;
  colors: ThemeColors;
}

export type CustomThemes = Record<string, CustomTheme>;

export const CUSTOM_THEME_PREFIX = 'custom:';

export function isCustomThemeId(id: string | undefined): boolean {
  return !!id && id.startsWith(CUSTOM_THEME_PREFIX);
}

/** Namespaced id for a new custom theme ('custom:<slug>'), de-duplicated
 *  against the ids already in use so two themes can share a display name. */
export function newCustomThemeId(name: string, existing?: CustomThemes): string {
  const slug =
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '') || 'theme';
  let id = CUSTOM_THEME_PREFIX + slug;
  let n = 2;
  while (existing && id in existing) id = `${CUSTOM_THEME_PREFIX}${slug}-${n++}`;
  return id;
}

/**
 * Single theme resolver — use this instead of reading the `themes` registry
 * directly, so custom themes work everywhere a built-in does (CSS vars,
 * corners, light/dark detection, terminal palette, webview injection, the
 * Windows title bar). Unknown ids fall back to dark.
 */
export function resolveTheme(id: string | undefined, customThemes?: CustomThemes): Theme {
  const custom = id ? customThemes?.[id] : undefined;
  if (custom) {
    const base = themes[custom.base ?? ''] ?? darkTheme;
    // Saved themes are stored fully resolved, but spread over the base anyway
    // so a theme saved before a token existed still gets every value.
    return {
      ...base,
      ...custom.colors,
      terminal: { ...base.terminal, ...(custom.colors?.terminal ?? {}) },
      name: id as string,
    };
  }
  return themes[id ?? ''] ?? darkTheme;
}

/** Every color token of a resolved theme as a plain serializable map — the
 *  payload stored for a custom theme (name/corners are managed separately). */
export function themeColorsOf(theme: Theme): ThemeColors {
  const { name: _name, corners: _corners, terminal, ...colors } = theme;
  return { ...colors, terminal: { ...terminal } };
}

/** Human label for a theme id: the custom theme's display name, or the
 *  built-in id prettified ("tokyo-night" → "Tokyo Night"). */
export function themeDisplayName(id: string, customThemes?: CustomThemes): string {
  const custom = customThemes?.[id];
  if (custom) return custom.name;
  return (id || 'dark')
    .split('-')
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(' ');
}

// ── Corner styles ──

/** Curated default corner style per theme, so each theme keeps its character
 *  (neon/retro themes feel sharper, soft themes feel rounder). The user can
 *  override this in Settings; switching themes re-adopts the theme default. */
const THEME_CORNERS: Record<string, CornerStyle> = {
  light: 'rounded',
  midnight: 'rounded',
  'rose-pine': 'rounded',
  cyberpunk: 'square',
  monokai: 'square',
};

// Backfill each theme's `corners` from the curated map (default 'soft').
for (const [name, t] of Object.entries(themes)) {
  if (t.corners === undefined) t.corners = THEME_CORNERS[name] ?? 'soft';
}

/** Effective corner style: explicit user override wins, else the theme's own. */
export function cornersOf(theme: Theme, override?: string): CornerStyle {
  if (override === 'rounded' || override === 'soft' || override === 'square') return override;
  return theme.corners ?? 'soft';
}

/** Radius scale → CSS custom properties. Square collapses every radius to 0
 *  (circular dots use an explicit 50% and are unaffected). */
export function radiusVarsFor(corners: CornerStyle): Record<string, string> {
  const scale: Record<CornerStyle, [number, number, number]> = {
    rounded: [8, 13, 18],
    soft: [5, 8, 12],
    square: [0, 0, 0],
  };
  const [sm, md, lg] = scale[corners] ?? scale.soft;
  return {
    '--wks-radius-sm': `${sm}px`,
    '--wks-radius-md': `${md}px`,
    '--wks-radius-lg': `${lg}px`,
    '--wks-radius-pill': corners === 'square' ? '0px' : '999px',
  };
}

// ── Apply theme as CSS custom properties ──

/** Full token → CSS custom property map. Single source of truth — used for
 *  the app's own documentElement AND injected into plugin webviews. */
function parseRgb(color: string): [number, number, number] | null {
  const c = color.trim();
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(c);
  if (hex) {
    let h = hex[1];
    if (h.length === 3)
      h = h
        .split('')
        .map((ch) => ch + ch)
        .join('');
    return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
  }
  const m = /rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/.exec(c);
  if (m) return [+m[1], +m[2], +m[3]];
  return null;
}

/** Re-express any theme color (hex / rgb / rgba) as rgba() with a new alpha.
 *  Used to derive the translucent "glass" surfaces from each theme's palette. */
export function toRgba(color: string, alpha: number): string {
  const rgb = parseRgb(color);
  return rgb ? `rgba(${rgb[0]}, ${rgb[1]}, ${rgb[2]}, ${alpha})` : color;
}

/** Normalize any theme color to #rrggbb (for <input type="color"> swatches). */
export function toHex(color: string): string {
  const rgb = parseRgb(color);
  if (!rgb) return '#000000';
  return (
    '#' +
    rgb
      .map((c) =>
        Math.max(0, Math.min(255, Math.round(c)))
          .toString(16)
          .padStart(2, '0'),
      )
      .join('')
  );
}

export function cssVarsOf(theme: Theme): Record<string, string> {
  const light = isLightTheme(theme);
  return {
    // Frosted-glass surfaces — derived from the theme so blur/translucency
    // keeps each theme's hue. Consumed with backdrop-filter: blur().
    '--wks-glass-bg': toRgba(theme.bgSurface, light ? 0.72 : 0.6),
    '--wks-glass-strong': toRgba(theme.bgElevated, light ? 0.86 : 0.78),
    '--wks-glass-border': light ? 'rgba(0, 0, 0, 0.14)' : 'rgba(255, 255, 255, 0.16)',
    '--wks-glass-highlight': light ? 'rgba(255, 255, 255, 0.9)' : 'rgba(255, 255, 255, 0.10)',
    '--wks-glass-blur': '18px',
    '--wks-glass-shadow': theme.shadow,
    '--wks-glass-tint': toRgba(theme.accent, light ? 0.05 : 0.08),
    '--wks-bg-base': theme.bgBase,
    '--wks-bg-raised': theme.bgRaised,
    '--wks-bg-surface': theme.bgSurface,
    '--wks-bg-elevated': theme.bgElevated,
    '--wks-bg-header': theme.bgHeader,
    '--wks-bg-input': theme.bgInput,
    '--wks-bg-hover': theme.bgHover,
    '--wks-bg-selected': theme.bgSelected,
    '--wks-bg-terminal': theme.bgTerminal,
    '--wks-border': theme.border,
    '--wks-border-subtle': theme.borderSubtle,
    '--wks-border-input': theme.borderInput,
    '--wks-text-primary': theme.textPrimary,
    '--wks-text-secondary': theme.textSecondary,
    '--wks-text-tertiary': theme.textTertiary,
    '--wks-text-muted': theme.textMuted,
    '--wks-text-faint': theme.textFaint,
    '--wks-text-disabled': theme.textDisabled,
    '--wks-accent': theme.accent,
    '--wks-accent-text': theme.accentText,
    '--wks-accent-glow': theme.accentGlow,
    '--wks-accent-bg': theme.accentBg,
    '--wks-border-active': theme.borderActive ?? theme.accent,
    '--wks-success': theme.success,
    '--wks-error': theme.error,
    '--wks-warning': theme.warning,
    '--wks-purple': theme.purple ?? '#c084fc',
    '--wks-busy': theme.busy ?? theme.accent,
    '--wks-overlay': theme.overlay,
    '--wks-shadow': theme.shadow,
    '--wks-scrollbar-thumb': theme.scrollbarThumb,
    '--wks-scrollbar-hover': theme.scrollbarHover,
    '--wks-claude-bg': theme.claudeBg,
    '--wks-claude-user-bubble': theme.claudeUserBubble,
    '--wks-claude-user-border': theme.claudeUserBorder,
    '--wks-claude-divider': theme.claudeDivider,
    '--wks-claude-border': theme.claudeBorder,
    '--wks-claude-border-subtle': theme.claudeBorderSubtle,
  };
}

/** Colors for the Windows native caption buttons (titleBarOverlay). Electron
 *  needs an opaque color, but the navbar is translucent "glass-strong" over the
 *  base — so we flatten that glass onto the base to get the perceived navbar
 *  color, and use the theme's secondary text for the min/max/close glyphs. This
 *  is what lets the native buttons blend into the themed title bar. */
export function titleBarOverlayOf(theme: Theme): { color: string; symbolColor: string } {
  const alpha = isLightTheme(theme) ? 0.86 : 0.78; // mirrors --wks-glass-strong
  const fg = parseRgb(theme.bgElevated);
  const bg = parseRgb(theme.bgBase);
  if (fg && bg) {
    const blend = fg.map((c, i) => Math.round(c * alpha + bg[i] * (1 - alpha)));
    const color =
      '#' + blend.map((c) => Math.max(0, Math.min(255, c)).toString(16).padStart(2, '0')).join('');
    return { color, symbolColor: toHex(theme.textSecondary) };
  }
  return { color: toHex(theme.bgElevated), symbolColor: toHex(theme.textSecondary) };
}

/** Perceived lightness of the theme, from its base background. Drives the
 *  guest page's `color-scheme` so native controls/scrollbars match. */
export function isLightTheme(theme: Theme): boolean {
  const m = /^#([0-9a-f]{6})$/i.exec(theme.bgBase.trim());
  let r: number, g: number, b: number;
  if (m) {
    r = parseInt(m[1].slice(0, 2), 16);
    g = parseInt(m[1].slice(2, 4), 16);
    b = parseInt(m[1].slice(4, 6), 16);
  } else {
    const rgb = /rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/.exec(theme.bgBase);
    if (!rgb) return false; // unknown format — assume dark
    r = +rgb[1];
    g = +rgb[2];
    b = +rgb[3];
  }
  return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255 > 0.5;
}

export function applyTheme(theme: Theme): void {
  const s = document.documentElement.style;
  for (const [prop, value] of Object.entries(cssVarsOf(theme))) {
    s.setProperty(prop, value);
  }
  // Native controls (select popups, scrollbars) follow the document's
  // color-scheme, not our tokens — without this a dark theme still gets the
  // OS-light white dropdown list.
  s.colorScheme = isLightTheme(theme) ? 'light' : 'dark';
}

/** Apply the radius scale for the resolved corner style as CSS custom props. */
export function applyCorners(corners: CornerStyle): void {
  const s = document.documentElement.style;
  for (const [prop, value] of Object.entries(radiusVarsFor(corners))) {
    s.setProperty(prop, value);
  }
}
