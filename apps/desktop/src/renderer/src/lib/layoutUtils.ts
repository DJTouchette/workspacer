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

/** Grid cell a pane occupies, in column/row units (not pixels). */
export interface PaneGridLayout {
  col: number;
  row: number;
  colSpan: number;
  rowSpan: number;
}

/**
 * Grid geometry for `count` panes, in tab order. This is the actual shape
 * ScrollContainer renders — including the count===3 special case (one
 * full-height pane on the left, two stacked on the right) — so keyboard
 * nav's spatial lookup (findAdjacentPaneIndex) can agree with what's drawn
 * instead of re-deriving a uniform grid that doesn't match it.
 */
export function computePaneLayouts(count: number, isSmallScreen: boolean): PaneGridLayout[] {
  const cols = isSmallScreen ? 1 : tilingColumns(count);
  const layouts: PaneGridLayout[] = [];

  if (count <= 1) {
    layouts.push({ col: 0, row: 0, colSpan: 1, rowSpan: 1 });
  } else if (isSmallScreen) {
    for (let i = 0; i < count; i++) {
      layouts.push({ col: 0, row: i, colSpan: 1, rowSpan: 1 });
    }
  } else if (count === 2) {
    layouts.push({ col: 0, row: 0, colSpan: 1, rowSpan: 1 });
    layouts.push({ col: 1, row: 0, colSpan: 1, rowSpan: 1 });
  } else if (count === 3) {
    layouts.push({ col: 0, row: 0, colSpan: 1, rowSpan: 2 });
    layouts.push({ col: 1, row: 0, colSpan: 1, rowSpan: 1 });
    layouts.push({ col: 1, row: 1, colSpan: 1, rowSpan: 1 });
  } else {
    for (let i = 0; i < count; i++) {
      layouts.push({ col: i % cols, row: Math.floor(i / cols), colSpan: 1, rowSpan: 1 });
    }
    const lastRowStart = Math.floor((count - 1) / cols) * cols;
    const lastRowCount = count - lastRowStart;
    if (lastRowCount < cols) {
      layouts[count - 1].colSpan = cols - lastRowCount + 1;
    }
  }

  return layouts;
}

export type NavDirection = 'left' | 'right' | 'up' | 'down';

/**
 * Find the pane index adjacent to `currentIdx` in `direction`, using the same
 * grid geometry ScrollContainer renders (computePaneLayouts) rather than flat
 * index arithmetic. Flat arithmetic (idx ± cols) assumes every cell is a
 * uniform 1x1 grid square, which breaks for layouts with spanning cells (the
 * count===3 main-and-stack layout: idx ± cols pointed at cells that aren't
 * actually above/below the current one).
 *
 * A candidate qualifies when it sits on the correct side of the current
 * pane's edge (non-overlapping, gap >= 0) AND overlaps it on the
 * perpendicular axis (so "down" only considers panes actually under the
 * current column span, not merely later in tab order). Ties (equal gap) are
 * broken in reading order: topmost, then leftmost.
 *
 * Returns -1 when there is no pane in that direction.
 */
export function findAdjacentPaneIndex(
  count: number,
  currentIdx: number,
  direction: NavDirection,
  isSmallScreen = false,
): number {
  if (count <= 1 || currentIdx < 0 || currentIdx >= count) return -1;
  const layouts = computePaneLayouts(count, isSmallScreen);
  const current = layouts[currentIdx];
  if (!current) return -1;

  let best = -1;
  let bestGap = Infinity;
  let bestRow = Infinity;
  let bestCol = Infinity;

  for (let i = 0; i < layouts.length; i++) {
    if (i === currentIdx) continue;
    const cand = layouts[i];
    let gap: number;
    let overlaps: boolean;

    if (direction === 'right') {
      gap = cand.col - (current.col + current.colSpan);
      overlaps = cand.row < current.row + current.rowSpan && cand.row + cand.rowSpan > current.row;
    } else if (direction === 'left') {
      gap = current.col - (cand.col + cand.colSpan);
      overlaps = cand.row < current.row + current.rowSpan && cand.row + cand.rowSpan > current.row;
    } else if (direction === 'down') {
      gap = cand.row - (current.row + current.rowSpan);
      overlaps = cand.col < current.col + current.colSpan && cand.col + cand.colSpan > current.col;
    } else {
      gap = current.row - (cand.row + cand.rowSpan);
      overlaps = cand.col < current.col + current.colSpan && cand.col + cand.colSpan > current.col;
    }

    if (gap < 0 || !overlaps) continue;
    if (
      gap < bestGap ||
      (gap === bestGap && cand.row < bestRow) ||
      (gap === bestGap && cand.row === bestRow && cand.col < bestCol)
    ) {
      best = i;
      bestGap = gap;
      bestRow = cand.row;
      bestCol = cand.col;
    }
  }

  return best;
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
