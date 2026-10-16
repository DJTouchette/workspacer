---
title: GPUI debug_bounds outlive the element; title actions have no room to grow
date: 2026-10-16
confidence: high
related_paths:
  - vendor/gpui/src/window.rs
  - apps/native/src/ui/chrome.rs
  - apps/native/src/ui/island.rs
promoted: false
---

# GPUI debug_bounds outlive the element; title actions have no room to grow

## Observation
`Frame::clear` in vendor/gpui never clears `debug_bounds`, so once a selector
has drawn, `visual.debug_bounds(sel).is_none()` stays false even after the
element stops rendering (the fixture's first frame counts). Assert absence via
workspace state, or only for selectors that never drew.

Adding a 7th title-capsule action (a pull request button, 2026-10-16) hung
`cold_cache::the_context_gauge_carries_the_cold_mark` forever and broke
`title_actions_show_only_on_hover_focus_or_tap`: revealed actions push the
capsule to its column cap, `chrome.rs` measures `title_base_width = bounds -
extra` too small, `capsule_fits` drops the context gauge, the actions narrow,
the gauge returns — an every-frame oscillation. Gating shrink on hidden
actions breaks three other title tests. New per-session entry points belong in
Session details or a status chip, not the title actions, until that
measurement is fixed.
