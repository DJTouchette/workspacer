/**
 * useSessionLifecycle — manages session load / save / auto-resume / picker.
 *
 * Extracted verbatim from App.tsx; all logic is unchanged.
 */
import { useRef, useCallback, useState, useEffect, type MutableRefObject, type Dispatch, type SetStateAction } from 'react';
import type { AgentWorkspace } from '../types/pane';
import { migrateSessionData } from '../App';

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

  const handlePtyReady = useCallback((paneId: string, ptySessionId: string) => {
    setPtyMapping((prev) => (prev[paneId] === ptySessionId ? prev : { ...prev, [paneId]: ptySessionId }));
  }, []);

  // Reconcile saved agents against the daemon's live sessions — mark any whose
  // session no longer exists as stopped (so the sidebar offers a respawn).
  const reconcileWithDaemon = useCallback(() => {
    window.electronAPI.getAllClaudeSessions().then((sessions: any[]) => {
      reconcileAgents(new Set(sessions.map((s) => s.sessionId)));
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

  const saveCurrentSession = useCallback((force?: boolean) => {
    if (sessionPhase !== 'active') return;
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
        + ag.tabs.map((t) => t.id + t.title + t.panes.map((p) => p.id + p.type + (p.url || '')).join()).join()),
    });
    if (!force && hash === lastSaveHashRef.current) return;
    lastSaveHashRef.current = hash;
    window.electronAPI.saveSession(payload).catch((err: any) => {
      console.error('[Session] save failed:', err);
    });
  }, [agents, activeAgentId, sessionName, sessionPhase, ptyMapping]);

  useEffect(() => {
    if (sessionPhase !== 'active') return;
    const interval = setInterval(saveCurrentSession, 30000);
    return () => clearInterval(interval);
  }, [sessionPhase, saveCurrentSession]);

  useEffect(() => {
    const handler = () => saveCurrentSession();
    window.addEventListener('beforeunload', handler);
    return () => window.removeEventListener('beforeunload', handler);
  }, [saveCurrentSession]);

  useEffect(() => {
    const unsub = window.electronAPI.onBeforeQuit(() => saveCurrentSession(true));
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
