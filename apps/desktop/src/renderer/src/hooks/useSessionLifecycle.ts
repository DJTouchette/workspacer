/**
 * useSessionLifecycle — manages session load / save / auto-resume / picker.
 *
 * Extracted verbatim from App.tsx; all logic is unchanged.
 */
import { useRef, useCallback, useState, useEffect, type MutableRefObject, type Dispatch, type SetStateAction } from 'react';
import type { AgentWorkspace } from '../types/pane';
import { migrateSessionData } from '../App';
import { usePageVisible } from './usePageVisible';

interface UseSessionLifecycleOptions {
  configLoaded: boolean;
  autoResume: boolean | undefined;
  agents: AgentWorkspace[];
  activeAgentId: string;
  loadAgentsFromSession: (agents: AgentWorkspace[], activeAgentId: string) => void;
  reconcileAgents: (liveSessionIds: Set<string>) => void;
  appCwdRef: MutableRefObject<string>;
}

export interface SessionLifecycleResult {
  sessionPhase: 'loading' | 'picker' | 'active';
  setSessionPhase: Dispatch<SetStateAction<'loading' | 'picker' | 'active'>>;
  sessionList: any[];
  pickerCancellable: boolean;
  setPickerCancellable: Dispatch<SetStateAction<boolean>>;
  sessionName: string;
  ptyMapping: Record<string, string>;
  handlePtyReady: (paneId: string, ptySessionId: string) => void;
  handleNewSession: () => void;
  handleResumeSession: (filename: string) => void;
  handleDeleteSession: (filename: string) => void;
  saveCurrentSession: (force?: boolean) => void;
  switchSession: () => void;
}

export function useSessionLifecycle({
  configLoaded,
  autoResume,
  agents,
  activeAgentId,
  loadAgentsFromSession,
  reconcileAgents,
  appCwdRef,
}: UseSessionLifecycleOptions): SessionLifecycleResult {
  const [sessionPhase, setSessionPhase] = useState<'loading' | 'picker' | 'active'>('loading');
  const [sessionList, setSessionList] = useState<any[]>([]);
  // True only when the picker is reopened mid-session (so it can be dismissed).
  const [pickerCancellable, setPickerCancellable] = useState(false);
  const [sessionName, setSessionName] = useState('Default');

  // PTY mapping: paneId -> ptySessionId. For Claude panes, ptySessionId is the
  // Claude session id; used to resolve "which pane shows this session".
  const [ptyMapping, setPtyMapping] = useState<Record<string, string>>({});
  const lastSaveHashRef = useRef<string>('');
  const pageVisible = usePageVisible();

  const handlePtyReady = useCallback((paneId: string, ptySessionId: string) => {
    setPtyMapping((prev) => (prev[paneId] === ptySessionId ? prev : { ...prev, [paneId]: ptySessionId }));
  }, []);

  // Reconcile saved agents against the daemon's live sessions — mark any whose
  // session no longer exists as stopped (so the sidebar offers a respawn).
  // Ended sessions must not count as live: the store keeps a snapshot around
  // for a ~30s grace window after SessionEnd, and treating those ids as alive
  // left restored agents looking live while attached to a dead session.
  const reconcileWithDaemon = useCallback(() => {
    window.electronAPI.getAllClaudeSessions().then((sessions: any[]) => {
      reconcileAgents(new Set(sessions.filter((s) => s.status !== 'ended').map((s) => s.sessionId)));
    }).catch(() => {});
  }, [reconcileAgents]);

  // --- Session lifecycle ---
  const handleNewSession = useCallback(() => {
    loadAgentsFromSession([], '');
    setSessionName('Default');
    setPtyMapping({});
    setPickerCancellable(false);
    setSessionPhase('active');
  }, [loadAgentsFromSession]);

  const handleResumeSession = useCallback((filename: string) => {
    setPickerCancellable(false);
    window.electronAPI.loadSession(filename).then((data: any) => {
      const { agents: migratedAgents, activeAgentId: migratedActiveId, name: migratedName } = migrateSessionData(data, appCwdRef.current);
      loadAgentsFromSession(migratedAgents, migratedActiveId);
      setSessionName(migratedName);
      setPtyMapping({});
      setSessionPhase('active');
      reconcileWithDaemon();
    }).catch(() => {
      loadAgentsFromSession([], '');
      setSessionPhase('active');
    });
  }, [loadAgentsFromSession, reconcileWithDaemon, appCwdRef]);

  const handleDeleteSession = useCallback((filename: string) => {
    window.electronAPI.deleteSession(filename).then(() => {
      setSessionList((prev) => prev.filter((s) => s.filename !== filename));
    });
  }, []);

  const saveCurrentSession = useCallback((force?: boolean): Promise<void> => {
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
    const hash = JSON.stringify({
      n: payload.name,
      a: payload.activeAgentId,
      g: payload.agents.map((ag) => ag.id + ag.name + (ag.sessionId || '') + ag.activeTabId
        // Include canvas so a spatial-mode drag (which only changes t.canvas)
        // isn't deduped away and actually persists across reloads.
        + ag.tabs.map((t) => t.id + t.title + (t.activePaneId || '') + (t.canvas ? `${t.canvas.x},${t.canvas.y},${t.canvas.w},${t.canvas.h}` : '')
          + t.panes.map((p) => p.id + p.type + (p.url || '') + (p.notes || '')).join()).join()),
    });
    if (!force && hash === lastSaveHashRef.current) return Promise.resolve();
    lastSaveHashRef.current = hash;
    return window.electronAPI.saveSession(payload).then(() => undefined, (err: any) => {
      console.error('[Session] save failed:', err);
    });
  }, [agents, activeAgentId, sessionName, sessionPhase, ptyMapping]);

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

  // Decide what to show on launch once config is loaded (so a user's saved
  // autoResume preference is respected, not the in-memory default). With
  // autoResume on we restore the most recent session straight away; otherwise
  // we fall back to the picker. Runs exactly once.
  const startupDoneRef = useRef(false);
  useEffect(() => {
    if (!configLoaded || startupDoneRef.current) return;
    startupDoneRef.current = true;
    const shouldAutoResume = autoResume ?? false;
    window.electronAPI.listSessions().then((sessions) => {
      if (sessions.length === 0) {
        setSessionPhase('active');
        return;
      }
      setSessionList(sessions);
      if (shouldAutoResume) {
        handleResumeSession(sessions[0].filename); // most recent (list is sorted desc)
      } else {
        setSessionPhase('picker');
      }
    }).catch(() => setSessionPhase('active'));
  }, [configLoaded, autoResume, handleResumeSession]);

  // Re-open the picker mid-session (Command palette → "Switch session"). Saves
  // the current layout first so nothing is lost when switching, and marks the
  // picker dismissable so Escape/Cancel returns to the running app.
  const switchSession = useCallback(() => {
    saveCurrentSession(true);
    setPickerCancellable(true);
    window.electronAPI.listSessions()
      .then((sessions) => { setSessionList(sessions); setSessionPhase('picker'); })
      .catch(() => setSessionPhase('picker'));
  }, [saveCurrentSession]);

  return {
    sessionPhase,
    setSessionPhase,
    sessionList,
    pickerCancellable,
    setPickerCancellable,
    sessionName,
    ptyMapping,
    handlePtyReady,
    handleNewSession,
    handleResumeSession,
    handleDeleteSession,
    saveCurrentSession,
    switchSession,
  };
}
