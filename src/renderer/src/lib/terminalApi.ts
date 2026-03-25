export function CreateTerminal(shell: string): Promise<string> {
  return window.electronAPI.createTerminal(shell);
}

export function WriteTerminal(id: string, data: string): Promise<void> {
  return window.electronAPI.writeTerminal(id, data);
}

export function ResizeTerminal(id: string, cols: number, rows: number): Promise<void> {
  return window.electronAPI.resizeTerminal(id, cols, rows);
}

export function CloseTerminal(id: string): Promise<void> {
  return window.electronAPI.closeTerminal(id);
}
