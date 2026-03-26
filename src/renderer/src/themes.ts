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

export interface Theme {
  name: string;

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

  // Status
  success: string;
  error: string;
  warning: string;

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
  textFaint: 'rgb(120, 120, 135)',
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
  textMuted: '#71717a',
  textFaint: '#a1a1aa',
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
  textFaint: '#585b70',
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

// ── Theme registry ──

export const themes: Record<string, Theme> = {
  dark: darkTheme,
  light: lightTheme,
  midnight: midnightTheme,
};

// ── Apply theme as CSS custom properties ──

export function applyTheme(theme: Theme): void {
  const s = document.documentElement.style;
  s.setProperty('--wks-bg-base', theme.bgBase);
  s.setProperty('--wks-bg-raised', theme.bgRaised);
  s.setProperty('--wks-bg-surface', theme.bgSurface);
  s.setProperty('--wks-bg-elevated', theme.bgElevated);
  s.setProperty('--wks-bg-header', theme.bgHeader);
  s.setProperty('--wks-bg-input', theme.bgInput);
  s.setProperty('--wks-bg-hover', theme.bgHover);
  s.setProperty('--wks-bg-selected', theme.bgSelected);
  s.setProperty('--wks-bg-terminal', theme.bgTerminal);
  s.setProperty('--wks-border', theme.border);
  s.setProperty('--wks-border-subtle', theme.borderSubtle);
  s.setProperty('--wks-border-input', theme.borderInput);
  s.setProperty('--wks-text-primary', theme.textPrimary);
  s.setProperty('--wks-text-secondary', theme.textSecondary);
  s.setProperty('--wks-text-tertiary', theme.textTertiary);
  s.setProperty('--wks-text-muted', theme.textMuted);
  s.setProperty('--wks-text-faint', theme.textFaint);
  s.setProperty('--wks-text-disabled', theme.textDisabled);
  s.setProperty('--wks-accent', theme.accent);
  s.setProperty('--wks-accent-text', theme.accentText);
  s.setProperty('--wks-accent-glow', theme.accentGlow);
  s.setProperty('--wks-accent-bg', theme.accentBg);
  s.setProperty('--wks-success', theme.success);
  s.setProperty('--wks-error', theme.error);
  s.setProperty('--wks-warning', theme.warning);
  s.setProperty('--wks-overlay', theme.overlay);
  s.setProperty('--wks-shadow', theme.shadow);
  s.setProperty('--wks-scrollbar-thumb', theme.scrollbarThumb);
  s.setProperty('--wks-scrollbar-hover', theme.scrollbarHover);
  s.setProperty('--wks-claude-bg', theme.claudeBg);
  s.setProperty('--wks-claude-user-bubble', theme.claudeUserBubble);
  s.setProperty('--wks-claude-user-border', theme.claudeUserBorder);
  s.setProperty('--wks-claude-divider', theme.claudeDivider);
  s.setProperty('--wks-claude-border', theme.claudeBorder);
  s.setProperty('--wks-claude-border-subtle', theme.claudeBorderSubtle);
}
