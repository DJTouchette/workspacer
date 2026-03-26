import { ipcMain, BrowserWindow, dialog } from 'electron';
import { terminalService } from './services/terminalService';
import { configService } from './services/configService';
import { sessionService } from './services/sessionService';
import { claudeSessionStore } from './services/claudeSessionStore';

export function registerIpcHandlers(mainWindow: BrowserWindow): void {
  // Wire terminal service to use this window for push events
  terminalService.setMainWindow(mainWindow);

  // Terminal handlers
  ipcMain.handle('terminal:create', (_event, shell: string, cwd?: string) => {
    try {
      return terminalService.createTerminal(shell, cwd);
    } catch (err: any) {
      console.error('[IPC] terminal:create failed:', err?.message);
      throw err;
    }
  });

  // Claude terminal — spawns claude CLI with headless mirroring
  ipcMain.handle('terminal:createClaude', (_event, cwd?: string) => {
    try {
      return terminalService.createClaudeTerminal(cwd);
    } catch (err: any) {
      console.error('[IPC] terminal:createClaude failed:', err?.message);
      throw err;
    }
  });

  // Claude session state queries
  ipcMain.handle('claude-session:getByPty', (_event, ptyId: string) => {
    return claudeSessionStore.getSnapshotByPty(ptyId);
  });

  ipcMain.handle('claude-session:getAll', () => {
    return claudeSessionStore.getAllSnapshots();
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
    // Enrich terminal panes with CWD within tabs
    const ptyMapping = data.ptyMapping || {};
    const enrichedTabs = (data.tabs || []).map((tab: any) => ({
      ...tab,
      panes: sessionService.enrichPanesWithCwd(tab.panes || [], ptyMapping),
    }));

    return sessionService.saveSession({
      name: data.name,
      timestamp: new Date().toISOString(),
      activeTabId: data.activeTabId,
      tabs: enrichedTabs,
    });
  });

  ipcMain.handle('session:delete', (_event, filename: string) => {
    sessionService.deleteSession(filename);
  });

  // App info
  ipcMain.handle('app:getCwd', () => {
    return process.cwd();
  });

  // Dialog
  ipcMain.handle('dialog:pickFolder', async (_event, defaultPath?: string) => {
    const result = await dialog.showOpenDialog(mainWindow, {
      title: 'Choose working directory for Claude',
      defaultPath: defaultPath || process.cwd(),
      properties: ['openDirectory'],
    });
    if (result.canceled || result.filePaths.length === 0) return null;
    return result.filePaths[0];
  });
}
