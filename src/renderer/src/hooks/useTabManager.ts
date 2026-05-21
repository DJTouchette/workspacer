import { useState, useCallback } from 'react';
import { PaneConfig, PaneType, TabConfig } from '../types/pane';

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

export const defaultTabs: TabConfig[] = [
  {
    id: 'tab-0',
    title: 'Dashboard',
    panes: [{ id: 'dashboard-1', type: 'dashboard', title: 'Dashboard' }],
    activePaneId: 'dashboard-1',
  },
];

export function useTabManager() {
  const [tabs, setTabs] = useState<TabConfig[]>([]);
  const [activeTabId, setActiveTabId] = useState<string>('');

  const loadFromSession = useCallback((sessionTabs: TabConfig[], activeId: string) => {
    setTabs(sessionTabs);
    setActiveTabId(activeId || sessionTabs[0]?.id || '');
  }, []);

  // Create a new tab with a single pane
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
    const paneId = generateId(type);
    const tabId = generateId('tab');
    const paneTitle = title ?? defaultTitles[type];

    const pane: PaneConfig = {
      id: paneId,
      type,
      title: paneTitle,
      shell,
      url,
      appMode,
      cwd,
      profileId,
      resumeSessionId,
      attachSessionId,
    };

    const tab: TabConfig = {
      id: tabId,
      title: paneTitle,
      panes: [pane],
      activePaneId: paneId,
    };

    setTabs((prev) => {
      if (insertPosition === 'after') {
        const activeIdx = prev.findIndex((t) => t.id === activeTabId);
        if (activeIdx >= 0) {
          const copy = [...prev];
          copy.splice(activeIdx + 1, 0, tab);
          return copy;
        }
      }
      return [...prev, tab];
    });
    setActiveTabId(tabId);
    return tabId;
  }, [activeTabId]);

  // Split the current tab by adding a sub-pane
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

    const pane: PaneConfig = {
      id: paneId,
      type,
      title: paneTitle,
      shell,
      url,
      appMode,
      cwd,
    };

    setTabs((prev) =>
      prev.map((t) => {
        if (t.id !== tabId) return t;
        return {
          ...t,
          panes: [...t.panes, pane],
          activePaneId: paneId,
        };
      })
    );
    return paneId;
  }, []);

  const removeTab = useCallback((tabId: string) => {
    setTabs((prev) => {
      const filtered = prev.filter((t) => t.id !== tabId);
      if (filtered.length === 0) return prev;

      setActiveTabId((current) => {
        if (current === tabId) {
          const idx = prev.findIndex((t) => t.id === tabId);
          const next = prev[idx + 1] ?? prev[idx - 1];
          return next ? next.id : prev[0].id;
        }
        return current;
      });

      return filtered;
    });
  }, []);

  // Remove a sub-pane from a tab. If it was the last pane, remove the tab.
  const removePane = useCallback((tabId: string, paneId: string) => {
    setTabs((prev) => {
      const tab = prev.find((t) => t.id === tabId);
      if (!tab) return prev;

      if (tab.panes.length <= 1) {
        // Last pane — remove the whole tab
        const filtered = prev.filter((t) => t.id !== tabId);
        if (filtered.length === 0) return prev;

        setActiveTabId((current) => {
          if (current === tabId) {
            const idx = prev.findIndex((t) => t.id === tabId);
            const next = prev[idx + 1] ?? prev[idx - 1];
            return next ? next.id : prev[0].id;
          }
          return current;
        });

        return filtered;
      }

      // Remove the sub-pane
      return prev.map((t) => {
        if (t.id !== tabId) return t;
        const newPanes = t.panes.filter((p) => p.id !== paneId);
        const newActive = t.activePaneId === paneId
          ? (newPanes[0]?.id || '')
          : t.activePaneId;
        return { ...t, panes: newPanes, activePaneId: newActive };
      });
    });
  }, []);

  const renameTab = useCallback((tabId: string, title: string) => {
    setTabs((prev) =>
      prev.map((t) => (t.id === tabId ? { ...t, title } : t))
    );
  }, []);

  const moveTab = useCallback((tabId: string, toIndex: number) => {
    setTabs((prev) => {
      const fromIndex = prev.findIndex((t) => t.id === tabId);
      if (fromIndex < 0) return prev;
      const clamped = Math.max(0, Math.min(toIndex, prev.length - 1));
      if (fromIndex === clamped) return prev;
      const copy = [...prev];
      const [moved] = copy.splice(fromIndex, 1);
      copy.splice(clamped, 0, moved);
      return copy;
    });
  }, []);

  const setActivePane = useCallback((tabId: string, paneId: string) => {
    setTabs((prev) =>
      prev.map((t) => (t.id === tabId ? { ...t, activePaneId: paneId } : t))
    );
  }, []);

  const hibernatePane = useCallback((tabId: string, paneId: string) => {
    setTabs((prev) =>
      prev.map((t) => {
        if (t.id !== tabId) return t;
        return { ...t, panes: t.panes.map((p) => (p.id === paneId ? { ...p, hibernated: true } : p)) };
      })
    );
  }, []);

  const wakePane = useCallback((tabId: string, paneId: string) => {
    setTabs((prev) =>
      prev.map((t) => {
        if (t.id !== tabId) return t;
        return { ...t, panes: t.panes.map((p) => (p.id === paneId ? { ...p, hibernated: false } : p)) };
      })
    );
  }, []);

  const updatePaneUrl = useCallback((tabId: string, paneId: string, url: string) => {
    setTabs((prev) =>
      prev.map((t) => {
        if (t.id !== tabId) return t;
        return { ...t, panes: t.panes.map((p) => (p.id === paneId ? { ...p, url } : p)) };
      })
    );
  }, []);

  // Helpers
  const getActiveTab = useCallback((): TabConfig | undefined => {
    return tabs.find((t) => t.id === activeTabId);
  }, [tabs, activeTabId]);

  return {
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
    loadFromSession,
    getActiveTab,
  };
}
