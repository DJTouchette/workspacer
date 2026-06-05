/**
 * usePluginHotkeys — binds hotkeys contributed by hub plugins and the
 * library-picker shortcut.
 *
 * Extracted verbatim from App.tsx; all logic is unchanged.
 */
import { useEffect } from 'react';
import type { PluginPane, PluginHotkey } from '../types/plugin';

interface UsePluginHotkeysOptions {
  pluginHotkeys: PluginHotkey[];
  pluginPanes: PluginPane[];
  handleOpenPlugin: (pane: PluginPane) => void;
  libraryPickerCombo: string | undefined;
  openLibraryPicker: () => void;
}

export function usePluginHotkeys({
  pluginHotkeys,
  pluginPanes,
  handleOpenPlugin,
  libraryPickerCombo,
  openLibraryPicker,
}: UsePluginHotkeysOptions): void {
  // Bind plugin-contributed hotkeys: open-pane:<type> or emit:<eventType>.
  useEffect(() => {
    if (pluginHotkeys.length === 0) return;
    const matches = (combo: string, e: KeyboardEvent): boolean => {
      const parts = combo.toLowerCase().split('+');
      const key = parts[parts.length - 1];
      return e.ctrlKey === parts.includes('ctrl')
        && e.shiftKey === parts.includes('shift')
        && e.altKey === parts.includes('alt')
        && e.metaKey === parts.includes('meta')
        && e.key.toLowerCase() === key;
    };
    const handler = (e: KeyboardEvent) => {
      for (const h of pluginHotkeys) {
        if (!matches(h.combo, e)) continue;
        e.preventDefault();
        if (h.command.startsWith('open-pane:')) {
          const type = h.command.slice('open-pane:'.length);
          const pane = pluginPanes.find((p) => p.type === type);
          if (pane) handleOpenPlugin(pane);
        } else if (h.command.startsWith('emit:')) {
          window.electronAPI.hubPublish?.({ type: h.command.slice('emit:'.length), data: {} });
        }
        return;
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [pluginHotkeys, pluginPanes, handleOpenPlugin]);

  // Library quick-picker hotkey (default ctrl+shift+l): opens the palette
  // restricted to prompts & skills.
  useEffect(() => {
    const combo = libraryPickerCombo;
    if (!combo) return;
    const parts = combo.toLowerCase().split('+');
    const key = parts[parts.length - 1];
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey === parts.includes('ctrl') && e.shiftKey === parts.includes('shift')
        && e.altKey === parts.includes('alt') && e.metaKey === parts.includes('meta')
        && e.key.toLowerCase() === key) {
        e.preventDefault();
        openLibraryPicker();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [libraryPickerCombo, openLibraryPicker]);
}
