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
import { CLAUDEMON_API_URL, isClaudemonAdopted } from './claudemonDaemon';
import { DELEGATE_CATALOG_TO_BRAIN, DESKTOP_RENDERER_USES_BUS } from './brainDelegation';
import { getRemoteServer } from './remoteServer';
import {
  killStaleListener,
  waitForHealth as waitForHealthShared,
  probeHealth,
  PORTS,
  RestartBackoff,
  daemonSpawnOptions,
  gracefulStop,
} from '../lib/daemonUtils';
import { getConfigDir } from './configService';
import { notifySystem } from './systemNotice';

const PORT = PORTS.hub;
const HEALTH_TIMEOUT_MS = 5000;

let child: ChildProcess | null = null;
let readyPromise: Promise<void> | null = null;
/** Set by stopHub() / app shutdown so an intentional kill isn't respawned. */
let intentionalStop = false;
/**
 * True when we ADOPTED an already-running external hub (e.g. `workspacer
 * serve`, which runs the hub with --brain-scope full). Adopted hubs are not
 * ours to manage: no supervision/restart, no signal on quit — only an owned
 * child (spawned by us, tracked in `child`) is stopped. Capability overlap
 * with the external hub's full-scope brain is resolved by the hub itself:
 * its router is first-registration-wins (see services/hub internal/bus/rpc.go
 * and the `registered` ack handling in hubClient.ts), so our registrations
 * for brain-owned methods are simply withheld while desktop-only extras land.
 */
let adopted = false;
const backoff = new RestartBackoff();

/** Whether the running hub was adopted from an external server. */
export function isHubAdopted(): boolean {
  return adopted;
}

/**
 * Remote sharing (opt-in, toggleable at runtime). When enabled the hub binds
 * beyond loopback so another PC/phone — ideally over a Tailscale tailnet — can
 * reach the bus + the /remote web client. Binding off localhost is meaningless
 * without auth, so in this mode we also require a shared token on /bus.
 *
 * The toggle is persisted to `<config>/remote-share-enabled` and flipped from
 * the UI (Remote control → Start/Stop sharing), which restarts the hub bound
 * accordingly. `WORKSPACER_REMOTE_SHARE=1` force-enables it (dev / `make dev`).
 *
 *   WORKSPACER_REMOTE_ADDR=host:port     bind address (default 0.0.0.0:7895;
 *                                        pin to your tailnet IP to avoid LAN)
 *
 * Default (off): exactly today's behavior — 127.0.0.1, loopback only.
 */
function remoteFlagFile(): string {
  return path.join(getConfigDir(), 'remote-share-enabled');
}

/** Cached effective state so hot paths (per-snapshot telemetry) don't stat the
 *  flag file each call. Invalidated on toggle and recomputed lazily. */
let remoteEnabledCache: boolean | null = null;

/** Whether remote sharing is currently on: the env var force-enables it,
 *  otherwise the persisted UI toggle decides. Cached. */
function isRemoteEnabled(): boolean {
  if (remoteEnabledCache !== null) return remoteEnabledCache;
  let enabled = false;
  if (process.env.WORKSPACER_REMOTE_SHARE) {
    enabled = true;
  } else {
    try {
      enabled = fs.readFileSync(remoteFlagFile(), 'utf-8').trim() === '1';
    } catch {
      enabled = false;
    }
  }
  remoteEnabledCache = enabled;
  return enabled;
}

/** Cheap, cached accessor for other modules (e.g. snapshot telemetry). */
export function isRemoteShareEnabled(): boolean {
  return isRemoteEnabled();
}

/** Persist the remote-share toggle (presence of the flag file = enabled). */
function writeRemoteShareFlag(enabled: boolean): void {
  const file = remoteFlagFile();
  try {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    if (enabled) fs.writeFileSync(file, '1', { mode: 0o600 });
    else fs.rmSync(file, { force: true });
  } catch (err) {
    console.error('[hub] failed to persist remote-share flag:', err);
  }
  remoteEnabledCache = null; // recompute on next read (env still force-enables)
}

