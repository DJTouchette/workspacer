---
title: Native GPUI keyboard controls activate on key release
date: 2026-10-01
confidence: high
related_paths:
  - apps/native/src/ui/chrome.rs
  - apps/native/src/ui/sidebar.rs
  - apps/native/src/ui.rs
promoted: false
---

# Native GPUI keyboard controls activate on key release

## Observation
Focusable GPUI divs retain per-element focus handles, join Tab order with tab_stop(true), and invoke their existing click listeners on unmodified Enter/Space KeyUp. TestAppContext.simulate_keystrokes dispatches only KeyDown; activation regressions must explicitly dispatch KeyUp as well. Native controls now share a reserved transparent border that becomes an accent focus ring, disabled controls are not tab stops, and sidebar row identities use session IDs to keep focus stable through reordering.

## Impact
Reuse GPUI activation rather than duplicating click handlers on key-down; keyboard tests must include real release events.
