import { useState, useCallback, useEffect, useRef } from 'react';
import { PaneConfig, PaneType, TabConfig, AgentWorkspace } from '../types/pane';

let nextId = 1;

function generateId(prefix: string): string {
  nextId += 1;
  return `${prefix}-${Date.now()}-${nextId}`;
}

const defaultTitles: Record<PaneType, string> = {
  terminal: 'Terminal',
  browser: 'Browser',
  notes: 'Notes',
  agent: 'Agent',
  claude: 'Claude',
  settings: 'Settings',
  dashboard: 'Dashboard',
  tracker: 'Tracker',
  devops: 'DevOps',
  'agent-manager': 'Agent Manager',
  devdaemon: 'Daemon',
  inbox: 'Inbox',
};

/** Derive a human label from a working directory (its basename). */
export function deriveAgentName(cwd: string): string {
  if (!cwd) return 'Agent';
  const parts = cwd.replace(/[/\\]+$/, '').split(/[/\\]/);
  return parts[parts.length - 1] || cwd;
}

/**
 * Default tab layout for a freshly spawned agent: a single Claude pane attached
 * to the agent's daemon session as a viewer (the session itself was already
 * spawned, so the pane never owns its lifetime).
 */
function defaultAgentTabs(sessionId: string | undefined, cwd: string): { tabs: TabConfig[]; activeTabId: string } {
  const paneId = generateId('claude');
  const tabId = generateId('tab');
  const pane: PaneConfig = {
    id: paneId,
    type: 'claude',
    title: 'Claude',
    cwd,
    attachSessionId: sessionId,
  };
  return {
    tabs: [{ id: tabId, title: 'Claude', panes: [pane], activePaneId: paneId }],
    activeTabId: tabId,
  };
}

