import React, { createContext, useContext } from 'react';
import { usePlugins } from '../hooks/usePlugins';
import type { PluginManifest, PluginPane, PluginWidget, PluginHotkey } from '../types/plugin';

/**
 * One shared subscription to the hub's plugin list.
 *
 * usePlugins fetches /plugins, retries a booting hub with backoff, and holds a
 * bus subscription for `plugin.*` events — so it wants exactly one caller. It
 * had one (App), until widgets needed the same data from the inspector rail,
 * which is several layers down past ScrollContainer and ClaudePane. Rather than
 * prop-drill through both, or pay a second fetch + subscription per mounted
 * rail, the hook runs once here and everything reads the context.
 *
 * Mirrors ConfigContext deliberately — same shape, same reason to exist.
 */
export interface PluginsContextValue {
  plugins: PluginManifest[];
  panes: PluginPane[];
  widgets: PluginWidget[];
  hotkeys: PluginHotkey[];
  /** Origin a browser frames hub-served plugin UI from, or `''` for "the base
   *  the manifest was stamped with" (see lib/pluginOrigin). A pane restored from
   *  the shared layout needs it to rehome a URL another client resolved. */
  frameOrigin: string;
}

/**
 * Defaults to empty rather than null, and reading it outside a provider is not
 * an error — unlike ConfigContext, which throws.
 *
 * The difference is that "no plugins" is a legitimate runtime state: a fresh
 * install contributes nothing, and every consumer already renders that case. A
 * component whose only use of this is an optional contribution list shouldn't
 * have to be wrapped to be rendered — the inspector rail is mounted bare by its
 * own tests, and panes are lazy-loaded into trees this provider doesn't own.
 */
const EMPTY: PluginsContextValue = {
  plugins: [],
  panes: [],
  widgets: [],
  hotkeys: [],
  frameOrigin: '',
};

// Exported so a test or a harness can provide a fabricated plugin set. The
// PROVIDER owns the real fetch (usePlugins), so injecting through it is the
// only way to render plugin-dependent UI without a live hub.
export const PluginsContext = createContext<PluginsContextValue>(EMPTY);

export function PluginsProvider({ children }: { children: React.ReactNode }) {
  const value = usePlugins();
  return <PluginsContext.Provider value={value}>{children}</PluginsContext.Provider>;
}

export function usePluginsContext(): PluginsContextValue {
  return useContext(PluginsContext);
}
