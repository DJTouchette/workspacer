import { useEffect, useState } from 'react';

/** 1s ticker so elapsed clocks advance between session snapshots */
export function useNowTicker(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const iv = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(iv);
  }, [active]);
  return active ? now : Date.now();
}
