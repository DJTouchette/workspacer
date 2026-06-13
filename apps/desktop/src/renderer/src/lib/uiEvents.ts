/**
 * Publishes workspacer's own UI actions onto the hub bus so plugins (and, later,
 * MCP) can react to them — "a new pane opened", "focus changed to agent X", etc.
 * Namespaced under `ui.*`, source `workspacer.ui`. Fire-and-forget: if the hub
 * is down the event is simply dropped.
 */
export function emitUiEvent(type: string, data?: Record<string, unknown>): void {
  try {
    window.electronAPI.hubPublish?.({ type, source: 'workspacer.ui', data: data ?? {} });
  } catch {
    /* hub not connected — non-critical */
  }
}