/** Bind address for the current remote-share state: tailnet-reachable when on,
 *  loopback-only when off. */
function bindAddr(): string {
  return isRemoteEnabled()
    ? process.env.WORKSPACER_REMOTE_ADDR || `0.0.0.0:${PORT}`
    : `127.0.0.1:${PORT}`;
}

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
  } catch {
    /* not created yet */
  }
  const token = crypto.randomBytes(24).toString('base64url');
  try {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, token, { mode: 0o600 });
  } catch (err) {
    // Non-fatal for this run (we still return the in-memory token), but the
    // token now lives only in this process: on the next restart a fresh token
    // is generated, invalidating every previously saved share/webview URL that
    // carried the old bearer secret. Surface loudly so the cause is visible.
    console.error(
      `[hub] failed to persist remote token to ${file} — saved share/webview URLs will stop working after restart:`,
      err,
    );
  }
  return token;
}

const HUB_TOKEN = loadOrCreateToken();

/** Best-effort: a tailnet/LAN IP to advertise in the remote URL. */
function advertiseHost(): string {
  const [host] = bindAddr().split(':');
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
  /** True when the local hub was ADOPTED from an external `workspacer serve`
   *  (we didn't spawn it and won't stop it on quit). */
  hubAdopted: boolean;
  /** Same, for claudemon. */
  claudemonAdopted: boolean;
  /** True when sharing is on but the hub does NOT answer at the advertised
   *  LAN/Tailscale address — the phone QR would point at a dead endpoint.
   *  The common case: an ADOPTED `workspacer serve` bound to loopback (its
   *  deliberate default); we can't rebind a hub we don't own, so the dialog
   *  must say "restart serve with --host" instead of rendering the QR. */
  advertisedUnreachable: boolean;
  /** Configured "connect to remote server" target (client mode), or null.
   *  When set, the renderer boots against this REMOTE hub instead of the
   *  local one and main skips spawning local daemons — see remoteServer.ts. */
  remoteClient: { httpUrl: string; busUrl: string; token: string } | null;
}

/** Location of the built web app (dist/web) the hub serves at /app/. */
function webappDir(): string {
  if (process.env.ELECTRON_DEV || !app.isPackaged) {
    return path.join(app.getAppPath(), 'dist', 'web');
  }
  return path.join(process.resourcesPath, 'web');
}

/** Connection info for the remote-control client — for the UI / logs. */
export async function getRemoteShareInfo(): Promise<RemoteShareInfo> {
  const enabled = isRemoteEnabled();
  const host = advertiseHost();
  const q = HUB_TOKEN ? `?token=${encodeURIComponent(HUB_TOKEN)}` : '';
  // Whether /app (the full web renderer) is actually served. An owned hub
  // serves whatever dist/web we passed it, so the on-disk check is the truth;
  // an ADOPTED hub serves only what ITS --webapp-dir pointed at (often
  // nothing) — the local build's existence says nothing about it, so ask the
  // hub itself rather than advertise a 404.
  const hasWebApp =
    enabled &&
    (adopted ? await probeHealth(`http://127.0.0.1:${PORT}/app/${q}`) : fs.existsSync(webappDir()));
  // An owned hub is (re)spawned with the right binding when sharing flips on,
  // so only the adopted case can advertise an address nothing listens on —
  // probe it rather than hand out a QR that scans to a dead endpoint.
  const advertisedUnreachable =
    enabled && adopted && !(await probeHealth(`http://${host}:${PORT}/health`));
  return {
    enabled,
    token: HUB_TOKEN,
    remoteUrl: `http://${host}:${PORT}/m${q}`,
    appUrl: hasWebApp ? `http://${host}:${PORT}/app/${q}` : '',
    busUrl: `ws://${host}:${PORT}/bus`,
    desktopBus: DESKTOP_RENDERER_USES_BUS,
    hubAdopted: adopted,
    claudemonAdopted: isClaudemonAdopted(),
    advertisedUnreachable,
    remoteClient: getRemoteServer(),
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
 * any machine. Sidecar examples (the clock needs python3) are NOT seeded
 * (they'd crash-loop without the runtime); users add those on demand from the
 * examples gallery, which labels each with its requirement.
 */
const DEFAULT_SEEDED_EXAMPLES = ['editor'];

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
      console.log(
        `[hub] seeded plugins dir with default examples: ${seeded.join(', ') || '(none found)'}`,
      );
    }
  } catch (err) {
    console.error('[hub] failed to prepare plugins dir:', err);
  }
  return dir;
}

