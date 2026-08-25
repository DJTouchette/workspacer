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
import { useEffect, useRef, useState } from 'react';
import type { RecentAgentSession } from '../../../main/shared/ipcTypes';
import { RECENT_SESSIONS_REFRESH_EVENT } from '../lib/watchBus';
import { postNotification } from '../lib/notificationBus';

const POLL_MS = 60_000;
/** Refetch delays after a refresh request, covering daemon teardown latency. */
const BURST_DELAYS_MS = [0, 2_000, 5_000, 10_000];

export interface RecentSessionsResult {
  sessions: RecentAgentSession[];
  /**
   * Why the daemon's list could not be read, or null when it could.
   *
   * "I cannot see your history" and "you have no history" are different
   * answers and used to collapse into the same one: `sessions.recent` has no
   * provider on a headless hub (the desktop main process registers it, because
   * it enriches the daemon rows from its own history DB), the web backend
   * swallowed the rejection into `[]`, and the Sessions pane concluded "No past
   * sessions — everything is already in your workspace". Consumers render this
   * instead of that sentence.
   */
  unavailable: string | null;
}

export function useRecentSessions(enabled = true): RecentSessionsResult {
  const [sessions, setSessions] = useState<RecentAgentSession[]>([]);
  const [unavailable, setUnavailable] = useState<string | null>(null);
  /** One notification per mount — this polls every 60s and a nag is not news. */
  const toldRef = useRef(false);

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    const tick = () => {
      // Optional-chained: absent on older preloads and the web polyfill.
      window.electronAPI
        .listRecentAgentSessions?.()
        .then((list) => {
          if (!alive) return;
          setSessions(list);
          setUnavailable(null);
        })
        .catch((err: unknown) => {
          if (!alive) return;
          const reason = err instanceof Error ? err.message : String(err);
          setUnavailable(reason);
          // The sidebar's RECENT section just goes quiet, and a user who never
          // opens the Sessions pane would never learn why.
          if (!toldRef.current) {
            toldRef.current = true;
            postNotification({
              level: 'warn',
              title: 'Session history is unavailable',
              body: `${reason}. Agents already in your workspace are unaffected; past sessions cannot be listed or resumed from here.`,
              source: 'workspacer',
            });
          }
        });
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

  return { sessions, unavailable };
}
