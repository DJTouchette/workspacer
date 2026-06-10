import { useEffect, useState } from 'react';

/**
 * Subscribe to a CSS media query and re-render on match changes.
 * SSR/headless safe — falls back to `false` when matchMedia is unavailable.
 */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState<boolean>(() =>
    typeof window !== 'undefined' && typeof window.matchMedia === 'function'
      ? window.matchMedia(query).matches
      : false,
  );

  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const mql = window.matchMedia(query);
    const onChange = () => setMatches(mql.matches);
    onChange(); // sync in case the query changed between render and effect
    mql.addEventListener('change', onChange);
    return () => mql.removeEventListener('change', onChange);
  }, [query]);

  return matches;
}

/** Shared breakpoint below which the app switches to its compact, phone-friendly layout. */
export const SMALL_SCREEN_QUERY = '(max-width: 768px)';

/** Convenience wrapper: true on phone-sized viewports. */
export function useIsSmallScreen(): boolean {
  return useMediaQuery(SMALL_SCREEN_QUERY);
}
