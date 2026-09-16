import { noteRuntimePhase, observeRuntimeStart } from './agentRuntimeStatus';
/**
 * Spawns and supervises the `mcp` facade — the workspacer MCP server (Go, in
 * `services/hub/cmd/mcp`). It exposes the hub's capabilities (list / spawn /
 * drive agents, notify, …) as MCP tools over HTTP at http://127.0.0.1:7897/mcp,
 * so a supervisor Claude Code session pointed there via `--mcp-config` gets the
 * `mcp__workspacer__*` control plane.
 *
 * The facade is a thin adapter: every tool call is forwarded to the hub bus as a
 * capability `call`, which the Electron main process (hubCapabilities.ts)
 * executes. So this must start AFTER the hub is up; it connects to the bus and
 * retries on its own if the hub is briefly unavailable.
 *
 * The facade's OWN inbound surface REFUSES credential-less callers by default:
 * cmd/mcp ships `-untokened deny`, so a local process presenting nothing gets
 * 401 rather than the whole operator tool surface. Nothing here has to pass a
 * flag for that — the binary's own default is the source of truth, and every
 * session this app spawns with the facade already carries a per-session bearer
 * token (see mcpConfig.ts / remoteTokens.ts). The optional config key
 * `facade.untokenedAccess` (operator | view | deny) is the dial on that
 * default, passed through as --untokened; `operator` is the explicit opt-in for
 * a hand-configured local MCP client that cannot carry a token. Spawned-agent
 * bearers always receive the ambient operator/plugin surface.
 *
 * Mirrors hubDaemon.ts (binary resolution, health poll, restart backoff). Fully
 * optional from the rest of the app's point of view: if it fails to start, only
 * the supervisor's action tools are missing — agents and the in-app dock work
 * regardless.
 *
 * Binary resolution:
 *   - dev (ELECTRON_DEV=1): <repo>/services/hub/mcp[.exe]
 *   - packaged:             <resourcesPath>/hub/mcp[.exe]
 */

import * as fs from 'fs';
import * as path from 'path';
import { spawn, ChildProcess } from 'child_process';
import { app } from 'electron';
import {
  killStaleListener,
  waitForHealth,
  PORTS,
  RestartBackoff,
  daemonSpawnOptions,
  gracefulStop,
} from '../lib/daemonUtils';
import { hubBusUrl, getHubToken } from './hubDaemon';
import { configService } from './configService';

const PORT = PORTS.mcpFacade;
const ADDR = `127.0.0.1:${PORT}`;
const HEALTH_TIMEOUT_MS = 5000;

interface FacadeHealth {
  status: 'ok';
  service: 'workspacer-mcp-facade';
  hubConnected: true;
  pluginCatalogReady: true;
  listenAddr: string;
  hubUrl: string;
}

/** A 200 from an arbitrary/disconnected listener is not an adoptable facade. */
async function probeFacadeHealth(url: string, timeoutMs = 1200): Promise<boolean> {
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), timeoutMs);
  try {
    const response = await fetch(url, { signal: ctl.signal });
    if (!response.ok) return false;
    const health = (await response.json()) as Partial<FacadeHealth>;
    return (
      health.status === 'ok' &&
      health.service === 'workspacer-mcp-facade' &&
      health.hubConnected === true &&
      health.pluginCatalogReady === true &&
      health.listenAddr === ADDR &&
      health.hubUrl === hubBusUrl()
    );
  } catch {
    return false;
  } finally {
    clearTimeout(timer);
  }
}

let child: ChildProcess | null = null;
let readyPromise: Promise<void> | null = null;
let ensurePromise: Promise<void> | null = null;
/** Set by stopMcpFacade() / app shutdown so an intentional kill isn't respawned. */
let intentionalStop = false;
/** True when a healthy facade owned by `workspacer serve` was adopted. */
let adoptedExternal = false;
/** Fence every owned-child/restart attempt. A timer or exit handler from an
 * older generation must never disturb a listener adopted by a newer start. */
let generation = 0;
let restartTimer: NodeJS.Timeout | null = null;
const backoff = new RestartBackoff();

function cancelScheduledRestart(): void {
  if (restartTimer) clearTimeout(restartTimer);
  restartTimer = null;
}

function exeName(): string {
  return process.platform === 'win32' ? 'mcp.exe' : 'mcp';
}

