import { app, BrowserWindow, Menu, protocol, net } from 'electron';
import * as path from 'path';
import * as fs from 'fs';
import * as os from 'os';
import { registerIpcHandlers } from './ipc';
import { terminalService } from './services/terminalService';
import { claudeSessionStore } from './services/claudeSessionStore';
import { startHookServer } from './services/hookServer';
import { installHooks, uninstallHooks } from './services/claudeHooksConfig';

// Font file registry: filename → absolute path (populated during discovery)
const fontFileMap = new Map<string, string>();

function discoverFontDirs(): string[] {
  return process.platform === 'win32'
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
}

/** Discover Nerd Font files and inject @font-face CSS using workspacer-font:// protocol */
function injectNerdFonts(win: BrowserWindow): void {
  let css = '';
  for (const dir of discoverFontDirs()) {
    try {
      if (!fs.existsSync(dir)) continue;
      for (const file of fs.readdirSync(dir)) {
        if (!/NerdFont.*-Regular\.(ttf|otf)$/i.test(file)) continue;
        const fullPath = path.join(dir, file);
        fontFileMap.set(file, fullPath);
        const family = file
          .replace(/-Regular\.(ttf|otf)$/i, '')
          .replace(/NerdFontMono/, ' Nerd Font Mono')
          .replace(/NerdFontPropo/, ' Nerd Font Propo')
          .replace(/NerdFont/, ' Nerd Font')
          .replace(/  +/g, ' ')
          .trim();
        const fmt = file.endsWith('.otf') ? 'opentype' : 'truetype';
        css += `@font-face { font-family: "${family}"; src: url("workspacer-font://${encodeURIComponent(file)}") format('${fmt}'); font-display: block; }\n`;
        const generic = family.replace(/NL\s*/g, '');
        if (generic !== family) {
          css += `@font-face { font-family: "${generic}"; src: url("workspacer-font://${encodeURIComponent(file)}") format('${fmt}'); font-display: block; }\n`;
        }
        console.log(`[Fonts] registered: "${family}" → ${file}`);
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

// Register custom protocol to serve local font files to the renderer
protocol.registerSchemesAsPrivileged([
  { scheme: 'workspacer-font', privileges: { standard: false, supportFetchAPI: true, corsEnabled: true, bypassCSP: true } },
]);

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

app.whenReady().then(() => {
  // Register protocol handler to serve local font files
  protocol.handle('workspacer-font', (request) => {
    const filename = decodeURIComponent(request.url.replace('workspacer-font://', ''));
    const filePath = fontFileMap.get(filename);
    if (filePath && fs.existsSync(filePath)) {
      const data = fs.readFileSync(filePath);
      const mime = filename.endsWith('.otf') ? 'font/otf' : 'font/ttf';
      return new Response(data, { headers: { 'Content-Type': mime, 'Access-Control-Allow-Origin': '*' } });
    }
    return new Response('Not found', { status: 404 });
  });
  createWindow();
});

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
