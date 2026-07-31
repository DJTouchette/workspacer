/**
 * App-wide text scale.
 *
 * Every text size in the app is rem-based, so scaling the document root's
 * font-size scales all of it — chrome and chat alike. (The chat's own
 * `guiFontScale` still multiplies on top of this.) `mod+=` / `mod+-` nudge by
 * a step and `mod+0` resets.
 */

/** Smallest legible scale. Below this the chrome starts clipping. */
export const MIN_TEXT_SCALE = 0.8;
/** Largest scale that still fits the sidebar's fixed-width rail. */
export const MAX_TEXT_SCALE = 1.5;
/** One `mod+=` / `mod+-` press. */
export const TEXT_SCALE_STEP = 0.05;
export const DEFAULT_TEXT_SCALE = 1.0;

/**
 * Clamp a requested scale into range and quantise it to whole percents.
 *
 * The rounding is what keeps repeated nudges stable: 0.05 is not exactly
 * representable in binary floating point, so stepping up and back down without
 * it drifts (1 → 1.05 → 0.9999999999999999) and every step would then write a
 * config that differs from the default by a rounding error.
 */
export function clampTextScale(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_TEXT_SCALE;
  const bounded = Math.min(MAX_TEXT_SCALE, Math.max(MIN_TEXT_SCALE, value));
  return Math.round(bounded * 100) / 100;
}

/**
 * The `font-size` to put on the document root for a given scale.
 *
 * Returns `''` at 1.0 so the element falls back to the stylesheet's own value
 * rather than being pinned at a hard-coded `100%` — that keeps the user's
 * browser/OS font-size preference working at the default scale.
 */
export function textScaleToRootFontSize(scale: number): string {
  return scale === DEFAULT_TEXT_SCALE ? '' : `${(scale * 100).toFixed(1)}%`;
}
