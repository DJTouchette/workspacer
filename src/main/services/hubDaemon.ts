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
import { spawn, ChildProcess, execSync } from 'child_process';
import { app } from 'electron';
import { CLAUDEMON_API_URL } from './claudemonDaemon';

const PORT = 7895;
const HEALTH_TIMEOUT_MS = 5000;

let child: ChildProcess | null = null;
let readyPromise: Promise<void> | null = null;

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
  const cfg = process.env.XDG_CONFIG_HOME || path.join(os.homedir(), '.config');
  return path.join(cfg, 'workspacer', 'plugins');
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

function killStaleListener(port: number): void {
  try {
    if (process.platform === 'win32') {
      const out = execSync(`netstat -ano | findstr "127.0.0.1:${port}"`, { encoding: 'utf-8', timeout: 3000 });
      const match = out.match(/LISTENING\s+(\d+)/);
      if (match) {
        const pid = parseInt(match[1], 10);
        if (pid && pid !== process.pid) execSync(`taskkill /F /PID ${pid}`, { timeout: 3000 });
      }
    } else {
      const out = execSync(`lsof -ti :${port}`, { encoding: 'utf-8', timeout: 3000 });
      for (const line of out.trim().split('\n')) {
        const pid = parseInt(line, 10);
        if (pid && pid !== process.pid) process.kill(pid, 'SIGTERM');
      }
    }
  } catch {
    // No listener, or we lack rights — let the daemon try anyway.
  }
}

async function waitForHealth(): Promise<void> {
  const deadline = Date.now() + HEALTH_TIMEOUT_MS;
  let lastErr: unknown = null;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${PORT}/health`);
      if (res.ok) return;
    } catch (err) {
      lastErr = err;
    }
    await new Promise(r => setTimeout(r, 100));
  }
  throw new Error(`hub /health did not respond within ${HEALTH_TIMEOUT_MS}ms (last error: ${String(lastErr)})`);
}

/** Spawn the hub. Idempotent — repeat calls return the existing ready promise. */
export function startHub(): Promise<void> {
  if (readyPromise) return readyPromise;

  const bin = hubBinaryPath();
  if (!fs.existsSync(bin)) {
    return Promise.reject(new Error(`hub binary not found at ${bin} (run: cd hub && go build -o hub ./cmd/hub)`));
  }

  killStaleListener(PORT);

  const pluginsDir = ensurePluginsDir();
  console.log(`[hub] spawning ${bin}`);
  child = spawn(bin, [
    '--addr', `127.0.0.1:${PORT}`,
    '--claudemon-events', `${CLAUDEMON_API_URL}/events`,
    '--plugins-dir', pluginsDir,
  ], {
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  });

  child.stdout?.on('data', d => process.stdout.write(`[hub] ${d}`));
  child.stderr?.on('data', d => process.stderr.write(`[hub] ${d}`));
  child.on('exit', (code, signal) => {
    console.log(`[hub] exited code=${code} signal=${signal}`);
    child = null;
    readyPromise = null;
  });

  readyPromise = waitForHealth();
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
