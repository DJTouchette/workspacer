import { app, BrowserWindow, Menu, protocol, net, session } from 'electron';
import * as path from 'path';
import * as fs from 'fs';
import * as os from 'os';
import { registerIpcHandlers } from './ipc';
import { getConfigDir } from './services/configService';
import { claudeSessionStore } from './services/claudeSessionStore';
import { agentNotifier } from './services/agentNotifier';
import { claudemonSessionClient } from './services/claudemonSessionClient';
import { startClaudemon, stopClaudemon, runClaudemonInit } from './services/claudemonDaemon';
import { startClaudemonHookBridge, stopClaudemonHookBridge } from './services/claudemonHookBridge';
import { startClaudemonStatusLineBridge, stopClaudemonStatusLineBridge } from './services/claudemonStatusLineBridge';
import { startClaudemonConversationBridge, stopClaudemonConversationBridge } from './services/claudemonConversationBridge';
import { startHub, stopHub } from './services/hubDaemon';
import { startMcpFacade, stopMcpFacade } from './services/mcpFacadeDaemon';
import { setHubMainWindow, startHubClient, stopHubClient } from './services/hubClient';
import { stopAllTerminals } from './services/terminalShare';
import { workflowWatcher } from './services/workflowWatcher';
import { registerHubCapabilities } from './services/hubCapabilities';
import { database } from './services/db';
import { appIcon } from './lib/appIcon';
import { IPC } from './shared/ipcChannels';

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

// Font cache: persists discovered font file→path mappings so we skip
// filesystem scanning on subsequent launches.
const fontCachePath = path.join(getConfigDir(), '.font-cache.json');

interface FontCacheEntry { file: string; fullPath: string; mtime: number }

function loadFontCache(): FontCacheEntry[] | null {
  try {
    if (!fs.existsSync(fontCachePath)) return null;
    const data = JSON.parse(fs.readFileSync(fontCachePath, 'utf-8'));
    if (!Array.isArray(data.fonts)) return null;
    // Validate that cached paths still exist (quick stat check on first entry)
    if (data.fonts.length > 0 && !fs.existsSync(data.fonts[0].fullPath)) return null;
    return data.fonts as FontCacheEntry[];
  } catch {
    return null;
  }
}

function saveFontCache(entries: FontCacheEntry[]): void {
  try {
    const dir = path.dirname(fontCachePath);
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(fontCachePath, JSON.stringify({ fonts: entries }, null, 2));
  } catch {}
}

/** Discover Nerd Font files (with disk cache) and inject @font-face CSS.
 *  Runs fully async so font-dir scanning (incl. /usr/share/fonts) never
 *  blocks the main thread on first launch. */
