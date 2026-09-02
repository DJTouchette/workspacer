/**
 * The Overview usage card's SESSION-FREE source.
 *
 * Every rate-limit surface in the app renders from a live session's status
 * line, so on a cold start — the app opened, nothing running — they render
 * nothing, no matter how good the daemon's own reading is. claudemon answers
 * `GET /usage/report` from disk and its account poller with zero sessions;
 * this hook is how the renderer asks.
 *
 * ONE fetch for the whole window, not one per card. The Overview draws a card
 * per provider and per Claude account, and they all want the same document —
 * so the timer, the in-flight request and the last answer live here at module
 * scope and every card subscribes. Same reason the status-line cache below the
 * card is module-level: it must survive a pane remount.
 *
 * The cadence is deliberately slack. claudemon re-polls an idle account every
 * 15 minutes (account_usage.rs IDLE_INTERVAL_SECS), so anything faster asks a
 * question whose answer cannot have changed; a minute is the floor, and this
 * sits well above it. The first fetch is immediate, because "accurate at boot"
 * is the whole point.
 */
import { useEffect, useState } from 'react';
import type { UsageReportWire } from '../../../main/shared/usageReport';

/** Well above the daemon's own 15-minute idle cadence's floor and far above
 *  the 60s the renderer is allowed to poll at. */
export const USAGE_REPORT_REFRESH_MS = 5 * 60 * 1000;

let cached: UsageReportWire | null = null;
let inFlight: Promise<void> | null = null;
let timer: ReturnType<typeof setInterval> | null = null;
const subscribers = new Set<(r: UsageReportWire | null) => void>();

function refresh(): Promise<void> {
  if (inFlight) return inFlight;
  const fetcher = window.electronAPI?.usageReport;
  // Absent on a transport that cannot reach the daemon (an older preload, a
  // web build predating the stub). Nothing to poll, so don't.
  if (typeof fetcher !== 'function') return Promise.resolve();
  inFlight = Promise.resolve(fetcher())
    .then((report) => {
      // A failed read must not retract a good reading: null means "could not
      // ask", and the last answer is still the best thing anyone has.
      if (!report) return;
      cached = report;
      for (const fn of subscribers) fn(cached);
    })
    .catch(() => {})
    .finally(() => {
      inFlight = null;
    });
  return inFlight;
}

/** The daemon's latest usage report, refreshed while any card is mounted. */
export function useUsageReport(): UsageReportWire | null {
  const [report, setReport] = useState<UsageReportWire | null>(cached);
  useEffect(() => {
    subscribers.add(setReport);
    if (!timer) timer = setInterval(() => void refresh(), USAGE_REPORT_REFRESH_MS);
    void refresh().then(() => setReport(cached));
    return () => {
      subscribers.delete(setReport);
      if (subscribers.size === 0 && timer) {
        clearInterval(timer);
        timer = null;
      }
    };
  }, []);
  return report;
}

/** Test seam: drop the module-level cache/timer between renders. */
export function __resetUsageReportCache(): void {
  cached = null;
  inFlight = null;
  if (timer) clearInterval(timer);
  timer = null;
  subscribers.clear();
}
