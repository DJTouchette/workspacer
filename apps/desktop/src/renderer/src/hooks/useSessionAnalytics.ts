/**
 * The desktop's own session-history analytics, read over the IPC that already
 * existed and that nothing called.
 *
 * `analytics:summary` and `analytics:recent` have been wired end to end — main
 * handler, preload, web-backend passthrough, typed on `electronAPI` — since
 * before the analytics pane that consumed them was deleted. The store behind
 * them is `~/.config/workspacer/workspacer.db`, which on a real machine holds
 * hundreds of sessions and five figures of recorded spend. This hook is the
 * only consumer; the Overview's lifetime tile and the History pane's per-row
 * and header figures both read it, so they can never disagree.
 *
 * THREE STATES, NOT TWO. Every caller has to be able to tell apart:
 *   - a figure we have          → `summary.totals`
 *   - a figure nobody recorded  → an absent per-row value (see `recorded`)
 *   - a store we cannot reach   → `unavailable`
 * A headless hub answers these methods with a well-formed ALL-ZERO stub
 * carrying `unavailable: "headless"` (see brain/handlers.go), so "$0.00 across
 * 0 sessions" is a shape this hook receives routinely and must never render as
 * a measurement. `summary` is null whenever that marker is present.
 */
import { useCallback, useEffect, useState } from 'react';
import type { AnalyticsSummary, SessionHistoryRecord } from '../types/analytics';
import { useHubReconnect } from './useHubReconnect';

/** How many history rows the History pane pulls. Comfortably past the ~750
 *  rows a heavy machine has accumulated; the read is one local SQLite query. */
export const RECENT_LIMIT = 2000;

/** One session's recorded figures, with zero read as absence. */
export interface RecordedRow {
  costUSD?: number;
  billedTokens?: number;
  toolCalls?: number;
  startedAt?: string;
  model?: string;
}

export interface SessionAnalytics {
  /** Lifetime totals + breakdowns, or null while loading / when unreachable. */
  summary: AnalyticsSummary | null;
  /** sessionId → recorded figures, for rows the daemon list can't cover
   *  (transcript-only sessions the daemon forgot). Empty until loaded. */
  bySessionId: Record<string, RecordedRow>;
  /** How many history rows carry no usage at all — the honest denominator for
   *  "this total covers N of M sessions". Counted over the rows actually READ
   *  (see {@link RECENT_LIMIT}), which is why {@link unrecordedComplete} rides
   *  beside it. */
  unrecordedSessions: number;
  /** False when the row read hit its cap, so `unrecordedSessions` is a floor
   *  rather than a count. `summary.totals.sessions` is the whole store; the
   *  two have different denominators the moment this goes false, and a surface
   *  that prints them side by side has to say so. */
  unrecordedComplete: boolean;
  loading: boolean;
  /** Why the store could not be read, or null when it could. */
  unavailable: string | null;
  refresh: () => void;
}

/** `session_history` stores cost/tokens as `DEFAULT 0` with no NULL, so a zero
 *  is indistinguishable from never-written — and a third of the rows on a real
 *  machine are exactly that. Report it as absent rather than as a measured 0. */
function recorded(n: number | undefined | null): number | undefined {
  return n ? n : undefined;
}

/** Index history rows by session id, dropping the zeros (see {@link recorded}). */
export function indexHistoryRows(rows: SessionHistoryRecord[]): {
  bySessionId: Record<string, RecordedRow>;
  unrecordedSessions: number;
} {
  const bySessionId: Record<string, RecordedRow> = {};
  let unrecordedSessions = 0;
  for (const r of rows) {
    if (!r?.sessionId) continue;
    const costUSD = recorded(r.costUSD);
    const billedTokens = recorded((r.inputTokens ?? 0) + (r.outputTokens ?? 0));
    if (costUSD === undefined && billedTokens === undefined) unrecordedSessions++;
    bySessionId[r.sessionId] = {
      ...(costUSD !== undefined && { costUSD }),
      ...(billedTokens !== undefined && { billedTokens }),
      ...(recorded(r.toolCalls) !== undefined && { toolCalls: r.toolCalls }),
      ...(r.startedAt && { startedAt: r.startedAt }),
      ...(r.model && { model: r.model }),
    };
  }
  return { bySessionId, unrecordedSessions };
}

/** The `unavailable` marker a headless brain stub carries, if any. */
function stubMarker(summary: unknown): string | null {
  const marker = (summary as { unavailable?: unknown } | null)?.unavailable;
  return typeof marker === 'string' && marker ? marker : null;
}

export function useSessionAnalytics(enabled = true): SessionAnalytics {
  const [summary, setSummary] = useState<AnalyticsSummary | null>(null);
  const [bySessionId, setBySessionId] = useState<Record<string, RecordedRow>>({});
  const [unrecordedSessions, setUnrecorded] = useState(0);
  const [unrecordedComplete, setUnrecordedComplete] = useState(true);
  const [loading, setLoading] = useState(enabled);
  const [unavailable, setUnavailable] = useState<string | null>(null);
  const [nonce, setNonce] = useState(0);

  const refresh = useCallback(() => setNonce((n) => n + 1), []);
  // The hub bus replays subscriptions but never one-shot fetches, so a web tab
  // whose socket dropped holds a stale summary until this fires.
  useHubReconnect(refresh);

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    setLoading(true);
    const api = window.electronAPI;
    Promise.all([
      api.analyticsSummary?.() ?? Promise.reject(new Error('analytics unavailable')),
      api.analyticsRecent?.(RECENT_LIMIT) ?? Promise.resolve([]),
    ])
      .then(([sum, rows]) => {
        if (!alive) return;
        const marker = stubMarker(sum);
        if (marker) {
          // A well-formed all-zero answer from a store that has no data to
          // give. Reporting it as "$0.00 across 0 sessions" would be a
          // measurement we never took.
          setSummary(null);
          setBySessionId({});
          setUnrecorded(0);
          setUnrecordedComplete(true);
          setUnavailable(marker);
        } else {
          setSummary(sum ?? null);
          const list = Array.isArray(rows) ? rows : [];
          const idx = indexHistoryRows(list);
          setBySessionId(idx.bySessionId);
          setUnrecorded(idx.unrecordedSessions);
          setUnrecordedComplete(list.length < RECENT_LIMIT);
          setUnavailable(null);
        }
        setLoading(false);
      })
      .catch((err: unknown) => {
        if (!alive) return;
        // Drop the rows too. A failed REFRESH would otherwise leave per-row
        // figures on screen under a header that says the store is unreachable
        // — two surfaces disagreeing about whether we know anything.
        setSummary(null);
        setBySessionId({});
        setUnrecorded(0);
        setUnrecordedComplete(true);
        setUnavailable(err instanceof Error ? err.message : String(err));
        setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [enabled, nonce]);

  return {
    summary,
    bySessionId,
    unrecordedSessions,
    unrecordedComplete,
    loading,
    unavailable,
    refresh,
  };
}
