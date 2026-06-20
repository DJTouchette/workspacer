/**
 * usePluginHotkeys — binds hotkeys contributed by hub plugins and the
 * library-picker shortcut.
 *
 * Extracted verbatim from App.tsx; all logic is unchanged.
 */
import { useEffect, useRef } from 'react';
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
  // Stash unstable callbacks in refs so the keydown listener is registered once
  // and never torn down/re-added on parent re-renders (mirrors useUiCommands pattern).
  const handlersRef = useRef({ handleOpenPlugin, pluginHotkeys, pluginPanes });
  handlersRef.current = { handleOpenPlugin, pluginHotkeys, pluginPanes };

  // Bind plugin-contributed hotkeys: open-pane:<type> or emit:<eventType>.
  // Stable dep array — re-registration is not needed because the ref is always current.
  useEffect(() => {
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
      const { pluginHotkeys: hotkeys, pluginPanes: panes, handleOpenPlugin: openPlugin } = handlersRef.current;
      for (const h of hotkeys) {
        if (!matches(h.combo, e)) continue;
        e.preventDefault();
        if (h.command.startsWith('open-pane:')) {
          const type = h.command.slice('open-pane:'.length);
          const pane = panes.find((p) => p.type === type);
          if (pane) openPlugin(pane);
        } else if (h.command.startsWith('emit:')) {
          window.electronAPI.hubPublish?.({ type: h.command.slice('emit:'.length), data: {} });
        }
        return;
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Library quick-picker hotkey (default ctrl+shift+l): opens the palette
  // restricted to prompts & skills.
  const libraryRef = useRef({ libraryPickerCombo, openLibraryPicker });
  libraryRef.current = { libraryPickerCombo, openLibraryPicker };

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const { libraryPickerCombo: combo, openLibraryPicker: open } = libraryRef.current;
      if (!combo) return;
      const parts = combo.toLowerCase().split('+');
      const key = parts[parts.length - 1];
      if (e.ctrlKey === parts.includes('ctrl') && e.shiftKey === parts.includes('shift')
        && e.altKey === parts.includes('alt') && e.metaKey === parts.includes('meta')
        && e.key.toLowerCase() === key) {
        e.preventDefault();
        open();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
}
