import { useEffect, useRef } from 'react';
import type { TabConfig } from '../types/pane';
import { markVisible, selectPanesToHibernate, selectPanesToWake } from '../lib/hibernation';

/** How often the idle sweep runs. Coarse on purpose — this is a memory
 *  reclaim, not something the user should be able to feel. */
export const HIBERNATION_SWEEP_MS = 15_000;

export interface BrowserHibernationOptions {
  tabs: TabConfig[];
  activeTabId: string | undefined;
  /** Idle budget in ms. Zero or negative disables the sweep. */
  hibernateAfter: number;
  /** False while the session is still loading — hibernating a pane mid-restore
   *  would tear down a webview the layout is in the middle of building. */
  enabled: boolean;
  hibernatePane: (tabId: string, paneId: string) => void;
  wakePane: (tabId: string, paneId: string) => void;
}

/**
 * Reclaim webviews from browser panes the user isn't looking at, and wake them
 * again when their tab comes back.
 *
 * Owns three things: the record of when each pane was last on screen, the
 * periodic sweep, and the wake-on-focus. The decisions themselves live in
 * `lib/hibernation` — this hook is the wiring, so the policy stays testable
 * without timers.
 */
export function useBrowserHibernation({
  tabs,
  activeTabId,
  hibernateAfter,
  enabled,
  hibernatePane,
  wakePane,
}: BrowserHibernationOptions): void {
  // paneId → epoch ms last on screen. A ref, not state: the sweep reads it on
  // a timer and nothing renders off it.
  const lastVisibleRef = useRef<Record<string, number>>({});

  useEffect(() => {
    lastVisibleRef.current = markVisible(lastVisibleRef.current, tabs, activeTabId, Date.now());
  }, [activeTabId, tabs]);

  useEffect(() => {
    for (const { tabId, paneId } of selectPanesToWake(tabs, activeTabId)) {
      wakePane(tabId, paneId);
    }
  }, [activeTabId, tabs, wakePane]);

  useEffect(() => {
    if (hibernateAfter <= 0 || !enabled) return;
    const interval = setInterval(() => {
      const due = selectPanesToHibernate({
        tabs,
        activeTabId,
        lastVisible: lastVisibleRef.current,
        now: Date.now(),
        hibernateAfter,
      });
      for (const { tabId, paneId } of due) hibernatePane(tabId, paneId);
    }, HIBERNATION_SWEEP_MS);
    return () => clearInterval(interval);
  }, [tabs, activeTabId, hibernateAfter, hibernatePane, enabled]);
}
