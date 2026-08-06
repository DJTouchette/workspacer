/**
 * Browser-pane hibernation policy.
 *
 * A browser pane in a tab you aren't looking at keeps a whole webview alive.
 * After `hibernateAfter` ms out of sight we tear the webview down and leave a
 * placeholder; bringing the tab back wakes it. This module is the decision —
 * *which* panes are due — kept separate from the timer and the state writes so
 * it can be reasoned about without a running React tree.
 *
 * The rules are subtler than "old enough":
 *  - only `browser` panes are reclaimed. `plugin` panes hold a webview too (they
 *    ARE a BrowserPane — see PluginPane) and the teardown plumbing would work on
 *    them, but a plugin webview can hold unsaved state the host knows nothing
 *    about: the bundled editor plugin tracks a dirty buffer with no autosave, so
 *    reclaiming it after five minutes out of view would silently discard edits.
 *    Widening this needs a way for a plugin to say it's safe (a manifest opt-in,
 *    or an event it can persist on) — not just a wider type test. Widgets are
 *    unaffected: they live in the inspector rail rather than in a tab, so the
 *    sweep below never sees them and closing the rail unmounts them outright;
 *  - a pane in the active tab is on screen by definition;
 *  - a pane with no recorded sighting is left alone, because "never seen" is
 *    indistinguishable from "seen before we started tracking" and hibernating
 *    on that reading would collapse panes the user just restored;
 *  - already-hibernated panes are skipped so the caller isn't handed no-ops.
 */

import type { PaneConfig, TabConfig } from '../types/pane';

/** A pane the policy has picked out, addressed by the tab that owns it. */
export interface PaneRef {
  tabId: string;
  paneId: string;
}

export interface HibernationInput {
  tabs: TabConfig[];
  /** The tab on screen. Its panes are never hibernated. */
  activeTabId: string | undefined;
  /** paneId → epoch ms the pane was last on screen. */
  lastVisible: Record<string, number>;
  now: number;
  /** Idle budget in ms. Zero or negative disables hibernation entirely. */
  hibernateAfter: number;
}

/**
 * The browser panes that have been out of sight longer than the budget.
 *
 * Returns them in tab order, then pane order, so a caller applying them in
 * sequence produces a deterministic set of state writes.
 */
export function selectPanesToHibernate({
  tabs,
  activeTabId,
  lastVisible,
  now,
  hibernateAfter,
}: HibernationInput): PaneRef[] {
  if (hibernateAfter <= 0) return [];
  const due: PaneRef[] = [];
  for (const tab of tabs) {
    if (tab.id === activeTabId) continue;
    for (const pane of tab.panes) {
      if (!isHibernatable(pane)) continue;
      const lastSeen = lastVisible[pane.id] ?? 0;
      // `> 0` keeps a never-sighted pane alive; see the module note.
      if (lastSeen > 0 && now - lastSeen > hibernateAfter) {
        due.push({ tabId: tab.id, paneId: pane.id });
      }
    }
  }
  return due;
}

/**
 * A pane worth reclaiming: holds a webview, is safe to tear down, and isn't
 * already torn down.
 *
 * Deliberately excludes `plugin` panes despite them holding a webview — see the
 * module note. This is a data-loss guard, not an oversight.
 */
export function isHibernatable(pane: PaneConfig): boolean {
  return pane.type === 'browser' && !pane.hibernated;
}

/**
 * The hibernated panes in the tab that just became active — they need waking
 * before the user sees a placeholder where their page was.
 */
export function selectPanesToWake(tabs: TabConfig[], activeTabId: string | undefined): PaneRef[] {
  if (!activeTabId) return [];
  const tab = tabs.find((t) => t.id === activeTabId);
  if (!tab) return [];
  return tab.panes.filter((p) => p.hibernated).map((p) => ({ tabId: tab.id, paneId: p.id }));
}

/**
 * Stamp every pane in the active tab as seen at `now`.
 *
 * Returns a new record rather than mutating, so a caller holding the previous
 * map in a ref can decide whether the write is worth it. Panes that aren't on
 * screen keep whatever sighting they had.
 */
export function markVisible(
  lastVisible: Record<string, number>,
  tabs: TabConfig[],
  activeTabId: string | undefined,
  now: number,
): Record<string, number> {
  if (!activeTabId) return lastVisible;
  const tab = tabs.find((t) => t.id === activeTabId);
  if (!tab) return lastVisible;
  const next = { ...lastVisible };
  for (const pane of tab.panes) next[pane.id] = now;
  return next;
}