function mcpBinaryPath(): string {
  if (process.env.ELECTRON_DEV || !app.isPackaged) {
    return path.join(app.getAppPath(), '..', '..', 'services', 'hub', exeName());
  }
  return path.join(process.resourcesPath, 'hub', exeName());
}

/** Spawn the facade. Idempotent — repeat calls return the existing ready promise. */
export function startMcpFacade(): Promise<void> {
  if (readyPromise) return readyPromise;
  cancelScheduledRestart();
  const myGeneration = ++generation;
  intentionalStop = false;
  readyPromise = observeRuntimeStart(
    'facade',
    (async () => {
      // `workspacer serve` may already own the canonical facade. Adopt a
      // healthy listener before considering stale-port cleanup; otherwise the
      // desktop kills the CLI's child and two supervisors fight over :7897.
      if (await probeFacadeHealth(`http://${ADDR}/health`)) {
        if (myGeneration !== generation) return;
        adoptedExternal = true;
        backoff.reset();
        console.log(`[mcp] adopted healthy external facade at ${ADDR}`);
        return;
      }

      const bin = mcpBinaryPath();
      if (!fs.existsSync(bin)) {
        throw new Error(
          `mcp facade binary not found at ${bin} (run: cd services/hub && go build -o mcp ./cmd/mcp)`,
        );
      }
      adoptedExternal = false;
      await launch(bin, myGeneration);
    })(),
  );
  return readyPromise;
}

/** Launch gate used by every supported agent spawn. App startup is allowed to
 * start the facade optimistically, but no session receives its URL and no
 * bearer token is minted until this verifies the exact facade identity, bind,
 * hub connection, and initial plugin catalog. Re-probing after the startup
 * promise resolves also covers an adopted facade disappearing or becoming
 * disconnected between app boot and a later spawn. */
export function ensureMcpFacadeReady(): Promise<void> {
  if (ensurePromise) return ensurePromise;
  ensurePromise = (async () => {
    await startMcpFacade();
    if (await probeFacadeHealth(`http://${ADDR}/health`)) return;

    // A resolved startup promise is stale. Stop an owned child (or forget an
    // adopted external listener) without letting its exit race spawn a second
    // replacement, then run the normal start/adopt path once more.
    await stopMcpFacade();
    await startMcpFacade();
    if (!(await probeFacadeHealth(`http://${ADDR}/health`))) {
      await stopMcpFacade();
      throw new Error('mcp facade is not connected to the hub with a ready plugin catalog');
    }
  })().finally(() => {
    ensurePromise = null;
  });
  return ensurePromise;
}

/**
 * The STATIC bearer token the facade would accept on its own inbound surface
 * (/mcp, /sse) — the hub bus token, deliberately, since holding it already
 * grants the same capabilities directly over the bus, so reusing it adds no
 * authority while keeping one secret to mint, persist (0600) and rotate.
 *
 * It is still not handed to the facade, and that is now a shrug rather than a
 * gap. A facade-wide static secret was only ever one way to answer "who may
 * call this". The per-session bearers answer it better (an identity per
 * lifecycle, revoked when the session ends) and the binary's `-untokened deny`
 * default answers the rest — a caller with no credential gets 401 — so there is
 * nothing left for a static token to close. Setting it would only ADD a shared,
 * long-lived operator credential in a file, which is strictly weaker than what
 * is there now.
 *
 * Kept exported because it is the value to pass as WKS_MCP_TOKEN if the facade
 * is ever bound to a non-loopback address with untokened access dialled back
 * open (cmd/mcp's checkBindPolicy refuses that combination outright).
 */
export function getMcpFacadeToken(): string {
  return getHubToken();
}

/**
 * The untokened-access dial from config: `facade.untokenedAccess`, an OPTIONAL
 * key read leniently off the config object (it is deliberately not part of the
 * defaults pipeline — absent means "pass no flag" and the facade keeps its own
 * default, which is `deny`). Setting it to `operator` is the OPT-IN that
 * restores the pre-lockdown behaviour for a hand-configured local MCP client
 * that cannot carry a token; `view` is the halfway house — read-only tools, but
 * still every transcript, to anyone who reaches the port. Only the three values
 * the binary accepts pass through;
 * anything else is ignored with a warning rather than forwarded, because
 * cmd/mcp fails startup on an unknown -untokened value and a config typo must
 * not take down the whole control plane.
 */
