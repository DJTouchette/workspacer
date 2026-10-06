---
title: Native context gauge rides the title capsule; GPUI debug bounds never go away
date: 2026-10-06
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui/gauge.rs
  - apps/native/src/ui/chrome.rs
  - apps/native/src/ui/island.rs
  - vendor/gpui/src/window.rs
promoted: false
---

# Native context gauge rides the title capsule; GPUI debug bounds never go away

## Observation
The context-window meter moved from the composer to the title capsule (ui/gauge.rs). It is a 2px hairline at the bottom of the bar's padding box, so it stays on the title row when notices grow the island. From 70% the capsule carries a steady tone_glow (strength 0.4 on the greeting scale). From 90% it adds a "context" island row. The exact figures sit at the head of the revealed actions. One function, Workspace::title_carries_context, decides between the capsule and the composer meter, so the two never show together. The capsule falls back to the composer when its measured resting width (title_base_width) is under 120px, which in practice means a short title in a narrow window. The capsule is never hidden in compact or narrow layouts. Gauge placement cannot feed back into title_base_width, because the figures live in the clipped actions and base excludes them.

Island Notice now has a key separate from its text. A changed key greets again and re-measures. A changed text under the same key just updates. The context row is keyed `{session}@{band}` (band 90 or 95), and dismissal records that key. Its retain keeps the dismissal while any session still has that band key, so it survives chat switches but not a band change.

Trap: in vendored GPUI, Frame::clear never clears rendered_frame.debug_bounds. An element that stops drawing keeps its last bounds forever, so `visual.debug_bounds(sel).is_none()` only proves the element was never drawn. Tests have to assert disappearance from state (hairline_drawn, island_slots) or from geometry that has visibly moved, such as a resize that shifts the composer.

## Impact
Any UI test that checks an element vanished through debug_bounds passes or fails for the wrong reason.

## Recommendation
For absence checks, use workspace state, or move the layout first so stale bounds cannot satisfy the geometric check. New island rows whose words change live (counters, figures) should pass a stable key.
