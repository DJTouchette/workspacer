/**
 * "Connect to remote server" — persistence + mode selection for running the
 * Electron app as a *client* of an external workspacer server (`workspacer
 * serve` on another machine, typically over a Tailscale tailnet).
 *
 * When a remote server is configured, main skips spawning the local daemons
 * entirely (see index.ts) and the renderer boots against the REMOTE hub bus
 * through the web backend — exactly what a browser does at the server's /app
 * URL, but inside the Electron shell (see renderer backend/install.ts).
 *
 * The setting lives in `<config>/remote-server.json` (mode 0600 — it holds the
 * bearer token) rather than config.yaml so main can read it before any config
 * plumbing is up, mirroring the `remote-share-enabled` flag-file pattern in
 * hubDaemon.ts. Changes take effect on relaunch: the daemon-vs-remote decision
 * is made once at startup, so the UI relaunches the app after connect/disconnect.
 */

import * as fs from 'fs';
import * as path from 'path';
import { getConfigDir } from './configService';

/** What the user enters + persists. */
export interface RemoteServerSetting {
  /** Server address in any reasonable form (host, host:port, http(s)://…, ws(s)://…/bus). */
  url: string;
  /** Hub bus bearer token (the pairing token `workspacer serve` prints). */
  token: string;
}

/** Normalized, ready-to-dial form of the setting. */
export interface ResolvedRemoteServer {
  /** The hub's HTTP origin, e.g. http://100.64.1.2:7895 */
  httpUrl: string;
  /** The hub's bus WebSocket URL, e.g. ws://100.64.1.2:7895/bus */
  busUrl: string;
  token: string;
}

const DEFAULT_HUB_PORT = 7895;

function settingFile(): string {
  return path.join(getConfigDir(), 'remote-server.json');
}

/**
 * Normalize a user-entered server address into the http origin + ws bus URL.
 * Accepts bare `host`, `host:port`, `http(s)://host[:port]`, and
 * `ws(s)://host[:port][/bus]`; anything unparseable returns null so callers
 * fail closed instead of persisting a broken setting.
 */
export function normalizeRemoteServerUrl(raw: string): { httpUrl: string; busUrl: string } | null {
  let s = (raw ?? '').trim();
  if (!s) return null;
  // Bare host[:port] → assume plain http (tailnet traffic is already encrypted).
  const hadScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(s);
  if (!hadScheme) s = `http://${s}`;
  let u: URL;
  try {
    u = new URL(s);
  } catch {
    return null;
  }
  const secure = u.protocol === 'https:' || u.protocol === 'wss:';
  if (!['http:', 'https:', 'ws:', 'wss:'].includes(u.protocol)) return null;
  if (!u.hostname) return null;
  // Port: explicit wins; a bare host gets the hub default; an explicit scheme
  // without a port keeps the scheme's default (e.g. https://node.ts.net → 443,
  // the `tailscale serve` front), which URLs express by omitting the port.
  const port = u.port ? `:${u.port}` : hadScheme ? '' : `:${DEFAULT_HUB_PORT}`;
  // IPv6: WHATWG URL keeps the brackets in hostname; re-add only if missing.
  const host =
    u.hostname.includes(':') && !u.hostname.startsWith('[') ? `[${u.hostname}]` : u.hostname;
  const httpUrl = `${secure ? 'https' : 'http'}://${host}${port}`;
  const busUrl = `${secure ? 'wss' : 'ws'}://${host}${port}/bus`;
  return { httpUrl, busUrl };
}

// Cached so the hot path (getRemoteShareInfo per open dialog / backend boot)
// doesn't re-read the file; invalidated by setRemoteServer.
let cache: ResolvedRemoteServer | null | undefined;

/** The configured remote server, normalized — or null (local mode). */
export function getRemoteServer(): ResolvedRemoteServer | null {
  if (cache !== undefined) return cache;
  cache = null;
  try {
    const parsed = JSON.parse(fs.readFileSync(settingFile(), 'utf-8')) as RemoteServerSetting;
    const normalized = parsed?.url ? normalizeRemoteServerUrl(parsed.url) : null;
    if (normalized && typeof parsed.token === 'string') {
      cache = { ...normalized, token: parsed.token };
    }
  } catch {
    /* absent or unreadable → local mode */
  }
  return cache;
}

/** True when the app should run as a client of an external server. */
export function isRemoteClientMode(): boolean {
  return getRemoteServer() !== null;
}

/**
 * Persist (or clear, with null) the remote-server setting. Throws on an
 * unparseable URL so the UI can show the problem instead of silently storing
 * a setting that would brick the next launch.
 */
export function setRemoteServer(setting: RemoteServerSetting | null): void {
  const file = settingFile();
  cache = undefined; // recompute on next read
  if (!setting) {
    try {
      fs.rmSync(file, { force: true });
    } catch (err) {
      console.error('[remote-server] failed to clear setting:', err);
    }
    return;
  }
  if (!normalizeRemoteServerUrl(setting.url)) {
    throw new Error(`unrecognized server address: ${setting.url}`);
  }
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify({ url: setting.url, token: setting.token }, null, 2), {
    mode: 0o600,
  });
}
