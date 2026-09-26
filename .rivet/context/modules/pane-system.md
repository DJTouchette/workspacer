---
title: Pane System
tags: [panes, ui, exhaustive-maps, tabs, rendering]
related_paths:
  - "apps/desktop/src/renderer/src/types/pane.ts"
  - "apps/desktop/src/renderer/src/components/ScrollContainer.tsx"
  - "apps/desktop/src/renderer/src/components/icons.tsx"
  - "apps/desktop/src/renderer/src/hooks/useAgentManager.ts"
  - "apps/desktop/src/renderer/src/panes/**"
  - "apps/desktop/src/renderer/src/lib/paneMenu.ts"
  - "apps/desktop/src/renderer/src/lib/layoutUtils.ts"
  - "apps/desktop/src/renderer/src/contexts/PaneMenuContext.tsx"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Pane System

## Types, rendering, and persistence

`apps/desktop/src/renderer/src/types/pane.ts` defines the `PaneType` string-literal
union and `PaneConfig`, `TabConfig`, and `AgentWorkspace` interfaces. Pane state
includes cwd, session attachment/resume IDs, URLs, file paths, plugin identity,
and view-specific fields. Persisted old sessions can contain legacy pane types;
keep their restoration behavior explicit when retiring a type.

`apps/desktop/src/renderer/src/components/ScrollContainer.tsx` dispatches pane
rendering through `renderPaneContent`. Most components are loaded through
`React.lazy` with a `Suspense` loading fallback. Each pane is inside an
`ErrorBoundary`, so import/render failures show its recoverable error UI.
Missing imports can fail at build time; a rejected lazy import is not an
indefinitely pending Suspense spinner.

The rendering switch has a default “Unknown pane type” element. It does **not**
assert `never`, so adding a type without adding a switch case need not fail
TypeScript checking. The exhaustive `Record<PaneType, ...>` maps are
`PANE_ICONS` in `components/icons.tsx` and `defaultTitles` in
`hooks/useAgentManager.ts`. Those maps enforce keys, but do not prove that a
pane has a render branch or can be opened from a menu.

`PaneIcon` retains a Globe fallback for unexpected runtime values. Title lookup
uses `title ?? defaultTitles[type]`; it does not add an empty-string fallback
for an unknown persisted type.

## Creation menus

`apps/desktop/src/renderer/src/lib/paneMenu.ts` owns the shared split/new-tab
menu. `App` resolves it once and supplies `PaneMenuProvider`; `Pane` and `NavBar`
consume it. There is no separate `SPLIT_TYPES` array in `Pane.tsx`.

- Absent `ui.paneMenu`: `DEFAULT_PANE_MENU` followed by contributed plugin panes.
- Explicit array, including empty: use exactly those recognized entries, in order.
- IDs resolve against `MENU_BUILTIN_LABELS` first, then loaded plugin pane types;
  stale/unknown IDs are dropped.

`MENU_BUILTIN_LABELS` is a partial map: not every renderable pane is offered in
these creation menus. `CommandPalette.tsx` maintains its own built-in actions
and special flows, so adding a PaneType does not automatically add a palette
entry. Plugin split actions use `onSplitPlugin` when supplied, otherwise the
context's `onOpenPlugin` fallback.

## Adding or changing a pane

1. Update `PaneType` and the exhaustive icon/title maps.
2. Add the component import and `renderPaneContent` branch. The existing lazy
   import pattern expects a default-exported React component.
3. Add any persisted fields to `PaneConfig` and thread them through creation,
   save/restore, and relevant backend/layout mappings.
4. Decide whether the pane belongs in `MENU_BUILTIN_LABELS`, the default menu,
   the command palette, or only a contextual action.
5. Check remote-session routing before interpreting a pane's cwd or session ID
   as a local object.

The legacy `editor` type renders a terminal editor when the engine is
`terminal`. Otherwise it shows a message to reopen the editor from the command
palette. New in-app editor flows open the trusted editor plugin as a `plugin`
pane; restoring an old `editor` pane does not automatically convert it.

## Geometry and keyboard navigation

`computePaneLayouts(count, isSmallScreen)` in
`apps/desktop/src/renderer/src/lib/layoutUtils.ts` is shared by rendering and
`findAdjacentPaneIndex`. On a wide screen, three panes are one full-height pane
on the left and two stacked on the right; small screens use one column.
Keyboard navigation uses cell edges and perpendicular overlap, not flat
`index ± columnCount` arithmetic. Keep new geometry in this shared function so
navigation agrees with the visible arrangement.

Zoom keeps sibling panes mounted but hidden, preserving terminal sessions.
Structural/focus operations must preserve the `zoomedPaneId` lifecycle described
in `TabConfig` instead of treating zoom as a second pane collection.

## Verification

From `apps/desktop/src/renderer`:

```bash
npx vitest run tests/paneMenu.test.ts tests/layoutUtils.test.ts tests/commandPaletteNav.test.tsx
```

These cover menu resolution, geometry/navigation, and palette navigation.
They do not establish render coverage for every PaneType; review the switch
and run the relevant pane's browser flow when changing rendering.
