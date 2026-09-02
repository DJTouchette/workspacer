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
import {
  killStaleListener,
  waitForHealth as waitForHealthShared,
  probeHealth,
  PORTS,
  RestartBackoff,
  daemonSpawnOptions,
  gracefulStop,
} from '../lib/daemonUtils';
import { notifySystem } from './systemNotice';
import { configService } from './configService';

const HOOK_PORT = PORTS.claudemonHook;
const API_PORT = PORTS.claudemonApi;
const HEALTH_TIMEOUT_MS = 5000;

/** Private overlay settings file claudemon writes when `claude.settingsOverlay`
 *  is on — passed to `claude` via `--settings` so we never touch the user's
 *  global `~/.claude/settings.json`. Resolved lazily (not at module load) so it
 *  doesn't call into electron's `app` before it's ready or under test mocks. */
export function claudemonOverlayPath(): string {
  return path.join(app.getPath('home'), '.workspacer', 'claude-settings.json');
}

/** True when the experimental settings-overlay mode is enabled in config. */
export function claudeSettingsOverlayEnabled(): boolean {
  return configService.getConfig().claude?.settingsOverlay === true;
}

/**
 * `agents.binaries.<provider>` as the daemon's `WKS_<PROVIDER>_BIN` environment.
 *
 * claudemon resolves a provider launcher itself — it stopped honouring a
 * caller-supplied `bin` on the models route, because that route is reachable
 * cross-origin by any page in the user's browser and a caller-supplied path is
 * a caller-supplied program. Its own resolution reads this environment first,
 * then PATH. The Go brain reads the same config key directly, so without this
 * the two would disagree about which binary a provider means: the picker would
 * silently use whatever is on PATH while everything else used the configured
 * path. Config is read at spawn, so an edit takes effect on the next daemon
 * start (the same rule as every other flag passed here).
 */
export function providerBinaryEnv(): NodeJS.ProcessEnv {
  const binaries = configService.getConfig().agents?.binaries ?? {};
  const env: NodeJS.ProcessEnv = {};
  for (const [provider, bin] of Object.entries(binaries)) {
    const trimmed = typeof bin === 'string' ? bin.trim() : '';
    if (trimmed) env[`WKS_${provider.toUpperCase()}_BIN`] = trimmed;
  }
  return env;
}

/**
 * `usage.pollOnBoot` as the daemon's environment.
 *
 * claudemon's account-usage poller decides at boot whether to iterate every
 * CONFIGURED Claude root or only the roots of live sessions, and it has no
 * config file of its own — it reads a handful of env vars at the point of use
 * (CLAUDE_CONFIG_DIR is the precedent). So the setting travels in the spawn
 * environment, exactly like the provider binaries above.
 *
 * Sent explicitly in BOTH directions rather than only when off: this process
 * respawns the daemon after a crash, and an env var that is only ever added
 * would leave a stale "0" behind from the previous spawn's environment. The
 * Go launcher (services/hub/cmd/workspacer/plan.go) writes the same variable
 * with the same 0/1 spelling. Absent means ON, which is what an older desktop
 * that knows nothing about the key gives a newer daemon.
 */
export function usagePollEnv(): NodeJS.ProcessEnv {
  const on = configService.getConfig().usage?.pollOnBoot !== false;
  return { WORKSPACER_USAGE_POLL_ON_BOOT: on ? '1' : '0' };
}

let child: ChildProcess | null = null;
let readyPromise: Promise<void> | null = null;
/** Set by stopClaudemon() / app shutdown so an intentional kill isn't respawned. */
let intentionalStop = false;
/**
 * True when we ADOPTED an already-running external claudemon (e.g. one spawned
 * by `workspacer serve`) instead of spawning our own. Adopted daemons are not
 * ours to manage: we never supervise/restart them and never signal them on
 * quit — only an owned child (spawned by us, tracked in `child`) is stopped.
 */
let adopted = false;
const backoff = new RestartBackoff();

/** Whether the running claudemon was adopted from an external server. */
export function isClaudemonAdopted(): boolean {
  return adopted;
}

function exeName(): string {
  return process.platform === 'win32' ? 'claudemon.exe' : 'claudemon';
}

/** Resolve the claudemon binary path for the current run mode. */
export function claudemonBinaryPath(): string {
  if (process.env.ELECTRON_DEV || !app.isPackaged) {
    // app.getAppPath() in dev points at apps/desktop (where package.json lives);
    // the claudemon source sits at <repo>/services/claudemon.
    return path.join(
      app.getAppPath(),
      '..',
      '..',
      'services',
      'claudemon',
      'target',
      'release',
      exeName(),
    );
  }
  return path.join(process.resourcesPath, 'claudemon', exeName());
}

/**
 * Start the daemon: adopt a healthy external one, else spawn our own.
 * Idempotent — repeat calls return the existing ready promise.
 *
 * Adopt-don't-kill: `workspacer serve` runs claudemon on the same default
 * ports. If something HEALTHY already answers /health we adopt it — resolve
 * ready without spawning, without killing, and without supervising it (its
 * lifetime belongs to whoever started it). We only fall through to the
 * kill-stale + spawn path when the port is dead or answering garbage — the
 * stale-orphan case killStaleListener exists for. The probe runs before the
 * binary check on purpose: adopting an external daemon needs no local binary.
 */
