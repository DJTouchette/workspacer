/**
 * Spawns and supervises the bundled `claudemon` daemon, which replaces the
 * old in-process hook server. The daemon ingests Claude Code hook events on
 * 7890, exposes session state + bidirectional control on 7891, and parses
 * `~/.claude/projects/*.jsonl` transcripts for us.
 *
 * Binary resolution:
 *   - dev (ELECTRON_DEV=1): <repo>/services/claudemon/target/release/claudemon[.exe]
 *   - packaged:             <resourcesPath>/claudemon/claudemon[.exe]
 */

import * as fs from 'fs';
import * as path from 'path';
import { spawn, ChildProcess } from 'child_process';
import { app } from 'electron';
import { killStaleListener, waitForHealth as waitForHealthShared, PORTS, RestartBackoff } from '../lib/daemonUtils';

const HOOK_PORT = PORTS.claudemonHook;
const API_PORT = PORTS.claudemonApi;
const HEALTH_TIMEOUT_MS = 5000;

let child: ChildProcess | null = null;
let readyPromise: Promise<void> | null = null;
/** Set by stopClaudemon() / app shutdown so an intentional kill isn't respawned. */
let intentionalStop = false;
const backoff = new RestartBackoff();

function exeName(): string {
  return process.platform === 'win32' ? 'claudemon.exe' : 'claudemon';
}

/** Resolve the claudemon binary path for the current run mode. */
export function claudemonBinaryPath(): string {
  if (process.env.ELECTRON_DEV || !app.isPackaged) {
    // app.getAppPath() in dev points at apps/desktop (where package.json lives);
    // the claudemon source sits at <repo>/services/claudemon.
    return path.join(app.getAppPath(), '..', '..', 'services', 'claudemon', 'target', 'release', exeName());
  }
  return path.join(process.resourcesPath, 'claudemon', exeName());
}


/** Spawn the daemon. Idempotent — repeat calls return the existing ready promise. */
export function startClaudemon(): Promise<void> {
  if (readyPromise) return readyPromise;

  const bin = claudemonBinaryPath();
  if (!fs.existsSync(bin)) {
    return Promise.reject(new Error(`claudemon binary not found at ${bin}`));
  }

  intentionalStop = false;
  return launch(bin);
}

/** Spawn the process and wire up exit-driven restart. Returns the health promise. */
function launch(bin: string): Promise<void> {
  killStaleListener(HOOK_PORT, 'claudemon');
  killStaleListener(API_PORT, 'claudemon');

  console.log(`[claudemon] spawning ${bin}`);
  backoff.markStarted();
  child = spawn(bin, ['serve', '--hook-port', String(HOOK_PORT), '--api-port', String(API_PORT)], {
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, RUST_LOG: process.env.RUST_LOG ?? 'claudemon=info' },
    windowsHide: true,
  });

  child.stdout?.on('data', d => process.stdout.write(`[claudemon] ${d}`));
  child.stderr?.on('data', d => process.stderr.write(`[claudemon] ${d}`));
  child.on('exit', (code, signal) => {
    console.log(`[claudemon] exited code=${code} signal=${signal}`);
    child = null;
    readyPromise = null;
    if (!intentionalStop) scheduleRestart(bin);
  });

  readyPromise = waitForHealthShared(`http://127.0.0.1:${API_PORT}/health`, HEALTH_TIMEOUT_MS, 'claudemon')
    .then(() => { backoff.reset(); });
  return readyPromise;
}

/** Respawn after an unexpected exit, with exponential backoff. */
function scheduleRestart(bin: string): void {
  const delay = backoff.nextDelay();
  if (delay === null) {
    console.error('[claudemon] crashed too many times; giving up auto-restart. Restart the app to recover.');
    return;
  }
  console.warn(`[claudemon] unexpected exit — restarting in ${delay}ms`);
  setTimeout(() => {
    if (intentionalStop || child) return; // stopped, or already back up
    launch(bin).catch(err => console.error('[claudemon] restart failed health check:', err));
  }, delay);
}

/** Run `claudemon init` to merge hook entries into ~/.claude/settings.json. */
export function runClaudemonInit(): Promise<void> {
  const bin = claudemonBinaryPath();
  if (!fs.existsSync(bin)) {
    return Promise.reject(new Error(`claudemon binary not found at ${bin}`));
  }
  return new Promise((resolve, reject) => {
    const proc = spawn(bin, ['init', '--hook-port', String(HOOK_PORT)], {
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true,
    });
    let stdout = '';
    let stderr = '';
    proc.stdout?.on('data', d => { stdout += d.toString(); });
    proc.stderr?.on('data', d => { stderr += d.toString(); });
    proc.on('exit', code => {
      if (code === 0) {
        if (stdout.trim()) console.log(`[claudemon init] ${stdout.trim()}`);
        resolve();
      } else {
        reject(new Error(`claudemon init failed (code=${code}): ${stderr.trim() || stdout.trim()}`));
      }
    });
    proc.on('error', reject);
  });
}

export function stopClaudemon(): void {
  intentionalStop = true;
  if (child) {
    try { child.kill(); } catch {}
    child = null;
  }
  readyPromise = null;
}

export const CLAUDEMON_HOOK_PORT = HOOK_PORT;
export const CLAUDEMON_API_PORT = API_PORT;
export const CLAUDEMON_API_URL = `http://127.0.0.1:${API_PORT}`;
