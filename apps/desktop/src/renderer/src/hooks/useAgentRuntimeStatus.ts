import { useCallback, useEffect, useRef, useState } from 'react';
import {
  UNKNOWN_RUNTIME,
  runtimeLaunchState,
  type AgentRuntimeStatus,
  type RuntimePhase,
} from '../../../main/shared/agentRuntimeStatus';

/** Advisory read-only snapshot. Older hosts fail open, with an explicit unknown.
 * Every refresh supersedes prior requests. Nothing here clears a spawn error. */
export function useAgentRuntimeStatus(managed = false, remote = false) {
  const [status, setStatus] = useState<AgentRuntimeStatus>(UNKNOWN_RUNTIME);
  const generation = useRef(0);
  const refresh = useCallback(async () => {
    const request = ++generation.current;
    let next = UNKNOWN_RUNTIME;
    try {
      const result = remote ? undefined : await window.electronAPI.agentRuntimeStatus?.();
      if (result) {
        const phase = (value: unknown): RuntimePhase =>
          value === 'starting' || value === 'ready' || value === 'failed' ? value : 'unknown';
        next = {
          claudemon: phase(result.claudemon),
          hub: phase(result.hub),
          facade: phase(result.facade),
        };
      }
    } catch {
      /* old/disconnected host is not evidence of a missing runtime */
    }
    if (request === generation.current) setStatus(next);
  }, [remote]);
  useEffect(() => {
    setStatus(UNKNOWN_RUNTIME);
    void refresh();
    const timer = setInterval(() => void refresh(), 3000);
    const off = window.electronAPI.onSystemNotice?.(() => void refresh());
    return () => {
      generation.current++;
      clearInterval(timer);
      off?.();
    };
  }, [refresh]);
  return { ...runtimeLaunchState(remote ? UNKNOWN_RUNTIME : status, managed), refresh };
}
