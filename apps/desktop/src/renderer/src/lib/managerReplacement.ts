import { useSyncExternalStore } from 'react';
import {
  MANAGER_REPLACEMENT_UNAVAILABLE,
  type ManagerReplacementRequest,
  type ManagerReplacementResponse,
  type ManagerReplacementView,
} from '../../../main/shared/managerReplacement';
import type { AgentWorkspace } from '../types/pane';

let current: ManagerReplacementResponse = {
  available: false,
  operations: [],
  error: MANAGER_REPLACEMENT_UNAVAILABLE,
};
const listeners = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | undefined;
let polling = false;
function publish(next: ManagerReplacementResponse) {
  if (JSON.stringify(current) === JSON.stringify(next)) return;
  current = next;
  listeners.forEach((fn) => fn());
}
export async function managerReplacementRequest(
  request: ManagerReplacementRequest,
): Promise<ManagerReplacementResponse> {
  try {
    const api = window.electronAPI.managerReplacement;
    const response = api ? await api(request) : undefined;
    const result =
      response && typeof response.available === 'boolean' && Array.isArray(response.operations)
        ? response
        : { available: false, operations: [], error: MANAGER_REPLACEMENT_UNAVAILABLE };
    publish(result);
    return result;
  } catch (error) {
    const result = { ...current, error: error instanceof Error ? error.message : String(error) };
    publish(result);
    return result;
  }
}
async function poll() {
  if (polling) return;
  polling = true;
  try {
    await managerReplacementRequest({ action: 'list' });
  } finally {
    polling = false;
  }
}
function subscribe(fn: () => void) {
  listeners.add(fn);
  if (!timer && window.electronAPI.managerReplacement) {
    void poll();
    timer = setInterval(() => void poll(), 1000);
  }
  return () => {
    listeners.delete(fn);
    if (!listeners.size && timer) {
      clearInterval(timer);
      timer = undefined;
    }
  };
}
export function useManagerReplacementStatus(): ManagerReplacementResponse {
  return useSyncExternalStore(
    subscribe,
    () => current,
    () => current,
  );
}

/** Bind the existing workspace/pane, including after saved-layout hydration.
 * Never changes another pane or creates a successor workspace. */
export function bindManagerReplacements(
  agents: AgentWorkspace[],
  operations: ManagerReplacementView[],
): AgentWorkspace[] {
  let result = agents;
  for (const op of operations) {
    if (!op.committed) continue;
    const owner = result.find(
      (a) =>
        !a.hub &&
        a.tabs.some((t) => t.panes.some((p) => p.id === op.paneId)) &&
        [op.sourceSessionId, op.successorSessionId].includes(a.sessionId ?? a.lastSessionId ?? ''),
    );
    if (!owner) continue;
    const pane = owner.tabs.flatMap((t) => t.panes).find((p) => p.id === op.paneId)!;
    if (
      owner.id === op.workspaceId &&
      owner.sessionId === op.successorSessionId &&
      pane.attachSessionId === op.successorSessionId &&
      !pane.resumeSessionId
    )
      continue;
    result = result
      .filter((a) => a === owner || a.sessionId !== op.successorSessionId)
      .map((a) => {
        if (a !== owner) return a;
        return {
          ...a,
          id: op.workspaceId,
          manager: true,
          sessionId: op.successorSessionId,
          lastSessionId: undefined,
          tabs: a.tabs.map((t) => ({
            ...t,
            panes: t.panes.map((p) =>
              p.id === op.paneId
                ? {
                    ...p,
                    attachSessionId: op.successorSessionId,
                    resumeSessionId: undefined,
                    initialPrompt: undefined,
                    expectHistory: false,
                  }
                : p,
            ),
          })),
        };
      });
  }
  return result;
}
