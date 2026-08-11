/**
 * useSessionLifecycle — loads and continuously saves THE workspace session.
 *
 * Single implicit session: boot always restores the most recent saved layout
 * (no picker, no named-session switching — the sidebar's live feed is the
 * "what was I doing" surface now). Saving is unchanged: 30s ticks while
 * visible, a 1s debounce after layout changes, and the quit handshake.
 */
import {
  useRef,
  useCallback,
  useState,
  useEffect,
  type MutableRefObject,
  type Dispatch,
  type SetStateAction,
} from 'react';
import type { AgentWorkspace } from '../types/pane';
import { migrateSessionData } from '../App';
import { postNotification } from '../lib/notificationBus';
import { usePageVisible } from './usePageVisible';

interface UseSessionLifecycleOptions {
  configLoaded: boolean;
  agents: AgentWorkspace[];
  activeAgentId: string;
  loadAgentsFromSession: (agents: AgentWorkspace[], activeAgentId: string) => void;
  reconcileAgents: (liveSessionIds: Set<string>, opts?: { respawnStopped?: boolean }) => void;
  appCwdRef: MutableRefObject<string>;
}

export interface SessionLifecycleResult {
  sessionPhase: 'loading' | 'active';
  setSessionPhase: Dispatch<SetStateAction<'loading' | 'active'>>;
  sessionName: string;
  ptyMapping: Record<string, string>;
  handlePtyReady: (paneId: string, ptySessionId: string) => void;
  saveCurrentSession: (force?: boolean) => void;
}

