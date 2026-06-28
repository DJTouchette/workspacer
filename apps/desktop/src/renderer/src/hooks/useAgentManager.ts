import { useState, useCallback, useEffect, useRef } from 'react';
import { PaneConfig, PaneType, TabConfig, AgentWorkspace, AgentProvider, resolveProvider } from '../types/pane';

/** Human label for an agent provider (tab/pane titles). */
export function providerLabel(provider: AgentProvider | undefined): string {
  switch (resolveProvider(provider)) {
    case 'codex': return 'Codex';
    case 'opencode': return 'OpenCode';
    default: return 'Claude';
  }
}
import { agentIdForSession, dedupeBySessionId } from '../lib/agentIdentity';

let nextId = 1;

function generateId(prefix: string): string {
  nextId += 1;
  return `${prefix}-${Date.now()}-${nextId}`;
}

const defaultTitles: Record<PaneType, string> = {
  terminal: 'Terminal',
  browser: 'Browser',
  notes: 'Notes',
  claude: 'Claude',
  settings: 'Settings',
  review: 'Review',
  plugin: 'Plugin',
  plugins: 'Plugins',
  overview: 'Overview',
  library: 'Library',
  analytics: 'Analytics',
  ask: 'Ask',
  editor: 'Editor',
};

/** Derive a human label from a working directory (its basename). */
export function deriveAgentName(cwd: string): string {
  if (!cwd) return 'Agent';
  const parts = cwd.replace(/[/\\]+$/, '').split(/[/\\]/);
  return parts[parts.length - 1] || cwd;
}

/**
 * Re-point an agent-scope plugin pane's webview URL at a (possibly new) session.
 * Agent-scope plugin panes carry the agent's sessionId/cwd as query params
 * (baked in at open time by handleOpenPlugin); on respawn — which is also how a
 * stopped agent is restored after an app restart — the session id changes, so
 * without this the webview would query a dead session. Non-plugin / global
 * panes (no sessionId param) are left untouched by the caller's guard. A
 * non-URL value (e.g. about:blank) is returned unchanged.
 */
function withAgentContext(rawUrl: string, sessionId: string | undefined, cwd: string | undefined): string {
  try {
    const u = new URL(rawUrl);
    if (sessionId) u.searchParams.set('sessionId', sessionId);
    else u.searchParams.delete('sessionId');
    if (cwd) u.searchParams.set('cwd', cwd);
    return u.toString();
  } catch {
    return rawUrl;
  }
}

/** Derive a short display name for a supervisor from its seed question.
 *  Produces e.g. "🧭 why is the build failing" (truncated to ~40 chars). */
export function deriveSupervisorName(question: string): string {
  const words = question.trim().split(/\s+/).slice(0, 5).join(' ');
  const truncated = words.length > 37 ? `${words.slice(0, 37)}…` : words;
  return `\u{1F9ED} ${truncated}`;
}

/**
 * Default tab layout for a freshly spawned agent: a single Claude pane attached
 * to the agent's daemon session as a viewer (the session itself was already
 * spawned, so the pane never owns its lifetime).
 */
function defaultAgentTabs(sessionId: string | undefined, cwd: string, initialPrompt?: string, provider?: AgentProvider): { tabs: TabConfig[]; activeTabId: string } {
  const paneId = generateId('claude');
  const tabId = generateId('tab');
  const label = providerLabel(provider);
  const pane: PaneConfig = {
    id: paneId,
    type: 'claude',
    title: label,
    cwd,
    provider,
    attachSessionId: sessionId,
    initialPrompt,
  };
  return {
    tabs: [{ id: tabId, title: label, panes: [pane], activePaneId: paneId, lastActiveAt: Date.now() }],
    activeTabId: tabId,
  };
}

/** Fixed id for the singleton global "Overview" workspace. */
export const GLOBAL_WORKSPACE_ID = 'global';

/** The default Overview tab — a dashboard pane, so the global workspace isn't empty. */
function overviewTab(): TabConfig {
  const paneId = generateId('pane');
  const tabId = generateId('tab');
  return { id: tabId, title: 'Overview', panes: [{ id: paneId, type: 'overview', title: 'Overview' }], activePaneId: paneId };
}

/** The agent-less workspace that hosts cross-agent / plugin panes. */
function makeGlobalWorkspace(): AgentWorkspace {
  const tab = overviewTab();
  return { id: GLOBAL_WORKSPACE_ID, name: 'Overview', cwd: '', global: true, tabs: [tab], activeTabId: tab.id };
}

