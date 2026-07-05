import { useEffect, useRef } from 'react';

/**
 * Invokes `onReconnect` after the hub connection drops and comes back — never on
 * the first connect. Use it to re-fetch state that can go stale while the socket
 * is down: the bus re-asserts topic subscriptions on reconnect, but it does not
 * replay the snapshots a client fetched once at mount (session list, layout doc,
 * config), so a backgrounded web tab would show stale data until a manual
 * refresh. On the Electron desktop the hub status stays connected for the life
 * of the app, so this is effectively a no-op there.
 */
export function useHubReconnect(onReconnect: () => void): void {
  const cb = useRef(onReconnect);
  cb.current = onReconnect;
  useEffect(() => {
    let hasConnected = false;
    // setConnected only fires handlers on change, so any 2nd+ `connected: true`
    // is a genuine reconnect (it must have gone false in between).
    const off = window.electronAPI.onHubStatus?.(({ connected }) => {
      if (!connected) return;
      if (hasConnected) cb.current();
      hasConnected = true;
    });
    return () => {
      off?.();
    };
  }, []);
}
