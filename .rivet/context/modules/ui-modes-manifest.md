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
last_reviewed: 2026-07-24
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
- `setMode` awaits nothing; if the save IPC fails the UI silently stays on the old mode.
- Manifest flags gate effects, so toggling mid-flight can race; the code guards by re-checking `viewLevel`/`uiManifest.fleetDeck` at effect entry rather than assuming synchronous consistency.

## Gotchas
- **A field belongs in the manifest only if the two modes actually differ on it.** `tests/uiMode.test.ts` enforces this — a field with the same value in both entries fails the suite. This is why `inspectorRail` was deleted rather than set to `true` twice: once focus stopped hiding the inspector, it was no longer a mode difference and became unconditional behavior in `ClaudePane`.
- Never branch on `mode === 'focus'` in a new component; add a manifest field (set in *both* entries) instead.
- `useUiMode()` returns `MODE_MANIFEST[mode]` by reference, so identity checks work — don't mutate it.
- Mode is genuinely a lens: nothing mode-gated unmounts/remounts a pane. Gate visibility/props, never mount identity.
- **Focus is not a width control.** The sidebar collapse toggle (`toggle-sidebar`, `Ctrl+B`) owns width, in either mode, and the two compose. Re-coupling them is the mistake the 2026-07-24 rewrite undid.

## History — why this was rewritten (2026-07-24)
The original manifest (`c76cb3b`) was a "chat-first" *subtractive* lens: `sidebar: 'rail'`, `inspectorRail: false`, `fleetDeck: false`, `attention: 'badge'`, `hubFooter: 'compact'`. That made sense when the sidebar was a navigation list with an EARLIER/RECENT dock.

`4556b4d` turned the sidebar into pure live triage — attention-sorted activity cards with inline Approve/Reply. Focus mode then deleted the app's best attention surface and replaced it with a bare count, which is backwards. Three further incoherences: `sidebar: 'rail'` duplicated the collapse toggle; `attention: 'badge'` existed only to patch the rail hiding attention; `hubFooter` had no consumer at all.

Also deleted with it: `App.tsx`'s `sidebarRailForced` / `focusSidebarOverlay` scrim-and-floating-panel machinery, which existed purely to get the sidebar back after focus forced it away.

⚠️ Note for future audits: the pre-2026-07-24 version of this doc claimed `inspectorRail` had **no consumer**. It did (`ClaudePane.tsx`). The claim came from a grep that silently skipped `ClaudePane.tsx` because the file contained a literal NUL byte (fixed in `968be1d`). Treat old "nothing reads X" notes in these docs with suspicion.
