/**
 * Spawns and supervises devdaemon and agent-manager.
 *
 * Binary resolution:
 *   - Uses binaries from PATH or a configured path.
 *   - devdaemon listens on :7880, agent-manager on :9800.
 *
 * Startup:
 *   1. Check if devdaemon is already running (GET /healthz).
 *   2. If not, spawn `devdaemon serve`.
 *   3. Check if agent-manager is already running (GET /api/health).
 *   4. If not, spawn `agent-manager serve --port 9800`.
 */

import { spawn, ChildProcess } from 'child_process';

const DAEMON_PORT = 7880;
const AGENT_MANAGER_PORT = 9800;
const HEALTH_TIMEOUT_MS = 8000;

let daemonChild: ChildProcess | null = null;
let agentManagerChild: ChildProcess | null = null;
let startPromise: Promise<void> | null = null;

async function isReachable(url: string): Promise<boolean> {
  try {
    const res = await fetch(url);
    return res.ok || res.status < 500;
  } catch {
    return false;
  }
}

async function waitForHealth(url: string, label: string): Promise<void> {
  const deadline = Date.now() + HEALTH_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (await isReachable(url)) return;
    await new Promise(r => setTimeout(r, 200));
  }
  throw new Error(`${label} did not respond within ${HEALTH_TIMEOUT_MS}ms`);
}

function spawnDaemon(): ChildProcess {
  console.log('[devdaemon] spawning devdaemon serve');
  const child = spawn('devdaemon', ['serve'], {
    stdio: ['ignore', 'pipe', 'pipe'],
    env: process.env,
    windowsHide: true,
    shell: true,
  });
  child.stdout?.on('data', d => process.stdout.write(`[devdaemon] ${d}`));
  child.stderr?.on('data', d => process.stderr.write(`[devdaemon] ${d}`));
  child.on('exit', (code, signal) => {
    console.log(`[devdaemon] exited code=${code} signal=${signal}`);
    daemonChild = null;
  });
  return child;
}

function spawnAgentManager(): ChildProcess {
  console.log('[agent-manager] spawning agent-manager serve --port 9800');
  const child = spawn('agent-manager', ['serve', '--port', String(AGENT_MANAGER_PORT)], {
    stdio: ['ignore', 'pipe', 'pipe'],
    env: process.env,
    windowsHide: true,
    shell: true,
  });
  child.stdout?.on('data', d => process.stdout.write(`[agent-manager] ${d}`));
  child.stderr?.on('data', d => process.stderr.write(`[agent-manager] ${d}`));
  child.on('exit', (code, signal) => {
    console.log(`[agent-manager] exited code=${code} signal=${signal}`);
    agentManagerChild = null;
  });
  return child;
}

/** Start devdaemon + agent-manager. Idempotent. */
export function startDevDaemon(): Promise<void> {
  if (startPromise) return startPromise;

  startPromise = (async () => {
    // 1. devdaemon
    const daemonUp = await isReachable(`http://127.0.0.1:${DAEMON_PORT}/healthz`);
    if (!daemonUp) {
      daemonChild = spawnDaemon();
      await waitForHealth(`http://127.0.0.1:${DAEMON_PORT}/healthz`, 'devdaemon');
    } else {
      console.log('[devdaemon] already running');
    }

    // 2. agent-manager
    const amUp = await isReachable(`http://127.0.0.1:${AGENT_MANAGER_PORT}/api/health`);
    if (!amUp) {
      agentManagerChild = spawnAgentManager();
      await waitForHealth(`http://127.0.0.1:${AGENT_MANAGER_PORT}/api/health`, 'agent-manager');
    } else {
      console.log('[agent-manager] already running');
    }

    console.log('[devdaemon] all services ready');
  })();

  startPromise.catch(() => {
    startPromise = null;
  });

  return startPromise;
}

/** Stop both child processes (if we spawned them). */
export function stopDevDaemon(): void {
  if (agentManagerChild) {
    try { agentManagerChild.kill(); } catch {}
    agentManagerChild = null;
  }
  if (daemonChild) {
    try { daemonChild.kill(); } catch {}
    daemonChild = null;
  }
  startPromise = null;
}

export const DEVDAEMON_URL = `http://127.0.0.1:${DAEMON_PORT}`;
export const AGENT_MANAGER_URL = `http://127.0.0.1:${AGENT_MANAGER_PORT}`;
