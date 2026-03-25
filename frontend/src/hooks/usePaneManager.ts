import { useState, useCallback } from 'react';
import { PaneConfig, PaneType } from '../types/pane';

const DEFAULT_PANE_WIDTH = 800;

const defaultPanes: PaneConfig[] = [
  { id: 'terminal-1', type: 'terminal', title: 'Terminal 1', width: DEFAULT_PANE_WIDTH },
  { id: 'terminal-2', type: 'terminal', title: 'Terminal 2', width: DEFAULT_PANE_WIDTH },
  { id: 'terminal-3', type: 'terminal', title: 'Terminal 3', width: DEFAULT_PANE_WIDTH },
  { id: 'notes-1', type: 'notes', title: 'Notes', width: DEFAULT_PANE_WIDTH },
];

let nextId = 1;

function generateId(type: PaneType): string {
  nextId += 1;
  return `${type}-${Date.now()}-${nextId}`;
}

export function usePaneManager() {
  const [panes, setPanes] = useState<PaneConfig[]>(defaultPanes);
  const [activePaneId, setActivePaneId] = useState<string>(defaultPanes[0].id);

  const addPane = useCallback((type: PaneType, title?: string, width?: number, insertPosition: string = 'after', shell?: string) => {
    const id = generateId(type);
    const defaultTitles: Record<PaneType, string> = {
      terminal: 'Terminal',
      browser: 'Browser',
      notes: 'Notes',
      agent: 'Agent',
    };
    const newPane: PaneConfig = {
      id,
      type,
      title: title ?? defaultTitles[type],
      width: width ?? DEFAULT_PANE_WIDTH,
      shell,
    };
    setPanes((prev) => {
      if (insertPosition === 'after') {
        const activeIdx = prev.findIndex((p) => p.id === activePaneId);
        if (activeIdx >= 0) {
          const copy = [...prev];
          copy.splice(activeIdx + 1, 0, newPane);
          return copy;
        }
      }
      return [...prev, newPane];
    });
    setActivePaneId(id);
    return id;
  }, [activePaneId]);

  const removePane = useCallback((id: string) => {
    setPanes((prev) => {
      const filtered = prev.filter((p) => p.id !== id);
      if (filtered.length === 0) return prev; // don't remove the last pane
      return filtered;
    });
    setActivePaneId((currentActive) => {
      if (currentActive === id) {
        // find a neighbor to activate
        const idx = panes.findIndex((p) => p.id === id);
        const next = panes[idx + 1] ?? panes[idx - 1];
        return next ? next.id : panes[0].id;
      }
      return currentActive;
    });
  }, [panes]);

  const resizePane = useCallback((id: string, width: number) => {
    setPanes((prev) =>
      prev.map((p) => (p.id === id ? { ...p, widthOverride: width } : p))
    );
  }, []);

  const resetPaneWidth = useCallback((id: string) => {
    setPanes((prev) =>
      prev.map((p) => (p.id === id ? { ...p, widthOverride: undefined } : p))
    );
  }, []);

  const movePane = useCallback((id: string, toIndex: number) => {
    setPanes((prev) => {
      const fromIndex = prev.findIndex((p) => p.id === id);
      if (fromIndex < 0) return prev;
      const clamped = Math.max(0, Math.min(toIndex, prev.length - 1));
      if (fromIndex === clamped) return prev;
      const copy = [...prev];
      const [moved] = copy.splice(fromIndex, 1);
      copy.splice(clamped, 0, moved);
      return copy;
    });
  }, []);

  return {
    panes,
    addPane,
    removePane,
    resizePane,
    resetPaneWidth,
    movePane,
    activePaneId,
    setActivePaneId,
  };
}
