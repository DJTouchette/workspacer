import { ipcMain, BrowserWindow } from 'electron';
import { terminalService } from './services/terminalService';
import { configService } from './services/configService';

export function registerIpcHandlers(mainWindow: BrowserWindow): void {
  // Wire terminal service to use this window for push events
  terminalService.setMainWindow(mainWindow);

  // Terminal handlers
  ipcMain.handle('terminal:create', (_event, shell: string) => {
    return terminalService.createTerminal(shell);
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
}
