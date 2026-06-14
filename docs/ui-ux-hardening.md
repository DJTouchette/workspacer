# UI / UX Hardening Sweep

Tracking doc for the polish pass on the desktop renderer
(`apps/desktop/src/renderer/src`). Goal: the app *feels good* but isn't *finished* —
close the gaps that make it feel incomplete.

Status legend: ☐ todo · ◐ in progress · ☑ done · ✗ won't do

---

## Flagged bugs (from user)

- ☑ **Context menus mis-position** — right-click menus render far to the right or
  don't appear at all. Root cause: every menu did `position:fixed; top:clientY;
  left:clientX` with **no viewport clamping**, so menus opened near the right/bottom
  edge overflowed off-screen. `SideBar` only stored `y` and hardcoded `left`.
  Fixed: new shared `components/ContextMenu.tsx` primitive measures itself and
  clamps/flips into the viewport, with built-in outside-click + Escape close and
  `role=menu`/`role=menuitem`. Migrated `SideBar.tsx` (now uses real clientX),
  `NavBar.tsx` tab menu, and `review/FileTree.tsx`. Removed the per-site
  outside-click effects and dead `SideMenuItem`. (NavBar "+" add-menu is an
  anchored dropdown, not point-anchored — left as-is for now.)
- ☑ **Sidebar context bar ignores status-bar data** — the sidebar's context bar
  computed `contextTokens/contextLimit` from the transcript-derived `SessionUsage`,
  while `SessionStatusBar` preferred Claude's authoritative `statusLine.contextUsedPct`
  — so the two could show different context %. Fixed: extracted the precedence logic
  into `lib/sessionStats.ts#deriveSessionStats` (statusLine first, transcript usage
  fallback). Both `SessionStatusBar` and `SideBar` now derive from it. SideBar now
  takes `snapshotBySession` instead of `usageBySession`; dropped the now-dead
  `usageBySession` state from `App.tsx`.
- ◐ **Hotkeys have no UI affordance** — lots of keybindings exist but many actions
  had no clickable/discoverable way to trigger them.
  - Done: the command palette now surfaces every global action as a first-class,
    searchable, keyboard-navigable entry — Spawn Agent, Toggle Sidebar/Inbox/Fleet,
    Save Session, Settings, Keyboard Shortcuts, Ask the Fleet, Analytics, Layouts,
    Switch Session, Remote, Manage/Install Plugins (previously mouse-only
    `CommandRow`s or keyboard-only). Each row shows its shortcut as a `Kbd` badge.
    New `lib/shortcuts.ts#formatCombo`/`shortcutFor`, shared with the help overlay.
    Shortcut maps now merge defaults under user overrides (`resolvedShortcuts`).
  - Remaining: tooltips on the standalone toolbar/sidebar buttons showing their
    shortcut (some already have `title=`, not all); per-pane right-click affordances.

---

## Resilience & robustness

- ☑ **No error boundaries anywhere** — one component crash white-screened the whole
  app. Added `components/ErrorBoundary.tsx` (recoverable fallback with "Try again",
  `resetKeys` for auto-recovery, `role=alert`). Wrapped: each pane (keyed by pane id),
  each agent workspace, the SideBar, the NavBar, and a top-level last-resort in
  `main.tsx`. A pane crash now shows an inline fallback and the rest keeps running.
- ◐ **Missing loading states** — added shared `components/PaneMessage.tsx`
  (`LoadingState`/`EmptyState`). LibraryPane now shows "Loading…" vs "No items yet"
  (using the `loaded` flag it already received but ignored). EditorPane file tree
  shows "Loading…"/"empty" rows instead of nothing. App's "No agent selected" block
  now uses the shared `EmptyState`. Remaining: ClaudePane spawn skeleton,
  OverviewPane analytics fetch.
- ☐ `migrateSessionData` uses `data: any` (App.tsx) — tighten to `unknown`.

## Visual polish & motion

- ☐ Pane drag (header) flickers when dragging across columns. **Deferred** —
  reorder-based drag (moves ±1 past a 60px threshold); the pane jumps out from
  under the cursor and re-triggers. A proper fix = hit-test the pane under the
  cursor (real DnD). Interaction-sensitive; needs a live multi-pane session to
  verify, so not changed blind.
- ☑ Spatial canvas has no grid snapping → added a 20px world-unit `snap()` applied
  to card move (x/y) and resize (w/h) in ScrollContainer. *Code-verified; needs
  live spatial mode + backend to see.*
