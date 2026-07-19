/**
 * Poll the daemon's full session list (via main) for the sidebar's "Recent"
 * section. Cheap — one IPC + one daemon GET per tick — and deliberately dumb:
 * consumers filter against the live layout themselves (lib/recentSessionFilter)
 * so the list reacts instantly to spawns/terminates between ticks.
 */
import { useEffect, useState } from 'react';
import type { RecentAgentSession } from '../../../main/shared/ipcTypes';

const POLL_MS = 60_000;

export function useRecentSessions(enabled = true): RecentAgentSession[] {
  const [sessions, setSessions] = useState<RecentAgentSession[]>([]);

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    const tick = () => {
      window.electronAPI
        .listRecentAgentSessions()
        .then((list) => {
          if (alive) setSessions(list);
        })
        .catch(() => {});
    };
    tick();
    const interval = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(interval);
    };
  }, [enabled]);

  return sessions;
}