/** Ensure exactly one global workspace exists (pinned first) and that it always
 *  has at least the Overview pane — backfills it into an empty/legacy global. */
function withGlobalWorkspace(list: AgentWorkspace[]): AgentWorkspace[] {
  const existing = list.find((a) => a.global);
  if (!existing) return [makeGlobalWorkspace(), ...list];
  if (existing.tabs.length === 0) {
    const tab = overviewTab();
    return list.map((a) => (a.global ? { ...a, tabs: [tab], activeTabId: tab.id } : a));
  }
  return list;
}

export function useAgentManager() {
  const [agents, setAgents] = useState<AgentWorkspace[]>(() => withGlobalWorkspace([]));
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
  const spawnAgent = useCallback(async (opts: {
    cwd: string;
    name?: string;
    /** Coding-agent backend to run. Defaults to 'claude'. */
    provider?: AgentProvider;
    profileId?: string;
    model?: string;
    skipPermissions?: boolean;
    /** Library item ids (kind 'mcp') to load for this session. */
    mcpItemIds?: string[];
    initialPrompt?: string;
    /** Resume an existing Claude session (`--resume <id>`) instead of starting fresh. */
    resumeSessionId?: string;
    /** When true the spawned session receives the workspacer MCP facade. */
    supervisor?: boolean;
    /** Marks this workspace as a supervisor. */
    kind?: 'supervisor';
    /** For supervisors: the id of the agent being supervised. */
    parentId?: string;
  }) => {
    const cwd = opts.cwd;
    let sessionId: string | undefined;
    try {
      sessionId = await window.electronAPI.spawnClaude({ cwd, provider: opts.provider, profileId: opts.profileId, model: opts.model, skipPermissions: opts.skipPermissions, mcpItemIds: opts.mcpItemIds, resumeSessionId: opts.resumeSessionId, supervisor: opts.supervisor, cols: 120, rows: 32 });
    } catch (err) {
      console.error('[Agent] spawn failed:', err);
    }
    const { tabs: agentTabs, activeTabId: agentActiveTab } = defaultAgentTabs(sessionId, cwd, opts.initialPrompt, opts.provider);
    const agent: AgentWorkspace = {
      // Deterministic id when we have a session, so every client converges on one
      // card for it; fall back to a random id only if the spawn failed.
      id: sessionId ? agentIdForSession(sessionId) : generateId('agent'),
      name: opts.name?.trim() || deriveAgentName(cwd),
      cwd,
      provider: opts.provider,
      profileId: opts.profileId,
      model: opts.model,
      skipPermissions: opts.skipPermissions,
      mcpItemIds: opts.mcpItemIds,
      sessionId,
      kind: opts.kind,
      parentId: opts.parentId,
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
    // Resume the prior conversation rather than starting blank: the id the agent
    // last held doubles as claude's transcript uuid (we pin `--session-id` at
    // spawn), so `--resume <id>` reopens it. `spawnClaude` returns that same id.
    const resumeSessionId = agent.lastSessionId;
    let sessionId: string | undefined;
    try {
      sessionId = await window.electronAPI.spawnClaude({ cwd: agent.cwd, provider: agent.provider, profileId: agent.profileId, model: agent.model, skipPermissions: agent.skipPermissions, mcpItemIds: agent.mcpItemIds, resumeSessionId, cols: 120, rows: 32 });
    } catch (err) {
      console.error('[Agent] respawn failed:', err);
    }
    if (!sessionId) return;
    const oldSession = agent.sessionId ?? agent.lastSessionId;
    mutateAgent(agentId, (a) => ({
      ...a,
      sessionId,
      lastSessionId: undefined,
      tabs: a.tabs.map((t) => ({
        ...t,
        panes: t.panes.map((p) => {
          if (p.type === 'claude' && (p.attachSessionId === oldSession || !p.attachSessionId)) {
            return { ...p, attachSessionId: sessionId, resumeSessionId: undefined };
          }
          // Agent-scope plugin panes carry the session in their webview URL —
          // re-resolve it to the fresh session so restored panes aren't stale.
          if (p.type === 'plugin' && typeof p.url === 'string' && p.url.includes('sessionId=')) {
            return { ...p, url: withAgentContext(p.url, sessionId, a.cwd) };
          }
          return p;
        }),
      })),
    }));
  }, [mutateAgent]);

  /**
   * Convenience wrapper to spawn a supervisor agent: derives a name from the
   * question, picks a sensible cwd, and calls `spawnAgent` with supervisor=true.
   * Returns the new agent id.
   */
  const spawnSupervisor = useCallback(async (opts: { question: string; parentId?: string; cwd?: string }): Promise<string> => {
    const name = deriveSupervisorName(opts.question);
    // A supervisor watches the whole fleet, so unless a cwd is given explicitly
    // it opens in its dedicated home (~/.workspacer) rather than inheriting some
    // agent's repo. Resolve it here so the card's cwd matches where the session
    // actually opens. parentId is kept only for UI nesting.
    let cwd = opts.cwd;
    if (!cwd) {
      try { cwd = await window.electronAPI.getSupervisorHome(); } catch { cwd = ''; }
    }
    return spawnAgent({
      cwd: cwd || '',
      name,
      kind: 'supervisor',
      parentId: opts.parentId,
      supervisor: true,
      initialPrompt: opts.question,
    });
  }, [spawnAgent]);

  /** Adopt a live daemon session that has no workspace yet (e.g. one spawned via
   *  the MCP facade / by another agent), so it appears as a card. Idempotent:
   *  does nothing if some workspace already owns this sessionId. Resolves nesting
   *  by matching parentSessionId to an existing agent's sessionId. */
  const adoptAgent = useCallback((opts: { sessionId: string; cwd: string; name?: string; parentSessionId?: string }) => {
    setAgents((prev) => {
      if (prev.some((a) => a.sessionId === opts.sessionId)) return prev; // already tracked — dedupe inside the updater (race-safe)
      const parent = opts.parentSessionId ? prev.find((a) => a.sessionId === opts.parentSessionId) : undefined;
      const { tabs, activeTabId } = defaultAgentTabs(opts.sessionId, opts.cwd);
      const agent: AgentWorkspace = {
        // Same deterministic id any other client would mint for this session, so
        // concurrent adoptions converge on one card instead of racing ids.
        id: agentIdForSession(opts.sessionId),
        name: opts.name?.trim() || deriveAgentName(opts.cwd),
        cwd: opts.cwd,
        sessionId: opts.sessionId,
        parentId: parent?.id,
        tabs,
        activeTabId,
      };
      return [...prev, agent];
    });
  }, []);

  /** Explicitly terminate an agent: kill its daemon session and drop it. */
  const terminateAgent = useCallback(async (agentId: string) => {
    const agent = agentsRef.current.find((a) => a.id === agentId);
    if (agent?.global) return; // the Overview workspace is permanent
    // Compute the post-removal list synchronously from `prev` so we never
    // read the stale agentsRef (which is updated asynchronously in an effect).
    let fallbackId: string | undefined;
    setAgents((prev) => {
      const next = prev.filter((a) => a.id !== agentId);
      fallbackId = next[0]?.id ?? '';
      return next;
    });
    setActiveAgentId((cur) => {
      if (cur !== agentId) return cur;
      return fallbackId ?? '';
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
      a.sessionId && !liveSessionIds.has(a.sessionId)
        ? { ...a, sessionId: undefined, lastSessionId: a.sessionId }
        : a,
    ));
  }, []);

  const loadAgentsFromSession = useCallback((sessionAgents: AgentWorkspace[], activeId: string) => {
    // Dedupe by sessionId on the way in: this is the merge point for every
    // cross-client layout update, so collapsing same-session cards here is what
    // stops the multi-client "spawn one, get seven" accumulation.
    const list = withGlobalWorkspace(dedupeBySessionId(sessionAgents));
    setAgents(list);
    // Choose an active id that actually survived dedupe. Both the caller's
    // activeId and the raw sessionAgents[0] can point at a same-session
    // duplicate that dedupe just dropped; selecting it would leave activeAgent
    // undefined and blank the workspace. Map such an id to the surviving card
    // for its session, else fall back to the first real agent (then Overview).
    const inList = (id?: string) => !!id && list.some((a) => a.id === id);
    const survivorIdFor = (id: string): string | undefined => {
      const raw = sessionAgents.find((a) => a.id === id);
      return raw?.sessionId ? list.find((a) => a.sessionId === raw.sessionId)?.id : undefined;
    };
    const preferred = activeId || sessionAgents[0]?.id || '';
    const chosenActiveId = inList(preferred)
      ? preferred
      : survivorIdFor(preferred) || list.find((a) => !a.global)?.id || list[0]?.id || '';
    setActiveAgentId(chosenActiveId);
    // Return the *normalized* layout (post dedupe/global-injection/active-id
    // resolution) so callers like useLayoutSync can record the echo-suppression
    // marker against what local state actually became — not the raw input,
    // which would otherwise look "changed" and bounce straight back to the hub.
    return { agents: list, activeAgentId: chosenActiveId };
  }, []);

  /** Open a pane in a specific workspace (agent or the global Overview) and
   *  switch to it. Used to place plugin/library panes by their declared scope.
   *  If the workspace already has a tab with the same pane type + title, it is
   *  focused instead of opening a duplicate. */
  const openPaneIn = useCallback((workspaceId: string, type: PaneType, title: string, url?: string, cwd?: string, pluginId?: string): void => {
    const paneId = generateId('pane');
    const tabId = generateId('tab');
    setAgents((prev) => withGlobalWorkspace(prev).map((a) => {
      if (a.id !== workspaceId) return a;
      const existing = a.tabs.find((t) => t.panes.length === 1 && t.panes[0].type === type && t.panes[0].title === title);
      if (existing) return { ...a, activeTabId: existing.id };
      const pane: PaneConfig = { id: paneId, type, title, url, cwd, appMode: true, pluginId };
      return { ...a, tabs: [...a.tabs, { id: tabId, title, panes: [pane], activePaneId: paneId, lastActiveAt: Date.now() }], activeTabId: tabId };
    }));
    setActiveAgentId(workspaceId);
  }, []);

  // ── Tab/pane operations (scoped to the active agent) ──────────────────────

  const setActiveTabId = useCallback((tabId: string) => {
    mutateActiveAgent((a) => ({
      ...a,
      activeTabId: tabId,
      // Record focus time (kept for potential recency features; the stacked
      // feed itself uses natural tab order, not activity).
      tabs: a.tabs.map((t) => (t.id === tabId ? { ...t, lastActiveAt: Date.now() } : t)),
    }));
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
    initialCommand?: string,
    filePath?: string,
  ) => {
    const aid = activeAgentIdRef.current;
    if (!aid) return '';
    const paneId = generateId(type);
    const tabId = generateId('tab');
    const paneTitle = title ?? defaultTitles[type];

    const pane: PaneConfig = {
      id: paneId, type, title: paneTitle, shell, url, appMode, cwd, profileId, resumeSessionId, attachSessionId, initialCommand, filePath,
    };
    const tab: TabConfig = { id: tabId, title: paneTitle, panes: [pane], activePaneId: paneId, lastActiveAt: Date.now() };

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
        t.id === tabId ? { ...t, panes: [...t.panes, pane], activePaneId: paneId, lastActiveAt: Date.now() } : t,
      ),
    }));
    return paneId;
  }, [mutateActiveAgent]);

  const removeTab = useCallback((tabId: string) => {
    // Closing the last tab terminates the (non-global) agent.
    const agent = agentsRef.current.find((a) => a.id === activeAgentIdRef.current);
    if (agent && !agent.global && agent.tabs.length <= 1 && agent.tabs.some((t) => t.id === tabId)) {
      terminateAgent(agent.id);
      return;
    }
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
  }, [mutateActiveAgent, terminateAgent]);

  const removePane = useCallback((tabId: string, paneId: string) => {
    // Closing the last pane of the last tab terminates the (non-global) agent.
    const agent = agentsRef.current.find((a) => a.id === activeAgentIdRef.current);
    const closingTab = agent?.tabs.find((t) => t.id === tabId);
    if (agent && !agent.global && closingTab && closingTab.panes.length <= 1 && agent.tabs.length <= 1) {
      terminateAgent(agent.id);
      return;
    }
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
  }, [mutateActiveAgent, terminateAgent]);

  const renameTab = useCallback((tabId: string, title: string) => {
    mutateActiveAgent((a) => ({
      ...a,
      tabs: a.tabs.map((t) => (t.id === tabId ? { ...t, title } : t)),
    }));
  }, [mutateActiveAgent]);

  const updateTabCanvas = useCallback((tabId: string, canvas: TabConfig['canvas']) => {
    mutateActiveAgent((a) => ({
      ...a,
      tabs: a.tabs.map((t) => (t.id === tabId ? { ...t, canvas } : t)),
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

  const updatePaneNotes = useCallback((tabId: string, paneId: string, notes: string) => {
    mutateActiveAgent((a) => ({
      ...a,
      tabs: a.tabs.map((t) =>
        t.id === tabId
          ? { ...t, panes: t.panes.map((p) => (p.id === paneId ? { ...p, notes } : p)) }
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
    spawnSupervisor,
    adoptAgent,
    respawnAgent,
    terminateAgent,
    renameAgent,
    reconcileAgents,
    loadAgentsFromSession,
    openPaneIn,
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
    updateTabCanvas,
    setActivePane,
    hibernatePane,
    wakePane,
    updatePaneUrl,
    updatePaneNotes,
    getActiveTab,
  };
}
