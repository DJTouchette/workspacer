/**
 * Spawns and supervises the bundled `claudemon` daemon, which replaces the
 * old in-process hook server. The daemon ingests Claude Code hook events on
 * 7890, exposes session state + bidirectional control on 7891, and parses
 * `~/.claude/projects/*.jsonl` transcripts for us.
 *
 * Binary resolution:
 *   - dev (ELECTRON_DEV=1): <repo>/claudemon/target/release/claudemon[.exe]
 *   - packaged:             <resourcesPath>/claudemon/claudemon[.exe]
 */

import * as fs from 'fs';
import * as path from 'path';
import { spawn, ChildProcess, execSync } from 'child_process';
import { app } from 'electron';

const HOOK_PORT = 7890;
const API_PORT = 7891;
const HEALTH_TIMEOUT_MS = 5000;

let child: ChildProcess | null = null;
let readyPromise: Promise<void> | null = null;

function exeName(): string {
  return process.platform === 'win32' ? 'claudemon.exe' : 'claudemon';
}

/** Resolve the claudemon binary path for the current run mode. */
export function claudemonBinaryPath(): string {
  if (process.env.ELECTRON_DEV || !app.isPackaged) {
    // app.getAppPath() in dev points at the repo root (where package.json lives)
    return path.join(app.getAppPath(), 'claudemon', 'target', 'release', exeName());
  }
  return path.join(process.resourcesPath, 'claudemon', exeName());
}

/** Kill any process listening on the daemon's ports (stale Workspacer instance). */
function killStaleListener(port: number): void {
  try {
    if (process.platform === 'win32') {
      const out = execSync(`netstat -ano | findstr "127.0.0.1:${port}"`, { encoding: 'utf-8', timeout: 3000 });
      const match = out.match(/LISTENING\s+(\d+)/);
      if (match) {
        const pid = parseInt(match[1], 10);
        if (pid && pid !== process.pid) {
          console.log(`[claudemon] killing stale listener on :${port} pid=${pid}`);
          execSync(`taskkill /F /PID ${pid}`, { timeout: 3000 });
        }
      }
    } else {
      const out = execSync(`lsof -ti :${port}`, { encoding: 'utf-8', timeout: 3000 });
      for (const line of out.trim().split('\n')) {
        const pid = parseInt(line, 10);
        if (pid && pid !== process.pid) {
          console.log(`[claudemon] killing stale listener on :${port} pid=${pid}`);
          process.kill(pid, 'SIGTERM');
        }
      }
    }
  } catch {
    // No listener, or we don't have rights — let the daemon try anyway.
  }
}

async function waitForHealth(): Promise<void> {
  const deadline = Date.now() + HEALTH_TIMEOUT_MS;
  let lastErr: unknown = null;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${API_PORT}/health`);
      if (res.ok) return;
    } catch (err) {
      lastErr = err;
    }
    await new Promise(r => setTimeout(r, 100));
  }
  throw new Error(`claudemon /health did not respond within ${HEALTH_TIMEOUT_MS}ms (last error: ${String(lastErr)})`);
}

/** Spawn the daemon. Idempotent — repeat calls return the existing ready promise. */
export function startClaudemon(): Promise<void> {
  if (readyPromise) return readyPromise;

  const bin = claudemonBinaryPath();
  if (!fs.existsSync(bin)) {
    return Promise.reject(new Error(`claudemon binary not found at ${bin}`));
  }

  killStaleListener(HOOK_PORT);
  killStaleListener(API_PORT);

  console.log(`[claudemon] spawning ${bin}`);
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
  });

  readyPromise = waitForHealth();
  return readyPromise;
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
  if (child) {
    try { child.kill(); } catch {}
    child = null;
  }
  readyPromise = null;
}

export const CLAUDEMON_HOOK_PORT = HOOK_PORT;
export const CLAUDEMON_API_PORT = API_PORT;
export const CLAUDEMON_API_URL = `http://127.0.0.1:${API_PORT}`;
