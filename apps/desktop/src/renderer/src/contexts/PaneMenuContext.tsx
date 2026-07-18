import React, { createContext, useContext } from 'react';
import type { PluginPane } from '../types/plugin';
import type { PaneMenuEntry } from '../lib/paneMenu';

/**
 * Shares the resolved pane-creation menu (built-ins + plugins, per the
 * `ui.paneMenu` setting) with the components that render it — the in-pane
 * "Split into…" button (Pane) and the "+" new-tab dropdown (NavBar) — without
 * threading the plugin list + open-plugin handler through the deep pane tree.
 *
 * Computed once in App (it already holds the plugin list and the open-plugin
 * handler) and provided at the root, mirroring how ConfigContext is consumed.
 */
export interface PaneMenuContextValue {
  entries: PaneMenuEntry[];
  onOpenPlugin: (pane: PluginPane) => void;
}

const EMPTY: PaneMenuContextValue = { entries: [], onOpenPlugin: () => {} };

export const PaneMenuContext = createContext<PaneMenuContextValue>(EMPTY);

/** The resolved pane-creation menu entries + the plugin-open handler. Falls back
 *  to an empty menu when used outside a provider (e.g. isolated tests). */
export function usePaneMenu(): PaneMenuContextValue {
  return useContext(PaneMenuContext);
}

export function PaneMenuProvider({
  value,
  children,
}: {
  value: PaneMenuContextValue;
  children: React.ReactNode;
}) {
  return <PaneMenuContext.Provider value={value}>{children}</PaneMenuContext.Provider>;
}
