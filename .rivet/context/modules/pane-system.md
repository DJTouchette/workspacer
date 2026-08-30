---
title: Pane System
tags: [panes, ui, exhaustive-maps, tabs]
related_paths:
  - "apps/desktop/src/renderer/src/types/pane.ts"
  - "apps/desktop/src/renderer/src/components/ScrollContainer.tsx"
  - "apps/desktop/src/renderer/src/components/icons.tsx"
  - "apps/desktop/src/renderer/src/hooks/useAgentManager.ts"
  - "apps/desktop/src/renderer/src/panes/**"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# Pane System

## Overview
The pane system defines 18 UI pane types that users can open as tabs and split within tabs. `PaneType` is a discriminated union enum in `pane.ts` that serves as the key for multiple exhaustive Record<PaneType, …> maps and a big switch statement. TypeScript's exhaustive checking forces all maps to stay in sync when a new pane type is added—if any map is missing a case, the build fails. The system handles configuration (stored pane state with sessionId/cwd/url/filePath), lazy-loaded React components, and toolbar icons.

## Key modules
- `apps/desktop/src/renderer/src/types/pane.ts` — PaneType union (18 types) + PaneConfig/TabConfig/AgentWorkspace interfaces
- `apps/desktop/src/renderer/src/components/ScrollContainer.tsx` — renderPaneContent() switch statement (155–385); lazy-loads all 17 pane components + Suspense fallback
- `apps/desktop/src/renderer/src/components/icons.tsx` — PANE_ICONS: Record<PaneType, IconComponent> mapping each type to a lucide glyph or custom icon
- `apps/desktop/src/renderer/src/hooks/useAgentManager.ts` — defaultTitles: Record<PaneType, string> fallback labels
- `apps/desktop/src/renderer/src/panes/*.tsx` — individual pane components (TerminalPane, ClaudePane, BrowserPane, etc.)

## Failure modes
- **Exhaustive switch missing a case**: renderPaneContent() in ScrollContainer.tsx will fail TypeScript compilation if a new PaneType isn't added to the switch—default case returns "Unknown pane type" div at runtime.
- **Icon missing**: PANE_ICONS fallback (line 98) silently substitutes Globe icon if a pane type is absent.
- **Title missing**: defaultTitles lookup (useAgentManager.ts) falls back to empty string if a type isn't in the map.
- **Split-button exclusion**: SPLIT_TYPES (Pane.tsx line 10) is a hardcoded array; new pane types won't appear in the in-pane "Split into" dropdown unless manually added.
- **Component lazy-import missing**: If renderPaneContent references a component (e.g., NewPane) that isn't lazy-imported at the top of ScrollContainer, the import at runtime fails and Suspense shows the PaneFallback spinner indefinitely.

## Gotchas
- **The exhaustive map trap**: A new PaneType added to the union will cause the TypeScript build to fail at renderPaneContent's switch statement because no case matches it. This is the safety mechanism—fix it by adding a case. However, adding the case alone is not enough: you must also add entries to PANE_ICONS, defaultTitles, and the lazy-import block.
- **SPLIT_TYPES is not auto-generated**: Unlike the Record<PaneType> maps, SPLIT_TYPES is a hardcoded array (line 10 in Pane.tsx). If you add a new pane type that should appear in the split menu, you must manually append it to the list.
- **Command palette doesn't auto-enumerate**: CommandPalette.tsx's builtInActions (line 76) is also hardcoded; new pane types won't appear as palette actions without adding a new action object manually.
- **Pane components must export default**: Each component in `panes/` must be a default export so `React.lazy(() => import(...))` works. A named export will silently fail to render (Suspense spinner hangs).
- **Config state persists pane-specific fields**: PaneConfig carries conditional fields like `resumeSessionId` (claude only), `url` (browser/plugin), `filePath` (editor), `watchSessionId` (agentwatch), etc. If you add a new pane type with custom state, add a new optional field to PaneConfig and thread it through the open-pane callbacks.
- **Editor pane is legacy**: The 'editor' type renders differently based on config.editor.engine: 'terminal' mode wraps a TerminalPane with a command, 'codemirror' (default) now routes to the sandboxed editor plugin instead.

## Hand-authored notes (2026-08-24) — grid geometry is now single-sourced; keyboard nav must not re-derive it

`ScrollContainer.tsx`'s `TilingLayout` special-cases exactly **3 panes** as a main
pane spanning both rows on the left (colSpan 1, rowSpan 2) with two panes stacked
on the right — **not a uniform grid**. `useKeyboardNav.ts`'s `navigatePane`
computed vertical/horizontal neighbours as `currentIdx ± tilingColumns(count)`,
which assumes every cell is a uniform 1×1 square. For the top-right pane (index 1)
that gave `1+2=3`, out of range for a 3-pane tab, so **nav-down (prefix `j`)
silently did nothing even though a pane sits directly below it on screen**.
nav-left/right happened to still move focus to SOME pane by luck of the index
arithmetic, which is why only nav-down was reported as broken.

The two are now unified: **`computePaneLayouts(count, isSmallScreen)` in
`apps/desktop/src/renderer/src/lib/layoutUtils.ts` is the single source of the
grid geometry** (col/row/colSpan/rowSpan per pane), consumed by both
`ScrollContainer`'s `TilingLayout` (rendering) and `useKeyboardNav`'s
`navigatePane` via `findAdjacentPaneIndex` (spatial neighbour lookup by edge gap
+ perpendicular overlap, **not flat index math**).

**Any new special-cased pane-count layout belongs in `computePaneLayouts`, not
inline in `ScrollContainer`** — otherwise the same class of bug reappears for a
different pane count.
