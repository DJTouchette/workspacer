/**
 * Tiling layout utilities shared between ScrollContainer (rendering) and
 * useKeyboardNav (adjacency navigation). Having a single formula ensures
 * keyboard nav always agrees with the visual grid.
 */

/**
 * Number of columns in the tiling grid for `count` panes.
 *
 * Formula matches what ScrollContainer.tsx renders:
 *   1 pane  → 1 col
 *   2 panes → 2 cols
 *   3–4     → 2 cols
 *   5–6     → 3 cols
 *   7+      → ceil(sqrt(count))
 */
export function tilingColumns(count: number): number {
  if (count <= 1) return 1;
  if (count <= 2) return 2;
  if (count <= 4) return 2;
  if (count <= 6) return 3;
  return Math.ceil(Math.sqrt(count));
}

/**
 * Resolve the effective nav-bar height. The app reserves `navHeight + 8px` of
 * top margin for content, so App.tsx and NavBar.tsx MUST agree — this is the
 * single source of truth. Clamps to a usable range so a stray config value
 * (0, 1, or 9999) can't clip panes or eat the viewport.
 */
export function resolveNavHeight(configHeight: number | undefined, isSmallScreen: boolean): number {
  const floor = isSmallScreen ? 44 : 32; // fingertip-friendly on phones
  return Math.min(Math.max(configHeight || 34, floor), 80);
}

/**
 * Height (px) of the Windows native caption-button overlay (the titleBarOverlay
 * configured in main/index.ts). Right-anchored, top:0 panels add this as top
 * padding so their header controls (close ✕, etc.) don't sit underneath the
 * min/maximize/close buttons. Zero off Windows.
 *
 * Keep in sync with `titleBarOverlay.height` in apps/desktop/src/main/index.ts.
 */
export const WINDOWS_CAPTION_HEIGHT = 28;
export function captionInsetTop(): number {
  return typeof window !== 'undefined' && window.electronAPI?.platform === 'win32'
    ? WINDOWS_CAPTION_HEIGHT
    : 0;
}
