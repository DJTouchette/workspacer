import { useCallback, useEffect, useMemo, useState } from 'react';
import type { PluginManifest, PluginPane, PluginHotkey } from '../types/plugin';
import { pluginPaneURL } from '../types/plugin';

/**
 * Loads the hub's plugin list and keeps it fresh: refetches whenever a
 * `plugin.*` event crosses the bus (loaded / unloaded). Exposes the panes and
 * hotkeys plugins contribute, ready to inject into the UI.
 */
export function usePlugins(): { plugins: PluginManifest[]; panes: PluginPane[]; hotkeys: PluginHotkey[] } {
  const [plugins, setPlugins] = useState<PluginManifest[]>([]);

  const refresh = useCallback(() => {
    window.electronAPI.listHubPlugins?.()
      .then((list) => setPlugins(Array.isArray(list) ? list : []))
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const off = window.electronAPI.onHubEvent?.((ev) => {
      if (ev.type?.startsWith('plugin.')) refresh();
    });
    return () => off?.();
  }, [refresh]);

  const panes = useMemo<PluginPane[]>(
    () => plugins.flatMap((p) =>
      (p.panes ?? []).map((pane) => ({
        pluginId: p.id,
        type: pane.type,
        title: pane.title,
        icon: pane.icon,
        url: pluginPaneURL(p, pane),
        scope: pane.scope ?? 'both',
      })),
    ),
    [plugins],
  );

  const hotkeys = useMemo<PluginHotkey[]>(
    () => plugins.flatMap((p) =>
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
