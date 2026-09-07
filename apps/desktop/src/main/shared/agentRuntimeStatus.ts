/** Read-only host lifecycle facts. Absence on older/remote hosts means unknown.
 * This is advisory: a successful probe never substitutes for a spawn result. */
export type RuntimePhase = 'unknown' | 'starting' | 'ready' | 'failed';
export interface AgentRuntimeStatus {
  claudemon: RuntimePhase;
  hub: RuntimePhase;
  facade: RuntimePhase;
}
export const UNKNOWN_RUNTIME: AgentRuntimeStatus = {
  claudemon: 'unknown',
  hub: 'unknown',
  facade: 'unknown',
};
export function runtimeLaunchState(
  status: AgentRuntimeStatus,
  managed: boolean,
): {
  blocked: boolean;
  detail: string;
} {
  if (status.claudemon === 'starting')
    return { blocked: true, detail: 'Agent runtime is starting. Check again shortly.' };
  if (status.claudemon === 'failed')
    return {
      blocked: true,
      detail:
        'Agent daemon (claudemon) is unavailable. Open system logs for details, then check again.',
    };
  if (managed && (status.hub === 'failed' || status.facade === 'failed'))
    return {
      blocked: true,
      detail:
        'Fleet Manager is unavailable: its hub or action tools failed. Direct local agents may still start. Open system logs, then check again.',
    };
  if (managed && (status.hub === 'starting' || status.facade === 'starting'))
    return {
      blocked: true,
      detail: 'Fleet Manager is starting its hub and action tools. Check again shortly.',
    };
  if (
    status.claudemon === 'unknown' ||
    (managed && (status.hub === 'unknown' || status.facade === 'unknown'))
  )
    return {
      blocked: false,
      detail:
        'Runtime readiness is unknown on this host. You can try dispatch; any launch failure stays retryable.',
    };
  if (!managed && (status.hub !== 'ready' || status.facade !== 'ready'))
    return {
      blocked: false,
      detail: 'Local agent runtime is ready; Fleet Manager and hub features may be unavailable.',
    };
  return {
    blocked: false,
    detail: managed
      ? 'Fleet Manager runtime is ready. Provider sign-in may still be required.'
      : 'Local agent runtime is ready. Provider sign-in may still be required.',
  };
}