export function startClaudemon(): Promise<void> {
  if (readyPromise) return readyPromise;

  intentionalStop = false;
  const starting = (async () => {
    if (await probeHealth(`http://127.0.0.1:${API_PORT}/health`)) {
      adopted = true;
      console.log(
        `[claudemon] adopted external daemon on :${API_PORT} (workspacer serve?) — not spawning, not supervising`,
      );
      return;
    }
    adopted = false;
    const bin = claudemonBinaryPath();
    if (!fs.existsSync(bin)) {
      throw new Error(`claudemon binary not found at ${bin}`);
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
  // anything) is a stale orphan, not an adoptable daemon — clear it.
  killStaleListener(HOOK_PORT, 'claudemon', bin);
  killStaleListener(API_PORT, 'claudemon', bin);

  console.log(`[claudemon] spawning ${bin}`);
  backoff.markStarted();
  child = spawn(
    bin,
    ['serve', '--hook-port', String(HOOK_PORT), '--api-port', String(API_PORT)],
    daemonSpawnOptions({
      RUST_LOG: process.env.RUST_LOG ?? 'claudemon=info',
      ...providerBinaryEnv(),
      ...usagePollEnv(),
    }),
  );

  // AbortController so a fast-exiting daemon cancels the health-check poll
  // instead of spinning for the full HEALTH_TIMEOUT_MS.
  const healthAbort = new AbortController();

  child.stdout?.on('data', (d) => process.stdout.write(`[claudemon] ${d}`));
  child.stderr?.on('data', (d) => process.stderr.write(`[claudemon] ${d}`));
  child.on('exit', (code, signal) => {
    console.log(`[claudemon] exited code=${code} signal=${signal}`);
    child = null;
    readyPromise = null;
    healthAbort.abort(); // cancel any in-progress health poll
    if (!intentionalStop) scheduleRestart(bin);
  });

  readyPromise = waitForHealthShared(
    `http://127.0.0.1:${API_PORT}/health`,
    HEALTH_TIMEOUT_MS,
    'claudemon',
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
      level: 'error',
      key: 'claudemon-crashloop',
      title: 'Agent daemon (claudemon) keeps crashing',
      detail:
        'Gave up restarting it after repeated failures. Claude sessions won’t work until you restart the app.',
    });
    return;
  }
  console.warn(`[claudemon] unexpected exit — restarting in ${delay}ms`);
  setTimeout(() => {
    if (intentionalStop || child) return; // stopped, or already back up
    launch(bin).catch((err) => console.error('[claudemon] restart failed health check:', err));
  }, delay);
}

/**
 * Run `claudemon init`. By default this merges hook entries into the user's
 * global `~/.claude/settings.json`. When `claude.settingsOverlay` is enabled it
 * instead writes them to our private overlay file (`--overlay`) and strips any
 * stale global entries, leaving the global file otherwise untouched.
 */
export function runClaudemonInit(): Promise<void> {
  const bin = claudemonBinaryPath();
  if (!fs.existsSync(bin)) {
    return Promise.reject(new Error(`claudemon binary not found at ${bin}`));
  }
  const args = ['init', '--hook-port', String(HOOK_PORT)];
  if (claudeSettingsOverlayEnabled()) {
    args.push('--overlay', claudemonOverlayPath());
  }
  return new Promise((resolve, reject) => {
    const proc = spawn(bin, args, {
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true,
    });
    let stdout = '';
    let stderr = '';
    proc.stdout?.on('data', (d) => {
      stdout += d.toString();
    });
    proc.stderr?.on('data', (d) => {
      stderr += d.toString();
    });
    proc.on('exit', (code) => {
      if (code === 0) {
        if (stdout.trim()) console.log(`[claudemon init] ${stdout.trim()}`);
        resolve();
      } else {
        reject(
          new Error(`claudemon init failed (code=${code}): ${stderr.trim() || stdout.trim()}`),
        );
      }
    });
    proc.on('error', reject);
  });
}

export function stopClaudemon(): Promise<void> {
  intentionalStop = true;
  backoff.reset(); // clear failure counter so the next startClaudemon() begins fresh
  // Adopted daemon: it isn't ours — leave it running (quitting the app must not
  // take down `workspacer serve`). Only an owned child gets the graceful stop.
  if (adopted) {
    console.log('[claudemon] adopted daemon left running (owned by the external server)');
    adopted = false;
    readyPromise = null;
    return Promise.resolve();
  }
  const c = child;
  child = null;
  readyPromise = null;
  return gracefulStop(c, 'claudemon');
}

export const CLAUDEMON_HOOK_PORT = HOOK_PORT;
export const CLAUDEMON_API_PORT = API_PORT;
export const CLAUDEMON_API_URL = `http://127.0.0.1:${API_PORT}`;