async function injectNerdFonts(win: BrowserWindow): Promise<void> {
  let entries: FontCacheEntry[];
  const cached = loadFontCache();

  if (cached) {
    entries = cached;
    console.log(`[Fonts] loaded ${entries.length} fonts from cache`);
  } else {
    entries = [];
    for (const dir of discoverFontDirs()) {
      try {
        await fs.promises.access(dir).catch(() => { throw new Error('skip'); });
        const files = await fs.promises.readdir(dir);
        for (const file of files) {
          if (!/NerdFont.*-Regular\.(ttf|otf)$/i.test(file)) continue;
          const fullPath = path.join(dir, file);
          try {
            const stat = await fs.promises.stat(fullPath);
            entries.push({ file, fullPath, mtime: stat.mtimeMs });
          } catch {}
        }
      } catch {}
    }
    saveFontCache(entries);
    console.log(`[Fonts] discovered ${entries.length} fonts, cached to disk`);
  }

  let css = '';
  for (const { file, fullPath } of entries) {
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
  }

  if (css && !win.isDestroyed()) {
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

// ── Chromium performance flags (must be set before app.whenReady) ──

// Cap renderer heap to avoid runaway memory (512 MB per renderer)
app.commandLine.appendSwitch('js-flags', '--max-old-space-size=512');
// Background timer throttling is intentionally left ENABLED (the default).
// Main-process daemons and SSE bridges are unaffected (they run in the Node.js
// event loop, not in Chromium renderer timers). The renderer self-throttles via
// usePageVisible; disabling Chromium's throttling kept hidden windows fully awake
// and wasted CPU on an otherwise mostly-idle app.
// Disable renderer code integrity checks for faster startup on Windows
if (process.platform === 'win32') {
  app.commandLine.appendSwitch('disable-features', 'RendererCodeIntegrity');
}
// Enable V8 code caching for faster subsequent launches
app.commandLine.appendSwitch('v8-cache-options', 'code');

// Some Linux GPUs (notably hybrid Intel+AMD/NVIDIA laptops under Wayland)
// render the Chromium surface as garbage — speckled "dots everywhere"
// corruption. Let those machines opt out of GPU rasterization without
// affecting single-GPU machines that render fine. Set WORKSPACER_DISABLE_GPU=1.
if (process.env.WORKSPACER_DISABLE_GPU === '1') {
  app.disableHardwareAcceleration();
  app.commandLine.appendSwitch('disable-gpu-compositing');
}

// Spoof a plain Chrome user-agent for every webContents/webview/session.
// Microsoft sign-in (and Google's, and a few others) refuse to OAuth into
// embedded web views — they sniff for "Electron/x.y.z" or app-name tokens
// in the UA string. Stripping those makes the in-app browser look like a
// regular Chrome install.
function buildChromeUserAgent(): string {
  // Take Electron's UA, drop our app token and the "Electron/X" segment.
  const original = app.userAgentFallback || '';
  return original
    .replace(/\sworkspacer\/\S+/i, '')
    .replace(/\sElectron\/\S+/i, '');
}
app.userAgentFallback = buildChromeUserAgent();

function createWindow(): void {
  // Remove default menu to prevent Ctrl+T/W conflicts
  Menu.setApplicationMenu(null);

  // Transparent shell so the renderer can paint its own rounded corners (the
  // app-root is clipped to a radius). Skipped on Windows, where the native
  // titleBarOverlay window controls need an opaque frame.
  const transparentShell = process.platform !== 'win32';

  // Window/taskbar icon on Linux & Windows (macOS uses the bundle icon).
  const windowIcon = appIcon();

  mainWindow = new BrowserWindow({
    width: 1600,
    height: 900,
    minWidth: 800,
    minHeight: 600,
    ...(windowIcon ? { icon: windowIcon } : {}),
    backgroundColor: transparentShell ? '#00000000' : '#1b2636',
    transparent: transparentShell,
    frame: false,
    titleBarStyle: 'hidden',
    // Initial colors approximate the default dark theme's title bar; the
    // renderer repaints this to the active theme on mount (window:setOverlay).
    titleBarOverlay: process.platform === 'win32' ? {
      color: '#1f1e22',
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
  agentNotifier.setMainWindow(mainWindow);
  setHubMainWindow(mainWindow);

  // claudemon daemon owns hook ingestion + transcript parsing. We spawn it,
  // run `claudemon init` to merge our hooks into ~/.claude/settings.json,
  // then subscribe to its /hooks/stream SSE feed.
  startClaudemon()
    .then(async () => {
      try {
        await runClaudemonInit();
      } catch (err) {
        console.error('[main] claudemon init failed:', err);
      }
      startClaudemonHookBridge().catch(err =>
        console.error('[main] hook bridge crashed:', err)
      );
      startClaudemonStatusLineBridge().catch(err =>
        console.error('[main] statusline bridge crashed:', err)
      );
      startClaudemonConversationBridge().catch(err =>
        console.error('[main] conversation bridge crashed:', err)
      );
      // Hub (control-plane / event bus) bridges claudemon onto its bus; the
      // main process connects as a client, forwards events to the renderer, and
      // registers the capabilities plugins/MCP can call. Started after claudemon
      // so the bridge has a live /events source.
      registerHubCapabilities();
      startHub()
        .then(() => {
          startHubClient();
          // The MCP facade bridges hub capabilities to MCP tools for supervisor
          // sessions. Started after the hub so its bus connection has a target.
          // Optional: a failure only costs the supervisor its action tools.
          startMcpFacade().catch(err =>
            console.error('[main] failed to start mcp facade — supervisors will lack action tools:', err)
          );
        })
        .catch(err => console.error('[main] failed to start hub:', err));
    })
    .catch(err => {
      console.error('[main] failed to start claudemon — Claude sessions will not get hook events:', err);
    });

  // Prevent Electron from navigating to dropped files
  mainWindow.webContents.on('will-navigate', (event) => { event.preventDefault(); });

  if (process.env.ELECTRON_DEV) {
    mainWindow.loadURL('http://localhost:5173');
    // Auto-open DevTools in dev — menu is nulled so the default Ctrl+Shift+I
    // binding is gone, and a renderer crash would otherwise leave a blank
    // window with no way to inspect it.
    mainWindow.webContents.openDevTools({ mode: 'detach' });
  } else {
    mainWindow.loadFile(path.join(__dirname, '../renderer/index.html'));
  }

  // Always-available DevTools shortcuts (work even after the menu is nulled).
  mainWindow.webContents.on('before-input-event', (event, input) => {
    if (input.type !== 'keyDown') return;
    const isToggle =
      (input.control && input.shift && input.key.toLowerCase() === 'i') ||
      input.key === 'F12';
    if (isToggle) {
      mainWindow!.webContents.toggleDevTools();
      event.preventDefault();
    }
  });

  // Inject Nerd Font @font-face rules once the page DOM is ready.
  // Invoked non-blocking — async font-dir scanning must not stall the main thread.
  mainWindow.webContents.on('dom-ready', () => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      injectNerdFonts(mainWindow).catch((err) =>
        console.error('[Fonts] discovery failed:', err)
      );
    }
  });

  mainWindow.on('closed', () => {
    mainWindow = null;
  });
}

app.whenReady().then(() => {
  // Dock/taskbar icon. The BrowserWindow `icon` option covers the window icon
  // on Linux/Windows, but the macOS dock ignores it (it reads the app bundle,
  // which is Electron's default in dev), and Windows needs an explicit
  // AppUserModelId so the taskbar shows our icon instead of grouping under the
  // generic electron.exe one.
  if (process.platform === 'win32') {
    app.setAppUserModelId('com.workspacer.app');
  } else if (process.platform === 'darwin') {
    const dockIcon = appIcon();
    if (dockIcon && app.dock) {
      app.dock.setIcon(dockIcon);
    }
  }

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

  // Also apply the Chrome UA at the session level — `app.userAgentFallback`
  // alone doesn't cover every request, and Microsoft is picky. BrowserPane's
  // webview lives in the `persist:browser` partition; set the UA there too
  // so cookies persist across restarts AND OAuth doesn't trip the embedded
  // webview sniffer.
  const chromeUA = buildChromeUserAgent();
  session.defaultSession.setUserAgent(chromeUA);
  const browserSession = session.fromPartition('persist:browser');
  browserSession.setUserAgent(chromeUA);

  // Aggressive fingerprint spoof for the browser partition: rewrite the
  // `Sec-CH-UA*` Client Hints headers so they describe Chrome cleanly,
  // strip headers that mention Electron, and rewrite Origin/Referer rules
  // so MS sign-in's secondary checks pass. Pulled the brand version out of
  // the spoofed UA so the two stay consistent.
  const chromeVersionMatch = chromeUA.match(/Chrome\/(\d+)/);
  const chromeVersion = chromeVersionMatch ? chromeVersionMatch[1] : '130';
  const secChUa = `"Chromium";v="${chromeVersion}", "Not?A_Brand";v="99", "Google Chrome";v="${chromeVersion}"`;

  for (const sess of [session.defaultSession, browserSession]) {
    sess.webRequest.onBeforeSendHeaders((details, callback) => {
      const headers = { ...details.requestHeaders };
      // Always drop anything Electron-y from any header that takes a UA string.
      for (const k of Object.keys(headers)) {
        if (/electron|workspacer/i.test(String(headers[k]))) {
          headers[k] = String(headers[k]).replace(/\sElectron\/\S+/i, '').replace(/\sworkspacer\/\S+/i, '');
        }
      }
      // Standardise Client Hints to look like real Chrome.
      headers['Sec-CH-UA'] = secChUa;
      headers['Sec-CH-UA-Mobile'] = '?0';
      headers['Sec-CH-UA-Platform'] = process.platform === 'darwin' ? '"macOS"' :
                                       process.platform === 'win32' ? '"Windows"' : '"Linux"';
      // User-Agent itself (belt-and-suspenders — the session-level UA usually
      // sets this, but in some edge cases requests slip through with the old UA).
      headers['User-Agent'] = chromeUA;
      callback({ requestHeaders: headers });
    });
  }

  // Apply the Chrome UA to every webview as it's created. Don't install a
  // window-open handler — `allowpopups="true"` on the <webview> tag already
  // handles popups natively with the same partition, and intercepting via
  // setWindowOpenHandler was aborting unrelated navigations.
  app.on('web-contents-created', (_event, contents) => {
    if (contents.getType() === 'webview') {
      contents.setUserAgent(chromeUA);
    }
  });

  app.on('browser-window-created', (_e, win) => {
    win.webContents.setUserAgent(chromeUA);
  });

  createWindow();
});

app.on('before-quit', () => {
  // Signal renderer to save session before quit
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.webContents.send(IPC.APP_BEFORE_QUIT);
  }
  workflowWatcher.detachAll();
  stopAllTerminals();
  stopClaudemonHookBridge();
  stopClaudemonStatusLineBridge();
  stopClaudemonConversationBridge();
  stopHubClient();
  stopMcpFacade();
  stopHub();
  stopClaudemon();
  database.close();
});

app.on('window-all-closed', () => {
  claudemonSessionClient.closeAll();
  workflowWatcher.detachAll();
  stopAllTerminals();
  stopClaudemonHookBridge();
  stopClaudemonStatusLineBridge();
  stopClaudemonConversationBridge();
  stopHubClient();
  stopMcpFacade();
  stopHub();
  stopClaudemon();
  app.quit();
});

app.on('activate', () => {
  if (mainWindow === null) {
    createWindow();
  }
});
