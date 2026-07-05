import { useEffect, useRef } from 'react';
import type { AgentWorkspace } from '../types/pane';
import { emitUiEvent } from '../lib/uiEvents';

interface PaneInfo {
  type: string;
  title: string;
  workspaceId: string;
  tabId: string;
  /** Editor panes only: the file being edited, so plugins can react to it. */
  filePath?: string;
}

function flattenPanes(agents: AgentWorkspace[]): Map<string, PaneInfo> {
  const m = new Map<string, PaneInfo>();
  for (const a of agents) {
    for (const t of a.tabs) {
      for (const p of t.panes) {
        m.set(p.id, {
          type: p.type,
          title: p.title,
          workspaceId: a.id,
          tabId: t.id,
          filePath: p.filePath,
        });
      }
    }
  }
  return m;
}

/**
 * Single source of UI events. By diffing the whole workspace tree (+ the active
 * ids) it derives pane open/close and workspace/tab/pane focus changes without
 * having to instrument every action site. The first run seeds state silently so
 * a restored session doesn't replay as a burst of "opened" events.
 */
export function useUiEventBus(agents: AgentWorkspace[], activeAgentId: string): void {
  const prevPanes = useRef<Map<string, PaneInfo> | null>(null);
  const prevFocus = useRef<{ ws: string; tab: string; pane: string } | null>(null);

  // Pane lifecycle via tree diff.
  useEffect(() => {
    const cur = flattenPanes(agents);
    const prev = prevPanes.current;
    if (prev) {
      for (const [id, info] of cur) {
        if (!prev.has(id)) {
          emitUiEvent('ui.pane.opened', {
            paneId: id,
            type: info.type,
            title: info.title,
            workspaceId: info.workspaceId,
            tabId: info.tabId,
            ...(info.filePath ? { filePath: info.filePath } : {}),
          });
        }
      }
      for (const [id, info] of prev) {
        if (!cur.has(id)) {
          emitUiEvent('ui.pane.closed', {
            paneId: id,
            type: info.type,
            workspaceId: info.workspaceId,
            ...(info.filePath ? { filePath: info.filePath } : {}),
          });
        }
      }
    }
    prevPanes.current = cur;
  }, [agents]);

  // Focus changes: workspace → tab → pane.
  useEffect(() => {
    const a = agents.find((x) => x.id === activeAgentId);
    const tab = a?.tabs.find((t) => t.id === a.activeTabId);
    const ws = activeAgentId;
    const tabId = a?.activeTabId ?? '';
    const paneId = tab?.activePaneId ?? '';
    const prev = prevFocus.current;

    if (prev) {
      if (ws && ws !== prev.ws) emitUiEvent('ui.workspace.focused', { workspaceId: ws });
      if (tabId && tabId !== prev.tab) emitUiEvent('ui.tab.focused', { tabId, workspaceId: ws });
      if (paneId && paneId !== prev.pane) {
        const pane = tab?.panes.find((p) => p.id === paneId);
        emitUiEvent('ui.pane.focused', {
          paneId,
          type: pane?.type,
          workspaceId: ws,
          tabId,
          ...(pane?.filePath ? { filePath: pane.filePath } : {}),
        });
      }
    }
    prevFocus.current = { ws, tab: tabId, pane: paneId };
  }, [agents, activeAgentId]);
}
