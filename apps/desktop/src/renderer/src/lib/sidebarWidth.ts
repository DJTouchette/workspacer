/**
 * The sidebar's layout constants and the one place a stored/dragged width is
 * sanitized. The width is user-resizable and persisted (`config.ui.sidebarWidth`),
 * which means it arrives from three untrusted-ish directions — a hand-edited
 * config.yaml, a drag on a since-resized window, and the brain's defaults — so
 * every path funnels through `resolveSidebarWidth`.
 */

/** Width the sidebar ships at (mirrors `ui.sidebarWidth` in config_defaults.json). */
export const SIDEBAR_DEFAULT_WIDTH = 296;
/** Width of the collapsed monogram rail. Not resizable — it's a fixed rail. */
export const SIDEBAR_RAIL_WIDTH = 74;

/** Narrow enough to be useful, wide enough that agent names still read. */
export const SIDEBAR_MIN_WIDTH = 220;
export const SIDEBAR_MAX_WIDTH = 560;
/**
 * Hard ceiling as a share of the window: whatever the stored width says, the
 * panes must keep the majority of a small window. Without this, a width dragged
 * out on a 3440px monitor would swallow a laptop screen on the next launch.
 */
export const SIDEBAR_MAX_VIEWPORT_SHARE = 0.45;

/**
 * Clamp a candidate width into the allowed band for the current viewport.
 * `viewportWidth` is optional so callers with no window (tests, headless) get
 * the plain min/max clamp.
 */
export function clampSidebarWidth(px: number, viewportWidth?: number): number {
  const ceiling =
    viewportWidth && viewportWidth > 0
      ? Math.min(SIDEBAR_MAX_WIDTH, Math.round(viewportWidth * SIDEBAR_MAX_VIEWPORT_SHARE))
      : SIDEBAR_MAX_WIDTH;
  // A tiny viewport can push the share ceiling below the minimum; the minimum
  // wins there (the sidebar overlays the content at those widths anyway).
  return Math.round(
    Math.min(Math.max(px, SIDEBAR_MIN_WIDTH), Math.max(ceiling, SIDEBAR_MIN_WIDTH)),
  );
}

/**
 * The width to render from a stored config value: the default when it's absent
 * or nonsense (NaN/Infinity/0 from a hand-edited config), clamped otherwise.
 */
export function resolveSidebarWidth(stored: number | undefined, viewportWidth?: number): number {
  if (typeof stored !== 'number' || !Number.isFinite(stored) || stored <= 0) {
    return clampSidebarWidth(SIDEBAR_DEFAULT_WIDTH, viewportWidth);
  }
  return clampSidebarWidth(stored, viewportWidth);
}
