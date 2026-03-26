import { useEffect, useMemo } from 'react';
import { useConfig } from './useConfig';
import { themes, darkTheme, applyTheme } from '../themes';
import type { Theme, TerminalTheme } from '../themes';

export function useTheme(): { theme: Theme; terminalTheme: TerminalTheme } {
  const { config } = useConfig();
  const themeName = config.ui.theme || 'dark';

  const theme = useMemo(() => themes[themeName] ?? darkTheme, [themeName]);

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  return { theme, terminalTheme: theme.terminal };
}
