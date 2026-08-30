---
title: Stopping nodes must disable wake affordances across desktop, mobile, and TUI
date: 2026-08-28
confidence: high
suggested_doc: desktop-remote-client-mode
related_paths:
  - apps/desktop/src/renderer/src/lib/remoteNodes.ts
  - apps/desktop/src/renderer/src/components/RemoteNodesBar.tsx
  - services/hub/cmd/hub/mobile.html
  - apps/tui/src/nodes.rs
  - services/hub/internal/nodes/wake.go
promoted: true
sources:
  - desktop-m-refuse-wake-while-stopping/.rivet/learnings/2026-08-25-remote-node-clients-knowing-stopping-is-not-the-15af6a.md
  - tui-handle-the-stopping-state/.rivet/learnings/2026-08-25-the-desktops-wakeaffordance-also-offers-an-enabl-9390dd.md
promoted_to: desktop-remote-client-mode
---

# Stopping nodes must disable wake affordances across desktop, mobile, and TUI

## Observation

Knowing that a node is `stopping` in presentation state is not enough: every wake affordance must refuse a wake while the hub is draining it. The desktop and `/app` renderer's `wakeAffordance()` previously handled `available`, `waking`/`pending`, `!wakeable`, and `!canWake`, then fell through to an enabled Connect action for `stopping`. `RemoteNodesBar.tsx` calls that helper without additional gating. Its sibling `sleepAffordance()` already had the mirrored stopping guard, making the omission easy to overlook.

The hub intentionally accepts a wake during drain (`wake.go` clears `stopping` and increments the generation), so the issue is UX rather than an authorization failure: one click can silently cancel a user-requested shutdown and restart billing. The authoritative hub state must be checked before an optimistic local `pending` flag. `/m` already put its `stopping` branch first; the TUI had the same gap and its `NodeState::Stopping` wake-affordance arm was fixed independently on `wks/tui-handle-the-stopping-state` (commit `05cfa350`). The desktop counterpart was fixed on `wks/desktop-m-refuse-wake-while-stopping` (commit `35fa9fd4`).

## Impact

An action affordance can contradict a correctly rendered status. A draining Fly node can visibly say it is shutting down while still offering Connect; selecting it restarts metered work. Stale cross-client copy can compound this: claims that there is no stop verb or that a failed wake necessarily leaves billing running no longer match the `nodes.sleep` and `stopAfterFailedWake` behavior.

## Recommendation

For each `NodeState`, audit `wakeAffordance`, `sleepAffordance`, `/m`'s button chain, and the TUI equivalent—not merely their labels, tones, or spinners. Add the `stopping` wake branch before local pending checks, returning a visible disabled action with clear shutdown text. Keep the state/action parity matrix explicit whenever a new node state is introduced.
