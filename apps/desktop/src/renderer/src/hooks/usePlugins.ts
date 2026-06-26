import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { PluginManifest, PluginPane, PluginHotkey } from '../types/plugin';
import { pluginPaneURL } from '../types/plugin';

/**
 * Loads the hub's plugin list and keeps it fresh: refetches whenever a
 * `plugin.*` event crosses the bus (loaded / unloaded). Exposes the panes and
 * hotkeys plugins contribute, ready to inject into the UI.
 */
export function usePlugins(): { plugins: PluginManifest[]; panes: PluginPane[]; hotkeys: PluginHotkey[] } {
  const [plugins, setPlugins] = useState<PluginManifest[]>([]);

  // Sequence counter so only the latest in-flight fetch's result is applied.
  const fetchSeqRef = useRef(0);
  // Debounce timer for bursts of plugin.* events.
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refresh = useCallback(() => {
    const seq = ++fetchSeqRef.current;
    window.electronAPI.listHubPlugins?.()
      .then((list) => {
        if (seq !== fetchSeqRef.current) return; // superseded by a later fetch
        setPlugins(Array.isArray(list) ? list : []);
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const off = window.electronAPI.onHubEvent?.((ev) => {
      if (!ev.type?.startsWith('plugin.')) return;
      // Trailing debounce: coalesce a burst of plugin.* events into one refresh.
      if (debounceTimerRef.current) clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = setTimeout(() => {
        debounceTimerRef.current = null;
        refresh();
      }, 150);
    });
    return () => {
      off?.();
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
        debounceTimerRef.current = null;
      }
    };
  }, [refresh]);

  // Disabled plugins stay in `plugins` (so the manager pane can show + re-enable
  // them) but contribute no panes or hotkeys to the rest of the UI.
  const panes = useMemo<PluginPane[]>(
    () => plugins.filter((p) => !p.disabled).flatMap((p) =>
      (p.panes ?? []).map((pane) => ({
        pluginId: p.id,
        type: pane.type,
        title: pane.title,
        icon: pane.icon,
        url: pluginPaneURL(p, pane),
        scope: pane.scope ?? 'both',
        busToken: p.busToken,
      })),
    ),
    [plugins],
  );

  const hotkeys = useMemo<PluginHotkey[]>(
    () => plugins.filter((p) => !p.disabled).flatMap((p) =>
      (p.hotkeys ?? []).map((h) => ({
        pluginId: p.id,
        id: h.id,
        combo: h.default,
        command: h.command,
      })),
    ),
    [plugins],
  );

  return { plugins, panes, hotkeys };
}
