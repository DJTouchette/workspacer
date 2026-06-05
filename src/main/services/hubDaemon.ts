/**
 * Spawns and supervises the `hub` daemon — workspacer's control-plane / event
 * bus (Go, in `hub/`). It runs independently of the UI so plugins (and, later,
 * an MCP facade) can broker events with or without a window open.
 *
 * On startup we point it at claudemon's /events so claudemon becomes the first
 * producer on the bus; the renderer then consumes a single normalized stream.
 *
 * Binary resolution:
 *   - dev (ELECTRON_DEV=1): <repo>/hub/hub[.exe]
 *   - packaged:             <resourcesPath>/hub/hub[.exe]
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as crypto from 'crypto';
import { spawn, ChildProcess } from 'child_process';
import { app } from 'electron';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { killStaleListener, waitForHealth as waitForHealthShared, PORTS } from '../lib/daemonUtils';
import { getConfigDir } from './configService';

const PORT = PORTS.hub;
const HEALTH_TIMEOUT_MS = 5000;

let child: ChildProcess | null = null;
let readyPromise: Promise<void> | null = null;

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

/** Load (or create + persist) the shared bus token. Empty unless remote is on. */
function loadOrCreateToken(): string {
  if (!REMOTE_ENABLED) return '';
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
  /** URL to open on the phone/other PC, token included. */
  remoteUrl: string;
  /** Bare bus URL (no token) for diagnostics. */
  busUrl: string;
}

/** Connection info for the remote-control client — for the UI / logs. */
export function getRemoteShareInfo(): RemoteShareInfo {
  const host = advertiseHost();
  const q = HUB_TOKEN ? `?token=${encodeURIComponent(HUB_TOKEN)}` : '';
  return {
    enabled: REMOTE_ENABLED,
    token: HUB_TOKEN,
    remoteUrl: `http://${host}:${PORT}/remote${q}`,
    busUrl: `ws://${host}:${PORT}/bus`,
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
    return path.join(app.getAppPath(), 'hub', exeName());
  }
  return path.join(process.resourcesPath, 'hub', exeName());
}

/** Bundled example plugins, copied into the user dir on first run. */
function bundledExamplesDir(): string {
  if (process.env.ELECTRON_DEV || !app.isPackaged) {
    return path.join(app.getAppPath(), 'hub', 'examples');
  }
  return path.join(process.resourcesPath, 'hub', 'examples');
}

/** Persistent, writable plugins directory where installs land. */
function userPluginsDir(): string {
  return path.join(getConfigDir(), 'plugins');
}

/** Ensure the user plugins dir exists; seed it from bundled examples once. */
function ensurePluginsDir(): string {
  const dir = userPluginsDir();
  try {
    fs.mkdirSync(dir, { recursive: true });
    const empty = fs.readdirSync(dir).length === 0;
    const examples = bundledExamplesDir();
    if (empty && fs.existsSync(examples)) {
      fs.cpSync(examples, dir, { recursive: true });
      console.log(`[hub] seeded plugins dir from ${examples}`);
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
    return Promise.reject(new Error(`hub binary not found at ${bin} (run: cd hub && go build -o hub ./cmd/hub)`));
  }

  killStaleListener(PORT, 'hub');

  const pluginsDir = ensurePluginsDir();
  console.log(`[hub] spawning ${bin} (addr ${BIND_ADDR})`);
  const hubArgs = [
    '--addr', BIND_ADDR,
    '--claudemon-events', `${CLAUDEMON_API_URL}/events`,
    '--plugins-dir', pluginsDir,
  ];
  if (HUB_TOKEN) hubArgs.push('--token', HUB_TOKEN);
  child = spawn(bin, hubArgs, {
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  });

  if (REMOTE_ENABLED) {
    const info = getRemoteShareInfo();
    console.log(`[hub] remote sharing ON — open on your phone/PC: ${info.remoteUrl}`);
  }

  child.stdout?.on('data', d => process.stdout.write(`[hub] ${d}`));
  child.stderr?.on('data', d => process.stderr.write(`[hub] ${d}`));
  child.on('exit', (code, signal) => {
    console.log(`[hub] exited code=${code} signal=${signal}`);
    child = null;
    readyPromise = null;
  });

  readyPromise = waitForHealthShared(`http://127.0.0.1:${PORT}/health`, HEALTH_TIMEOUT_MS, 'hub');
  return readyPromise;
}

export function stopHub(): void {
  if (child) {
    try { child.kill(); } catch {}
    child = null;
  }
  readyPromise = null;
}

export const HUB_PORT = PORT;
export const HUB_BUS_URL = `ws://127.0.0.1:${PORT}/bus`;
export const HUB_HTTP_URL = `http://127.0.0.1:${PORT}`;
