import { useEffect, useState } from 'react';
import { usePageVisible } from '../../hooks/usePageVisible';

/** 1s ticker so elapsed clocks advance between session snapshots.
 *  Pauses automatically when the window is hidden to save CPU. */
export function useNowTicker(active: boolean): number {
  const visible = usePageVisible();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active || !visible) return;
    const iv = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(iv);
  }, [active, visible]);
  return active ? now : Date.now();
}
