import { useEffect, useState } from 'react';

/**
 * The running app's version, pulled from the update service ('' while loading,
 * and in web mode where the bridge doesn't expose one). Nightly builds are
 * stamped `X.Y.Z-nightly.YYYYMMDD.<sha>` by the release workflow — that suffix
 * is what drives the NIGHTLY badge.
 */
export function useAppVersion(): { version: string; isNightly: boolean } {
  const [version, setVersion] = useState('');

  useEffect(() => {
    let cancelled = false;
    window.electronAPI
      ?.updatesGetStatus?.()
      .then((st) => {
        if (!cancelled && st?.current) setVersion(st.current);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  return { version, isNightly: version.includes('-nightly') };
}
