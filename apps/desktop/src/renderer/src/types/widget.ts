/**
 * The widget vocabulary: size classes, grid geometry, and placement.
 *
 * A widget is a small, glanceable view pinned to a *project directory* — the
 * board lives in the inspector rail and is keyed by cwd, so it answers "what's
 * true about this repo" next to the session-scoped inspector tabs.
 *
 * Deliberately modelled on iPhone widgets, for two reasons beyond familiarity:
 *
 *  1. A widget is a SIBLING of a pane, never a shrunken one. Apple refused to let
 *     apps render themselves small, and the same logic applies harder here: a
 *     pane may own a PTY or a 1.2MB editor bundle, and none of that reads at
 *     150px. Plugins declare `widgets` separately from `panes` (see
 *     types/plugin.ts and the Go WidgetContribution).
 *  2. The size set is CLOSED. Free resizing would let an author ship something
 *     legible at exactly one arbitrary width; three classes force the design
 *     decision at authoring time and keep every board aligned. There is
 *     deliberately no resize handle.
 *
 * Sizes are iPhone's own, expressed in a two-column board:
 *   small  1x1 — a square
 *   medium 2x1 — full width, one row
 *   large  2x2 — a full-width square
 */

/** Mirrors Go plugin.WidgetSize — keep in sync. */
export type WidgetSize = 'small' | 'medium' | 'large';

/** Every size, smallest first. Order drives the size picker. */
export const WIDGET_SIZES: readonly WidgetSize[] = ['small', 'medium', 'large'];

/** How many grid cells a size class occupies. */
export const WIDGET_SPANS: Record<WidgetSize, { cols: number; rows: number }> = {
  small: { cols: 1, rows: 1 },
  medium: { cols: 2, rows: 1 },
  large: { cols: 2, rows: 2 },
};

/**
 * Board geometry. The board is two widget-columns wide — which is what an iPhone
 * home screen is, once you count in widget cells rather than icon cells.
 *
 * CELL is the row height and the *nominal* column width; columns are actually
 * fluid (`minmax(0, 1fr)`) so the board survives a rail that gains a resize
 * handle later. Rows stay locked to CELL so a medium is always exactly as tall
 * as a small beside it. 148px at the default rail width puts a small widget
 * within a few points of Apple's ~155pt square.
 */
export const WIDGET_COLUMNS = 2;
export const WIDGET_CELL = 148;
export const WIDGET_GAP = 12;
export const WIDGET_PAD = 12;

/**
 * The rail width that makes a small widget square. InspectorRail derives its
 * width from this rather than hardcoding one, so the two can't drift.
 */
export const WIDGET_BOARD_WIDTH =
  WIDGET_COLUMNS * WIDGET_CELL + WIDGET_GAP * (WIDGET_COLUMNS - 1) + WIDGET_PAD * 2;

/**
 * One widget placed on one project's board.
 *
 * `plugin` absent means a host (built-in) widget, so the YAML reads naturally:
 *
 *   widgets:
 *     C:/Users/me/work/repo:
 *       - { widget: git, size: large }
 *       - { plugin: djtouchette.shiplight, widget: lamp, size: small }
 *
 * Persisted in config.yaml keyed by normalized cwd, mirroring `scripts` — NOT in
 * the hub layout doc, which is per-AgentWorkspace and broadcast to every
 * connected client.
 */
export interface WidgetPlacement {
  /** Plugin id for a plugin widget; omitted for a host widget. */
  plugin?: string;
  /** Widget id — unique within its plugin, or within the host set. */
  widget: string;
  size: WidgetSize;
}

/** Stable identity for a placement, for React keys and dedupe. */
export function widgetKey(p: Pick<WidgetPlacement, 'plugin' | 'widget'>): string {
  return p.plugin ? `${p.plugin}:${p.widget}` : `host:${p.widget}`;
}

/**
 * Clamp a requested size to what the widget actually declared.
 *
 * A placement can outlive the widget that owns it — a plugin update may drop a
 * size class, and config.yaml is hand-editable. Rendering a widget at a footprint
 * it never designed for is worse than quietly using one it did, so fall back to
 * the largest supported size that isn't bigger than what was asked for, and to
 * the smallest supported otherwise.
 */
export function clampWidgetSize(want: WidgetSize, supported: readonly WidgetSize[]): WidgetSize {
  if (supported.length === 0) return 'small';
  if (supported.includes(want)) return want;
  const wantIdx = WIDGET_SIZES.indexOf(want);
  const ordered = [...supported].sort((a, b) => WIDGET_SIZES.indexOf(a) - WIDGET_SIZES.indexOf(b));
  const smallerOrEqual = ordered.filter((s) => WIDGET_SIZES.indexOf(s) <= wantIdx);
  return smallerOrEqual.length > 0 ? smallerOrEqual[smallerOrEqual.length - 1] : ordered[0];
}
