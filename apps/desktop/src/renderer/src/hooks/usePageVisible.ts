import { useEffect, useState } from 'react';

/**
 * True while the document is visible; false when the window is minimized,
 * hidden, or behind another window. Gate timers / animations / polls on this so
 * the app idles toward ~0% CPU when the user has switched away — the common
 * state for a babysitting tool left open for hours.
 *
 * Cheap and shared: a single `visibilitychange` listener per consumer, no
 * provider needed. Safe in non-DOM contexts (SSR/tests) where it returns true.
 */
export function usePageVisible(): boolean {
  const [visible, setVisible] = useState(() =>
    typeof document === 'undefined' ? true : document.visibilityState !== 'hidden',
  );
  useEffect(() => {
    if (typeof document === 'undefined') return;
    const onChange = () => setVisible(document.visibilityState !== 'hidden');
    document.addEventListener('visibilitychange', onChange);
    return () => document.removeEventListener('visibilitychange', onChange);
  }, []);
  return visible;
}
