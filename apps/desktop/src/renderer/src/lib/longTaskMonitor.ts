/**
 * Renderer-thread stall detection — the other half of `main/lib/stallDiagnostics.ts`.
 *
 * Every pane lives in this one renderer, so a long synchronous task here looks
 * exactly like a main-process freeze from the user's side: nothing repaints and
 * no agent's output moves. The two monitors exist to tell those cases apart —
 * if a freeze shows up here and not in the main-process log, it's a React
 * render/layout problem, not blocking IPC.
 *
 * `longtask` entries carry no useful attribution (the spec deliberately reports
 * only "unknown" for same-origin work), so we pair each report with the last
 * marked UI event. That's what turns "something blocked for 3s" into "opening
 * the spawn dialog blocked for 3s".
 */

/** Tasks at or above this are user-visible jank worth reporting. */
const LONG_TASK_MS = 200;
/** A marker older than this is stale — the stall probably isn't related. */
const MARK_FRESH_MS = 5_000;

let lastMark: { label: string; at: number } | null = null;

/**
 * Note that a UI action just happened, so a stall reported in the next few
 * moments can name it. Call this from anything suspected of being expensive —
 * opening a heavy dialog, switching tabs, expanding a large diff.
 */
export function markUiEvent(label: string): void {
  lastMark = { label, at: performance.now() };
}

/** Begin watching for long tasks. Returns a disposer; no-op where unsupported. */
export function startLongTaskMonitor(): () => void {
  // Safari and older Chromium lack longtask; the app must not care.
  if (typeof PerformanceObserver === 'undefined') return () => {};
  if (!PerformanceObserver.supportedEntryTypes?.includes('longtask')) return () => {};

  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) {
      if (entry.duration < LONG_TASK_MS) continue;
      const mark =
        lastMark && entry.startTime - lastMark.at < MARK_FRESH_MS
          ? ` after ui:${lastMark.label}`
          : '';
      console.warn(`[stall] renderer blocked ~${Math.round(entry.duration)}ms${mark}`);
    }
  });
  try {
    observer.observe({ entryTypes: ['longtask'] });
  } catch {
    return () => {}; // observing threw — diagnostics are best-effort
  }
  return () => observer.disconnect();
}
