/** One sampled report and request per backend, shared by every Overview card.
 * Network refresh is bounded to once per minute. The shared local clock only
 * rerenders reset/deadline guards; it never extrapolates the hub's pace math.
 */
import { useEffect, useState } from 'react';
import type { UsageReportWire } from '../../../main/shared/usageReport';

export const USAGE_REPORT_REFRESH_MS = 60_000;
let cached: UsageReportWire | null = null;
let owner: typeof window.electronAPI.usageReport | undefined;
let inFlight: Promise<void> | null = null;
// Two counters, not one. `generation` is BACKEND identity (a replaced backend
// invalidates the cache entirely); `issued`/`applied` order replies from the
// SAME backend, which the forced refresh below needs: a settings save starts a
// second fetch while the poll's is still open, and the older reply must not
// land on top of the newer one just because it finished last.
let issued = 0;
let applied = 0;
let timer: ReturnType<typeof setInterval> | null = null;
let clock: ReturnType<typeof setInterval> | null = null;
let generation = 0;
const subscribers = new Set<(r: UsageReportWire | null) => void>();

function publish() {
  if (cached) cached = { ...cached };
  for (const fn of subscribers) fn(cached);
}
function failed() {
  if (cached) cached = { ...cached, transport_stale: true };
  publish();
}
function refresh(force = false): Promise<void> {
  if (inFlight && !force) return inFlight;
  const fetcher = owner;
  if (typeof fetcher !== 'function') return Promise.resolve();
  const epoch = generation;
  const seq = ++issued;
  const run = Promise.resolve()
    .then(() => fetcher())
    .then((report) => {
      if (epoch !== generation || seq < applied) return;
      applied = seq;
      if (!report) {
        failed();
        return;
      }
      cached = report;
      publish();
    })
    .catch(() => {
      if (epoch === generation && seq >= applied) {
        applied = seq;
        failed();
      }
    })
    .finally(() => {
      if (epoch === generation && inFlight === run) inFlight = null;
    });
  inFlight = run;
  return run;
}

/**
 * Re-read the report NOW, ignoring the once-a-minute poll and any fetch already
 * in flight.
 *
 * This exists for one caller: the Settings "Usage schedule" control. The
 * schedule lives hub-side and changes what `usage.report` computes, so a save
 * that left the Overview showing the previous week's curve for up to a minute
 * would read as a setting that did not take. Forcing past `inFlight` is the
 * point — a poll that started BEFORE the save would otherwise be the reply the
 * user sees, which is why the sequence guard above exists rather than a plain
 * "latest wins".
 */
export function refreshUsageReport(): Promise<void> {
  return refresh(true);
}

export function useUsageReport(): UsageReportWire | null {
  const fetcher = window.electronAPI?.usageReport;
  const [report, setReport] = useState<UsageReportWire | null>(owner === fetcher ? cached : null);
  useEffect(() => {
    if (owner !== fetcher) {
      generation++;
      cached = null;
      inFlight = null;
      owner = fetcher;
      publish();
    }
    const firstSubscriber = subscribers.size === 0;
    subscribers.add(setReport);
    setReport(cached);
    if (!timer) timer = setInterval(() => void refresh(), USAGE_REPORT_REFRESH_MS);
    if (!clock) clock = setInterval(publish, 1000);
    if (firstSubscriber) void refresh();
    return () => {
      subscribers.delete(setReport);
      if (!subscribers.size) {
        if (timer) clearInterval(timer);
        if (clock) clearInterval(clock);
        timer = clock = null;
      }
    };
  }, [fetcher]);
  return owner === fetcher ? report : null;
}

export function __resetUsageReportCache(): void {
  generation++;
  issued = applied = 0;
  cached = null;
  owner = undefined;
  inFlight = null;
  if (timer) clearInterval(timer);
  if (clock) clearInterval(clock);
  timer = clock = null;
  subscribers.clear();
}
