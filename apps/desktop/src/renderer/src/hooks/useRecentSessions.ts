/**
 * Poll the daemon's full session list (via main) for the sidebar's "Recent"
 * section. Cheap — one IPC + one daemon GET per tick — and deliberately dumb:
 * consumers filter against the live layout themselves (lib/recentSessionFilter)
 * so the list reacts instantly to spawns/terminates between ticks.
 *
 * Besides the 60s poll, a `requestRecentSessionsRefresh()` event (fired after
 * a terminate) triggers a refetch burst. The burst retries a few times because
 * the daemon only flips the dying session to its resumable Stopped row once
 * teardown finishes — a single immediate fetch would usually still see it live
 * and the row would miss this poll cycle entirely.
 */
import { useEffect, useState } from 'react';
import type { RecentAgentSession } from '../../../main/shared/ipcTypes';
import { RECENT_SESSIONS_REFRESH_EVENT } from '../lib/watchBus';

const POLL_MS = 60_000;
/** Refetch delays after a refresh request, covering daemon teardown latency. */
const BURST_DELAYS_MS = [0, 2_000, 5_000, 10_000];

export function useRecentSessions(enabled = true): RecentAgentSession[] {
  const [sessions, setSessions] = useState<RecentAgentSession[]>([]);

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    const tick = () => {
      // Optional-chained: absent on older preloads and the web polyfill.
      window.electronAPI
        .listRecentAgentSessions?.()
        .then((list) => {
          if (alive) setSessions(list);
        })
        .catch(() => {});
    };
    tick();
    const interval = setInterval(tick, POLL_MS);
    let burstTimers: ReturnType<typeof setTimeout>[] = [];
    const onRefresh = () => {
      for (const t of burstTimers) clearTimeout(t);
      burstTimers = BURST_DELAYS_MS.map((ms) => setTimeout(tick, ms));
    };
    window.addEventListener(RECENT_SESSIONS_REFRESH_EVENT, onRefresh);
    return () => {
      alive = false;
      clearInterval(interval);
      for (const t of burstTimers) clearTimeout(t);
      window.removeEventListener(RECENT_SESSIONS_REFRESH_EVENT, onRefresh);
    };
  }, [enabled]);

  return sessions;
}
