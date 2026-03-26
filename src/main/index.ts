import { app, BrowserWindow, Menu } from 'electron';
import * as path from 'path';
import { registerIpcHandlers } from './ipc';
import { terminalService } from './services/terminalService';
import { claudeSessionStore } from './services/claudeSessionStore';
import { startHookServer } from './services/hookServer';
import { installHooks, uninstallHooks } from './services/claudeHooksConfig';

let mainWindow: BrowserWindow | null = null;

function createWindow(): void {
  // Remove default menu to prevent Ctrl+T/W conflicts
  Menu.setApplicationMenu(null);

  mainWindow = new BrowserWindow({
    width: 1600,
    height: 900,
    minWidth: 800,
    minHeight: 600,
    backgroundColor: '#1b2636',
    frame: false,
    titleBarStyle: 'hidden',
    titleBarOverlay: process.platform === 'win32' ? {
      color: '#141418',
      symbolColor: '#c8c8d2',
      height: 28,
    } : undefined,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      webviewTag: true,
    },
  });

  registerIpcHandlers(mainWindow);
  claudeSessionStore.setMainWindow(mainWindow);
  startHookServer();
  installHooks();

  if (process.env.ELECTRON_DEV) {
    mainWindow.loadURL('http://localhost:5173');
  } else {
    mainWindow.loadFile(path.join(__dirname, '../renderer/index.html'));
  }

  mainWindow.on('closed', () => {
    mainWindow = null;
  });
}

app.whenReady().then(createWindow);

app.on('before-quit', () => {
  // Signal renderer to save session before quit
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.webContents.send('app:before-quit');
  }
});

app.on('window-all-closed', () => {
  terminalService.closeAll();
  uninstallHooks();
  app.quit();
});

app.on('activate', () => {
  if (mainWindow === null) {
    createWindow();
  }
});
