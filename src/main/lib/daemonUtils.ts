/**
 * Shared utilities for daemon management (claudemon, hub, …).
 * All daemons use the same port-kill and health-poll logic; this file is the
 * single source of truth so neither daemon duplicates it.
 */

import { execSync } from 'child_process';

// ── Port registry ────────────────────────────────────────────────────────────

export const PORTS = {
  /** claudemon: hook ingestion (receives Claude Code hook events) */
  claudemonHook: 7890,
  /** claudemon: API / session state / control */
  claudemonApi: 7891,
  /** hub: control-plane event bus */
  hub: 7895,
} as const;

// ── killStaleListener ────────────────────────────────────────────────────────

/**
 * Kill any process currently listening on `port` that isn't this process.
 * `label` is used only for the log line, e.g. "[claudemon]" or "[hub]".
 */
export function killStaleListener(port: number, label: string): void {
  try {
    if (process.platform === 'win32') {
      const out = execSync(`netstat -ano | findstr "127.0.0.1:${port}"`, {
        encoding: 'utf-8',
        timeout: 3000,
      });
      const match = out.match(/LISTENING\s+(\d+)/);
      if (match) {
        const pid = parseInt(match[1], 10);
        if (pid && pid !== process.pid) {
          console.log(`[${label}] killing stale listener on :${port} pid=${pid}`);
          execSync(`taskkill /F /PID ${pid}`, { timeout: 3000 });
        }
      }
    } else {
      const out = execSync(`lsof -ti :${port}`, { encoding: 'utf-8', timeout: 3000 });
      for (const line of out.trim().split('\n')) {
        const pid = parseInt(line, 10);
        if (pid && pid !== process.pid) {
          console.log(`[${label}] killing stale listener on :${port} pid=${pid}`);
          process.kill(pid, 'SIGTERM');
        }
      }
    }
  } catch {
    // No listener, or we don't have rights — let the daemon try anyway.
  }
}

// ── waitForHealth ────────────────────────────────────────────────────────────

/**
 * Poll `url` until it returns HTTP 200 OK, or until `timeoutMs` elapses.
 * `label` appears in the error message, e.g. "claudemon" or "hub".
 */
export async function waitForHealth(
  url: string,
  timeoutMs: number,
  label: string,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let lastErr: unknown = null;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url);
      if (res.ok) return;
    } catch (err) {
      lastErr = err;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(
    `${label} /health did not respond within ${timeoutMs}ms (last error: ${String(lastErr)})`,
  );
}
