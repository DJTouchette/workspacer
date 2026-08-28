import { useCallback, useEffect, useState } from 'react';
import type { ProviderDetection } from '../lib/providerAvailability';

/**
 * Live-ish "which agent CLIs are installed" for every provider picker.
 *
 * One shared store rather than a fetch per picker: Spawn, Ask the Fleet,
 * Handoff and two Settings sections all ask the same question, and the answer
 * is a PATH scan in main (`provider:checkAll` → checkAllProviders). The store
 * is a stale-while-revalidate cache:
 *
 *  - Every mount (= a picker opening) calls `refresh()`. Inside the TTL that
 *    returns the cached answer, so opening the same dialog twice in a row is
 *    free; past it, a rescan runs and the picker updates in place. A user who
 *    installs `codex` mid-session sees it on the next dialog open, not after a
 *    restart — which is why this is not a boot-time constant.
 *  - A config change invalidates immediately: `agents.binaries` overrides feed
 *    the scan (a custom path that exists = installed), so a changed override
 *    must not be answered from a cache keyed on the old one.
 *
 * `detection` is null until the first answer lands; helpers in
 * lib/providerAvailability treat null (and any provider missing from the list)
 * as `unknown` and keep it VISIBLE, so a picker never hides a harness on the
 * strength of an answer it doesn't have yet.
 */

/** Rescan window. Binaries don't move often; a picker reopened inside this
 *  window reuses the last scan instead of re-walking PATH. */
const TTL_MS = 5000;

let cached: ProviderDetection[] | null = null;
let cachedAt = 0;
let inflight: Promise<void> | null = null;
const listeners = new Set<(d: ProviderDetection[] | null) => void>();
let configHookInstalled = false;

function emit() {
  for (const fn of listeners) fn(cached);
}

/**
 * Re-run detection now, ignoring this module's TTL. Concurrent callers share
 * one call.
 *
 * `force` is passed through to main, which keeps its OWN short TTL in front of
 * the PATH walk (agentProviders.checkAllProvidersCached) — so a plain refetch
 * can still be answered from there. The explicit "re-check" button is exactly
 * the case that must not be: the user installed the CLI in a terminal seconds
 * ago and is asking us to look again, so it forces a rescan on both sides.
 */
function fetchNow(force = false): Promise<void> {
  if (inflight) return inflight;
  const api = window.electronAPI?.providerCheckAll;
  if (!api) return Promise.resolve();
  inflight = Promise.resolve(api(force))
    .then((list) => {
      // An empty array is not "nothing is installed" — every host that answers
      // this returns one row per provider. Treat it as no answer (unknown)
      // rather than hiding every harness in the app.
      cached = Array.isArray(list) && list.length ? (list as ProviderDetection[]) : null;
      cachedAt = Date.now();
      emit();
    })
    .catch(() => {
      // Leave the last good answer in place; unknown beats wrongly-hidden.
    })
    .finally(() => {
      inflight = null;
    });
  return inflight;
}

/**
 * Drop the cache and rescan — used when config (and so the overrides) change.
 *
 * Deliberately NOT forced: main keys its cache on the `agents.binaries` map, so
 * the one config field that changes this answer already defeats it, while every
 * other save (and the Settings panel saves on each keystroke-blur) is served
 * from the cache instead of re-walking PATH.
 */
export function invalidateProviderDetection(): void {
  cachedAt = 0;
  if (listeners.size) void fetchNow();
}

function ensureConfigHook() {
  if (configHookInstalled) return;
  configHookInstalled = true;
  window.electronAPI?.onConfigChanged?.(() => invalidateProviderDetection());
}

/** Test seam: forget everything this module remembers. */
export function __resetProviderDetectionCache(): void {
  cached = null;
  cachedAt = 0;
  inflight = null;
  configHookInstalled = false;
}

export function useProviderDetection(): {
  detection: ProviderDetection[] | null;
  /** Force a rescan (a "check again" button, or after writing an override). */
  refresh: () => void;
} {
  const [detection, setDetection] = useState<ProviderDetection[] | null>(cached);

  useEffect(() => {
    listeners.add(setDetection);
    ensureConfigHook();
    // Mounting is a picker opening: answer from cache if it's fresh, else
    // rescan. Either way the current value renders immediately.
    setDetection(cached);
    if (Date.now() - cachedAt > TTL_MS) void fetchNow();
    return () => {
      listeners.delete(setDetection);
    };
  }, []);

  const refresh = useCallback(() => {
    // Forced: this is a person asking us to look again, usually right after
    // installing the CLI, so neither TTL may answer for us.
    void fetchNow(true);
  }, []);

  return { detection, refresh };
}