- ☑ View-mode switch loses canvas positions on reload — root cause was the
  session-save **dedup hash** (useSessionLifecycle) omitting `t.canvas`, so a drag
  that only moved a card was deduped away and never persisted. Hash now includes
  canvas x/y/w/h. *Code-verified; needs backend to see.*
- ☐ Inconsistent hover/active/transition feel — **deferred**: hovers are inline JS
  `onMouseEnter` mutations across many components, and the `animations` config
  (default off) hardcodes `transition:'none'` in places. A safe unify is a broad
  sweep that must respect that setting everywhere.
- ☑ `navBarHeight` — min was already enforced via `Math.max(…,32)`, but the formula
  was duplicated in App + NavBar (drift risk) with no upper bound. Extracted
  `lib/layoutUtils.ts#resolveNavHeight` (single source of truth, clamps 32/44–80).

## Empty & first-run states

- ◐ Inconsistent coverage: good (Inbox, Fleet, SideBar, scripts). Fixed: Editor
  file tree empty, Library loading/empty, "No agent selected" (shared `EmptyState`).
  Remaining: new Notes pane placeholder.
- ☑ First-run onboarding — `components/Onboarding.tsx` shows a dismissible welcome
  (spawn + the user's actual shortcuts for palette/inbox/fleet/settings/help) when
  there are no agents and `config.onboardingDismissed` isn't set; falls back to the
  plain "No agent selected" empty state once dismissed.

## Accessibility & input

- ☐ No ARIA roles on popovers/menus (`SearchableSelect` should be `role=listbox`,
  menus `role=menu`).
- ☐ No focus trap in modals (Tab escapes SessionPicker/SpawnAgent to background).
- ☐ Attention cards lack visible keyboard focus styles.
- ☐ InboxDrawer feed has no ARIA live region (screen readers miss dismiss/snooze).

## Mobile / responsive

- ☑ Modals don't size responsively — SessionPicker now `width: min(450px,92vw)`
  (was a hard 340px min that overflowed); PromptVarsDialog, CommandPalette (440→
  `min(440px,94vw)`), ShortcutOverlay, and the NavBar add-menu sub-popovers all
  capped to `min(…, 9Xvw)` + `box-sizing: border-box`. (SpawnAgent already had
  `maxWidth:90vw`.) Verified at 360px in the web client — palette + spawn dialog
  fit with margins, no page horizontal overflow.
- ☑ FleetDeck grid `minmax(360px,1fr)` → `minmax(min(360px,100%),1fr)` so cards
  collapse to one clean column on phones instead of overflowing.
- ☑ NavBar script menu could overflow < 400px — now `width: min(380px, calc(100vw
  - 20px))` + `box-sizing: border-box`.

## Scale / performance

- ☑ Virtualization — FleetDeck (row-windowed responsive grid) and the InboxDrawer
  feed (dynamic-height list) now use `@tanstack/react-virtual`, so 50+ agents /
  100+ items stay smooth. Test harness (`tests/setup.ts`) gives jsdom a fake
  viewport + firing ResizeObserver so virtualized lists render under test.
- ☐ Pane headers re-render on every status update (no memoization).

---

## Log

- 2026-06-13 — Created doc. Audit complete.
- 2026-06-13 — Fixed context-menu positioning (shared `ContextMenu` primitive,
  migrated SideBar/NavBar/FileTree) and sidebar/status-bar data divergence
  (shared `deriveSessionStats`). Renderer typecheck clean.
- 2026-06-13 — Added `ErrorBoundary` (per-pane, per-workspace, sidebar, nav,
  top-level). Command palette now exposes all global commands with shortcut
  badges; added `lib/shortcuts.ts`. Typecheck + 76 tests pass.
- 2026-06-13 — Added `components/PaneMessage.tsx` (LoadingState/EmptyState);
  wired loading/empty states in LibraryPane, EditorPane tree, and App's
  no-agent state. Typecheck + 76 tests pass.
- 2026-06-13 — Mobile/responsive batch: phone-safe widths on all modals/popovers,
  responsive FleetDeck grid, NavBar script-menu overflow fix. Verified live in the
  web client at 360px (agent-browser): palette/spawn-dialog fit, no h-overflow.
  Typecheck + 76 tests pass.
- 2026-06-13 — Visual-polish batch: spatial-canvas grid snapping, canvas-persistence
  hash fix (positions survive reload), `resolveNavHeight` helper (dedup + clamp).
  Deferred pane-drag flicker + hover-consistency (interaction/broad, need live).
  Typecheck + 76 tests pass.
