/**
 * useLayoutSync — makes the workspace layout mirror across clients, tmux-style.
 *
 * claudemon owns the live sessions/PTYs; the hub owns the *window manager*
 * state — which agent cards exist, their tabs/panes, the active tab. This hook
 * is the renderer's half of that contract:
 *
 *   • hydrate — on startup, read the hub's layout document. If it already holds
 *     a layout (another client — the desktop, or a prior web session — seeded
 *     it), adopt it and skip the local session picker. If it's empty, report
 *     `empty` so the normal session-restore runs and then seeds the hub.
 *   • apply   — when the hub broadcasts `layout.changed` from another client,
 *     replace local state with it.
 *   • push    — when local state changes (a tab opened, a pane split, the
 *     active tab switched), write it back to the hub (debounced), which
 *     re-broadcasts so every other client converges.
 *
 * Convergence is last-writer-wins (see the hub's layout package). Echo loops are
 * broken by content, not just version: the projection we last sent/received is
 * remembered, and an incoming document equal to it is acknowledged (version
 * bumped) but not re-applied — so our own broadcast never bounces back as a
 * spurious local update.
 *
 * The reducer stays in `useAgentManager`; this only moves bytes in and out, so
 * there is exactly one place that knows how to mutate a layout.
 */
import { useCallback, useEffect, useRef } from 'react';
import type { AgentWorkspace } from '../types/pane';
import type { LayoutDoc } from '../types/electron';
import { useHubReconnect } from './useHubReconnect';

/** The slice of app state that is shared across clients. */
interface SharedLayout {
  agents: AgentWorkspace[];
  activeAgentId: string;
}

export type HydrationResult = 'pending' | 'adopted' | 'empty';

interface UseLayoutSyncOptions {
  agents: AgentWorkspace[];
  activeAgentId: string;
  /** Replace local layout state wholesale (same path session-restore uses).
   *  Returns the *normalized* layout it stored (dedupe + global-injection +
   *  active-id resolution applied) so the echo-breaker can mark against it. */
  loadAgentsFromSession: (
    agents: AgentWorkspace[],
    activeAgentId: string,
  ) => { agents: AgentWorkspace[]; activeAgentId: string } | void;
  sessionPhase: 'loading' | 'picker' | 'active';
  setSessionPhase: (phase: 'loading' | 'picker' | 'active') => void;
  /** Don't touch the hub until config has loaded (mirrors session lifecycle). */
  enabled: boolean;
  /** Reports the outcome of the initial hub read so App can gate session restore. */
  onHydration: (result: HydrationResult) => void;
}

/** Stable projection used for both sending and echo-detection. Built identically
 *  on every client so a round-tripped document compares equal to what we sent. */
function project(agents: AgentWorkspace[], activeAgentId: string): string {
  return JSON.stringify({ agents, activeAgentId });
}

/** A layout document counts as "seeded" only if it carries real agent state —
 *  a lone Overview workspace (or nothing) means no client has populated it yet. */
function hasRealLayout(data: SharedLayout | null): boolean {
  if (!data || !Array.isArray(data.agents)) return false;
  return data.agents.some((a) => !a.global);
}

const PUSH_DEBOUNCE_MS = 250;