function untokenedAccessSetting(): 'operator' | 'view' | 'deny' | null {
  try {
    const cfg = configService.getConfig() as unknown as {
      facade?: { untokenedAccess?: unknown };
    };
    const v = cfg.facade?.untokenedAccess;
    if (v === 'operator' || v === 'view' || v === 'deny') return v;
    if (v != null) {
      console.warn(
        `[mcp] ignoring invalid facade.untokenedAccess ${JSON.stringify(v)} (want operator|view|deny)`,
      );
    }
  } catch {
    /* config unavailable — fall through to the facade's own default */
  }
  return null;
}

/** Spawn the process and wire up exit-driven restart. Returns the health promise. */
function launch(bin: string, launchGeneration: number): Promise<void> {
  if (launchGeneration !== generation || intentionalStop || adoptedExternal) {
    return Promise.resolve();
  }
  killStaleListener(PORT, 'mcp', bin);

  const args = ['--addr', ADDR, '--hub', hubBusUrl()];
  const untokened = untokenedAccessSetting();
  if (untokened) args.push('--untokened', untokened);
  // The bus token rides the environment rather than argv: /proc/<pid>/cmdline is
  // world-readable, so a `--token <secret>` flag hands the secret to every local
  // user. The facade's --token flag already defaults to os.Getenv("HUB_TOKEN")
  // (cmd/mcp/main.go), so dropping the flag changes nothing else.
  //
  // WKS_MCP_TOKEN is deliberately NOT set: credential-less callers are already
  // refused by the binary's `-untokened deny` default, and every session this
  // app spawns presents its own scoped token, so a shared static secret would
  // add authority without closing anything. See getMcpFacadeToken.
  //
  // No --untokened flag either, unless the user set facade.untokenedAccess: the
  // binary's default IS the shipped policy, and passing it from here too would
  // only make two places to change it.
  const token = getHubToken();
  const env = token ? { HUB_TOKEN: token } : undefined;

  console.log(`[mcp] spawning ${bin} (addr ${ADDR}, hub ${hubBusUrl()})`);
  backoff.markStarted();
  const launchedChild = spawn(bin, args, daemonSpawnOptions(env));
  child = launchedChild;

  const healthAbort = new AbortController();

  launchedChild.stdout?.on('data', (d) => process.stdout.write(`[mcp] ${d}`));
  launchedChild.stderr?.on('data', (d) => process.stderr.write(`[mcp] ${d}`));
  launchedChild.on('exit', (code, signal) => {
    console.log(`[mcp] exited code=${code} signal=${signal}`);
    if (child === launchedChild) child = null;
    noteRuntimePhase('facade', 'failed');
    if (launchGeneration === generation) readyPromise = null;
    healthAbort.abort();
    if (!intentionalStop && launchGeneration === generation && !adoptedExternal)
      scheduleRestart(bin, launchGeneration);
  });

  return (async () => {
    try {
      await waitForHealth(`http://${ADDR}/health`, HEALTH_TIMEOUT_MS, 'mcp', healthAbort.signal);
      if (!(await probeFacadeHealth(`http://${ADDR}/health`))) {
        throw new Error('mcp facade health did not confirm hub and plugin catalog readiness');
      }
      backoff.reset();
    } catch (error) {
      // A process that merely bound the port but never became our connected,
      // catalog-ready facade is not adoptable. Tear down the process we own;
      // the exit handler retains the normal restart/backoff policy.
      await gracefulStop(launchedChild, 'mcp');
      throw error;
    }
  })();
}

/** Respawn after an unexpected exit, with exponential backoff. */
function scheduleRestart(bin: string, launchGeneration: number): void {
  const delay = backoff.nextDelay();
  if (delay === null) {
    console.error(
      '[mcp] crashed too many times; giving up auto-restart. Restart the app to recover.',
    );
    return;
  }
  console.warn(`[mcp] unexpected exit — restarting in ${delay}ms`);
  cancelScheduledRestart();
  restartTimer = setTimeout(() => {
    restartTimer = null;
    if (intentionalStop || child || adoptedExternal || launchGeneration !== generation) return;
    readyPromise = observeRuntimeStart('facade', launch(bin, launchGeneration));
    readyPromise.catch((err) => console.error('[mcp] restart failed health check:', err));
  }, delay);
}

export function stopMcpFacade(): Promise<void> {
  ++generation;
  cancelScheduledRestart();
  intentionalStop = true;
  backoff.reset();
  adoptedExternal = false;
  const c = child;
  child = null;
  readyPromise = null;
  return gracefulStop(c, 'mcp');
}

export const MCP_FACADE_PORT = PORT;