/**
 * Start the hub: adopt a healthy external one, else spawn our own.
 * Idempotent — repeat calls return the existing ready promise.
 *
 * Adopt-don't-kill: `workspacer serve` runs the hub (with a full-scope brain)
 * on the same default port. If something HEALTHY already answers /health we
 * adopt it — no spawn, no kill, no supervision. Kill-stale only runs when the
 * port is occupied but unhealthy (the orphan case it exists for). The probe
 * precedes the binary check on purpose: adoption needs no local binary.
 * Auth just works because both sides read/mint the same
 * `<config>/workspacer/remote-token` (see loadOrCreateToken + the CLI's
 * token.go), so getHubToken() matches the adopted hub's expected bearer.
 */
export function startHub(): Promise<void> {
  if (readyPromise) return readyPromise;

  intentionalStop = false;
  const starting = (async () => {
    if (await probeHealth(`http://127.0.0.1:${PORT}/health`)) {
      adopted = true;
      console.log(
        `[hub] adopted external hub on :${PORT} (workspacer serve?) — not spawning, not supervising; ` +
          'its full-scope brain keeps the capabilities it already provides',
      );
      return;
    }
    adopted = false;
    const bin = hubBinaryPath();
    if (!fs.existsSync(bin)) {
      throw new Error(
        `hub binary not found at ${bin} (run: cd services/hub && go build -o hub ./cmd/hub)`,
      );
    }
    return launch(bin);
  })();
  // launch() re-assigns readyPromise to its health promise (needed by the
  // crash-restart path); until then this placeholder keeps repeat callers off
  // a second probe/spawn.
  readyPromise = starting;
  return starting;
}

/** Spawn the process and wire up exit-driven restart. Returns the health promise. */
function launch(bin: string): Promise<void> {
  // Only reached when the health probe failed: whatever holds the port (if
  // anything) is a stale orphan, not an adoptable hub — clear it.
  killStaleListener(PORT, 'hub');

  const pluginsDir = ensurePluginsDir();
  const addr = bindAddr();
  const remote = isRemoteEnabled();
  console.log(`[hub] spawning ${bin} (addr ${addr})`);
  backoff.markStarted();
  const hubArgs = [
    '--addr',
    addr,
    '--claudemon-events',
    `${CLAUDEMON_API_URL}/events`,
    '--plugins-dir',
    pluginsDir,
    // Read-only catalog of bundled examples the user can add from the UI.
    '--examples-dir',
    bundledExamplesDir(),
    // Have the hub supervise the headless brain provider. With delegation on it
    // owns the file-backed "catalog" capabilities (main stops registering them —
    // see hubCapabilities + brainDelegation); the brain binary ships next to the
    // hub binary, so the hub auto-detects it. Off → no brain, main stays the
    // provider (kill switch: WORKSPACER_NO_BRAIN=1).
    '--brain-scope',
    DELEGATE_CATALOG_TO_BRAIN ? 'catalog' : 'off',
    '--claudemon',
    CLAUDEMON_API_URL,
  ];
  if (HUB_TOKEN) hubArgs.push('--token', HUB_TOKEN);
  // Serve the full web app (real renderer) at /app/ when remote sharing is on
  // and a web build exists. The lightweight /remote client works regardless.
  const webDir = webappDir();
  if (remote && fs.existsSync(webDir)) {
    hubArgs.push('--webapp-dir', webDir);
  }
  child = spawn(bin, hubArgs, daemonSpawnOptions());

  if (remote) {
    // Log the reachable address but NOT the token — the tokened URL/QR lives in
    // the Remote control panel only, so the secret never lands in logs/terminals.
    console.log(
      `[hub] remote sharing ON — bound to ${addr}. Open Remote control for the tokened link/QR.`,
    );
  }

  // AbortController so a fast-exiting daemon cancels the health-check poll
  // instead of spinning for the full HEALTH_TIMEOUT_MS.
  const healthAbort = new AbortController();

  child.stdout?.on('data', (d) => process.stdout.write(`[hub] ${d}`));
  child.stderr?.on('data', (d) => process.stderr.write(`[hub] ${d}`));
  child.on('exit', (code, signal) => {
    console.log(`[hub] exited code=${code} signal=${signal}`);
    child = null;
    readyPromise = null;
    healthAbort.abort(); // cancel any in-progress health poll
    if (!intentionalStop) scheduleRestart(bin);
  });

  readyPromise = waitForHealthShared(
    `http://127.0.0.1:${PORT}/health`,
    HEALTH_TIMEOUT_MS,
    'hub',
    healthAbort.signal,
  ).then(() => {
    backoff.reset();
  });
  return readyPromise;
}

