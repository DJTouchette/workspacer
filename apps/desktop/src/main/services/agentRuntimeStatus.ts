import { PORTS, probeHealth } from '../lib/daemonUtils';
import {
  UNKNOWN_RUNTIME,
  type AgentRuntimeStatus,
  type RuntimePhase,
} from '../shared/agentRuntimeStatus';

// Written only by the existing daemon lifecycle owners. No second supervisor.
const state: AgentRuntimeStatus = { ...UNKNOWN_RUNTIME };
const observed = new Set<keyof AgentRuntimeStatus>();
const revisions = { claudemon: 0, hub: 0, facade: 0 };
const healthUrls = {
  claudemon: `http://127.0.0.1:${PORTS.claudemonApi}/health`,
  hub: `http://127.0.0.1:${PORTS.hub}/health`,
  facade: `http://127.0.0.1:${PORTS.mcpFacade}/health`,
};
export function noteRuntimePhase(component: keyof AgentRuntimeStatus, phase: RuntimePhase): void {
  if (phase !== 'unknown') observed.add(component);
  else observed.delete(component);
  state[component] = phase;
  revisions[component]++;
}
/** Dependencies queued at first window creation. Reopening a window must not
 * reset already-running daemons whose start functions return cached promises. */
export function noteRuntimePending(component: keyof AgentRuntimeStatus): void {
  if (!observed.has(component)) noteRuntimePhase(component, 'starting');
}
/** Track the same health promise startup/adoption already awaits. */
export function observeRuntimeStart(
  component: keyof AgentRuntimeStatus,
  promise: Promise<void>,
  healthUrl?: string,
): Promise<void> {
  if (healthUrl) healthUrls[component] = healthUrl;
  noteRuntimePhase(component, 'starting');
  const revision = revisions[component];
  return promise.then(
    () => {
      if (revision === revisions[component]) noteRuntimePhase(component, 'ready');
    },
    (error) => {
      if (revision === revisions[component]) noteRuntimePhase(component, 'failed');
      throw error;
    },
  );
}
/** Checks only health endpoints, never starts/stops processes or authenticates.
 * Probe adopted daemons too: they have no child exit event owned by this app. */
export async function readAgentRuntimeStatus(): Promise<AgentRuntimeStatus> {
  await Promise.all(
    (Object.keys(state) as Array<keyof AgentRuntimeStatus>).map(async (key) => {
      if (!observed.has(key) || state[key] === 'starting') return;
      const revision = revisions[key];
      const url = healthUrls[key];
      // The facade's HTTP listener alone is not its action-tool connection.
      let phase: RuntimePhase;
      if (key === 'facade') {
        try {
          const response = await fetch(url, { signal: AbortSignal.timeout(1200) });
          const body = response.ok ? ((await response.json()) as { hubConnected?: boolean }) : null;
          phase =
            !response.ok || body?.hubConnected === false
              ? 'failed'
              : body?.hubConnected === true
                ? 'ready'
                : 'unknown';
        } catch {
          phase = 'failed';
        }
      } else {
        phase = (await probeHealth(url)) ? 'ready' : 'failed';
      }
      if (revision === revisions[key]) {
        noteRuntimePhase(key, phase);
        observed.add(key); // an old facade may gain its additive health field after reconnect
      }
    }),
  );
  return { ...state };
}
