/** Readiness for the MCP listener owned by the shared hub service. */
import { noteRuntimePhase, observeRuntimeStart } from './agentRuntimeStatus';
import { PORTS } from '../lib/daemonUtils';
import { getHubToken, hubBusUrl } from './hubDaemon';
const PORT = PORTS.mcpFacade;
const ADDR = `127.0.0.1:${PORT}`;
const HEALTH = `http://${ADDR}/health`;
let readyPromise: Promise<void> | null = null;
let ensurePromise: Promise<void> | null = null;
let generation = 0;
let polling: AbortController | null = null;
function current(epoch: number): void {
  if (generation !== epoch) throw new Error('MCP readiness canceled');
}
async function probe(signal?: AbortSignal): Promise<boolean> {
  const controller = new AbortController();
  const abort = () => controller.abort();
  if (signal?.aborted) return false;
  signal?.addEventListener('abort', abort, { once: true });
  const timer = setTimeout(abort, 1200);
  try {
    const response = await fetch(HEALTH, { signal: controller.signal });
    if (!response.ok) return false;
    const health = (await response.json()) as Record<string, unknown>;
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
    signal?.removeEventListener('abort', abort);
  }
}
/** Observe the hub's listener; never create a second process or kill a port. */
export function startMcpFacade(): Promise<void> {
  if (readyPromise) return readyPromise;
  const mine = ++generation;
  const controller = new AbortController();
  polling = controller;
  const operation = (async () => {
    const deadline = Date.now() + 5000;
    while (Date.now() < deadline) {
      current(mine);
      const ready = await probe(controller.signal);
      current(mine);
      if (ready) return;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    throw new Error('The hub-owned MCP facade is not ready for this hub');
  })();
  const observed = observeRuntimeStart('facade', operation, HEALTH);
  readyPromise = observed;
  void observed.catch(() => {
    if (mine === generation && readyPromise === observed) readyPromise = null;
  });
  return observed;
}
/** Recheck identity and catalog readiness before injecting any session bearer. */
export function ensureMcpFacadeReady(): Promise<void> {
  if (ensurePromise) return ensurePromise;
  let operation: Promise<void>;
  operation = (async () => {
    let started = startMcpFacade();
    let mine = generation;
    await started;
    current(mine);
    let healthy = await probe();
    current(mine);
    if (healthy) return;
    // The hub supervisor owns recovery. Forget only this readiness observation.
    readyPromise = null;
    started = startMcpFacade();
    mine = generation;
    await started;
    current(mine);
    healthy = await probe();
    current(mine);
    if (!healthy) throw new Error('MCP facade lost its hub or plugin catalog');
  })().finally(() => {
    if (ensurePromise === operation) ensurePromise = null;
  });
  ensurePromise = operation;
  return operation;
}
export function getMcpFacadeToken(): string {
  return getHubToken();
}
/** Cancel observations. The hub owner alone stops the actual MCP listener. */
export async function stopMcpFacade(): Promise<void> {
  ++generation;
  polling?.abort();
  polling = null;
  readyPromise = null;
  ensurePromise = null;
  noteRuntimePhase('facade', 'unknown');
}
export const MCP_FACADE_PORT = PORT;
