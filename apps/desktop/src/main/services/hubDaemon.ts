/**
 * Spawns and supervises the `hub` daemon — workspacer's control-plane / event
 * bus (Go, in `services/hub/`). It runs independently of the UI so plugins (and,
 * later,
 * an MCP facade) can broker events with or without a window open.
 *
 * On startup we point it at claudemon's /events so claudemon becomes the first
 * producer on the bus; the renderer then consumes a single normalized stream.
 *
 * Binary resolution:
 *   - dev (ELECTRON_DEV=1): <repo>/services/hub/hub[.exe]
 *   - packaged:             <resourcesPath>/hub/hub[.exe]
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as crypto from 'crypto';
import { spawn, ChildProcess } from 'child_process';
import { app } from 'electron';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { DELEGATE_CATALOG_TO_BRAIN, DESKTOP_RENDERER_USES_BUS } from './brainDelegation';
import { killStaleListener, waitForHealth as waitForHealthShared, PORTS, RestartBackoff, daemonSpawnOptions, gracefulStop } from '../lib/daemonUtils';
import { getConfigDir } from './configService';

const PORT = PORTS.hub;
const HEALTH_TIMEOUT_MS = 5000;

let child: ChildProcess | null = null;
let readyPromise: Promise<void> | null = null;
/** Set by stopHub() / app shutdown so an intentional kill isn't respawned. */
let intentionalStop = false;
const backoff = new RestartBackoff();

/**
 * Remote sharing (opt-in). When WORKSPACER_REMOTE_SHARE is set, the hub binds
 * beyond loopback so another PC/phone — ideally over a Tailscale tailnet — can
 * reach the bus + the /remote web client. Binding off localhost is meaningless
 * without auth, so in this mode we also require a shared token on /bus.
 *
 *   WORKSPACER_REMOTE_SHARE=1            enable remote sharing
 *   WORKSPACER_REMOTE_ADDR=host:port     bind address (default 0.0.0.0:7895;
 *                                        pin to your tailnet IP to avoid LAN)
 *
 * Default (unset): exactly today's behavior — 127.0.0.1, no token.
 */
const REMOTE_ENABLED = !!process.env.WORKSPACER_REMOTE_SHARE;
const BIND_ADDR = REMOTE_ENABLED
  ? (process.env.WORKSPACER_REMOTE_ADDR || `0.0.0.0:${PORT}`)
  : `127.0.0.1:${PORT}`;

/**
 * Load (or create + persist) the hub bus token. Always set now — even on the
 * localhost-only default — so the bus can distinguish the trusted host (this
 * token) from plugin sidecars/webviews (their own per-plugin tokens) and reject
 * unidentified connections. That's the basis of plugin capability enforcement.
 * Remote sharing reuses the same token as the bearer secret.
 */
function loadOrCreateToken(): string {
  const file = path.join(getConfigDir(), 'remote-token');
  try {
    const existing = fs.readFileSync(file, 'utf-8').trim();
    if (existing) return existing;
  } catch { /* not created yet */ }
  const token = crypto.randomBytes(24).toString('base64url');
  try {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, token, { mode: 0o600 });
  } catch (err) {
    console.error('[hub] failed to persist remote token:', err);
  }
  return token;
}

const HUB_TOKEN = loadOrCreateToken();

/** Best-effort: a tailnet/LAN IP to advertise in the remote URL. */
function advertiseHost(): string {
  const [host] = BIND_ADDR.split(':');
  if (host && host !== '0.0.0.0' && host !== '::') return host;
  // Prefer a Tailscale (100.64.0.0/10) address if present, else first non-internal IPv4.
  let fallback = '';
  for (const ifaces of Object.values(os.networkInterfaces())) {
    for (const ni of ifaces || []) {
      if (ni.family !== 'IPv4' || ni.internal) continue;
      if (ni.address.startsWith('100.')) return ni.address;
      if (!fallback) fallback = ni.address;
    }
  }
  return fallback || '127.0.0.1';
}

