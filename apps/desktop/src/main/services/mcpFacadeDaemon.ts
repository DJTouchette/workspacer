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
import { HUB_BUS_URL, getHubToken } from './hubDaemon';

const PORT = PORTS.mcpFacade;
const ADDR = `127.0.0.1:${PORT}`;
const HEALTH_TIMEOUT_MS = 5000;

let child: ChildProcess | null = null;
let readyPromise: Promise<void> | null = null;
/** Set by stopMcpFacade() / app shutdown so an intentional kill isn't respawned. */
let intentionalStop = false;
const backoff = new RestartBackoff();

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

  const bin = mcpBinaryPath();
  if (!fs.existsSync(bin)) {
    return Promise.reject(
      new Error(
        `mcp facade binary not found at ${bin} (run: cd services/hub && go build -o mcp ./cmd/mcp)`,
      ),
    );
  }

  intentionalStop = false;
  return launch(bin);
}

/** Spawn the process and wire up exit-driven restart. Returns the health promise. */
function launch(bin: string): Promise<void> {
  killStaleListener(PORT, 'mcp');

  const args = ['--addr', ADDR, '--hub', HUB_BUS_URL];
  const token = getHubToken();
  if (token) args.push('--token', token);

  console.log(`[mcp] spawning ${bin} (addr ${ADDR}, hub ${HUB_BUS_URL})`);
  backoff.markStarted();
  child = spawn(bin, args, daemonSpawnOptions());

  const healthAbort = new AbortController();

  child.stdout?.on('data', (d) => process.stdout.write(`[mcp] ${d}`));
  child.stderr?.on('data', (d) => process.stderr.write(`[mcp] ${d}`));
  child.on('exit', (code, signal) => {
    console.log(`[mcp] exited code=${code} signal=${signal}`);
    child = null;
    readyPromise = null;
    healthAbort.abort();
    if (!intentionalStop) scheduleRestart(bin);
  });

  readyPromise = waitForHealth(
    `http://${ADDR}/health`,
    HEALTH_TIMEOUT_MS,
    'mcp',
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
    console.error(
      '[mcp] crashed too many times; giving up auto-restart. Restart the app to recover.',
    );
    return;
  }
  console.warn(`[mcp] unexpected exit — restarting in ${delay}ms`);
  setTimeout(() => {
    if (intentionalStop || child) return; // stopped, or already back up
    launch(bin).catch((err) => console.error('[mcp] restart failed health check:', err));
  }, delay);
}

export function stopMcpFacade(): Promise<void> {
  intentionalStop = true;
  backoff.reset();
  const c = child;
  child = null;
  readyPromise = null;
  return gracefulStop(c, 'mcp');
}

export const MCP_FACADE_PORT = PORT;
