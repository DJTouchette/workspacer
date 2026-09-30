---
title: Native typography needs SFNT assets and reapplication after palette changes
date: 2026-09-29
confidence: high
suggested_doc: theme-system
related_paths:
  - apps/native/src/ui/typography.rs
  - apps/native/src/ui.rs
  - apps/native/assets/fonts/*
promoted: false
---

# Native typography needs SFNT assets and reapplication after palette changes

## Observation
GPUI's native font registration needs decompressed SFNT data instead of the desktop's WOFF2 assets. Native configure_theme resets font fields, so persisted interface/code families and size must be reapplied after each palette change. Changing transcript font metrics also requires invalidating ListState geometry while restoring its reading anchor.

## Recommendation
Keep bundled fonts with OFL attribution; use prepare-fonts.sh to regenerate from desktop WOFF2. Apply saved typography after theme setup and preserve the list scroll anchor.
