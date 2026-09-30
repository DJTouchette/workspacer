---
title: GPUI virtual chat rows need full-width parents for centering
date: 2026-09-30
confidence: high
related_paths:
  - apps/native/src/ui/transcript.rs
  - apps/native/src/ui.rs
promoted: false
---

# GPUI virtual chat rows need full-width parents for centering

## Observation
GPUI List places each root item at the viewport origin; max_w plus mx_auto on the item root does not center it. At 1600x900, the native chat column began at x284 while the composer began at x482 even though both were 900px wide. A full-width item containing chrome::chat_column fixes the geometry; the GPUI test now checks shared left/right edges and viewport center at 1600x900, 1000x700 and 720x480.

## Impact
Keep the virtual-list row full width and center its content child; preserve the full-width tail probe for scroll follow.
