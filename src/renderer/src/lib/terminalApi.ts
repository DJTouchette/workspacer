export function CreateTerminal(shell: string, cwd?: string, cols?: number, rows?: number): Promise<string> {
  return window.electronAPI.createTerminal(shell, cwd, cols, rows);
}

/** Send input to terminal via MessagePort (fire-and-forget, no IPC round-trip) */
export function WriteTerminal(id: string, data: string): void {
  window.electronAPI.writeTerminal(id, data);
}

export function ResizeTerminal(id: string, cols: number, rows: number): Promise<void> {
  return window.electronAPI.resizeTerminal(id, cols, rows);
}

export function CloseTerminal(id: string): Promise<void> {
  return window.electronAPI.closeTerminal(id);
}
