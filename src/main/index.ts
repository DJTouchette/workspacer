import { app, BrowserWindow, Menu } from 'electron';
import * as path from 'path';
import { registerIpcHandlers } from './ipc';
import { terminalService } from './services/terminalService';

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
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      webviewTag: true,
    },
  });

  registerIpcHandlers(mainWindow);

  if (process.env.ELECTRON_DEV) {
    mainWindow.loadURL('http://localhost:5173');
    mainWindow.webContents.openDevTools({ mode: 'detach' });
  } else {
    mainWindow.loadFile(path.join(__dirname, '../renderer/index.html'));
  }

  mainWindow.on('closed', () => {
    mainWindow = null;
  });
}

app.whenReady().then(createWindow);

app.on('window-all-closed', () => {
  terminalService.closeAll();
  app.quit();
});

app.on('activate', () => {
  if (mainWindow === null) {
    createWindow();
  }
});
