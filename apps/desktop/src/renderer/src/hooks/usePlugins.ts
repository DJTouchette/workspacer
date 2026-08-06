import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { PluginManifest, PluginPane, PluginWidget, PluginHotkey } from '../types/plugin';
import { pluginPaneURL, pluginWidgetURL, widgetSizesOf } from '../types/plugin';

/**
 * Loads the hub's plugin list and keeps it fresh: refetches whenever a
 * `plugin.*` event crosses the bus (loaded / unloaded). Exposes the panes,
 * widgets and hotkeys plugins contribute, ready to inject into the UI.
 *
 * Boot race, hardened: on a cold hub start (first launch after an app update —
 * a normal relaunch adopts the still-running hub) the mount-time fetch can land
 * before the hub is up, and the boot-time `plugin.loaded` events fire before
 * our bus subscription attaches. That combination left the palette permanently
 * plugin-less until a reinstall emitted a fresh event. So an unreachable hub
 * (list === null) is retried with backoff, and every hub `connected` status —
 * including the first — triggers a refetch.
 */
export function usePlugins(): {
  plugins: PluginManifest[];
  panes: PluginPane[];
  widgets: PluginWidget[];
  hotkeys: PluginHotkey[];
} {
  const [plugins, setPlugins] = useState<PluginManifest[]>([]);

  // Sequence counter so only the latest in-flight fetch's result is applied.
  const fetchSeqRef = useRef(0);
  // Debounce timer for bursts of plugin.* events.
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Retry state for an unreachable hub (null result / rejected fetch).
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const retryDelayRef = useRef(1000);

  const refresh = useCallback(function refresh() {
    const seq = ++fetchSeqRef.current;
    const scheduleRetry = () => {
      if (retryTimerRef.current) return; // one pending retry at a time
      const delay = retryDelayRef.current;
      retryDelayRef.current = Math.min(delay * 2, 15_000);
      retryTimerRef.current = setTimeout(() => {
        retryTimerRef.current = null;
        refresh();
      }, delay);
    };
    window.electronAPI
      .listHubPlugins?.()
      .then((list) => {
        if (seq !== fetchSeqRef.current) return; // superseded by a later fetch
        if (list === null) {
          scheduleRetry(); // hub unreachable (booting) — not an empty registry
          return;
        }
        retryDelayRef.current = 1000;
        setPlugins(Array.isArray(list) ? list : []);
      })
      .catch(scheduleRetry);
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
    // Refetch whenever the hub reports connected — the mount fetch may have
    // raced a cold hub start, and boot-time plugin.loaded events predate our
    // subscription. Deliberately fires on the FIRST connect too.
    const offStatus = window.electronAPI.onHubStatus?.(({ connected }) => {
      if (connected) refresh();
    });
    return () => {
      off?.();
      offStatus?.();
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
        debounceTimerRef.current = null;
      }
      if (retryTimerRef.current) {
        clearTimeout(retryTimerRef.current);
        retryTimerRef.current = null;
      }
    };
  }, [refresh]);

  // Disabled plugins stay in `plugins` (so the manager pane can show + re-enable
  // them) but contribute no panes, widgets or hotkeys to the rest of the UI.
  const panes = useMemo<PluginPane[]>(
    () =>
      plugins
        .filter((p) => !p.disabled)
        .flatMap((p) =>
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

  const widgets = useMemo<PluginWidget[]>(
    () =>
      plugins
        .filter((p) => !p.disabled)
        .flatMap((p) =>
          (p.widgets ?? []).map((w) => ({
            pluginId: p.id,
            pluginName: p.name || p.id,
            id: w.id,
            title: w.title,
            icon: w.icon,
            url: pluginWidgetURL(p, w),
            sizes: widgetSizesOf(w),
            busToken: p.busToken,
          })),
        ),
    [plugins],
  );

  const hotkeys = useMemo<PluginHotkey[]>(
    () =>
      plugins
        .filter((p) => !p.disabled)
        .flatMap((p) =>
          (p.hotkeys ?? []).map((h) => ({
            pluginId: p.id,
            id: h.id,
            combo: h.default,
            command: h.command,
          })),
        ),
    [plugins],
  );

  return { plugins, panes, widgets, hotkeys };
}