export interface RemoteShareInfo {
  enabled: boolean;
  token: string;
  /** URL to open on the phone/other PC, token included. The lightweight client. */
  remoteUrl: string;
  /** Full-app (real renderer) URL, token included. Served only when the web
   *  build exists; empty otherwise. */
  appUrl: string;
  /** Bare bus URL (no token) for diagnostics. */
  busUrl: string;
  /** Whether the desktop renderer should run on the hub bus (mirroring the TUI)
   *  rather than pure IPC. The renderer reads this to pick its transport. */
  desktopBus: boolean;
}

/** Location of the built web app (dist/web) the hub serves at /app/. */
function webappDir(): string {
  if (process.env.ELECTRON_DEV || !app.isPackaged) {
    return path.join(app.getAppPath(), 'dist', 'web');
  }
  return path.join(process.resourcesPath, 'web');
}

/** Connection info for the remote-control client — for the UI / logs. */
export function getRemoteShareInfo(): RemoteShareInfo {
  const host = advertiseHost();
  const q = HUB_TOKEN ? `?token=${encodeURIComponent(HUB_TOKEN)}` : '';
  const hasWebApp = REMOTE_ENABLED && fs.existsSync(webappDir());
  return {
    enabled: REMOTE_ENABLED,
    token: HUB_TOKEN,
    remoteUrl: `http://${host}:${PORT}/m${q}`,
    appUrl: hasWebApp ? `http://${host}:${PORT}/app/${q}` : '',
    busUrl: `ws://${host}:${PORT}/bus`,
    desktopBus: DESKTOP_RENDERER_USES_BUS,
  };
}

/** Token the local hub client must present when remote auth is on. */
export function getHubToken(): string {
  return HUB_TOKEN;
}

function exeName(): string {
  return process.platform === 'win32' ? 'hub.exe' : 'hub';
}

export function hubBinaryPath(): string {
  if (process.env.ELECTRON_DEV || !app.isPackaged) {
    return path.join(app.getAppPath(), '..', '..', 'services', 'hub', exeName());
  }
  return path.join(process.resourcesPath, 'hub', exeName());
}

/** Bundled example plugins, copied into the user dir on first run. */
function bundledExamplesDir(): string {
  if (process.env.ELECTRON_DEV || !app.isPackaged) {
    return path.join(app.getAppPath(), '..', '..', 'services', 'hub', 'examples');
  }
  return path.join(process.resourcesPath, 'hub', 'examples');
}

/** Persistent, writable plugins directory where installs land. */
function userPluginsDir(): string {
  return path.join(getConfigDir(), 'plugins');
}

/**
 * Webview-only example plugins with zero runtime dependencies — safe to seed on
 * any machine. The Python/Go sidecar examples are NOT seeded (they'd crash-loop
 * without the runtime); users add those on demand from the examples gallery,
 * which labels each with its requirement.
 */
const DEFAULT_SEEDED_EXAMPLES = ['editor', 'hello-scoped'];

/** Ensure the user plugins dir exists; seed the safe default examples once. */
function ensurePluginsDir(): string {
  const dir = userPluginsDir();
  try {
    fs.mkdirSync(dir, { recursive: true });
    const empty = fs.readdirSync(dir).length === 0;
    const examples = bundledExamplesDir();
    if (empty && fs.existsSync(examples)) {
      const seeded: string[] = [];
      for (const name of DEFAULT_SEEDED_EXAMPLES) {
        const src = path.join(examples, name);
        if (fs.existsSync(src)) {
          fs.cpSync(src, path.join(dir, name), { recursive: true });
          seeded.push(name);
        }
      }
      console.log(`[hub] seeded plugins dir with default examples: ${seeded.join(', ') || '(none found)'}`);
    }
  } catch (err) {
    console.error('[hub] failed to prepare plugins dir:', err);
  }
  return dir;
}

/** Spawn the hub. Idempotent — repeat calls return the existing ready promise. */
export function startHub(): Promise<void> {
  if (readyPromise) return readyPromise;

  const bin = hubBinaryPath();
  if (!fs.existsSync(bin)) {
    return Promise.reject(new Error(`hub binary not found at ${bin} (run: cd services/hub && go build -o hub ./cmd/hub)`));
  }

  intentionalStop = false;
  return launch(bin);
}

