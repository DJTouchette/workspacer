import { useEffect, useMemo } from 'react';
import { useConfig } from './useConfig';
import {
  resolveTheme,
  applyTheme,
  applyCorners,
  cornersOf,
  titleBarOverlayOf,
  DEFAULT_THEME,
} from '../themes';
import type { Theme, TerminalTheme } from '../themes';

export function useTheme(): { theme: Theme; terminalTheme: TerminalTheme } {
  const { config } = useConfig();
  const themeName = config.ui.theme || DEFAULT_THEME;
  const cornerOverride = config.ui.cornerStyle || '';
  const customThemes = config.ui.customThemes;

  const theme = useMemo(() => resolveTheme(themeName, customThemes), [themeName, customThemes]);
  const corners = useMemo(() => cornersOf(theme, cornerOverride), [theme, cornerOverride]);
  // Focused-pane border: user override wins, else the theme's own.
  const borderColor = config.ui.borderColor || theme.borderActive || theme.accent;

  useEffect(() => {
    applyTheme(theme);
    // Repaint the Windows native caption buttons to match the themed title bar.
    if (window.electronAPI?.platform === 'win32') {
      const { color, symbolColor } = titleBarOverlayOf(theme);
      window.electronAPI.setTitleBarOverlay(color, symbolColor);
    }
  }, [theme]);

  useEffect(() => {
    applyCorners(corners);
  }, [corners]);

  useEffect(() => {
    document.documentElement.style.setProperty('--wks-border-active', borderColor);
  }, [borderColor]);

  return { theme, terminalTheme: theme.terminal };
}
