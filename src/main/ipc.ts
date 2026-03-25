import { ipcMain, BrowserWindow } from 'electron';
import { terminalService } from './services/terminalService';
import { configService } from './services/configService';
import { sessionService } from './services/sessionService';

export function registerIpcHandlers(mainWindow: BrowserWindow): void {
  // Wire terminal service to use this window for push events
  terminalService.setMainWindow(mainWindow);

  // Terminal handlers
  ipcMain.handle('terminal:create', (_event, shell: string, cwd?: string) => {
    return terminalService.createTerminal(shell, cwd);
  });

  ipcMain.handle('terminal:write', (_event, id: string, data: string) => {
    terminalService.writeTerminal(id, data);
  });

  ipcMain.handle('terminal:resize', (_event, id: string, cols: number, rows: number) => {
    terminalService.resizeTerminal(id, cols, rows);
  });

  ipcMain.handle('terminal:close', (_event, id: string) => {
    terminalService.closeTerminal(id);
  });

  // Config handlers
  ipcMain.handle('config:get', () => {
    return configService.getConfig();
  });

  ipcMain.handle('config:reload', () => {
    return configService.reloadConfig();
  });

  ipcMain.handle('config:getPath', () => {
    return configService.getConfigPath();
  });

  ipcMain.handle('config:save', (_event, partial: unknown) => {
    return configService.saveConfig(partial as any);
  });

  // Session handlers
  ipcMain.handle('session:list', () => {
    return sessionService.listSessions();
  });

  ipcMain.handle('session:load', (_event, filename: string) => {
    return sessionService.loadSession(filename);
  });

  ipcMain.handle('session:save', (_event, data: any) => {
    const enrichedPanes = sessionService.enrichPanesWithCwd(data.panes, data.ptyMapping || {});
    return sessionService.saveSession({
      name: data.name,
      timestamp: new Date().toISOString(),
      activePaneId: data.activePaneId,
      panes: enrichedPanes,
    });
  });

  ipcMain.handle('session:delete', (_event, filename: string) => {
    sessionService.deleteSession(filename);
  });
}