export function useAgentManager() {
  const [agents, setAgents] = useState<AgentWorkspace[]>([]);
  const [activeAgentId, setActiveAgentId] = useState<string>('');

  // Refs let the (stable) callbacks below read current state without being
  // re-created on every agent/tab change.
  const agentsRef = useRef(agents);
  const activeAgentIdRef = useRef(activeAgentId);
  useEffect(() => { agentsRef.current = agents; }, [agents]);
  useEffect(() => { activeAgentIdRef.current = activeAgentId; }, [activeAgentId]);

  // Derived view of the active agent's tabs — App treats these like the old
  // flat workspace state.
  const activeAgent = agents.find((a) => a.id === activeAgentId);
  const tabs = activeAgent?.tabs ?? [];
  const activeTabId = activeAgent?.activeTabId ?? '';

  const mutateAgent = useCallback((agentId: string, fn: (a: AgentWorkspace) => AgentWorkspace) => {
    setAgents((prev) => prev.map((a) => (a.id === agentId ? fn(a) : a)));
  }, []);

  const mutateActiveAgent = useCallback((fn: (a: AgentWorkspace) => AgentWorkspace) => {
    const aid = activeAgentIdRef.current;
    if (!aid) return;
    mutateAgent(aid, fn);
  }, [mutateAgent]);

  // ── Agent lifecycle ────────────────────────────────────────────────────

  /** Spawn a long-lived claudemon session and open a workspace for it. */
  const spawnAgent = useCallback(async (opts: { cwd: string; name?: string; profileId?: string }) => {
    const cwd = opts.cwd;
    let sessionId: string | undefined;
    try {
      sessionId = await window.electronAPI.spawnClaude({ cwd, profileId: opts.profileId, cols: 120, rows: 32 });
    } catch (err) {
      console.error('[Agent] spawn failed:', err);
    }
    const { tabs: agentTabs, activeTabId: agentActiveTab } = defaultAgentTabs(sessionId, cwd);
    const agent: AgentWorkspace = {
      id: generateId('agent'),
      name: opts.name?.trim() || deriveAgentName(cwd),
      cwd,
      profileId: opts.profileId,
      sessionId,
      tabs: agentTabs,
      activeTabId: agentActiveTab,
    };
    setAgents((prev) => [...prev, agent]);
    setActiveAgentId(agent.id);
    return agent.id;
  }, []);

  /** Re-spawn a stopped agent (kept across restarts) and re-point its Claude
   *  panes at the new session. */
  const respawnAgent = useCallback(async (agentId: string) => {
    const agent = agentsRef.current.find((a) => a.id === agentId);
    if (!agent) return;
    let sessionId: string | undefined;
    try {
      sessionId = await window.electronAPI.spawnClaude({ cwd: agent.cwd, profileId: agent.profileId, cols: 120, rows: 32 });
    } catch (err) {
      console.error('[Agent] respawn failed:', err);
    }
    if (!sessionId) return;
    const oldSession = agent.sessionId;
    mutateAgent(agentId, (a) => ({
      ...a,
      sessionId,
      tabs: a.tabs.map((t) => ({
        ...t,
        panes: t.panes.map((p) =>
          p.type === 'claude' && (p.attachSessionId === oldSession || !p.attachSessionId)
            ? { ...p, attachSessionId: sessionId, resumeSessionId: undefined }
            : p,
        ),
      })),
    }));
  }, [mutateAgent]);

  /** Explicitly terminate an agent: kill its daemon session and drop it. */
  const terminateAgent = useCallback(async (agentId: string) => {
    const agent = agentsRef.current.find((a) => a.id === agentId);
    setAgents((prev) => prev.filter((a) => a.id !== agentId));
    setActiveAgentId((cur) => {
      if (cur !== agentId) return cur;
      const rest = agentsRef.current.filter((a) => a.id !== agentId);
      return rest[0]?.id ?? '';
    });
    if (agent?.sessionId) {
      try { await window.electronAPI.claudeClose(agent.sessionId); } catch { /* already gone */ }
    }
  }, []);

  const renameAgent = useCallback((agentId: string, name: string) => {
    mutateAgent(agentId, (a) => ({ ...a, name }));
  }, [mutateAgent]);

  /**
   * Reconcile saved agents against the daemon's live sessions: any agent whose
   * session no longer exists is marked stopped (sessionId cleared) so the
   * sidebar can offer a respawn.
   */
  const reconcileAgents = useCallback((liveSessionIds: Set<string>) => {
    setAgents((prev) => prev.map((a) =>
      a.sessionId && !liveSessionIds.has(a.sessionId) ? { ...a, sessionId: undefined } : a,
    ));
  }, []);

  const loadAgentsFromSession = useCallback((sessionAgents: AgentWorkspace[], activeId: string) => {
    setAgents(sessionAgents);
    setActiveAgentId(activeId || sessionAgents[0]?.id || '');
  }, []);

  // ── Tab/pane operations (scoped to the active agent) ──────────────────────

  const setActiveTabId = useCallback((tabId: string) => {
    mutateActiveAgent((a) => ({ ...a, activeTabId: tabId }));
  }, [mutateActiveAgent]);

  const addTab = useCallback((
    type: PaneType,
    title?: string,
    insertPosition: string = 'after',
    shell?: string,
    url?: string,
    appMode?: boolean,
    cwd?: string,
    profileId?: string,
    resumeSessionId?: string,
    attachSessionId?: string,
  ) => {
    const aid = activeAgentIdRef.current;
    if (!aid) return '';
    const paneId = generateId(type);
    const tabId = generateId('tab');
    const paneTitle = title ?? defaultTitles[type];

    const pane: PaneConfig = {
      id: paneId, type, title: paneTitle, shell, url, appMode, cwd, profileId, resumeSessionId, attachSessionId,
    };
    const tab: TabConfig = { id: tabId, title: paneTitle, panes: [pane], activePaneId: paneId };

    mutateAgent(aid, (a) => {
      let newTabs: TabConfig[];
      if (insertPosition === 'after') {
        const idx = a.tabs.findIndex((t) => t.id === a.activeTabId);
        if (idx >= 0) {
          newTabs = [...a.tabs];
          newTabs.splice(idx + 1, 0, tab);
        } else {
          newTabs = [...a.tabs, tab];
        }
      } else {
        newTabs = [...a.tabs, tab];
      }
      return { ...a, tabs: newTabs, activeTabId: tabId };
    });
    return tabId;
  }, [mutateAgent]);

  const splitTab = useCallback((
    tabId: string,
    type: PaneType,
    title?: string,
    shell?: string,
    url?: string,
    appMode?: boolean,
    cwd?: string,
  ) => {
    const paneId = generateId(type);
    const paneTitle = title ?? defaultTitles[type];
    const pane: PaneConfig = { id: paneId, type, title: paneTitle, shell, url, appMode, cwd };

    mutateActiveAgent((a) => ({
      ...a,
      tabs: a.tabs.map((t) =>
        t.id === tabId ? { ...t, panes: [...t.panes, pane], activePaneId: paneId } : t,
      ),
    }));
    return paneId;
  }, [mutateActiveAgent]);

  const removeTab = useCallback((tabId: string) => {
    mutateActiveAgent((a) => {
      const filtered = a.tabs.filter((t) => t.id !== tabId);
      if (filtered.length === 0) return a; // an agent always keeps at least one tab
      let nextActive = a.activeTabId;
      if (nextActive === tabId) {
        const idx = a.tabs.findIndex((t) => t.id === tabId);
        const next = a.tabs[idx + 1] ?? a.tabs[idx - 1];
        nextActive = next ? next.id : filtered[0].id;
      }
      return { ...a, tabs: filtered, activeTabId: nextActive };
    });
  }, [mutateActiveAgent]);

  const removePane = useCallback((tabId: string, paneId: string) => {
    mutateActiveAgent((a) => {
      const tab = a.tabs.find((t) => t.id === tabId);
      if (!tab) return a;

      if (tab.panes.length <= 1) {
        // Last pane → remove the whole tab (but keep at least one tab).
        const filtered = a.tabs.filter((t) => t.id !== tabId);
        if (filtered.length === 0) return a;
        let nextActive = a.activeTabId;
        if (nextActive === tabId) {
          const idx = a.tabs.findIndex((t) => t.id === tabId);
          const next = a.tabs[idx + 1] ?? a.tabs[idx - 1];
          nextActive = next ? next.id : filtered[0].id;
        }
        return { ...a, tabs: filtered, activeTabId: nextActive };
      }

      return {
        ...a,
        tabs: a.tabs.map((t) => {
          if (t.id !== tabId) return t;
          const newPanes = t.panes.filter((p) => p.id !== paneId);
          const newActive = t.activePaneId === paneId ? (newPanes[0]?.id || '') : t.activePaneId;
          return { ...t, panes: newPanes, activePaneId: newActive };
        }),
      };
    });
  }, [mutateActiveAgent]);

  const renameTab = useCallback((tabId: string, title: string) => {
    mutateActiveAgent((a) => ({
      ...a,
      tabs: a.tabs.map((t) => (t.id === tabId ? { ...t, title } : t)),
    }));
  }, [mutateActiveAgent]);

  const moveTab = useCallback((tabId: string, toIndex: number) => {
    mutateActiveAgent((a) => {
      const fromIndex = a.tabs.findIndex((t) => t.id === tabId);
      if (fromIndex < 0) return a;
      const clamped = Math.max(0, Math.min(toIndex, a.tabs.length - 1));
      if (fromIndex === clamped) return a;
      const copy = [...a.tabs];
      const [moved] = copy.splice(fromIndex, 1);
      copy.splice(clamped, 0, moved);
      return { ...a, tabs: copy };
    });
  }, [mutateActiveAgent]);

  const setActivePane = useCallback((tabId: string, paneId: string) => {
    mutateActiveAgent((a) => ({
      ...a,
      tabs: a.tabs.map((t) => (t.id === tabId ? { ...t, activePaneId: paneId } : t)),
    }));
  }, [mutateActiveAgent]);

  const hibernatePane = useCallback((tabId: string, paneId: string) => {
    mutateActiveAgent((a) => ({
      ...a,
      tabs: a.tabs.map((t) =>
        t.id === tabId
          ? { ...t, panes: t.panes.map((p) => (p.id === paneId ? { ...p, hibernated: true } : p)) }
          : t,
      ),
    }));
  }, [mutateActiveAgent]);

  const wakePane = useCallback((tabId: string, paneId: string) => {
    mutateActiveAgent((a) => ({
      ...a,
      tabs: a.tabs.map((t) =>
        t.id === tabId
          ? { ...t, panes: t.panes.map((p) => (p.id === paneId ? { ...p, hibernated: false } : p)) }
          : t,
      ),
    }));
  }, [mutateActiveAgent]);

  const updatePaneUrl = useCallback((tabId: string, paneId: string, url: string) => {
    mutateActiveAgent((a) => ({
      ...a,
      tabs: a.tabs.map((t) =>
        t.id === tabId
          ? { ...t, panes: t.panes.map((p) => (p.id === paneId ? { ...p, url } : p)) }
          : t,
      ),
    }));
  }, [mutateActiveAgent]);

  const getActiveTab = useCallback((): TabConfig | undefined => {
    const agent = agentsRef.current.find((a) => a.id === activeAgentIdRef.current);
    return agent?.tabs.find((t) => t.id === agent.activeTabId);
  }, []);

  return {
    // agents
    agents,
    activeAgentId,
    activeAgent,
    setActiveAgentId,
    spawnAgent,
    respawnAgent,
    terminateAgent,
    renameAgent,
    reconcileAgents,
    loadAgentsFromSession,
    // tabs (active agent)
    tabs,
    activeTabId,
    setActiveTabId,
    addTab,
    splitTab,
    removeTab,
    removePane,
    renameTab,
    moveTab,
    setActivePane,
    hibernatePane,
    wakePane,
    updatePaneUrl,
    getActiveTab,
  };
}
