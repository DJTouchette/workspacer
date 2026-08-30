import { useEffect, useState } from 'react';
import type { UpdateStatus } from '../types/electron';

/**
 * The running app's version and update state, pulled from the update service
 * ('' / undefined while loading, and in web mode where the bridge doesn't
 * expose one). Nightly builds are stamped `X.Y.Z-nightly.YYYYMMDD.<sha>` by the
 * release workflow — that suffix is what drives the NIGHTLY badge.
 *
 * `updateState` is read once, not subscribed: the states this hook's callers
 * care about ('unsupported', 'manual') are decided at startup and never change
 * for the life of the process. Live transitions (checking → downloaded) belong
 * to App's onUpdateStatus subscription.
 */
export function useAppVersion(): {
  version: string;
  isNightly: boolean;
  updateState?: UpdateStatus['state'];
} {
  const [version, setVersion] = useState('');
  const [updateState, setUpdateState] = useState<UpdateStatus['state'] | undefined>(undefined);

  useEffect(() => {
    let cancelled = false;
    window.electronAPI
      ?.updatesGetStatus?.()
      .then((st) => {
        if (cancelled || !st) return;
        if (st.current) setVersion(st.current);
        if (st.state) setUpdateState(st.state);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  return { version, isNightly: version.includes('-nightly'), updateState };
}
