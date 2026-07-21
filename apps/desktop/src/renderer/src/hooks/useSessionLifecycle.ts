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

  const saveCurrentSession = useCallback(
    (force?: boolean): Promise<void> => {
      if (sessionPhase !== 'active') return Promise.resolve();
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
      if (!force && hash === lastSaveHashRef.current) return Promise.resolve();
      lastSaveHashRef.current = hash;
      return window.electronAPI.saveSession(payload).then(
        () => undefined,
        (err: any) => {
          console.error('[Session] save failed:', err);
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
    // Quit handshake: main pauses teardown until we ack that the save landed
    // (or errored — ack regardless, so a save failure can't hang the quit).
    const unsub = window.electronAPI.onBeforeQuit(() => {
      saveCurrentSession(true).finally(() => {
        window.electronAPI.notifyQuitSaved?.();
      });
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
          setSessionPhase('active');
          return;
        }
        return window.electronAPI.loadSession(latest.filename).then((data: any) => {
          const {
            agents: migratedAgents,
            activeAgentId: migratedActiveId,
            name: migratedName,
          } = migrateSessionData(data, appCwdRef.current);
          loadAgentsFromSession(migratedAgents, migratedActiveId);
          setSessionName(migratedName);
          // Daemon reconciliation is NOT called here — the phase-triggered
          // effect above covers this path and the hub-adopted one alike.
          setSessionPhase('active');
        });
      })
      .catch(() => {
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
