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
