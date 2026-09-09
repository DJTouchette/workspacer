import { useCallback, useEffect, useRef, useState } from 'react';
import {
  normalizeProviderReadiness,
  providerReadinessDetail,
  UNCHECKED_PROVIDER,
  UNSUPPORTED_PROVIDER,
  type ProviderReadiness,
} from '../../../main/shared/providerReadiness';

/** Reads are free. Only the explicit refresh button requests inference. The
 * desktop process owns startup/dedup, so remounting a picker cannot spend.
 * Profiles/integrations and remote owners have no isolated account probe yet.
 */
export function useProviderReadiness(provider: string, owner = '', account = '') {
  const api = window.electronAPI;
  const [revision, setRevision] = useState(0);
  const key = JSON.stringify([provider, owner, account, revision]);
  const currentKey = useRef(key);
  currentKey.current = key;
  const generation = useRef(0);
  const [answer, setAnswer] = useState<{
    key: string;
    value: ProviderReadiness;
    api: typeof window.electronAPI;
  } | null>(null);
  const fetchStatus = useCallback(
    async (check = false) => {
      const request = ++generation.current;
      let value = UNCHECKED_PROVIDER;
      try {
        value =
          owner || account
            ? UNSUPPORTED_PROVIDER
            : normalizeProviderReadiness(await api.providerReadiness?.(provider, check));
      } catch {
        /* old hosts have no capability; never interpret this as logged out */
      }
      if (
        api === window.electronAPI &&
        currentKey.current === key &&
        generation.current === request
      )
        setAnswer({ key, value, api });
    },
    [provider, owner, account, key, api],
  );
  useEffect(() => {
    const off = window.electronAPI.onConfigChanged?.(() => {
      generation.current++;
      setAnswer(null);
      setRevision((r) => r + 1);
    });
    return () => off?.();
  }, []);
  useEffect(() => {
    void fetchStatus();
    // Poll snapshots to observe the process-owned startup check; no ping here.
    const timer = setInterval(() => void fetchStatus(), 1000);
    return () => {
      generation.current++;
      clearInterval(timer);
    };
  }, [fetchStatus]);
  const status =
    owner || account
      ? UNSUPPORTED_PROVIDER
      : answer?.key === key && answer.api === api
        ? answer.value
        : UNCHECKED_PROVIDER;
  return { status, detail: providerReadinessDetail(status), refresh: () => fetchStatus(true) };
}
