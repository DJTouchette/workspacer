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
function refresh(): Promise<void> {
  if (inFlight) return inFlight;
  const fetcher = owner;
  if (typeof fetcher !== 'function') return Promise.resolve();
  const epoch = generation;
  inFlight = Promise.resolve()
    .then(() => fetcher())
    .then((report) => {
      if (epoch !== generation) return;
      if (!report) {
        failed();
        return;
      }
      cached = report;
      publish();
    })
    .catch(() => {
      if (epoch === generation) failed();
    })
    .finally(() => {
      if (epoch === generation) inFlight = null;
    });
  return inFlight;
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
  cached = null;
  owner = undefined;
  inFlight = null;
  if (timer) clearInterval(timer);
  if (clock) clearInterval(clock);
  timer = clock = null;
  subscribers.clear();
}
