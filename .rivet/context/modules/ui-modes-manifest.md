---
title: UI modes (focus|fleet) manifest and mode-driven chrome
tags: [renderer-ui, ui-modes, focus-fleet, manifest, config]
related_paths:
  - "apps/desktop/src/renderer/src/lib/uiMode.ts"
  - "apps/desktop/src/renderer/src/hooks/useUiMode.ts"
  - "apps/desktop/src/renderer/src/components/SideBar.tsx"
  - "apps/desktop/src/renderer/src/components/FleetDeck.tsx"
  - "apps/desktop/src/renderer/src/App.tsx"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# UI modes (focus|fleet) manifest and mode-driven chrome

## Overview
`config.ui.mode` (`'fleet' | 'focus'`, absent = fleet) is a lens over the same workspace/session state, never a distinct layout — flipping it must never remount panes or touch sessions. `MODE_MANIFEST` in `apps/desktop/src/renderer/src/lib/uiMode.ts` is the single source of truth for what each mode shows; consumers read flags off the manifest via `useUiMode()` instead of branching on the mode string.

**The axis is how many agents you are attending to, not how much chrome you see.** `fleet` = supervising the whole fleet (every agent gets a live card, Fleet Deck available). `focus` = working with one agent, with the periphery quiet but never blind. Rewritten 2026-07-24; see "History" below for why the original design stopped making sense.

## Key modules
- `lib/uiMode.ts` — `UiMode`, `ModeManifest`, `MODE_MANIFEST`, `resolveUiMode(raw)` (anything but the literal `'focus'` → `'fleet'`). The manifest is now exactly two fields:
  - `feed: 'all' | 'active-and-blocked'` — which agents the sidebar renders as full cards.
  - `fleetDeck: boolean` — whether the Fleet Deck overlay may mount.
- `hooks/useUiMode.ts` — the only seam between `config.ui.mode` and consumers; `{ mode, manifest, setMode, toggle }`. `setMode` is fire-and-forget `void save(...)`.
- `components/SideBar.tsx` — the one `feed` consumer. Under `'active-and-blocked'` it keeps full cards for `agent.id === activeAgentId` or `cardStateOf(agent) === 'waiting'`, and folds the remainder into an expandable "N others · M working" row (local `othersExpanded` state, reset by an effect on `uiManifest.feed` so re-entering focus starts quiet). Subagents follow their parent into the fold and count toward the total.
- `App.tsx` — `effectiveViewLevel` forces `'piloting'` whenever `!uiManifest.fleetDeck`, which is what keeps `AttentionContext`'s piloting auto-dismiss correct (fixed in `391772a`; don't "simplify" it back to raw `viewLevel`). Also gates the deck's keyboard/auto-open effects and its mount.
- `components/CommandPalette.tsx` — reads raw `mode` only to label the toggle entry; the action comes from the parent.
- `tests/uiMode.test.ts` — pins both manifests **and** asserts no field is identical across the two modes (see Gotchas). `tests/components/sidebarFeedFilter.test.tsx` pins the filter behavior.

## Failure modes
- `resolveUiMode` is total: any unknown/corrupt value falls back to `fleet`, the fuller view.
- `setMode` awaits nothing; if the save rejects, `ConfigProvider` retains the prior configuration and posts a “Setting not saved” warning.
- Manifest flags gate effects, so toggling mid-flight can race; the code guards by re-checking `viewLevel`/`uiManifest.fleetDeck` at effect entry rather than assuming synchronous consistency.

## Gotchas
- **A field belongs in the manifest only if the two modes actually differ on it.** `tests/uiMode.test.ts` enforces this — a field with the same value in both entries fails the suite. This is why `inspectorRail` was deleted rather than set to `true` twice: once focus stopped hiding the inspector, it was no longer a mode difference and became unconditional behavior in `ClaudePane`.
- Never branch on `mode === 'focus'` in a new component; add a manifest field (set in *both* entries) instead.
- `useUiMode()` returns `MODE_MANIFEST[mode]` by reference, so identity checks work — don't mutate it.
- Mode is genuinely a lens: nothing mode-gated unmounts/remounts a pane. Gate visibility/props, never mount identity.
- **Focus is not a width control.** The sidebar collapse toggle (`toggle-sidebar`, `Ctrl+B`) owns width, in either mode, and the two compose. Re-coupling them is the mistake the 2026-07-24 rewrite undid.

## Maintaining the mode boundary

The two-field manifest is intentional: sidebar width has its own toggle, and
unconditional inspector/chrome behavior should not become duplicated flags.
When a field stops distinguishing the modes, remove it from the manifest and
update consumers/tests together. Do not rely on old source searches that claimed
a field had no readers—check both renderer source and parse/index caveats.

`ConfigProvider` can return the prior configuration after a rejected save; host
writers can also return an unchanged value after a persistence failure. A click
is not proof that the requested mode was persisted. The current snapshot remains
the rendering input.
