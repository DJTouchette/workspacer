---
title: GPUI zero-blur shadows do not make a reliable focus ring
date: 2026-10-01
confidence: high
related_paths:
  - apps/native/src/ui/chrome.rs
  - apps/native/src/ui/sidebar.rs
promoted: false
---

# GPUI zero-blur shadows do not make a reliable focus ring

## Observation
A real X11/Mesa capture showed no visible native focus ring when BoxShadow used blur_radius 0 and spread_radius 2. Blade shaders.wgsl gaussian and blur_along_x divide by the blur radius, making zero unsuitable for this hard ring. Native controls now reserve a transparent 2px border at rest and color it with the palette accent on focus, preserving geometry and remaining visible inside clipped rounded panels. Expanded session rows budget four more pixels for this reserved stroke.

## Impact
Use a reserved border for hard focus outlines instead of a zero-blur Gaussian shadow; verify GPU pixels as well as keyboard behavior.
