/**
 * useSessionLifecycle — manages session load / save / auto-resume / picker.
 *
 * Extracted verbatim from App.tsx; all logic is unchanged.
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
  handleNewSession: (name?: string) => void;
  handleResumeSession: (filename: string) => void;
  handleDeleteSession: (filename: string) => void;
  handleRenameSession: (filename: string, newName: string) => Promise<void>;
  saveCurrentSession: (force?: boolean) => void;
  switchSession: () => void;
}

/** "Session Jul 16" — the fallback name when the user starts a new session
 *  without typing one. Distinct per day, which is usually enough; collisions
 *  get a numeric suffix via uniqueSessionName. */
function defaultSessionName(): string {
  return `Session ${new Date().toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}`;
}

/** Sessions are stored one-file-per-name, so a duplicate name would silently
 *  overwrite the other session's file. Suffix until unique. */
function uniqueSessionName(base: string, taken: string[]): string {
  let name = base;
  let i = 2;
  while (taken.includes(name)) name = `${base} ${i++}`;
  return name;
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

  // --- Session lifecycle ---
  // A new session gets its own name (user-typed or "Session Jul 16"), unique
  // against the saved list — every save writes <name>.yaml, so reusing a name
  // (the old hardcoded 'Default') silently overwrote the previous session's
  // file and made the picker a list of one.
  const handleNewSession = useCallback(
    (name?: string) => {
      const base = name?.trim() || defaultSessionName();
      loadAgentsFromSession([], '');
      setSessionName(
        uniqueSessionName(
          base,
          sessionList.map((s) => s.name),
        ),
      );
      setPtyMapping({});
      setPickerCancellable(false);
      setSessionPhase('active');
    },
    [loadAgentsFromSession, sessionList],
  );

  const handleResumeSession = useCallback(
    (filename: string) => {
      setPickerCancellable(false);
      window.electronAPI
        .loadSession(filename)
        .then((data: any) => {
          const {
            agents: migratedAgents,
            activeAgentId: migratedActiveId,
            name: migratedName,
          } = migrateSessionData(data, appCwdRef.current);
          loadAgentsFromSession(migratedAgents, migratedActiveId);
          setSessionName(migratedName);
          setPtyMapping({});
          setSessionPhase('active');
          reconcileWithDaemon();
        })
        .catch(() => {
          loadAgentsFromSession([], '');
          setSessionPhase('active');
        });
    },
    [loadAgentsFromSession, reconcileWithDaemon, appCwdRef],
  );

  const handleDeleteSession = useCallback((filename: string) => {
    window.electronAPI.deleteSession(filename).then(() => {
      setSessionList((prev) => prev.filter((s) => s.filename !== filename));
    });
  }, []);

  /** Rename a saved session file: re-save its data under the new name, then
   *  delete the old file (unless sanitization collapsed both names to the same
   *  file). If it's the session we're currently in, follow the rename so the
   *  autosave keeps writing the new file instead of resurrecting the old name. */
  const handleRenameSession = useCallback(
    async (filename: string, newName: string): Promise<void> => {
      const clean = newName.trim();
      if (!clean) return;
      try {
        const data: any = await window.electronAPI.loadSession(filename);
        if (!data) return;
        const sessions: any[] = await window.electronAPI.listSessions().catch(() => []);
        const name = uniqueSessionName(
          clean,
          sessions.filter((s) => s.filename !== filename).map((s) => s.name),
        );
        if (name === data.name) return;
        const newFile = await window.electronAPI.saveSession({ ...data, name });
        if (newFile !== filename) {
          await window.electronAPI.deleteSession(filename).catch(() => {});
        }
        setSessionList((prev) =>
          prev.map((s) =>
            s.filename === filename ? { ...s, name, filename: newFile ?? s.filename } : s,
          ),
        );
        if (data.name === sessionName) setSessionName(name);
      } catch (err) {
        console.error('[Session] rename failed:', err);
      }
    },
    [sessionName],
  );

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

  // Decide what to show on launch once config is loaded (so a user's saved
  // autoResume preference is respected, not the in-memory default). With
  // autoResume on we restore the most recent session straight away; otherwise
  // we fall back to the picker. Runs exactly once.
  const startupDoneRef = useRef(false);
  useEffect(() => {
    if (!configLoaded || startupDoneRef.current) return;
    startupDoneRef.current = true;
    const shouldAutoResume = autoResume ?? false;
    window.electronAPI
      .listSessions()
      .then((sessions) => {
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
      })
      .catch(() => setSessionPhase('active'));
  }, [configLoaded, autoResume, handleResumeSession]);

  // Re-open the picker mid-session (Command palette → "Switch session"). Saves
  // the current layout first so nothing is lost when switching, and marks the
  // picker dismissable so Escape/Cancel returns to the running app.
  const switchSession = useCallback(() => {
    saveCurrentSession(true);
    setPickerCancellable(true);
    window.electronAPI
      .listSessions()
      .then((sessions) => {
        setSessionList(sessions);
        setSessionPhase('picker');
      })
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
    handleRenameSession,
    saveCurrentSession,
    switchSession,
  };
}
