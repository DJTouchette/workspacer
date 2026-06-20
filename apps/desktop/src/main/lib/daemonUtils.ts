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
      // On macOS, lsof may not be on the packaged app's PATH; prefer the known
      // absolute path. On Linux /usr/bin/lsof is standard.
      const lsof = process.platform === 'darwin' ? '/usr/sbin/lsof' : 'lsof';
      const out = execSync(`${lsof} -ti :${port}`, { encoding: 'utf-8', timeout: 3000 });
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
 * Poll `url` until it returns HTTP 200 OK, until `timeoutMs` elapses, or until
 * `signal` is aborted (e.g. because the daemon exited before becoming healthy).
 * `label` appears in the error message, e.g. "claudemon" or "hub".
 */
export async function waitForHealth(
  url: string,
  timeoutMs: number,
  label: string,
  signal?: AbortSignal,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let lastErr: unknown = null;
  while (Date.now() < deadline) {
    if (signal?.aborted) {
      throw new Error(`${label} health check cancelled (daemon exited early)`);
    }
    try {
      const res = await fetch(url, { signal });
      if (res.ok) return;
    } catch (err) {
      if (signal?.aborted) {
        throw new Error(`${label} health check cancelled (daemon exited early)`);
      }
      lastErr = err;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(
    `${label} /health did not respond within ${timeoutMs}ms (last error: ${String(lastErr)})`,
  );
}

// ── RestartBackoff ───────────────────────────────────────────────────────────

export interface RestartBackoffOptions {
  /** Delay before the first restart. Default 1000ms. */
  baseMs?: number;
  /** Maximum backoff delay. Default 30000ms. */
  maxMs?: number;
  /** Give up after this many consecutive failures. Default 10. */
  maxAttempts?: number;
  /** If the process stayed up at least this long, the next crash is treated as
   *  a fresh failure (counter resets). Default 60000ms. */
  resetAfterMs?: number;
}

/**
 * Exponential-backoff bookkeeping for restarting a supervised daemon. A daemon
 * that crashes is respawned with growing delays; one that ran healthily past
 * `resetAfterMs` gets a clean slate so transient crashes don't exhaust the
 * budget. Pure state — the caller owns the actual spawn + timer.
 */
export class RestartBackoff {
  private attempts = 0;
  private startedAt = 0;
  constructor(private readonly opts: RestartBackoffOptions = {}) {}

  /** Record that the process (re)started, for the uptime reset heuristic. */
  markStarted(): void {
    this.startedAt = Date.now();
  }

  /** Manually clear the failure counter (e.g. after a confirmed-healthy start). */
  reset(): void {
    this.attempts = 0;
  }

  /** Delay (ms) before the next restart, or null once the budget is exhausted. */
  nextDelay(): number | null {
    const { baseMs = 1000, maxMs = 30000, maxAttempts = 10, resetAfterMs = 60000 } = this.opts;
    if (this.startedAt && Date.now() - this.startedAt >= resetAfterMs) this.attempts = 0;
    if (this.attempts >= maxAttempts) return null;
    const delay = Math.min(maxMs, baseMs * 2 ** this.attempts);
    this.attempts++;
    return delay;
  }
}
