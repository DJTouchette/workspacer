/**
 * usePluginHotkeys — binds hotkeys contributed by hub plugins.
 *
 * Matching goes through the canonical comboMatcher (lib/shortcuts.ts) so a
 * plugin manifest may use the platform-neutral `mod` token like every other
 * binding. The old inline matcher split the combo without resolving `mod`,
 * which made any `mod+…` hotkey silently dead (`parts.includes('ctrl')` was
 * false while the real event carried ctrlKey) — the exact bug that killed the
 * library-picker binding, which now lives in useKeyboardNav's executeAction
 * instead of a second listener here.
 */
import { useEffect, useRef } from 'react';
import type { PluginPane, PluginHotkey } from '../types/plugin';
import { comboMatcher } from '../lib/shortcuts';

interface UsePluginHotkeysOptions {
  pluginHotkeys: PluginHotkey[];
  pluginPanes: PluginPane[];
  handleOpenPlugin: (pane: PluginPane) => void;
}

export function usePluginHotkeys({
  pluginHotkeys,
  pluginPanes,
  handleOpenPlugin,
}: UsePluginHotkeysOptions): void {
  // Stash unstable callbacks in refs so the keydown listener is registered once
  // and never torn down/re-added on parent re-renders (mirrors useUiCommands pattern).
  const handlersRef = useRef({ handleOpenPlugin, pluginHotkeys, pluginPanes });
  handlersRef.current = { handleOpenPlugin, pluginHotkeys, pluginPanes };

  // Bind plugin-contributed hotkeys: open-pane:<type> or emit:<eventType>.
  // Stable dep array — re-registration is not needed because the ref is always current.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const {
        pluginHotkeys: hotkeys,
        pluginPanes: panes,
        handleOpenPlugin: openPlugin,
      } = handlersRef.current;
      for (const h of hotkeys) {
        if (!comboMatcher(h.combo)(e)) continue;
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
}
