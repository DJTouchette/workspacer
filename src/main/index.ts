import { app, BrowserWindow, Menu } from 'electron';
import * as path from 'path';
import * as fs from 'fs';
import * as os from 'os';
import { registerIpcHandlers } from './ipc';
import { terminalService } from './services/terminalService';
import { claudeSessionStore } from './services/claudeSessionStore';
import { startHookServer } from './services/hookServer';
import { installHooks, uninstallHooks } from './services/claudeHooksConfig';

/** Discover Nerd Font Mono files and inject @font-face CSS into the renderer */
function injectNerdFonts(win: BrowserWindow): void {
  const fontDirs = process.platform === 'win32'
    ? [
        path.join(os.homedir(), 'AppData', 'Local', 'Microsoft', 'Windows', 'Fonts'),
        'C:\\Windows\\Fonts',
      ]
    : [
        path.join(os.homedir(), '.local', 'share', 'fonts'),
        path.join(os.homedir(), '.fonts'),
        '/usr/share/fonts',
        '/usr/local/share/fonts',
      ];

  let css = '';
  for (const dir of fontDirs) {
    try {
      if (!fs.existsSync(dir)) continue;
      for (const file of fs.readdirSync(dir)) {
        if (!/NerdFont.*-Regular\.(ttf|otf)$/i.test(file)) continue;
        const fullPath = path.join(dir, file);
        const family = file
          .replace(/-Regular\.(ttf|otf)$/i, '')
          .replace(/NerdFontMono/, ' Nerd Font Mono')
          .replace(/NerdFontPropo/, ' Nerd Font Propo')
          .replace(/NerdFont/, ' Nerd Font')
          .replace(/  +/g, ' ')
          .trim();
        try {
          const data = fs.readFileSync(fullPath);
          const b64 = data.toString('base64');
          const mime = file.endsWith('.otf') ? 'font/otf' : 'font/ttf';
          css += `@font-face { font-family: "${family}"; src: url(data:${mime};base64,${b64}) format('${file.endsWith('.otf') ? 'opentype' : 'truetype'}'); }\n`;
          // Also register without "NL" so "JetBrainsMono Nerd Font Mono" matches NL variant
          const generic = family.replace(/NL\s*/g, '');
          if (generic !== family) {
            css += `@font-face { font-family: "${generic}"; src: url(data:${mime};base64,${b64}) format('${file.endsWith('.otf') ? 'opentype' : 'truetype'}'); }\n`;
          }
          console.log(`[Fonts] registered: "${family}"`);
        } catch {}
      }
    } catch {}
  }

  if (css) {
    win.webContents.insertCSS(css).then(() => {
      console.log(`[Fonts] injected ${css.split('@font-face').length - 1} @font-face rules`);
    });
  }
}

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

  // Inject Nerd Font @font-face rules once the page DOM is ready
  mainWindow.webContents.on('dom-ready', () => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      injectNerdFonts(mainWindow);
    }
  });

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
