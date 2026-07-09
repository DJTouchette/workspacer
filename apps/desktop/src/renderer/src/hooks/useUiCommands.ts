import { useEffect, useRef } from 'react';

/**
 * Handlers the UI exposes to bus commands. The renderer already receives every
 * bus event (via onHubEvent), so we just listen for the `command.*` namespace
 * and dispatch. The `ui.*` event each action emits is the implicit confirmation,
 * so commands stay fire-and-forget.
 *
 * Any bus participant (plugin, or later MCP/Claude) can drive the UI by
 * publishing e.g. { type: 'command.open_plugin', data: { type: 'workspacer.agent-dashboard' } }.
 */
export interface UiCommandHandlers {
  focusAgent: (idOrSession: string) => void;
  spawnAgent: (opts: { cwd?: string; name?: string; model?: string }) => void;
  /** Open the new-agent view pre-filled (does not spawn until confirmed). */
  openSpawnDialog: (opts: { cwd?: string }) => void;
  openPane: (paneType: string, opts?: { cwd?: string }) => void;
  openPlugin: (paneType: string) => void;
  closePane: (paneId: string) => void;
  /** Open the Ask pane in the global workspace. */
  openAskPane?: () => void;
}

export function useUiCommands(handlers: UiCommandHandlers): void {
  const ref = useRef(handlers);
  ref.current = handlers;

  useEffect(() => {
    const off = window.electronAPI.onHubEvent?.((ev) => {
      if (!ev.type?.startsWith('command.')) return;
      const d = (ev.data ?? {}) as Record<string, any>;
      switch (ev.type) {
        case 'command.focus_agent':
          if (d.agentId || d.sessionId) ref.current.focusAgent(d.agentId ?? d.sessionId);
          break;
        case 'command.spawn_agent':
          ref.current.spawnAgent({ cwd: d.cwd, name: d.name, model: d.model });
          break;
        case 'command.open_spawn_dialog':
          ref.current.openSpawnDialog({ cwd: d.cwd });
          break;
        case 'command.open_pane':
          if (d.paneType) ref.current.openPane(d.paneType, { cwd: d.cwd });
          break;
        case 'command.open_plugin':
          if (d.type) ref.current.openPlugin(d.type);
          break;
        case 'command.close_pane':
          if (d.paneId) ref.current.closePane(d.paneId);
          break;
        case 'command.open_ask_pane':
          ref.current.openAskPane?.();
          break;
        default:
          break;
      }
    });
    return () => off?.();
  }, []);
}