export function useLayoutSync({
  agents,
  activeAgentId,
  loadAgentsFromSession,
  sessionPhase,
  setSessionPhase,
  enabled,
  onHydration,
}: UseLayoutSyncOptions): void {
  // Highest document version we've accounted for (applied or acknowledged).
  const appliedVersionRef = useRef<number>(-1);
  // Projection of the layout we last sent or received — the echo-breaker.
  const lastSyncedRef = useRef<string | null>(null);
  const hydratedRef = useRef(false);
  const pushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Always-current copies so the debounced timer callback reads the latest
  // state rather than the stale closure from the render that scheduled it.
  const agentsRef = useRef<AgentWorkspace[]>(agents);
  const activeAgentIdRef = useRef<string>(activeAgentId);
  agentsRef.current = agents;
  activeAgentIdRef.current = activeAgentId;

  // ── hydrate + subscribe ─────────────────────────────────────────────────
  useEffect(() => {
    if (!enabled || hydratedRef.current) return;
    hydratedRef.current = true;

    let cancelled = false;

    window.electronAPI
      .layoutGet()
      .then((doc: LayoutDoc) => {
        if (cancelled) return;
        const data = doc?.data as SharedLayout | null;
        const version = doc?.version ?? -1;
        // A live `layout.changed` broadcast can land and apply a newer document
        // before this initial read resolves (the read captured an older
        // snapshot). Such a read is stale: never regress the applied version,
        // and don't let the older snapshot clobber the layout the broadcast
        // already applied.
        const stale = version < appliedVersionRef.current;
        if (version > appliedVersionRef.current) appliedVersionRef.current = version;
        if (stale) {
          // A newer layout was already adopted via broadcast — we're mirroring.
          setSessionPhase('active');
          onHydration('adopted');
        } else if (hasRealLayout(data) && data) {
          // Another client already populated the layout — adopt it and skip the
          // local picker so this client comes up mirroring, not blank. Mark the
          // echo-breaker against the *normalized* result loadAgentsFromSession
          // produced, not the raw doc, so the adopted layout doesn't immediately
          // bounce back to the hub as a spurious "local change".
          const norm = loadAgentsFromSession(data.agents, data.activeAgentId);
          lastSyncedRef.current = norm
            ? project(norm.agents, norm.activeAgentId)
            : project(data.agents, data.activeAgentId);
          setSessionPhase('active');
          onHydration('adopted');
        } else {
          onHydration('empty');
        }
      })
      .catch(() => {
        // Hub unreachable — fall back to local-only behavior.
        if (!cancelled) onHydration('empty');
      });

    const unsub = window.electronAPI.onLayoutChanged((doc: LayoutDoc) => {
      if (!doc || doc.version <= appliedVersionRef.current) return;
      appliedVersionRef.current = doc.version;
      const data = doc.data as SharedLayout | null;
      if (!data || !Array.isArray(data.agents)) return;
      const incoming = project(data.agents, data.activeAgentId);
      if (incoming === lastSyncedRef.current) return; // our own echo / identical
      const norm = loadAgentsFromSession(data.agents, data.activeAgentId);
      // Mark against the normalized result so the post-apply push effect sees
      // local state as unchanged (no bounce-back to the hub).
      lastSyncedRef.current = norm ? project(norm.agents, norm.activeAgentId) : incoming;
    });

    return () => {
      cancelled = true;
      unsub();
    };
  }, [enabled, loadAgentsFromSession, setSessionPhase, onHydration]);

  // On reconnect, re-pull the authoritative doc: we may have missed
  // `layout.changed` broadcasts while the socket was down. Reuses the same
  // version + echo guards as the live `layout.changed` path so only a genuinely
  // newer document is adopted (last-writer-wins; the hub owns the doc).
  const resyncOnReconnect = useCallback(() => {
    if (!enabled) return;
    window.electronAPI
      .layoutGet()
      .then((doc: LayoutDoc) => {
        if (!doc || doc.version <= appliedVersionRef.current) return;
        appliedVersionRef.current = doc.version;
        const data = doc.data as SharedLayout | null;
        if (!hasRealLayout(data) || !data) return;
        const incoming = project(data.agents, data.activeAgentId);
        if (incoming === lastSyncedRef.current) return; // our own echo / identical
        const norm = loadAgentsFromSession(data.agents, data.activeAgentId);
        lastSyncedRef.current = norm ? project(norm.agents, norm.activeAgentId) : incoming;
      })
      .catch(() => { /* hub still unreachable — the bus will retry */ });
  }, [enabled, loadAgentsFromSession]);
  useHubReconnect(resyncOnReconnect);

  // ── push local changes ──────────────────────────────────────────────────
  useEffect(() => {
    if (!enabled || sessionPhase !== 'active') return;
    const json = project(agents, activeAgentId);
    if (json === lastSyncedRef.current) return; // nothing new vs. last sync
    // Mark optimistically *now* so the broadcast we're about to cause is
    // recognized as our own echo when it comes back.
    lastSyncedRef.current = json;

    if (pushTimerRef.current) clearTimeout(pushTimerRef.current);
    pushTimerRef.current = setTimeout(() => {
      // Read from refs so the debounced push always uses the *latest* state,
      // not the stale closure captured when the timer was scheduled.
      const latestAgents = agentsRef.current;
      const latestActiveAgentId = activeAgentIdRef.current;
      const latestJson = project(latestAgents, latestActiveAgentId);
      // Update the echo-suppression marker to the projection we're actually
      // sending so an incoming broadcast of this exact payload is still
      // recognised as our own echo.
      lastSyncedRef.current = latestJson;
      window.electronAPI
        .layoutSet({ agents: latestAgents, activeAgentId: latestActiveAgentId })
        .then((doc: LayoutDoc) => {
          if (doc?.version != null) {
            appliedVersionRef.current = Math.max(appliedVersionRef.current, doc.version);
          }
        })
        .catch(() => {
          // Push failed — drop the optimistic marker so the next change retries.
          if (lastSyncedRef.current === latestJson) lastSyncedRef.current = null;
        });
    }, PUSH_DEBOUNCE_MS);
  }, [agents, activeAgentId, enabled, sessionPhase]);

  useEffect(() => () => { if (pushTimerRef.current) clearTimeout(pushTimerRef.current); }, []);
}