/** Respawn after an unexpected exit, with exponential backoff. */
function scheduleRestart(bin: string): void {
  const delay = backoff.nextDelay();
  if (delay === null) {
    notifySystem({
      level: 'warn',
      key: 'hub-crashloop',
      title: 'Control plane (hub) keeps crashing',
      detail:
        'Gave up restarting it. Plugins and remote sharing stay unavailable until you restart the app.',
    });
    return;
  }
  console.warn(`[hub] unexpected exit — restarting in ${delay}ms`);
  setTimeout(() => {
    if (intentionalStop || child) return; // stopped, or already back up
    launch(bin).catch((err) => console.error('[hub] restart failed health check:', err));
  }, delay);
}

/**
 * Toggle remote sharing at runtime: persist the flag, then restart the hub so
 * it re-binds (loopback ⇄ tailnet) and (de)serves the web app. Returns the
 * fresh share info for the UI. Force-on via WORKSPACER_REMOTE_SHARE can't be
 * turned off here (the env var always wins) — we surface that to the caller.
 */
export async function setRemoteShare(enabled: boolean): Promise<RemoteShareInfo> {
  writeRemoteShareFlag(enabled);
  // An adopted hub isn't ours to restart — its binding was chosen by whoever
  // started `workspacer serve` (--host is its remote opt-in). Persist the flag
  // so an app-owned hub honors it next time, but don't touch the external one.
  if (adopted) {
    console.warn(
      '[hub] remote-share toggle noted, but the hub is an adopted external server — ' +
        're-bind it there (workspacer serve --host …); the toggle applies when the app owns the hub',
    );
    return getRemoteShareInfo();
  }
  await stopHub();
  // stopHub cleared readyPromise + set intentionalStop; startHub re-launches
  // bound for the new state. Await health so the UI's refreshed info is live.
  try {
    await startHub();
  } catch (err) {
    console.error('[hub] restart after remote-share toggle failed:', err);
  }
  return getRemoteShareInfo();
}

export function stopHub(): Promise<void> {
  intentionalStop = true;
  backoff.reset(); // clear failure counter so the next startHub() begins fresh
  // Adopted hub: it isn't ours — leave it (and its plugin sidecars/brain)
  // running. Quitting the app must not take down `workspacer serve`.
  if (adopted) {
    console.log('[hub] adopted hub left running (owned by the external server)');
    adopted = false;
    readyPromise = null;
    return Promise.resolve();
  }
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
