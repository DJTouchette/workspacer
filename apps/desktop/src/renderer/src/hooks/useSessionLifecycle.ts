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
  reconcileAgents: (liveSessionIds: Set<string>) => void;
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

  // Reconcile saved agents against the daemon's live sessions — mark any whose
  // session no longer exists as stopped (so the sidebar offers a respawn).
  // Ended sessions must not count as live: the store keeps a snapshot around
  // for a ~30s grace window after SessionEnd, and treating those ids as alive
  // left restored agents looking live while attached to a dead session.
  const reconcileWithDaemon = useCallback(() => {
    window.electronAPI
      .getAllClaudeSessions()
      .then((sessions: any[]) => {
        reconcileAgents(
          new Set(sessions.filter((s) => s.status !== 'ended').map((s) => s.sessionId)),
        );
      })
      .catch(() => {});
  }, [reconcileAgents]);

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
          setSessionPhase('active');
          reconcileWithDaemon();
        });
      })
      .catch(() => {
        loadAgentsFromSession([], '');
        setSessionPhase('active');
      });
  }, [configLoaded, loadAgentsFromSession, reconcileWithDaemon, appCwdRef]);

  return {
    sessionPhase,
    setSessionPhase,
    sessionName,
    ptyMapping,
    handlePtyReady,
    saveCurrentSession,
  };
}