/** Spawn the process and wire up exit-driven restart. Returns the health promise. */
function launch(bin: string): Promise<void> {
  killStaleListener(PORT, 'hub');

  const pluginsDir = ensurePluginsDir();
  console.log(`[hub] spawning ${bin} (addr ${BIND_ADDR})`);
  backoff.markStarted();
  const hubArgs = [
    '--addr', BIND_ADDR,
    '--claudemon-events', `${CLAUDEMON_API_URL}/events`,
    '--plugins-dir', pluginsDir,
    // Read-only catalog of bundled examples the user can add from the UI.
    '--examples-dir', bundledExamplesDir(),
    // Have the hub supervise the headless brain provider. With delegation on it
    // owns the file-backed "catalog" capabilities (main stops registering them —
    // see hubCapabilities + brainDelegation); the brain binary ships next to the
    // hub binary, so the hub auto-detects it. Off → no brain, main stays the
    // provider (kill switch: WORKSPACER_NO_BRAIN=1).
    '--brain-scope', DELEGATE_CATALOG_TO_BRAIN ? 'catalog' : 'off',
    '--claudemon', CLAUDEMON_API_URL,
  ];
  if (HUB_TOKEN) hubArgs.push('--token', HUB_TOKEN);
  // Serve the full web app (real renderer) at /app/ when remote sharing is on
  // and a web build exists. The lightweight /remote client works regardless.
  const webDir = webappDir();
  if (REMOTE_ENABLED && fs.existsSync(webDir)) {
    hubArgs.push('--webapp-dir', webDir);
  }
  child = spawn(bin, hubArgs, daemonSpawnOptions());

  if (REMOTE_ENABLED) {
    const info = getRemoteShareInfo();
    console.log(`[hub] remote sharing ON — lightweight client: ${info.remoteUrl}`);
    if (info.appUrl) console.log(`[hub] full app (real renderer): ${info.appUrl}`);
  }

  // AbortController so a fast-exiting daemon cancels the health-check poll
  // instead of spinning for the full HEALTH_TIMEOUT_MS.
  const healthAbort = new AbortController();

  child.stdout?.on('data', d => process.stdout.write(`[hub] ${d}`));
  child.stderr?.on('data', d => process.stderr.write(`[hub] ${d}`));
  child.on('exit', (code, signal) => {
    console.log(`[hub] exited code=${code} signal=${signal}`);
    child = null;
    readyPromise = null;
    healthAbort.abort(); // cancel any in-progress health poll
    if (!intentionalStop) scheduleRestart(bin);
  });

  readyPromise = waitForHealthShared(`http://127.0.0.1:${PORT}/health`, HEALTH_TIMEOUT_MS, 'hub', healthAbort.signal)
    .then(() => { backoff.reset(); });
  return readyPromise;
}

/** Respawn after an unexpected exit, with exponential backoff. */
function scheduleRestart(bin: string): void {
  const delay = backoff.nextDelay();
  if (delay === null) {
    console.error('[hub] crashed too many times; giving up auto-restart. Restart the app to recover.');
    return;
  }
  console.warn(`[hub] unexpected exit — restarting in ${delay}ms`);
  setTimeout(() => {
    if (intentionalStop || child) return; // stopped, or already back up
    launch(bin).catch(err => console.error('[hub] restart failed health check:', err));
  }, delay);
}

export function stopHub(): Promise<void> {
  intentionalStop = true;
  backoff.reset(); // clear failure counter so the next startHub() begins fresh
  const c = child;
  child = null;
  readyPromise = null;
  // Extra grace: closing stdin makes the hub run mgr.Stop(), which SIGTERMs each
  // supervised plugin sidecar (up to a few seconds apiece) before it exits.
  return gracefulStop(c, 'hub', 6000);
}

export const HUB_PORT = PORT;
export const HUB_BUS_URL = `ws://127.0.0.1:${PORT}/bus`;
export const HUB_HTTP_URL = `http://127.0.0.1:${PORT}`;
