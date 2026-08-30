---
title: navigatePane's flat idx±cols formula didn't match ScrollContainer's count===3 layout
date: 2026-08-24
confidence: high
suggested_doc: pane-system
related_paths:
  - apps/desktop/src/renderer/src/lib/layoutUtils.ts
  - apps/desktop/src/renderer/src/hooks/useKeyboardNav.ts
  - apps/desktop/src/renderer/src/components/ScrollContainer.tsx
promoted: true
promoted_to: pane-system
---

# navigatePane's flat idx±cols formula didn't match ScrollContainer's count===3 layout

## Observation
ScrollContainer.tsx's TilingLayout special-cases exactly 3 panes as a main pane spanning both rows on the left (colSpan 1, rowSpan 2) with two panes stacked on the right — not a uniform grid. useKeyboardNav.ts's navigatePane computed vertical/horizontal neighbours as currentIdx +/- tilingColumns(count), which assumes every cell is a uniform 1x1 square. For the top-right pane (index 1) that gave idx 1+2=3, out of range for a 3-pane tab, so nav-down (prefix j) silently did nothing even though a pane sits directly below it on screen. nav-left/right happened to still move focus to SOME pane by luck of the index arithmetic, which is why the user only noticed nav-down as broken.

## Impact
Any future change to TilingLayout's per-count layout table in ScrollContainer.tsx must also be reflected in the keyboard-nav neighbour lookup, or the same class of bug reappears for a different pane count.

## Recommendation
The two are now unified: computePaneLayouts(count, isSmallScreen) in layoutUtils.ts is the single source of the grid geometry (col/row/colSpan/rowSpan per pane), consumed by both ScrollContainer's TilingLayout (rendering) and useKeyboardNav's navigatePane via findAdjacentPaneIndex (spatial neighbour lookup by edge gap + perpendicular overlap, not flat index math). Any new special-cased pane-count layout belongs in computePaneLayouts, not inline in ScrollContainer.