export function useSessionLifecycle({
  configLoaded,
  agents,
  activeAgentId,
  loadAgentsFromSession,
  reconcileAgents,
  appCwdRef,
}: UseSessionLifecycleOptions): SessionLifecycleResult {
  const [sessionPhase, setSessionPhase] = useState<'loading' | 'active'>('loading');
  // The implicit session keeps whatever name its file carried (old named
  // sessions restore under their own name and keep saving to the same file).
  const [sessionName, setSessionName] = useState('Default');

  // PTY mapping: paneId -> ptySessionId. For Claude panes, ptySessionId is the
  // Claude session id; used to resolve "which pane shows this session".
  const [ptyMapping, setPtyMapping] = useState<Record<string, string>>({});
  const lastSaveHashRef = useRef<string>('');
  const pageVisible = usePageVisible();

  const handlePtyReady = useCallback((paneId: string, ptySessionId: string) => {
    setPtyMapping((prev) =>
      prev[paneId] === ptySessionId ? prev : { ...prev, [paneId]: ptySessionId },
    );
  }, []);

  // Reconcile saved agents against the daemon once the layout is up — on BOTH
  // hydration paths. The hub-adopted path used to skip this entirely (it was
  // buried in the local-restore branch), so after a machine reboot every agent
  // card kept a dead sessionId, looked live, and its pane sat on the
  // "Connecting…" spinner forever. Runs when sessionPhase flips to 'active'
  // (local restore and hub adoption both end there), asks claudemon itself for
  // the live ids (the renderer-side snapshot store is always empty at boot),
  // retries while the daemon is still coming up, then marks dead agents
  // stopped and auto-respawns them — resuming their old sessions — so the
  // restored layout reconnects instead of waiting for a manual respawn click.
  const reconcileDoneRef = useRef(false);
  useEffect(() => {
    if (sessionPhase !== 'active' || reconcileDoneRef.current) return;
    reconcileDoneRef.current = true;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const attempt = (retriesLeft: number, delayMs: number) => {
      // Optional: absent on older preloads / test mocks; the web polyfill
      // returns null (the desktop owns reconciliation).
      const call = window.electronAPI.listLiveClaudeSessionIds?.();
      if (!call) return;
      call
        .then((ids) => {
          if (cancelled) return;
          if (ids) {
            reconcileAgents(new Set(ids), { respawnStopped: true });
          } else if (retriesLeft > 0) {
            timer = setTimeout(
              () => attempt(retriesLeft - 1, Math.min(delayMs * 2, 5000)),
              delayMs,
            );
          }
        })
        .catch(() => {});
    };
    attempt(10, 500);
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [sessionPhase, reconcileAgents]);

  /**
   * Set once a boot restore could not be trusted, and never cleared: for the
   * rest of the run this window renders an empty roster that is NOT the user's
   * layout, so every save path stays shut. A ref rather than state because
   * `saveCurrentSession` is called from timers and `beforeunload` handlers that
   * captured an older closure — a stale `false` there would defeat the gate.
   */
  const restoreFailedRef = useRef(false);
  const blockSaves = useCallback((message: string): void => {
    restoreFailedRef.current = true;
    console.error('[Session]', message);
    postNotification({
      title: 'Session not restored',
      body: message,
      level: 'warn',
      source: 'session',
    });
  }, []);

  /** One "workspace not saved" notice per run, not one per failed autosave. */
  const saveFailureReportedRef = useRef(false);

  const saveCurrentSession = useCallback(
    (force?: boolean): Promise<boolean> => {
      if (sessionPhase !== 'active') return Promise.resolve(true);
      // A restore that failed leaves an empty roster in memory that is NOT the
      // user's layout. Writing it back is how a workspace gets erased — the
      // debounced autosave below fires a second after boot, long before anyone
      // could notice the agents are missing. Stay read-only for the run.
      if (restoreFailedRef.current) return Promise.resolve(true);
      const payload = {
        name: sessionName,
        activeAgentId,
        agents: agents.map((a) => ({
          ...a,
          tabs: a.tabs.map((t) => ({ ...t, panes: t.panes.map((p) => ({ ...p })) })),
        })),
        ptyMapping: { ...ptyMapping },
      };
      // Hash the full persisted payload so ANY field we actually write
      // (model, effort, permissionMode, cwd, skipPermissions, pane.cwd/shell,
      // ptyMapping, …) re-arms the autosave. A partial hash silently dropped
      // saves for edits confined to those fields until a forced quit-save, so a
      // crash/kill in the debounce window lost them.
      const hash = JSON.stringify(payload);
      if (!force && hash === lastSaveHashRef.current) return Promise.resolve(true);
      // Claim the hash up front so two saves racing in the same tick don't both
      // write — but give it back if the write fails. Committing it permanently
      // meant a failed save was remembered as done: the next attempt deduped
      // against a payload that never reached disk, so one transient fault became
      // a permanent loss until some other edit changed the hash.
      const previous = lastSaveHashRef.current;
      lastSaveHashRef.current = hash;
      return window.electronAPI.saveSession(payload).then(
        () => true,
        (err: any) => {
          if (lastSaveHashRef.current === hash) lastSaveHashRef.current = previous;
          console.error('[Session] save failed:', err);
          // console.error has no consumer in a packaged app. A workspace that
          // silently stops persisting is the same class of loss as a restore
          // that silently failed, and that case has used this channel since it
          // was found — use it here too, once per run so a flapping disk does
          // not spam the tray.
          if (!saveFailureReportedRef.current) {
            saveFailureReportedRef.current = true;
            postNotification({
              title: 'Workspace not saved',
              body: `Saving your agents and layout failed: ${err?.message ?? String(err)}. Changes since the last successful save will be lost if you quit.`,
              level: 'warn',
              source: 'session',
            });
          }
          return false;
        },
      );
    },
    [agents, activeAgentId, sessionName, sessionPhase, ptyMapping],
  );

  useEffect(() => {
    if (sessionPhase !== 'active' || !pageVisible) return;
    const interval = setInterval(saveCurrentSession, 30000);
    return () => clearInterval(interval);
  }, [sessionPhase, pageVisible, saveCurrentSession]);

  // Persist promptly after the layout actually changes (saveCurrentSession is
  // re-created whenever agents/activeAgentId/panes change, so this effect fires
  // per change and the timeout debounces bursts). Without it, a terminate or
  // spawn only reaches disk on the 30s tick or a graceful quit — kill the app
  // in that window and the terminated agent resurrects on the next boot. The
  // content hash inside saveCurrentSession keeps redundant writes cheap.
  useEffect(() => {
    if (sessionPhase !== 'active') return;
    const t = setTimeout(() => saveCurrentSession(), 1000);
    return () => clearTimeout(t);
  }, [sessionPhase, saveCurrentSession]);

  useEffect(() => {
    const handler = () => saveCurrentSession();
    window.addEventListener('beforeunload', handler);
    return () => window.removeEventListener('beforeunload', handler);
  }, [saveCurrentSession]);

  useEffect(() => {
    // Quit handshake: main pauses teardown until we ack. We ack even when the
    // save FAILED, so a bad disk can't hang the quit — but the ack CARRIES the
    // outcome, because acking unconditionally is what made a lost workspace
    // indistinguishable from a saved one (main then reports the failure).
    const unsub = window.electronAPI.onBeforeQuit(() => {
      saveCurrentSession(true).then(
        (ok) => window.electronAPI.notifyQuitSaved?.(ok),
        () => window.electronAPI.notifyQuitSaved?.(false),
      );
    });
    return unsub;
  }, [saveCurrentSession]);

  // Boot: restore the most recent saved layout (list is sorted desc), or start
  // fresh when none exists. Runs exactly once, after config load so the hub
  // hydration gate upstream (App wires configLoaded through it) has settled.
  const startupDoneRef = useRef(false);
  useEffect(() => {
    if (!configLoaded || startupDoneRef.current) return;
    startupDoneRef.current = true;
    window.electronAPI
      .listSessions()
      .then((sessions) => {
        const latest = sessions[0];
        if (!latest) {
          // Genuinely nothing saved (fresh install) — an empty roster IS the
          // truth here, so saving stays armed.
          setSessionPhase('active');
          return;
        }
        return window.electronAPI.loadSession(latest.filename).then((data: any) => {
          const {
            agents: migratedAgents,
            activeAgentId: migratedActiveId,
            name: migratedName,
            recognised,
          } = migrateSessionData(data, appCwdRef.current);
          if (!recognised) {
            // The file is there but this build cannot read it — a newer
            // nightly's schema, or a shape a bad write left behind. Do not
            // overwrite it with the empty roster we are about to render.
            blockSaves(
              `Could not read the saved session "${latest.filename}". Your layout is unchanged on disk; this window started empty and will not save over it.`,
            );
          }
          loadAgentsFromSession(migratedAgents, migratedActiveId);
          setSessionName(migratedName);
          // Daemon reconciliation is NOT called here — the phase-triggered
          // effect above covers this path and the hub-adopted one alike.
          setSessionPhase('active');
        });
      })
      .catch((err) => {
        // Listing or reading threw. We know a session file may exist and we
        // could not read it, so this empty roster must never reach disk.
        blockSaves(
          'Could not restore the previous session. Your layout is unchanged on disk; this window started empty and will not save over it.',
        );
        console.error('[Session] restore failed:', err);
        loadAgentsFromSession([], '');
        setSessionPhase('active');
      });
  }, [configLoaded, loadAgentsFromSession, appCwdRef]);

  return {
    sessionPhase,
    setSessionPhase,
    sessionName,
    ptyMapping,
    handlePtyReady,
    saveCurrentSession,
  };
}
