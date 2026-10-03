---
title: Xvfb+lavapipe debug native renders ~10fps; front-loaded easing hides animations
date: 2026-10-02
confidence: medium
suggested_doc: filelink-openable-files
related_paths:
  - apps/native/src/ui/file_viewer.rs
promoted: false
---

# Xvfb+lavapipe debug native renders ~10fps; front-loaded easing hides animations

## Observation
Debug wks-native on Xvfb with lavapipe presents roughly one frame per ~95ms at 1400x900. The old viewer slide (180ms ease_out_quint, 36px offset + fade) reached ~97% in the second presented frame, so timed XGetImage captures showed closed->open with no intermediate. Its first frame also snapped the chat narrow while the panel was at opacity 0.

## Impact
Animation verification on the private rig needs longer, less front-loaded animations or geometry that changes per frame; absence of intermediate frames is not proof the animation code is wired.

## Recommendation
Capture timed frames with /tmp-style anim.py (XGetImage loop) and measure panel edge per frame; the viewer now grows its slot width 0->W over 240ms ease-out-cubic and a UI test asserts the opening frame is narrower than settled.
